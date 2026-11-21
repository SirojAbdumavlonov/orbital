//! Anonymous usage statistics, only after the user said yes.
//!
//! What is sent is fixed here, not chosen by the UI: the event ("opened" or
//! "active", the latter every 30 minutes while Orbital is open), the app
//! version, the OS and processor type, and two numbers: servers connected now
//! and servers saved. A random install id (made on this computer, tied to
//! nothing) lets the counts say "how many people". Never host names,
//! addresses, user names, files, commands or anything typed.
//!
//! It goes to the project's own statistics server (`stats-server/` in this
//! repository: one Python file and SQLite, no outside service), whose
//! address is the `ORBITAL_STATS_URL` build variable. Builds without it
//! (development, forks) send nothing and never ask.

use crate::db::Database;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The statistics server, set at build time by the release workflow (an
/// unset GitHub variable arrives as an empty string, which means none). Only
/// https:// addresses count.
pub fn url() -> Option<&'static str> {
    option_env!("ORBITAL_STATS_URL")
        .map(|u| u.trim().trim_end_matches('/'))
        .filter(|u| u.starts_with("https://") && u.len() > "https://".len())
}
const SETTING: &str = "stats.enabled";
const ID: &str = "stats.install_id";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StatsEvent {
    /// Orbital started.
    Opened,
    /// Still open (sent every 30 minutes): gives how long Orbital is used.
    Active,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsState {
    /// This build can send statistics (it has a key).
    pub available: bool,
    /// The user's answer; None until asked.
    pub enabled: Option<bool>,
}

/// The only numbers sent besides version and OS.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    pub servers_connected: usize,
    pub servers_saved: usize,
}

pub fn state(db: &Database) -> Result<StatsState> {
    Ok(StatsState {
        available: url().is_some(),
        enabled: db.get_setting(SETTING)?.map(|v| v == "1"),
    })
}

pub fn set_enabled(db: &Database, on: bool) -> Result<()> {
    db.set_setting(SETTING, if on { "1" } else { "0" })
}

fn install_id(db: &Database) -> Result<String> {
    if let Some(id) = db.get_setting(ID)? {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.set_setting(ID, &id)?;
    Ok(id)
}

/// One id per run of the app, so a sitting can be told apart.
fn session_id() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// The request body. Everything that leaves the computer is in this function.
pub fn payload(install_id: &str, session: &str, event: StatsEvent, c: Counts) -> serde_json::Value {
    serde_json::json!({
        "install": install_id,
        "session": session,
        "event": match event {
            StatsEvent::Opened => "opened",
            StatsEvent::Active => "active",
        },
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "connected": c.servers_connected,
        "saved": c.servers_saved,
    })
}

/// Send one event if this build has a key and the user said yes. Never fails
/// the caller: no network, an outage or a refusal are all ignored.
pub async fn send(db: &Database, event: StatsEvent, c: Counts) {
    let Some(base) = url() else { return };
    if state(db).ok().and_then(|s| s.enabled) != Some(true) {
        return;
    }
    let Ok(id) = install_id(db) else { return };
    let body = payload(&id, session_id(), event, c);
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() else {
        return;
    };
    let _ = client
        .post(format!("{base}/v1/events"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_only_the_fixed_fields() {
        let p = payload(
            "11111111-1111-4111-8111-111111111111",
            "s",
            StatsEvent::Active,
            Counts {
                servers_connected: 2,
                servers_saved: 7,
            },
        );
        let mut keys: Vec<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["arch", "connected", "event", "install", "os", "saved", "session", "version"]);
        assert_eq!(
            (p["event"].as_str(), p["connected"].as_u64(), p["saved"].as_u64()),
            (Some("active"), Some(2), Some(7))
        );
        assert_eq!(payload("i", "s", StatsEvent::Opened, Counts::default())["event"], "opened");
    }

    /// The server in stats-server/ accepts exactly what `payload` sends (its own selftest, then this payload).
    #[cfg(unix)]
    #[test]
    fn the_stats_server_accepts_the_payload() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stats-server/orbital_stats.py");
        let out = match std::process::Command::new("python3").arg("-I").arg(file).arg("selftest").output() {
            Ok(o) => o,
            Err(_) => return, // no python3 here
        };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let body = payload(
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            StatsEvent::Active,
            Counts {
                servers_connected: 1,
                servers_saved: 4,
            },
        )
        .to_string();
        let check = format!(
            "import sys; sys.path.insert(0, {dir:?}); import orbital_stats as s; e = s.clean({body:?}.encode()); assert e and e['saved'] == 4, e",
            dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stats-server"),
        );
        let out = std::process::Command::new("python3").arg("-c").arg(check).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn answer_and_id_are_stored_and_nothing_is_sent_without_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(state(&db).unwrap().enabled, None);
        set_enabled(&db, true).unwrap();
        assert_eq!(state(&db).unwrap().enabled, Some(true));
        set_enabled(&db, false).unwrap();
        assert_eq!(state(&db).unwrap().enabled, Some(false));
        let a = install_id(&db).unwrap();
        assert_eq!(a, install_id(&db).unwrap());
        assert_eq!(a.len(), 36);
        // Test builds have no server address: `send` returns at once without touching the network.
        assert!(url().is_none());
        set_enabled(&db, true).unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(send(&db, StatsEvent::Opened, Counts::default()));
    }
}
