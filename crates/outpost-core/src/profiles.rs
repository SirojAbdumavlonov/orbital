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
}

impl ProfileInput {
    fn validate(&self) -> Result<()> {
        if self.host.trim().is_empty() || self.host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(Error::InvalidInput("host is required and must not contain spaces".into()));
        }
        if self.username.trim().is_empty() {
            return Err(Error::InvalidInput("username is required".into()));
        }
        if self.auth_method == AuthMethod::Key && self.key_path.as_deref().unwrap_or("").is_empty() {
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

pub fn create(db: &Database, input: ProfileInput) -> Result<Profile> {
    input.validate()?;
    let id = uuid::Uuid::new_v4().to_string();
    db.conn().execute(
        "INSERT INTO profiles (id, name, host, port, username, auth_method, key_path, sudo_mode, group_name, color, initial_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            if input.name.trim().is_empty() { input.host.clone() } else { input.name.clone() },
            input.host.trim(),
            input.port.unwrap_or(22),
            input.username.trim(),
            enum_str(&input.auth_method),
            input.key_path,
            enum_str(&input.sudo_mode.unwrap_or(SudoMode::Auto)),
            input.group_name,
            input.color,
            input.initial_path,
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    get(db, &id)
}

pub fn update(db: &Database, id: &str, input: ProfileInput) -> Result<Profile> {
    input.validate()?;
    let n = db.conn().execute(
        "UPDATE profiles SET name=?2, host=?3, port=?4, username=?5, auth_method=?6, key_path=?7, sudo_mode=?8,
         group_name=?9, color=?10, initial_path=?11 WHERE id=?1",
        params![
            id,
            input.name,
            input.host.trim(),
            input.port.unwrap_or(22),
            input.username.trim(),
            enum_str(&input.auth_method),
            input.key_path,
            enum_str(&input.sudo_mode.unwrap_or(SudoMode::Auto)),
            input.group_name,
            input.color,
            input.initial_path,
        ],
    )?;
    if n == 0 {
        return Err(Error::NotFound(format!("profile {id}")));
    }
    get(db, id)
}

pub fn delete(db: &Database, id: &str) -> Result<()> {
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
        }
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
