//! Databases on the server: PostgreSQL, MySQL/MariaDB and MongoDB (list
//! databases with sizes and connections, back one up and download it), and
//! Redis (memory, keys, clients; flush and snapshot).
//!
//! Everything runs through the servers' own admin login: PostgreSQL as the
//! `postgres` system user, MySQL as root over its socket (the Debian/Ubuntu
//! default), MongoDB and Redis on localhost. Database names that go into a
//! command must match a strict pattern and are quoted. Redis's `requirepass`
//! is read on the server into an environment variable, never sent anywhere.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DbKind {
    Postgres,
    Mysql,
    Mongo,
}

impl DbKind {
    pub fn label(self) -> &'static str {
        match self {
            DbKind::Postgres => "PostgreSQL",
            DbKind::Mysql => "MySQL",
            DbKind::Mongo => "MongoDB",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DbInfo {
    pub name: String,
    pub size_bytes: Option<u64>,
    /// Open connections to this database (PostgreSQL, MySQL).
    pub connections: Option<u32>,
    /// Tables (SQL) or collections (MongoDB).
    pub tables: Option<u32>,
    /// Built-in database the server needs (`mysql`, `admin`, …).
    pub system: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DbServer {
    pub version: Option<String>,
    pub databases: Vec<DbInfo>,
    pub connections: Option<u32>,
    pub max_connections: Option<u32>,
    /// What the server said when it couldn't be read (needs a password, not running, …).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackupFormat {
    /// The tool's own format: `pg_dump -Fc` (.dump), MongoDB archive (.archive.gz).
    /// MySQL has none and uses SQL.
    Native,
    /// Plain SQL, gzipped (.sql.gz). Not for MongoDB.
    SqlGz,
}

static DB_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_$.\-]{0,63}$").unwrap());

pub fn validate_db_name(name: &str) -> Result<&str> {
    if DB_NAME_RE.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!("invalid database name: {name:?}")))
    }
}

// ---------- listing ----------

const PG_LIST: &str = r#"export LC_ALL=C; cd /tmp
command -v psql >/dev/null 2>&1 || { echo @@missing; exit 0; }
as_pg() { if command -v runuser >/dev/null 2>&1; then runuser -u postgres -- "$@"; else sudo -n -u postgres -- "$@"; fi; }
v=$(as_pg psql -XAtq -c 'show server_version' 2>&1) || { echo "@@error $v"; exit 0; }
echo "@@version $v"
echo "@@max $(as_pg psql -XAtq -c 'show max_connections' 2>/dev/null)"
echo "@@conns $(as_pg psql -XAtq -c 'select count(*) from pg_stat_activity where backend_type = $$client backend$$' 2>/dev/null)"
echo @@dbs
as_pg psql -XAtq -F '|' -c "select d.datname, pg_database_size(d.datname), (select count(*) from pg_stat_activity a where a.datname = d.datname) from pg_database d where not d.datistemplate order by 1" 2>&1
"#;

const MYSQL_LIST: &str = r#"export LC_ALL=C
M=$(command -v mysql || command -v mariadb) || { echo @@missing; exit 0; }
v=$("$M" -N -B -e 'select version()' 2>&1) || { echo "@@error $v"; exit 0; }
echo "@@version $v"
echo "@@max $("$M" -N -B -e 'select @@max_connections' 2>/dev/null)"
echo "@@conns $("$M" -N -B -e "show status like 'Threads_connected'" 2>/dev/null | cut -f2)"
echo @@dbs
"$M" -N -B -e "select s.schema_name, coalesce(sum(t.data_length + t.index_length), 0), count(t.table_name), (select count(*) from information_schema.processlist p where p.db = s.schema_name) from information_schema.schemata s left join information_schema.tables t on t.table_schema = s.schema_name group by s.schema_name order by 1" 2>&1 | tr '\t' '|'
"#;

const MONGO_LIST: &str = r#"export LC_ALL=C
M=$(command -v mongosh || command -v mongo) || { echo @@missing; exit 0; }
"$M" --quiet --eval '
  print("@@version " + db.version());
  try { var s = db.serverStatus(); print("@@conns " + s.connections.current); print("@@max " + (s.connections.current + s.connections.available)); } catch (e) {}
  print("@@dbs");
  db.adminCommand({ listDatabases: 1 }).databases.forEach(function (d) {
    var n = 0; try { n = db.getSiblingDB(d.name).getCollectionNames().length; } catch (e) {}
    print(d.name + "|" + d.sizeOnDisk + "|" + "|" + n);
  });' 2>&1 | sed '1{/^@@version/!s/^/@@error /}'
