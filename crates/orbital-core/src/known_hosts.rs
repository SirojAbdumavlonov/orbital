//! Host-key verification (trust on first use with explicit confirmation,
//! hard failure on change) — the same model as Cyberduck and OpenSSH.
//! Keys already present in `~/.ssh/known_hosts` are trusted automatically.

use crate::db::Database;
use crate::error::Result;
use rusqlite::{params, OptionalExtension};
use russh::keys::{HashAlg, PublicKey};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    Unknown,
    Mismatch { expected: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub added_at: String,
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

pub fn algorithm(key: &PublicKey) -> String {
    key.algorithm().as_str().to_string()
}

pub fn check(db: &Database, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyStatus> {
    let fp = fingerprint(key);
    let stored: Option<String> = db
        .conn()
        .query_row(
            "SELECT fingerprint FROM known_hosts WHERE host=?1 AND port=?2 AND algorithm=?3",
            params![host, port, algorithm(key)],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(expected) = stored {
        return Ok(if expected == fp {
            HostKeyStatus::Trusted
        } else {
            HostKeyStatus::Mismatch { expected }
        });
    }
    // Fall back to the user's OpenSSH known_hosts.
    if let Some(home) = std::env::var_os("HOME") {
        let path = std::path::Path::new(&home).join(".ssh/known_hosts");
        if path.exists() {
            match russh::keys::check_known_hosts_path(host, port, key, &path) {
                Ok(true) => return Ok(HostKeyStatus::Trusted),
                Ok(false) => {}
                Err(russh::keys::Error::KeyChanged { line }) => {
                    return Ok(HostKeyStatus::Mismatch {
                        expected: format!("key in ~/.ssh/known_hosts line {line}"),
                    })
                }
                Err(_) => {}
            }
        }
    }
    Ok(HostKeyStatus::Unknown)
}

pub fn trust(db: &Database, host: &str, port: u16, key: &PublicKey) -> Result<()> {
    db.conn().execute(
        "INSERT INTO known_hosts (host, port, algorithm, fingerprint, public_key, added_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(host, port, algorithm) DO UPDATE SET fingerprint=excluded.fingerprint, public_key=excluded.public_key, added_at=excluded.added_at",
        params![
            host,
            port,
            algorithm(key),
            fingerprint(key),
            key.to_openssh().unwrap_or_default(),
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn list(db: &Database) -> Result<Vec<KnownHost>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT host, port, algorithm, fingerprint, added_at FROM known_hosts ORDER BY host, port")?;
    let rows = st
        .query_map([], |r| {
            Ok(KnownHost {
                host: r.get(0)?,
                port: r.get::<_, i64>(1)? as u16,
                algorithm: r.get(2)?,
                fingerprint: r.get(3)?,
                added_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn forget(db: &Database, host: &str, port: u16) -> Result<()> {
    db.conn()
        .execute("DELETE FROM known_hosts WHERE host=?1 AND port=?2", params![host, port])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    const K2: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA6rWI3G1sz07Qwq0Q8Gq0nIDf/9mkf7CdCpUbvwD1N5";

    #[test]
    fn tofu_and_mismatch() {
        std::env::set_var("HOME", "/nonexistent-home-for-test");
        let db = Database::in_memory().unwrap();
        let k1 = PublicKey::from_openssh(K1).unwrap();
        let k2 = PublicKey::from_openssh(K2).unwrap();
        assert_eq!(check(&db, "h", 22, &k1).unwrap(), HostKeyStatus::Unknown);
        trust(&db, "h", 22, &k1).unwrap();
        assert_eq!(check(&db, "h", 22, &k1).unwrap(), HostKeyStatus::Trusted);
        assert!(matches!(check(&db, "h", 22, &k2).unwrap(), HostKeyStatus::Mismatch { .. }));
        // Different port is a different identity.
        assert_eq!(check(&db, "h", 2222, &k1).unwrap(), HostKeyStatus::Unknown);
        assert!(fingerprint(&k1).starts_with("SHA256:"));
    }
}
