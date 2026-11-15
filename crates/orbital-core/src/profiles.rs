//! Connection profiles ("bookmarks" in Cyberduck terms). Contain no secrets.

use crate::db::Database;
use crate::error::{Error, Result};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthMethod {
    /// Password (with keyboard-interactive fallback), stored in the Keychain.
    Password,
    /// Private key file (OpenSSH / PEM); optional passphrase in the Keychain.
    Key,
    /// ssh-agent (incl. 1Password / Secretive / Yubikey agents).
    Agent,
}

/// How Orbital reaches the server. SFTP is the full experience (files plus
/// services, terminal, …, over SSH); the others are file access only.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    #[default]
    Sftp,
    Ftp,
    /// FTP with explicit TLS (AUTH TLS).
    Ftps,
    /// WebDAV over plain HTTP.
    Webdav,
    /// WebDAV over HTTPS (Nextcloud, ownCloud, Box, …).
    Webdavs,
    /// Amazon S3 or any S3-compatible storage (Backblaze B2, Google Cloud Storage, R2, Wasabi, MinIO, …).
    S3,
}

impl Protocol {
    pub fn is_ssh(self) -> bool {
        self == Protocol::Sftp
    }
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::Webdav => 80,
            Protocol::Webdavs | Protocol::S3 => 443,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SudoMode {
    /// root needs no sudo; others use `sudo -n` (passwordless).
    Auto,
    /// Never elevate.
    Never,
    /// `sudo -n` (NOPASSWD rules).
    NoPassword,
    /// `sudo -S` with a password from the Keychain.
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub key_path: Option<String>,
    pub sudo_mode: SudoMode,
    pub group_name: Option<String>,
    pub color: Option<String>,
    pub initial_path: Option<String>,
    /// Reach this server through another profile (ProxyJump / bastion).
    #[serde(default)]
    pub jump_profile_id: Option<String>,
    #[serde(default)]
    pub protocol: Protocol,
    /// S3: the region to sign requests for (e.g. `us-east-1`, `auto` for R2).
    #[serde(default)]
    pub region: Option<String>,
    /// S3: address buckets as `host/bucket` instead of `bucket.host` (MinIO, some providers).
    #[serde(default)]
    pub path_style: bool,
    pub created_at: String,
    pub last_connected_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth_method: AuthMethod,
    pub key_path: Option<String>,
    pub sudo_mode: Option<SudoMode>,
    pub group_name: Option<String>,
    pub color: Option<String>,
    pub initial_path: Option<String>,
    #[serde(default)]
    pub jump_profile_id: Option<String>,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub path_style: bool,
}

/// Longest allowed route: target plus up to three jump hosts.
pub const MAX_HOPS: usize = 4;

impl ProfileInput {
    fn validate(&self) -> Result<()> {
        if self.host.trim().is_empty() || self.host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(Error::InvalidInput("host is required and must not contain spaces".into()));
        }
        if self.username.trim().is_empty() {
            return Err(Error::InvalidInput(if self.protocol == Protocol::S3 {
                "the access key ID is required".into()
            } else {
                "username is required".into()
            }));
        }
        if !self.protocol.is_ssh() && self.jump_profile_id.as_deref().is_some_and(|j| !j.is_empty()) {
            return Err(Error::InvalidInput("jump hosts work only with SFTP connections".into()));
        }
        if self.protocol.is_ssh() && self.auth_method == AuthMethod::Key && self.key_path.as_deref().unwrap_or("").is_empty() {
            return Err(Error::InvalidInput("key authentication needs a private key path".into()));
        }
        Ok(())
    }
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

fn enum_parse<T: for<'de> Deserialize<'de>>(s: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(s.clone()))
        .map_err(|_| rusqlite::Error::InvalidColumnType(0, s, rusqlite::types::Type::Text))
}

fn from_row(r: &Row) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: r.get("id")?,
        name: r.get("name")?,
        host: r.get("host")?,
        port: r.get::<_, i64>("port")? as u16,
        username: r.get("username")?,
        auth_method: enum_parse(r.get("auth_method")?)?,
        key_path: r.get("key_path")?,
        sudo_mode: enum_parse(r.get("sudo_mode")?)?,
        group_name: r.get("group_name")?,
        color: r.get("color")?,
        initial_path: r.get("initial_path")?,
        jump_profile_id: r.get("jump_profile_id")?,
        protocol: enum_parse(r.get("protocol")?).unwrap_or_default(),
        region: r.get("region")?,
        path_style: r.get::<_, i64>("path_style")? != 0,
        created_at: r.get("created_at")?,
        last_connected_at: r.get("last_connected_at")?,
    })
}