"#;

pub fn list_script(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => PG_LIST,
        DbKind::Mysql => MYSQL_LIST,
        DbKind::Mongo => MONGO_LIST,
    }
}

fn is_system(kind: DbKind, name: &str) -> bool {
    match kind {
        DbKind::Postgres => name == "postgres",
        DbKind::Mysql => matches!(name, "mysql" | "information_schema" | "performance_schema" | "sys"),
        DbKind::Mongo => matches!(name, "admin" | "config" | "local"),
    }
}

/// Parse a list script's output. `None` when the database isn't installed.
pub fn parse_list(kind: DbKind, out: &str) -> Option<DbServer> {
    let mut srv = DbServer::default();
    let mut in_dbs = false;
    let mut errors: Vec<&str> = Vec::new();
    for line in out.lines() {
        let l = line.trim_end();
        if l == "@@missing" {
            return None;
        }
        if let Some(e) = l.strip_prefix("@@error") {
            srv.error = Some(e.trim().to_string());
            continue;
        }
        if let Some(v) = l.strip_prefix("@@version") {
            srv.version = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            continue;
        }
        if let Some(v) = l.strip_prefix("@@max") {
            srv.max_connections = v.trim().parse().ok();
            continue;
        }
        if let Some(v) = l.strip_prefix("@@conns") {
            srv.connections = v.trim().parse().ok();
            continue;
        }
        if l == "@@dbs" {
            in_dbs = true;
            continue;
        }
        if !in_dbs || l.is_empty() {
            if srv.error.is_some() && !l.is_empty() {
                errors.push(l);
            }
            continue;
        }
        // name|size|tables-or-connections…: PG "name|size|conns", MySQL "name|size|tables|conns", Mongo "name|size||collections".
        let f: Vec<&str> = l.split('|').collect();
        if f.len() < 2 || f[0].is_empty() {
            continue;
        }
        let n = |i: usize| f.get(i).and_then(|v| v.trim().parse::<u64>().ok());
        let (tables, connections) = match kind {
            DbKind::Postgres => (None, n(2)),
            DbKind::Mysql => (n(2), n(3)),
            DbKind::Mongo => (n(3), None),
        };
        // Sizes can be printed as floats by the Mongo shell ("8192.0").
        let size = n(1).or_else(|| f[1].trim().parse::<f64>().ok().map(|x| x as u64));
        srv.databases.push(DbInfo {
            name: f[0].to_string(),
            size_bytes: size,
            connections: connections.map(|c| c as u32),
            tables: tables.map(|t| t as u32),
            system: is_system(kind, f[0]),
        });
    }
    if let Some(e) = srv.error.as_mut() {
        for extra in errors {
            e.push('\n');
            e.push_str(extra);
        }
    }
    Some(srv)
}

// ---------- backups ----------

/// File name suffix for a backup, so the Mac knows what it is.
pub fn backup_extension(kind: DbKind, format: BackupFormat) -> &'static str {
    match (kind, format) {
        (DbKind::Postgres, BackupFormat::Native) => "dump",
        (DbKind::Mongo, _) => "archive.gz",
        _ => "sql.gz",
    }
}

