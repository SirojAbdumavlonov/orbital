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
use crate::error::{Error, Result};
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

/// What someone writes on the Feedback page. Sent only when they click Send,
/// whatever they answered about statistics, to the same server.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feedback {
    /// "problem", "idea", "praise" or "other".
    pub kind: String,
    /// The part of Orbital it's about ("Docker", "1.4.0: Test a website"), or empty.
    #[serde(default)]
    pub about: String,
    pub message: String,
    /// How to answer (email, Telegram), only if they want an answer.
    #[serde(default)]
    pub contact: String,
}

pub const FEEDBACK_KINDS: [&str; 4] = ["problem", "idea", "praise", "other"];
pub const MAX_MESSAGE: usize = 4000;
pub const MAX_ABOUT: usize = 80;
pub const MAX_CONTACT: usize = 200;

/// Whether this build has somewhere to send feedback.
pub fn feedback_available() -> bool {
    url().is_some()
}

fn typed(v: &str) -> String {
    v.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .trim()
        .to_string()
}

/// The request body for feedback: what they typed (checked, trimmed) plus the
/// app version and OS. Nothing about their servers, no install id.
pub fn feedback_payload(f: &Feedback) -> Result<serde_json::Value> {
    let kind = f.kind.trim();
    if !FEEDBACK_KINDS.contains(&kind) {
        return Err(Error::InvalidInput(format!("unknown feedback kind {kind:?}")));
    }
    let (about, message, contact) = (typed(&f.about), typed(&f.message), typed(&f.contact));
    if message.is_empty() {
        return Err(Error::InvalidInput("write a message first".into()));
    }
    for (name, v, max) in [
        ("message", &message, MAX_MESSAGE),
        ("topic", &about, MAX_ABOUT),
        ("contact", &contact, MAX_CONTACT),
    ] {
        if v.chars().count() > max {
            return Err(Error::InvalidInput(format!("the {name} is too long (at most {max} characters)")));
        }
    }
    Ok(serde_json::json!({
        "kind": kind,
        "about": about,
        "message": message,
        "contact": contact,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

/// Sends feedback and says plainly when it didn't arrive.
pub async fn send_feedback(f: &Feedback) -> Result<()> {
    let body = feedback_payload(f)?;
    let Some(base) = url() else {
        return Err(Error::Other(
            "this copy of Orbital has no feedback address (a development build); open an issue on GitHub instead".into(),
        ));
    };
    post_feedback(base, &body).await
}

async fn post_feedback(base: &str, body: &serde_json::Value) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| Error::Other(e.to_string()))?;
    let res = client
        .post(format!("{base}/v1/feedback"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| {
            Error::Other(format!(
                "couldn't reach the feedback server ({}); check the internet connection and try again",
                e.without_url()
            ))
        })?;
    match res.status().as_u16() {
        200..=299 => Ok(()),
        429 => Err(Error::Other(
            "a lot of feedback arrived in the last hour; please try again later".into(),
        )),
        400 | 413 => Err(Error::InvalidInput("the feedback server refused this message".into())),
        code => Err(Error::Other(format!("the feedback server answered {code}; please try again later"))),
    }
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

    fn fb(kind: &str, message: &str) -> Feedback {
        Feedback {
            kind: kind.into(),
            about: " Docker ".into(),
            message: message.into(),
            contact: String::new(),
        }
    }

    #[test]
    fn feedback_sends_what_was_typed_and_the_version_only() {
        let p = feedback_payload(&fb("idea", "  compose logs\u{7}\nplease ")).unwrap();
        let mut keys: Vec<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["about", "arch", "contact", "kind", "message", "os", "version"]);
        assert_eq!(p["message"], "compose logs\nplease");
        assert_eq!(p["about"], "Docker");
        assert!(feedback_payload(&fb("spam", "x")).is_err());
        assert!(feedback_payload(&fb("idea", "  ")).is_err());
        assert!(feedback_payload(&fb("idea", &"é".repeat(MAX_MESSAGE))).is_ok());
        assert!(feedback_payload(&fb("idea", &"x".repeat(MAX_MESSAGE + 1))).is_err());
        let mut long = fb("problem", "x");
        long.contact = "c".repeat(MAX_CONTACT + 1);
        assert!(feedback_payload(&long).is_err());
        // Test builds have no server: a clear error, no network.
        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(send_feedback(&fb("idea", "hi")))
            .unwrap_err();
        assert!(err.to_string().contains("GitHub"), "{err}");
    }

    /// The stats server stores exactly what `feedback_payload` sends.
    #[cfg(unix)]
    #[test]
    fn the_stats_server_accepts_feedback() {
        let body = feedback_payload(&fb("problem", "Files: \"permission denied\" <here>\nsecond line"))
            .unwrap()
            .to_string();
        let check = format!(
            "import sys; sys.path.insert(0, {dir:?}); import orbital_stats as s; f = s.clean_feedback({body:?}.encode()); assert f and f['kind'] == 'problem' and '\\n' in f['message'], f",
            dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stats-server"),
        );
        let out = match std::process::Command::new("python3").arg("-c").arg(check).output() {
            Ok(o) => o,
            Err(_) => return,
        };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    /// End to end: the real stats server on a local port takes feedback from
    /// `post_feedback`, shows it (escaped) on the dashboard, and refuses junk.
    #[cfg(unix)]
    #[test]
    fn feedback_reaches_the_stats_server_over_http() {
        use std::io::{BufRead, BufReader};
        let dir = tempfile::tempdir().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../stats-server/orbital_stats.py");
        let token = "t0ken-for-the-test-only";
        let Ok(mut child) = std::process::Command::new("python3")
            .arg("-I")
            .arg(file)
            .env("ORBITAL_STATS_TOKEN", token)
            .env("ORBITAL_STATS_DB", dir.path().join("s.db"))
            .env("ORBITAL_STATS_PORT", port.to_string())
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            return; // no python3 here
        };
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert!(line.contains("listening"), "{line}");
        let base = format!("http://127.0.0.1:{port}");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let body = feedback_payload(&fb("problem", "<script>alert(1)</script> breaks")).unwrap();
            post_feedback(&base, &body).await?;
            let refused = post_feedback(&base, &serde_json::json!({"kind": "spam", "message": "x"})).await;
            assert!(matches!(refused, Err(Error::InvalidInput(_))), "{refused:?}");
            let page = reqwest::Client::new()
                .get(format!("{base}/"))
                .basic_auth("orbital", Some(token))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt; breaks"), "{page}");
            assert!(!page.contains("<script>alert"));
            let list: serde_json::Value = serde_json::from_str(
                &reqwest::Client::new()
                    .get(format!("{base}/v1/feedback"))
                    .basic_auth("orbital", Some(token))
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(list[0]["kind"], "problem");
            assert_eq!(list[0]["about"], "Docker");
            let anonymous = reqwest::get(format!("{base}/v1/feedback")).await.unwrap().status();
            assert_eq!(anonymous.as_u16(), 401);
            Ok::<_, Error>(())
        });
        let _ = child.kill();
        result.unwrap();
    }
}
