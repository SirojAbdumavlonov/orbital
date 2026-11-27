//! Local SQLite store: connection profiles (no secrets), pinned host keys,
//! the audit log and app settings. Secrets live in the OS keychain only.

use crate::error::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub struct Database {
    conn: Mutex<Connection>,
}

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE profiles (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        host TEXT NOT NULL,
        port INTEGER NOT NULL DEFAULT 22,
        username TEXT NOT NULL,
        auth_method TEXT NOT NULL,
        key_path TEXT,
        sudo_mode TEXT NOT NULL DEFAULT 'auto',
        group_name TEXT,
        color TEXT,
        initial_path TEXT,
        created_at TEXT NOT NULL,
        last_connected_at TEXT
    );
    CREATE TABLE known_hosts (
        host TEXT NOT NULL,
        port INTEGER NOT NULL,
        algorithm TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        public_key TEXT NOT NULL,
        added_at TEXT NOT NULL,
        PRIMARY KEY (host, port, algorithm)
    );
    CREATE TABLE audit_log (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        ts TEXT NOT NULL,
        profile_id TEXT,
        server TEXT,
        actor TEXT NOT NULL,
        category TEXT NOT NULL,
        action TEXT NOT NULL,
        target TEXT,
        command TEXT,
        outcome TEXT NOT NULL,
        exit_code INTEGER,
        duration_ms INTEGER,
        details TEXT,
        prev_hash TEXT NOT NULL,
        hash TEXT NOT NULL
    );
    CREATE INDEX audit_profile_idx ON audit_log(profile_id, seq);
    CREATE INDEX audit_category_idx ON audit_log(category, seq);
    -- The audit log is append-only: updates and deletes are rejected at the DB level.
    CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_log
        BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
    CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_log
        BEGIN SELECT RAISE(ABORT, 'audit log is append-only'); END;
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    "#,
    // v2: jump hosts (ProxyJump). A profile may be reached through another profile.
    r#"
    ALTER TABLE profiles ADD COLUMN jump_profile_id TEXT;
    "#,
    // v3: apps deployed with the Deploy tab, and every deploy and rollback of them.
    r#"
    CREATE TABLE apps (
        id TEXT PRIMARY KEY,
        profile_id TEXT NOT NULL,
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        branch TEXT NOT NULL,
        build TEXT NOT NULL DEFAULT '',
        restart_kind TEXT NOT NULL DEFAULT 'none',
        restart_target TEXT,
        created_at TEXT NOT NULL
    );
    CREATE TABLE deploys (
        id TEXT PRIMARY KEY,
        app_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        started_at TEXT NOT NULL,
        finished_at TEXT,
        from_commit TEXT,
        to_commit TEXT,
        outcome TEXT NOT NULL,
        summary TEXT,
        log TEXT
    );
    CREATE INDEX deploys_app_idx ON deploys(app_id, started_at);
    "#,
    // v4: other kinds of file connections (FTP, WebDAV, S3, …) besides SSH/SFTP.
    r#"
    ALTER TABLE profiles ADD COLUMN protocol TEXT NOT NULL DEFAULT 'sftp';
    ALTER TABLE profiles ADD COLUMN region TEXT;
    ALTER TABLE profiles ADD COLUMN path_style INTEGER NOT NULL DEFAULT 0;
    "#,
    // v5: DNS provider accounts (their API tokens are in the vault, not here).
    r#"
    CREATE TABLE dns_accounts (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    "#,
    // v6: uptime checks run from this Mac, their recent results and up/down changes.
    r#"
    CREATE TABLE uptime_checks (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        target TEXT NOT NULL,
        expected_status INTEGER NOT NULL DEFAULT 200,
        contains TEXT,
        interval_minutes INTEGER NOT NULL DEFAULT 5,
        paused INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
    );
    CREATE TABLE uptime_results (
        check_id TEXT NOT NULL,
        at TEXT NOT NULL,
        up INTEGER NOT NULL,
        latency_ms INTEGER,
        status_code INTEGER,
        error TEXT,
        cert_expires TEXT,
        warning TEXT
    );
    CREATE INDEX uptime_results_idx ON uptime_results(check_id, at);
    CREATE TABLE uptime_changes (
        check_id TEXT NOT NULL,
        at TEXT NOT NULL,
        up INTEGER NOT NULL,
        error TEXT
    );
    CREATE INDEX uptime_changes_idx ON uptime_changes(check_id, at);
    "#,
    // v7: snippets, saved commands to run on one or many servers.
    r#"
    CREATE TABLE snippets (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        command TEXT NOT NULL,
        description TEXT,
        sudo INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    "#,
    // v8: where alerts go besides Orbital's window (Telegram, email, the AI agent).
    // Bot tokens and SMTP passwords are in the vault, never here.
    r#"
    CREATE TABLE notify_channels (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        config TEXT NOT NULL,
        events TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        created_at TEXT NOT NULL
    );
    "#,
];

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(&format!("BEGIN; {sql} COMMIT;"))?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}