/// A root script that dumps one database into a private temp file, hands the
/// file to the login user (so SFTP can read it) and prints its path. A failed
/// dump leaves nothing behind and exits non-zero.
pub fn backup_script(kind: DbKind, name: &str, format: BackupFormat, login_user: &str) -> Result<String> {
    let db = quote(validate_db_name(name)?);
    let dump = match (kind, format) {
        (DbKind::Postgres, BackupFormat::Native) => format!("as_pg pg_dump -Fc -d {db} > \"$f\""),
        (DbKind::Postgres, BackupFormat::SqlGz) => {
            format!("( as_pg pg_dump -d {db} || echo x > \"$f.failed\" ) | gzip > \"$f\"")
        }
        (DbKind::Mysql, _) => format!(
            "D=$(command -v mysqldump || command -v mariadb-dump); \
             ( \"$D\" --single-transaction --quick --routines --triggers --events --databases {db} || echo x > \"$f.failed\" ) | gzip > \"$f\""
        ),
        (DbKind::Mongo, BackupFormat::Native) => format!("mongodump --quiet --archive=\"$f\" --gzip --db={db}"),
        (DbKind::Mongo, BackupFormat::SqlGz) => {
            return Err(Error::InvalidInput("MongoDB backups use its archive format".into()))
        }
    };
    Ok(format!(
        "set -e; export LC_ALL=C; cd /tmp; umask 077\n\
         as_pg() {{ if command -v runuser >/dev/null 2>&1; then runuser -u postgres -- \"$@\"; else sudo -n -u postgres -- \"$@\"; fi; }}\n\
         f=$(mktemp /tmp/.orbital-backup-XXXXXX)\n\
         trap 'rm -f -- \"$f.failed\"' EXIT\n\
         if ! {{ {dump}; }} || [ -e \"$f.failed\" ]; then rm -f -- \"$f\"; echo 'the dump failed' >&2; exit 3; fi\n\
         chown {user} \"$f\"\n\
         echo \"@@file $f\"",
        user = quote(login_user)
    ))
}

pub fn parse_backup_path(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.strip_prefix("@@file "))
        .map(str::trim)
        .filter(|p| p.starts_with("/tmp/.orbital-backup-") && !p.contains(['\'', '"', ' ', ';']))
        .map(String::from)
}