pub fn list(db: &Database) -> Result<Vec<Profile>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM profiles ORDER BY COALESCE(group_name, ''), name COLLATE NOCASE")?;
    let rows = st.query_map([], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get(db: &Database, id: &str) -> Result<Profile> {
    db.conn()
        .query_row("SELECT * FROM profiles WHERE id = ?1", [id], from_row)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("profile {id}")))
}

/// Resolve the route to `id`: jump hosts first (outermost bastion at index 0),
/// the target itself last. Fails on a missing jump profile, a loop, or a route
/// longer than [`MAX_HOPS`] — never falls back to a direct connection.
pub fn chain(db: &Database, id: &str) -> Result<Vec<Profile>> {
    let mut route = vec![get(db, id)?];
    while let Some(jump_id) = route.last().and_then(|p| p.jump_profile_id.clone()) {
        if route.iter().any(|p| p.id == jump_id) {
            return Err(Error::InvalidInput("jump hosts form a loop".into()));
        }
        if route.len() >= MAX_HOPS {
            return Err(Error::InvalidInput(format!("too many jump hosts (at most {})", MAX_HOPS - 1)));
        }
        let jump = get(db, &jump_id).map_err(|_| {
            Error::InvalidInput(format!(
                "the jump host configured for {} no longer exists",
                route.last().map(|p| p.name.as_str()).unwrap_or("this server")
            ))
        })?;
        route.push(jump);
    }
    route.reverse();
    Ok(route)
}

/// Validate that `jump_id` is usable as the jump host of profile `id`
/// (`None` while the profile is being created).
fn check_jump(db: &Database, id: Option<&str>, jump_id: &str) -> Result<()> {
    if id == Some(jump_id) {
        return Err(Error::InvalidInput("a server can't be its own jump host".into()));
    }
    let route = chain(db, jump_id).map_err(|e| match e {
        Error::NotFound(_) => Error::InvalidInput("jump host not found".into()),
        e => e,
    })?;
    if let Some(id) = id {
        if route.iter().any(|p| p.id == id) {
            return Err(Error::InvalidInput("jump hosts form a loop".into()));
        }
    }
    if route.len() + 1 > MAX_HOPS {
        return Err(Error::InvalidInput(format!("too many jump hosts (at most {})", MAX_HOPS - 1)));
    }
    Ok(())
}

/// Names of profiles that connect through `id`.
pub fn dependents(db: &Database, id: &str) -> Result<Vec<String>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT name FROM profiles WHERE jump_profile_id = ?1 ORDER BY name COLLATE NOCASE")?;
    let rows = st.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

pub fn create(db: &Database, input: ProfileInput) -> Result<Profile> {
    input.validate()?;
    let jump = input.jump_profile_id.clone().filter(|j| !j.is_empty());
    if let Some(j) = &jump {
        check_jump(db, None, j)?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.conn().execute(
        "INSERT INTO profiles (id, name, host, port, username, auth_method, key_path, sudo_mode, group_name, color, initial_path, created_at, jump_profile_id, protocol, region, path_style)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            id,
            if input.name.trim().is_empty() { input.host.clone() } else { input.name.clone() },
            input.host.trim(),
            input.port.unwrap_or(input.protocol.default_port()),
            input.username.trim(),
            enum_str(&input.auth_method),
            input.key_path,
            enum_str(&input.sudo_mode.unwrap_or(SudoMode::Auto)),
            input.group_name,
            input.color,
            input.initial_path,
            chrono::Utc::now().to_rfc3339(),
            jump,
            enum_str(&input.protocol),
            input.region,
            input.path_style as i64,
        ],
    )?;
    get(db, &id)
}

pub fn update(db: &Database, id: &str, input: ProfileInput) -> Result<Profile> {
    input.validate()?;
    let jump = input.jump_profile_id.clone().filter(|j| !j.is_empty());
    if let Some(j) = &jump {
        check_jump(db, Some(id), j)?;
    }
    let n = db.conn().execute(
        "UPDATE profiles SET name=?2, host=?3, port=?4, username=?5, auth_method=?6, key_path=?7, sudo_mode=?8,
         group_name=?9, color=?10, initial_path=?11, jump_profile_id=?12, protocol=?13, region=?14, path_style=?15 WHERE id=?1",
        params![
            id,
            input.name,
            input.host.trim(),
            input.port.unwrap_or(input.protocol.default_port()),
            input.username.trim(),
            enum_str(&input.auth_method),
            input.key_path,
            enum_str(&input.sudo_mode.unwrap_or(SudoMode::Auto)),
            input.group_name,
            input.color,
            input.initial_path,
            jump,
            enum_str(&input.protocol),
            input.region,
            input.path_style as i64,
        ],
    )?;
    if n == 0 {
        return Err(Error::NotFound(format!("profile {id}")));
    }
    get(db, id)
}

