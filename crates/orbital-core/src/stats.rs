//! Anonymous usage statistics, only after the user said yes.
//!
//! What is sent is fixed here, not chosen by the UI: the event ("opened" or
//! "active", the latter every 30 minutes while Orbital is open), the app
//! version, the OS and processor type, and two numbers: servers connected now
//! and servers saved. A random install id (made on this computer, tied to
//! nothing) lets the counts say "how many people". Never host names,
//! addresses, user names, files, commands or anything typed.
//!
//! It goes to PostHog (EU) with GeoIP turned off. Builds without the
//! `ORBITAL_POSTHOG_KEY` build variable (development, forks) send nothing.

use crate::db::Database;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The project key, set at build time by the release workflow (an unset
/// GitHub variable arrives as an empty string, which means none).
pub fn key() -> Option<&'static str> {
    option_env!("ORBITAL_POSTHOG_KEY").map(str::trim).filter(|k| !k.is_empty())
}
pub const HOST: &str = match option_env!("ORBITAL_POSTHOG_HOST") {
    Some(h) => h,
    None => "https://eu.i.posthog.com",
};
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
        available: key().is_some(),
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

/// One id per run of the app, so PostHog can group a sitting.
fn session_id() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// The request body. Everything that leaves the computer is in this function.
pub fn payload(key: &str, install_id: &str, session: &str, event: StatsEvent, c: Counts, now: &str) -> serde_json::Value {
    let name = match event {
        StatsEvent::Opened => "app_opened",
        StatsEvent::Active => "app_active",
    };
    let mut props = serde_json::json!({
        "distinct_id": install_id,
        "$session_id": session,
        "$geoip_disable": true,
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "servers_connected": c.servers_connected,
        "servers_saved": c.servers_saved,
    });
    if event == StatsEvent::Active {
        props["minutes"] = 30.into();
    }
    serde_json::json!({
        "api_key": key,
        "batch": [{ "event": name, "timestamp": now, "properties": props }],
    })
}

/// Send one event if this build has a key and the user said yes. Never fails
/// the caller: no network, an outage or a refusal are all ignored.
pub async fn send(db: &Database, event: StatsEvent, c: Counts) {
    let Some(key) = key() else { return };
    if state(db).ok().and_then(|s| s.enabled) != Some(true) {
        return;
    }
    let Ok(id) = install_id(db) else { return };
    let now = chrono_now();
    let body = payload(key, &id, session_id(), event, c, &now);
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() else {
        return;
    };
    let _ = client
        .post(format!("{}/batch/", HOST.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await;
}

/// RFC 3339 UTC without a date crate.
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_only_the_fixed_fields() {
        let p = payload(
            "phc_x",
            "install-1",
            "sess-1",
            StatsEvent::Active,
            Counts {
                servers_connected: 2,
                servers_saved: 7,
            },
            "2026-11-21T10:00:00Z",
        );
        assert_eq!(p["api_key"], "phc_x");
        let e = &p["batch"][0];
        assert_eq!(e["event"], "app_active");
        let props = e["properties"].as_object().unwrap();
        let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "$geoip_disable",
                "$session_id",
                "app_version",
                "arch",
                "distinct_id",
                "minutes",
                "os",
                "servers_connected",
                "servers_saved"
            ]
        );
        assert_eq!(props["servers_connected"], 2);
        assert_eq!(props["$geoip_disable"], true);
        let opened = payload("k", "i", "s", StatsEvent::Opened, Counts::default(), "t");
        assert_eq!(opened["batch"][0]["event"], "app_opened");
        assert!(opened["batch"][0]["properties"].get("minutes").is_none());
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
        // Test builds have no key: `send` returns at once without touching the network.
        assert!(key().is_none());
        set_enabled(&db, true).unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(send(&db, StatsEvent::Opened, Counts::default()));
    }

    #[test]
    fn utc_timestamp() {
        let t = chrono_now();
        assert_eq!(t.len(), 20);
        assert!(t.starts_with("20") && t.ends_with('Z') && &t[10..11] == "T", "{t}");
    }
}