// ---------- Redis ----------

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisKeyspace {
    pub db: u32,
    pub keys: u64,
    pub expires: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedisInfo {
    pub version: Option<String>,
    pub role: Option<String>,
    pub uptime_secs: Option<u64>,
    pub connected_clients: Option<u64>,
    pub used_memory: Option<u64>,
    pub max_memory: Option<u64>,
    pub eviction_policy: Option<String>,
    pub hits: Option<u64>,
    pub misses: Option<u64>,
    pub last_save: Option<u64>,
    pub last_save_ok: Option<bool>,
    pub aof_enabled: Option<bool>,
    pub keyspace: Vec<RedisKeyspace>,
    pub error: Option<String>,
}

/// Runs `redis-cli` with the server's own password, if redis.conf sets one.
const REDIS_PRELUDE: &str = r#"export LC_ALL=C
command -v redis-cli >/dev/null 2>&1 || { echo @@missing; exit 0; }
for c in /etc/redis/redis.conf /etc/redis.conf /etc/valkey/valkey.conf; do
  [ -r "$c" ] && { pw=$(sed -n 's/^[[:space:]]*requirepass[[:space:]]\{1,\}//p' "$c" | head -1 | tr -d '"'); break; }
done
[ -n "$pw" ] && export REDISCLI_AUTH="$pw"
"#;

pub fn redis_info_script() -> String {
    format!("{REDIS_PRELUDE}redis-cli --no-auth-warning INFO 2>&1\n")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RedisAction {
    /// Write a snapshot to disk now (BGSAVE).
    Snapshot,
    /// Delete every key in one database.
    FlushDb(u32),
    /// Delete every key in every database.
    FlushAll,
}

pub fn redis_action_script(a: RedisAction) -> Result<String> {
    let cmd = match a {
        RedisAction::Snapshot => "BGSAVE".to_string(),
        RedisAction::FlushDb(n) if n < 1024 => format!("-n {n} FLUSHDB"),
        RedisAction::FlushDb(_) => return Err(Error::InvalidInput("no such Redis database".into())),
        RedisAction::FlushAll => "FLUSHALL".to_string(),
    };
    Ok(format!("{REDIS_PRELUDE}redis-cli --no-auth-warning {cmd} 2>&1\n"))
}

pub fn parse_redis_info(out: &str) -> Option<RedisInfo> {
    if out.lines().any(|l| l.trim() == "@@missing") {
        return None;
    }
    let mut kv: BTreeMap<&str, &str> = BTreeMap::new();
    let mut info = RedisInfo::default();
    for line in out.lines() {
        let l = line.trim();
        if let Some((k, v)) = l.split_once(':') {
            if let Some(n) = k.strip_prefix("db").and_then(|n| n.parse::<u32>().ok()) {
                let field = |name: &str| {
                    v.split(',')
                        .find_map(|p| p.strip_prefix(&format!("{name}=")))
                        .and_then(|x| x.parse().ok())
                        .unwrap_or(0)
                };
                info.keyspace.push(RedisKeyspace {
                    db: n,
                    keys: field("keys"),
                    expires: field("expires"),
                });
            } else {
                kv.insert(k, v);
            }
        }
    }
    if !kv.contains_key("redis_version") {
        let msg = out.trim();
        info.error = Some(if msg.is_empty() {
            "Redis didn't answer".into()
        } else {
            msg.to_string()
        });
        return Some(info);
    }
    let num = |k: &str| kv.get(k).and_then(|v| v.parse::<u64>().ok());
    info.version = kv.get("redis_version").or_else(|| kv.get("valkey_version")).map(|v| v.to_string());
    info.role = kv.get("role").map(|v| v.to_string());
    info.uptime_secs = num("uptime_in_seconds");
    info.connected_clients = num("connected_clients");
    info.used_memory = num("used_memory");
    info.max_memory = num("maxmemory").filter(|m| *m > 0);
    info.eviction_policy = kv.get("maxmemory_policy").map(|v| v.to_string());
    info.hits = num("keyspace_hits");
    info.misses = num("keyspace_misses");
    info.last_save = num("rdb_last_save_time");
    info.last_save_ok = kv.get("rdb_last_bgsave_status").map(|v| *v == "ok");
    info.aof_enabled = kv.get("aof_enabled").map(|v| *v == "1");
    Some(info)
}

impl SshSession {
    pub async fn list_databases(&self, kind: DbKind) -> Result<DbServer> {
        let (o, _) = self.exec_inspect(list_script(kind)).await?;
        parse_list(kind, &o.stdout).ok_or_else(|| Error::NotFound(format!("{} is not installed on this server", kind.label())))
    }

    pub async fn redis_info(&self) -> Result<RedisInfo> {
        let (o, _) = self.exec_inspect(&redis_info_script()).await?;
        parse_redis_info(&o.stdout).ok_or_else(|| Error::NotFound("Redis is not installed on this server".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_postgres_and_mysql_lists() {
        let pg = parse_list(
            DbKind::Postgres,
            "@@version 16.4 (Ubuntu 16.4-1)\n@@max 100\n@@conns 7\n@@dbs\npostgres|7660067|1\nshop|1048576000|5\n",
        )
        .unwrap();
        assert_eq!(pg.version.as_deref(), Some("16.4 (Ubuntu 16.4-1)"));
        assert_eq!((pg.connections, pg.max_connections), (Some(7), Some(100)));
        assert_eq!(pg.databases.len(), 2);
        assert!(pg.databases[0].system && !pg.databases[1].system);
        assert_eq!(
            (pg.databases[1].size_bytes, pg.databases[1].connections),
            (Some(1048576000), Some(5))
        );

        let my = parse_list(
            DbKind::Mysql,
            "@@version 8.0.39\n@@max 151\n@@conns 3\n@@dbs\nmysql|2637824|38|0\nwordpress|52428800|12|2\n",
        )
        .unwrap();
        let wp = &my.databases[1];
        assert_eq!((wp.tables, wp.connections, wp.system), (Some(12), Some(2), false));

        let mg = parse_list(
            DbKind::Mongo,
            "@@version 7.0.12\n@@conns 4\n@@max 838860\n@@dbs\nadmin|40960||1\napp|8192.0||3\n",
        )
        .unwrap();
        assert_eq!((mg.databases[1].size_bytes, mg.databases[1].tables), (Some(8192), Some(3)));
        assert!(mg.databases[0].system);
    }

    #[test]
    fn reports_errors_and_missing() {
        assert!(parse_list(DbKind::Mysql, "@@missing\n").is_none());
        let e = parse_list(
            DbKind::Mysql,
            "@@error ERROR 1045 (28000): Access denied for user 'root'@'localhost'\n",
        )
        .unwrap();
        assert!(e.error.unwrap().contains("Access denied") && e.databases.is_empty());
        let e = parse_list(
            DbKind::Mongo,
            "@@error MongoServerError: command listDatabases requires authentication\nmore detail\n",
        )
        .unwrap();
        assert!(e.error.unwrap().contains("requires authentication\nmore detail"));
    }

    #[test]
    fn backup_scripts_quote_and_validate() {
        let s = backup_script(DbKind::Postgres, "shop", BackupFormat::Native, "deploy").unwrap();
        assert!(s.contains("as_pg pg_dump -Fc -d shop > \"$f\""));
        assert!(s.contains("chown deploy \"$f\""));
        let s = backup_script(DbKind::Mysql, "wp-site", BackupFormat::SqlGz, "o'brien").unwrap();
        assert!(s.contains("--databases wp-site") && s.contains("chown 'o'\\''brien' \"$f\""));
        assert!(backup_script(DbKind::Mongo, "app", BackupFormat::SqlGz, "deploy").is_err());
        for bad in ["a b", "x;reboot", "$(id)", "-c", "", "../etc"] {
            assert!(
                backup_script(DbKind::Postgres, bad, BackupFormat::Native, "deploy").is_err(),
                "{bad}"
            );
        }
        assert_eq!(backup_extension(DbKind::Postgres, BackupFormat::Native), "dump");
        assert_eq!(backup_extension(DbKind::Mysql, BackupFormat::Native), "sql.gz");
        assert_eq!(
            parse_backup_path("noise\n@@file /tmp/.orbital-backup-Ab12Cd\n").as_deref(),
            Some("/tmp/.orbital-backup-Ab12Cd")
        );
        assert_eq!(parse_backup_path("@@file /etc/passwd\n"), None);
    }

    #[test]
    fn backup_script_is_valid_sh_and_cleans_up_on_failure() {
        // No database tools here: the dump fails, nothing is left in /tmp, exit is non-zero.
        let s = backup_script(DbKind::Mysql, "nothere", BackupFormat::SqlGz, &whoami())
            .unwrap()
            .replace("/tmp/.orbital-backup-", "/tmp/.orbital-test-backup-");
        let before = leftovers();
        let out = std::process::Command::new("sh").arg("-c").arg(&s).output().unwrap();
        assert!(!out.status.success());
        assert_eq!(leftovers(), before);
    }

    fn whoami() -> String {
        String::from_utf8(std::process::Command::new("id").arg("-un").output().unwrap().stdout)
            .unwrap()
            .trim()
            .into()
    }

    fn leftovers() -> usize {
        std::fs::read_dir("/tmp")
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with(".orbital-test-backup-"))
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn parses_redis_info() {
        let out = "# Server\r\nredis_version:7.2.4\r\nuptime_in_seconds:86400\r\n# Clients\r\nconnected_clients:12\r\n# Memory\r\nused_memory:1048576\r\nmaxmemory:268435456\r\nmaxmemory_policy:allkeys-lru\r\n# Persistence\r\nrdb_last_save_time:1763200000\r\nrdb_last_bgsave_status:ok\r\naof_enabled:0\r\n# Stats\r\nkeyspace_hits:900\r\nkeyspace_misses:100\r\n# Replication\r\nrole:master\r\n# Keyspace\r\ndb0:keys=1520,expires=300,avg_ttl=0\r\ndb2:keys=4,expires=0,avg_ttl=0\r\n";
        let r = parse_redis_info(out).unwrap();
        assert_eq!(r.version.as_deref(), Some("7.2.4"));
        assert_eq!(
            (r.connected_clients, r.used_memory, r.max_memory),
            (Some(12), Some(1048576), Some(268435456))
        );
        assert_eq!(
            r.keyspace,
            vec![
                RedisKeyspace {
                    db: 0,
                    keys: 1520,
                    expires: 300
                },
                RedisKeyspace {
                    db: 2,
                    keys: 4,
                    expires: 0
                }
            ]
        );
        assert_eq!((r.last_save_ok, r.aof_enabled), (Some(true), Some(false)));
        let e = parse_redis_info("NOAUTH Authentication required.\n").unwrap();
        assert!(e.error.unwrap().contains("NOAUTH"));
        assert!(parse_redis_info("@@missing\n").is_none());
        assert!(redis_action_script(RedisAction::FlushDb(5000)).is_err());
        assert!(redis_action_script(RedisAction::FlushDb(2))
            .unwrap()
            .contains("redis-cli --no-auth-warning -n 2 FLUSHDB"));
    }

    #[test]
    fn scripts_run_under_sh() {
        for script in [PG_LIST, MYSQL_LIST, MONGO_LIST] {
            let out = std::process::Command::new("sh").arg("-c").arg(script).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        }
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(redis_info_script())
            .output()
            .unwrap();
        assert!(out.status.success());
    }
}