pub fn delete(db: &Database, id: &str) -> Result<()> {
    // Refuse rather than orphan: a server that silently lost its bastion would
    // otherwise be dialled directly.
    let used_by = dependents(db, id)?;
    if !used_by.is_empty() {
        return Err(Error::InvalidInput(format!(
            "this server is the jump host for {}; change those servers first",
            used_by.join(", ")
        )));
    }
    db.conn().execute("DELETE FROM profiles WHERE id = ?1", [id])?;
    Ok(())
}

pub fn touch(db: &Database, id: &str) -> Result<()> {
    db.conn().execute(
        "UPDATE profiles SET last_connected_at = ?2 WHERE id = ?1",
        params![id, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ProfileInput {
        ProfileInput {
            name: "prod-web".into(),
            host: "10.0.0.5".into(),
            port: None,
            username: "deploy".into(),
            auth_method: AuthMethod::Agent,
            key_path: None,
            sudo_mode: Some(SudoMode::Password),
            group_name: Some("Production".into()),
            color: Some("#ef4444".into()),
            initial_path: None,
            jump_profile_id: None,
            protocol: Default::default(),
            region: None,
            path_style: false,
        }
    }

    fn via(jump: &str) -> ProfileInput {
        ProfileInput {
            jump_profile_id: Some(jump.into()),
            ..input()
        }
    }

    #[test]
    fn jump_chain_order_loops_and_limits() {
        let db = Database::in_memory().unwrap();
        let bastion = create(&db, input()).unwrap();
        let web = create(&db, via(&bastion.id)).unwrap();
        let route = chain(&db, &web.id).unwrap();
        assert_eq!(
            route.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            [bastion.id.as_str(), web.id.as_str()]
        );
        assert_eq!(chain(&db, &bastion.id).unwrap().len(), 1);

        // Self reference, loops and unknown jump hosts are rejected.
        assert!(update(&db, &bastion.id, via(&bastion.id)).is_err());
        assert!(update(&db, &bastion.id, via(&web.id)).is_err());
        assert!(create(&db, via("nope")).is_err());

        // At most MAX_HOPS profiles on a route.
        let c = create(&db, via(&web.id)).unwrap();
        let d = create(&db, via(&c.id)).unwrap();
        assert_eq!(chain(&db, &d.id).unwrap().len(), MAX_HOPS);
        assert!(create(&db, via(&d.id)).is_err());
        // Re-pointing an existing profile must respect the limit too.
        let lone = create(&db, input()).unwrap();
        assert!(update(&db, &lone.id, via(&d.id)).is_err());

        // A jump host in use can't be deleted (no silent direct connections).
        assert!(delete(&db, &bastion.id).is_err());
        delete(&db, &d.id).unwrap();

        // A loop planted directly in the DB is caught at resolve time.
        db.conn()
            .execute("UPDATE profiles SET jump_profile_id = ?1 WHERE id = ?2", params![c.id, bastion.id])
            .unwrap();
        assert!(chain(&db, &c.id).is_err());
        // ... and so is a dangling reference: no fallback to a direct route.
        db.conn()
            .execute("UPDATE profiles SET jump_profile_id = 'gone' WHERE id = ?1", params![bastion.id])
            .unwrap();
        assert!(chain(&db, &web.id).is_err());
    }

    #[test]
    fn crud() {
        let db = Database::in_memory().unwrap();
        let p = create(&db, input()).unwrap();
        assert_eq!(p.port, 22);
        assert_eq!(p.sudo_mode, SudoMode::Password);
        let mut i = input();
        i.port = Some(2222);
        let p2 = update(&db, &p.id, i).unwrap();
        assert_eq!(p2.port, 2222);
        assert_eq!(list(&db).unwrap().len(), 1);
        delete(&db, &p.id).unwrap();
        assert!(get(&db, &p.id).is_err());
    }

    #[test]
    fn rejects_bad_host() {
        let db = Database::in_memory().unwrap();
        let mut i = input();
        i.host = "evil host".into();
        assert!(create(&db, i).is_err());
    }
}
