//! Alerts beyond Orbital's own window (Settings → Notifications): health and
//! uptime alerts also go to a Telegram chat (through a bot you create), to
//! email over SMTP, or through the AI agent's bot on one of your servers.
//!
//! Channels are rows in `notify_channels`; a Telegram bot token or an SMTP
//! password lives in the vault under `notify:<id>:secret`, never in SQLite or
//! the audit log. Those keys are deliberately *not* behind the Touch ID gate:
//! alerts are sent unattended (nobody is there to touch the sensor), and the
//! secret can only send messages, not reach a server. Sending happens from
//! this computer, so it works while Orbital is open; the agent's own alerts on
//! the server keep working when it's closed.

use crate::db::Database;
use crate::error::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlertKind {
    /// A connected server needs attention (disk, memory, load, failed service or container) or recovered.
    Health,
    /// An uptime check went down, came back, or its certificate is expiring.
    Uptime,
    /// A long job (install, backup, upload, update…) finished or failed.
    Jobs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Bad,
    Ok,
    Warn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub kind: AlertKind,
    pub level: Level,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    /// Plain connection upgraded with STARTTLS (port 587). Refused if the server can't.
    StartTls,
    /// TLS from the first byte (port 465).
    Tls,
    /// No encryption (only for a relay on this computer or a private network).
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", rename_all_fields = "camelCase")]
pub enum ChannelConfig {
    /// A bot you made with @BotFather; the token is in the vault.
    Telegram { chat_id: String, chat_name: String },
    /// An SMTP account; the password (if any) is in the vault.
    Email {
        host: String,
        port: u16,
        security: Security,
        username: String,
        from: String,
        to: Vec<String>,
    },
    /// The AI agent's bot on this server (the server must be connected when an alert is sent).
    Agent { profile_id: String, server: String },
}

impl ChannelConfig {
    pub fn kind(&self) -> &'static str {
        match self {
            ChannelConfig::Telegram { .. } => "telegram",
            ChannelConfig::Email { .. } => "email",
            ChannelConfig::Agent { .. } => "agent",
        }
    }
    /// Telegram and SMTP keep a secret in the vault.
    pub fn has_secret(&self) -> bool {
        !matches!(self, ChannelConfig::Agent { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Which alerts it gets.
    pub events: Vec<AlertKind>,
    pub config: ChannelConfig,
    /// Hours when only problems are sent (recoveries, warnings and finished jobs wait).
    #[serde(default)]
    pub quiet: Option<Quiet>,
    pub created_at: String,
}

/// Local time of day, "HH:MM"; `from` may be later than `to` (overnight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quiet {
    pub from: String,
    pub to: String,
}

fn minutes(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60 && hhmm.trim().len() == 5).then_some(h * 60 + m)
}

/// Is `now` (minutes since local midnight) inside the quiet hours?
pub fn in_quiet(q: &Quiet, now: u32) -> bool {
    let (Some(f), Some(t)) = (minutes(&q.from), minutes(&q.to)) else {
        return false;
    };
    if f == t {
        return false;
    }
    if f < t {
        (f..t).contains(&now)
    } else {
        now >= f || now < t
    }
}

/// Whether an alert goes out now: problems always do; the rest wait out quiet hours.
pub fn goes_now(c: &Channel, a: &Alert, now: u32) -> bool {
    a.level == Level::Bad || !c.quiet.as_ref().is_some_and(|q| in_quiet(q, now))
}

/// What the dialog sends to add or change a channel. `secret` is the bot
/// token or SMTP password; empty keeps the saved one when editing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInput {
    pub name: String,
    pub enabled: bool,
    pub events: Vec<AlertKind>,
    pub config: ChannelConfig,
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub quiet: Option<Quiet>,
}

pub fn secret_key(id: &str) -> String {
    format!("notify:{id}:secret")
}

// ---------- validation ----------

static HOST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?i)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)*[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$|^[0-9.]+$|^\[[0-9a-f:]+\]$").unwrap()
});
static EMAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9._%+'-]{1,64}@([A-Za-z0-9-]{1,63}\.)+[A-Za-z]{2,63}$").unwrap());
static BOT_TOKEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9]{5,15}:[A-Za-z0-9_-]{30,64}$").unwrap());
static CHAT_ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(-?[0-9]{1,20}|@[A-Za-z][A-Za-z0-9_]{4,31})$").unwrap());

pub fn valid_email(s: &str) -> bool {
    EMAIL.is_match(s.trim())
}

pub fn check_bot_token(t: &str) -> Result<&str> {
    let t = t.trim();
    if BOT_TOKEN.is_match(t) {
        Ok(t)
    } else {
        Err(Error::InvalidInput(
            "that isn't a Telegram bot token: BotFather gives one like 123456789:AA… (digits, a colon, then letters)".into(),
        ))
    }
}

/// Checks a channel before it's saved; `has_saved_secret` lets an edit keep the stored token or password.
pub fn validate(input: &ChannelInput, has_saved_secret: bool) -> Result<()> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(Error::InvalidInput("give it a name of up to 60 characters".into()));
    }
    if input.events.is_empty() {
        return Err(Error::InvalidInput("choose at least one kind of alert to send".into()));
    }
    if let Some(q) = &input.quiet {
        if minutes(&q.from).is_none() || minutes(&q.to).is_none() || q.from.trim() == q.to.trim() {
            return Err(Error::InvalidInput(
                "give quiet hours as two different times, like 22:00 and 07:00".into(),
            ));
        }
    }
    let secret = input.secret.trim();
    match &input.config {
        ChannelConfig::Telegram { chat_id, .. } => {
            if !CHAT_ID.is_match(chat_id.trim()) {
                return Err(Error::InvalidInput(
                    "pick the chat to send to (message your bot first, then Find chats)".into(),
                ));
            }
            if secret.is_empty() && !has_saved_secret {
                return Err(Error::InvalidInput("paste the bot token from BotFather".into()));
            }
            if !secret.is_empty() {
                check_bot_token(secret)?;
            }
        }
        ChannelConfig::Email {
            host,
            port,
            security,
            username,
            from,
            to,
        } => {
            let h = host.trim();
            if h.is_empty() || h.len() > 253 || !HOST.is_match(h) {
                return Err(Error::InvalidInput(format!(
                    "{h:?} isn't a mail server name (like smtp.example.com)"
                )));
            }
            if *port == 0 {
                return Err(Error::InvalidInput("give the SMTP port (587 for STARTTLS, 465 for TLS)".into()));
            }
            if !valid_email(from) {
                return Err(Error::InvalidInput(format!(
                    "{:?} isn't an email address to send from",
                    from.trim()
                )));
            }
            if to.is_empty() || to.len() > 10 {
                return Err(Error::InvalidInput("give 1 to 10 addresses to send to".into()));
            }
            if let Some(bad) = to.iter().find(|a| !valid_email(a)) {
                return Err(Error::InvalidInput(format!("{:?} isn't an email address", bad.trim())));
            }
            if username.chars().count() > 200 || username.chars().any(char::is_control) {
                return Err(Error::InvalidInput("the user name is too long".into()));
            }
            if *security == Security::None && !username.trim().is_empty() {
                return Err(Error::InvalidInput(
                    "signing in without encryption would send the password in the clear: use STARTTLS or TLS".into(),
                ));
            }
            if !username.trim().is_empty() && secret.is_empty() && !has_saved_secret {
                return Err(Error::InvalidInput("give the SMTP password (or an app password)".into()));
            }
        }
        ChannelConfig::Agent { profile_id, .. } => {
            if profile_id.trim().is_empty() {
                return Err(Error::InvalidInput("pick the server whose AI agent should send it".into()));
            }
        }
    }
    Ok(())
}

// ---------- messages ----------

fn icon(level: Level) -> &'static str {
    match level {
        Level::Bad => "🔴",
        Level::Warn => "🟠",
        Level::Ok => "✅",
    }
}

/// A short plain-text message (Telegram and the agent).
pub fn text(a: &Alert) -> String {
    let body = a.body.trim();
    let mut s = format!("{} {}", icon(a.level), a.title.trim());
    if !body.is_empty() {
        s.push('\n');
        s.push_str(body);
    }
    s.push_str("\n— Orbital");
    s.chars().take(3500).collect()
}

/// The email's subject and body.
pub fn email(a: &Alert) -> (String, String) {
    let subject: String = format!("{} {}", icon(a.level), a.title.trim()).chars().take(200).collect();
    let what = match a.kind {
        AlertKind::Health => "a server's health",
        AlertKind::Uptime => "an uptime check",
        AlertKind::Jobs => "a job you started",
    };
    let body = format!(
        "{}\n\n{}\n\n--\nSent by Orbital on your computer about {what}. Change or stop these in Orbital → Settings → Notifications.\n",
        a.title.trim(),
        a.body.trim()
    );
    (subject.replace(['\r', '\n'], " "), body)
}

// ---------- storage ----------

fn row(r: &Row) -> rusqlite::Result<Channel> {
    let config: String = r.get("config")?;
    let events: String = r.get("events")?;
    Ok(Channel {
        id: r.get("id")?,
        name: r.get("name")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
        events: serde_json::from_str(&events).unwrap_or_default(),
        config: serde_json::from_str(&config).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        quiet: r.get::<_, Option<String>>("quiet")?.and_then(|q| serde_json::from_str(&q).ok()),
        created_at: r.get("created_at")?,
    })
}

pub fn list(db: &Database) -> Result<Vec<Channel>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM notify_channels ORDER BY created_at")?;
    let v = st.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

pub fn get(db: &Database, id: &str) -> Result<Channel> {
    db.conn()
        .query_row("SELECT * FROM notify_channels WHERE id = ?1", [id], row)
        .optional()?
        .ok_or_else(|| Error::InvalidInput("that notification channel no longer exists".into()))
}

/// Inserts (no `id`) or updates a channel; never stores the secret.
pub fn save(db: &Database, id: Option<&str>, input: &ChannelInput) -> Result<Channel> {
    let mut config = input.config.clone();
    if let ChannelConfig::Email {
        host, username, from, to, ..
    } = &mut config
    {
        *host = host.trim().to_string();
        *username = username.trim().to_string();
        *from = from.trim().to_string();
        *to = to.iter().map(|a| a.trim().to_string()).collect();
    }
    if let ChannelConfig::Telegram { chat_id, .. } = &mut config {
        *chat_id = chat_id.trim().to_string();
    }
    let mut events = input.events.clone();
    events.dedup();
    let cfg = serde_json::to_string(&config).map_err(|e| Error::Other(e.to_string()))?;
    let ev = serde_json::to_string(&events).map_err(|e| Error::Other(e.to_string()))?;
    let quiet = input
        .quiet
        .as_ref()
        .map(|q| serde_json::json!({ "from": q.from.trim(), "to": q.to.trim() }).to_string());
    let conn = db.conn();
    let id = match id {
        Some(id) => {
            let n = conn.execute(
                "UPDATE notify_channels SET name = ?2, kind = ?3, config = ?4, events = ?5, enabled = ?6, quiet = ?7 WHERE id = ?1",
                params![id, input.name.trim(), config.kind(), cfg, ev, input.enabled as i64, quiet],
            )?;
            if n == 0 {
                return Err(Error::InvalidInput("that notification channel no longer exists".into()));
            }
            id.to_string()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO notify_channels (id, name, kind, config, events, enabled, created_at, quiet) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id,
                    input.name.trim(),
                    config.kind(),
                    cfg,
                    ev,
                    input.enabled as i64,
                    chrono::Utc::now().to_rfc3339(),
                    quiet
                ],
            )?;
            id
        }
    };
    drop(conn);
    get(db, &id)
}

pub fn delete(db: &Database, id: &str) -> Result<()> {
    db.conn().execute("DELETE FROM notify_channels WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- the brake ----------

/// At most `MAX_PER_HOUR` messages per channel in any hour, so a flapping
/// server can't flood a phone or get the bot or mailbox blocked.
pub const MAX_PER_HOUR: usize = 30;

#[derive(Default)]
pub struct Brake(Mutex<HashMap<String, Vec<Instant>>>);

impl Brake {
    /// True if this channel may send now (and counts it).
    pub fn allow(&self, channel: &str, now: Instant) -> bool {
        let mut m = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let v = m.entry(channel.to_string()).or_default();
        v.retain(|t| now.duration_since(*t) < Duration::from_secs(3600));
        if v.len() >= MAX_PER_HOUR {
            return false;
        }
        v.push(now);
        true
    }
}

// ---------- Telegram ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChat {
    pub id: String,
    /// A person's name, @username or a group's title.
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramBot {
    pub username: String,
    /// Chats that wrote to the bot recently (newest first).
    pub chats: Vec<TelegramChat>,
}

/// Telegram's own error text, or a short one; never the URL (it holds the token).
fn telegram_error(status: u16, body: &serde_json::Value) -> Error {
    let d = body.get("description").and_then(|d| d.as_str()).unwrap_or("no answer");
    match status {
        401 | 404 => Error::InvalidInput("Telegram doesn't know that bot token: copy it again from BotFather".into()),
        403 => Error::Other(format!("the bot can't message that chat (blocked, or never started): {d}")),
        400 if d.contains("chat not found") => {
            Error::Other("Telegram doesn't know that chat: send your bot a message first, then Find chats".into())
        }
        _ => Error::Other(format!("Telegram said: {d}")),
    }
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Orbital/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Other(e.to_string()))
}

async fn telegram_call(base: &str, token: &str, method: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let url = format!("{}/bot{token}/{method}", base.trim_end_matches('/'));
    let r = client()?
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| Error::Other(format!("couldn't reach Telegram: {}", e.without_url())))?;
    let status = r.status().as_u16();
    let v: serde_json::Value = serde_json::from_str(&r.text().await.unwrap_or_default()).unwrap_or(serde_json::Value::Null);
    if status != 200 || v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        return Err(telegram_error(status, &v));
    }
    Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null))
}

/// Chats from `getUpdates`, newest first, each once.
pub fn parse_chats(result: &serde_json::Value) -> Vec<TelegramChat> {
    let mut out: Vec<TelegramChat> = Vec::new();
    let updates = result.as_array().cloned().unwrap_or_default();
    for u in updates.iter().rev() {
        let msg = ["message", "channel_post", "my_chat_member", "edited_message"]
            .iter()
            .find_map(|k| u.get(*k));
        let Some(chat) = msg.and_then(|m| m.get("chat")) else { continue };
        let Some(id) = chat.get("id").and_then(|i| i.as_i64()) else {
            continue;
        };
        let id = id.to_string();
        if out.iter().any(|c| c.id == id) {
            continue;
        }
        let s = |k: &str| chat.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = if !s("title").is_empty() {
            s("title")
        } else {
            let full = format!("{} {}", s("first_name"), s("last_name")).trim().to_string();
            match (full.is_empty(), s("username").is_empty()) {
                (false, false) => format!("{full} (@{})", s("username")),
                (false, true) => full,
                (true, false) => format!("@{}", s("username")),
                (true, true) => id.clone(),
            }
        };
        out.push(TelegramChat { id, name, kind: s("type") });
    }
    out
}

/// The bot's name and the chats that have written to it (to pick one).
pub async fn telegram_bot(base: &str, token: &str) -> Result<TelegramBot> {
    let token = check_bot_token(token)?;
    let me = telegram_call(base, token, "getMe", serde_json::json!({})).await?;
    let updates = telegram_call(base, token, "getUpdates", serde_json::json!({ "limit": 100, "timeout": 0 })).await?;
    Ok(TelegramBot {
        username: me.get("username").and_then(|u| u.as_str()).unwrap_or("").to_string(),
        chats: parse_chats(&updates),
    })
}

pub async fn telegram_send(base: &str, token: &str, chat_id: &str, text: &str) -> Result<()> {
    let id: serde_json::Value = match chat_id.parse::<i64>() {
        Ok(n) => n.into(),
        Err(_) => chat_id.into(),
    };
    telegram_call(
        base,
        token,
        "sendMessage",
        serde_json::json!({ "chat_id": id, "text": text, "disable_web_page_preview": true }),
    )
    .await?;
    Ok(())
}

// ---------- email ----------

pub async fn email_send(cfg: &ChannelConfig, password: Option<&str>, subject: &str, body: &str) -> Result<()> {
    use lettre::message::{header::ContentType, Mailbox};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::transport::smtp::client::{Tls, TlsParameters};
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
    let ChannelConfig::Email {
        host,
        port,
        security,
        username,
        from,
        to,
    } = cfg
    else {
        return Err(Error::InvalidInput("not an email channel".into()));
    };
    let parse = |a: &str| -> Result<Mailbox> { a.parse().map_err(|_| Error::InvalidInput(format!("{a:?} isn't an email address"))) };
    let mut m = Message::builder()
        .from(parse(from)?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN);
    for t in to {
        m = m.to(parse(t)?);
    }
    let message = m
        .body(body.to_string())
        .map_err(|e| Error::Other(format!("couldn't build the email: {e}")))?;
    let tls = || TlsParameters::new(host.clone()).map_err(|e| Error::Other(format!("TLS: {e}")));
    let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host.as_str())
        .port(*port)
        .timeout(Some(Duration::from_secs(20)));
    b = match security {
        Security::StartTls => b.tls(Tls::Required(tls()?)),
        Security::Tls => b.tls(Tls::Wrapper(tls()?)),
        Security::None => b.tls(Tls::None),
    };
    if !username.is_empty() {
        b = b.credentials(Credentials::new(username.clone(), password.unwrap_or("").to_string()));
    }
    b.build()
        .send(message)
        .await
        .map_err(|e| Error::Other(format!("the mail server refused it: {e}")))?;
    Ok(())
}

// ---------- the agent ----------

/// Sends stdin to the paired Telegram account through the agent on the server.
pub const AGENT_SAY_COMMAND: &str = "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py say";

// ---------- results ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub channel_id: String,
    pub channel: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tg(chat: &str, secret: &str) -> ChannelInput {
        ChannelInput {
            name: "My phone".into(),
            enabled: true,
            events: vec![AlertKind::Health, AlertKind::Uptime],
            config: ChannelConfig::Telegram {
                chat_id: chat.into(),
                chat_name: "Me".into(),
            },
            secret: secret.into(),
            quiet: None,
        }
    }

    fn mail(security: Security, user: &str, to: &[&str]) -> ChannelInput {
        ChannelInput {
            name: "Ops mail".into(),
            enabled: true,
            events: vec![AlertKind::Uptime],
            config: ChannelConfig::Email {
                host: "smtp.example.com".into(),
                port: 587,
                security,
                username: user.into(),
                from: "orbital@example.com".into(),
                to: to.iter().map(|s| s.to_string()).collect(),
            },
            secret: "app-password".into(),
            quiet: None,
        }
    }

    const TOKEN: &str = "123456789:AAH0abcdefghijklmnopqrstuvwxyz012345";

    #[test]
    fn validates_channels() {
        assert!(validate(&tg("123456", TOKEN), false).is_ok());
        assert!(validate(&tg("-1001234567890", ""), true).is_ok(), "an edit keeps the saved token");
        assert!(validate(&tg("123456", ""), false).is_err(), "no token");
        assert!(validate(&tg("123456", "not-a-token"), false).is_err());
        assert!(validate(&tg("12; rm", TOKEN), false).is_err());
        let mut none = tg("1", TOKEN);
        none.events.clear();
        assert!(validate(&none, false).is_err());

        assert!(validate(&mail(Security::StartTls, "ops", &["a@example.com", "b@example.org"]), false).is_ok());
        assert!(validate(&mail(Security::StartTls, "ops", &["not an address"]), false).is_err());
        assert!(validate(&mail(Security::StartTls, "ops", &[]), false).is_err());
        assert!(
            validate(&mail(Security::None, "ops", &["a@example.com"]), false).is_err(),
            "no password in the clear"
        );
        assert!(
            validate(&mail(Security::None, "", &["a@example.com"]), false).is_ok(),
            "an open relay"
        );
        let mut bad_host = mail(Security::Tls, "", &["a@example.com"]);
        if let ChannelConfig::Email { host, .. } = &mut bad_host.config {
            *host = "smtp.example.com; rm -rf /".into();
        }
        assert!(validate(&bad_host, false).is_err());
        assert!(validate(
            &ChannelInput {
                name: "Agent".into(),
                enabled: true,
                events: vec![AlertKind::Health],
                config: ChannelConfig::Agent {
                    profile_id: String::new(),
                    server: "web".into()
                },
                secret: String::new(),
                quiet: None,
            },
            false
        )
        .is_err());
    }

    #[test]
    fn messages_read_well() {
        let a = Alert {
            kind: AlertKind::Health,
            level: Level::Bad,
            title: "web-1: nginx failed".into(),
            body: "nginx".into(),
        };
        assert_eq!(text(&a), "🔴 web-1: nginx failed\nnginx\n— Orbital");
        let (subject, body) = email(&Alert {
            title: "shop is down\r\nBcc: x@example.com".into(),
            ..a.clone()
        });
        assert!(!subject.contains('\n') && !subject.contains('\r'), "no header injection: {subject}");
        assert!(body.contains("Settings → Notifications"));
        let long = Alert {
            body: "x".repeat(10_000),
            ..a
        };
        assert!(text(&long).chars().count() <= 3500);
    }

    #[test]
    fn stores_channels_without_secrets() {
        let db = Database::in_memory().unwrap();
        let c = save(&db, None, &tg(" 123456 ", TOKEN)).unwrap();
        assert_eq!(
            c.config,
            ChannelConfig::Telegram {
                chat_id: "123456".into(),
                chat_name: "Me".into()
            }
        );
        let all = list(&db).unwrap();
        assert_eq!(all.len(), 1);
        let raw: String = db
            .conn()
            .query_row("SELECT config || events || name FROM notify_channels", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.contains(TOKEN), "the token never reaches SQLite");
        let mut edit = mail(Security::Tls, "ops", &["a@example.com"]);
        edit.enabled = false;
        let c2 = save(&db, Some(&c.id), &edit).unwrap();
        assert!(!c2.enabled && c2.config.kind() == "email");
        assert!(save(&db, Some("nope"), &edit).is_err());
        delete(&db, &c.id).unwrap();
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn the_brake_holds_a_flood() {
        let b = Brake::default();
        let t = Instant::now();
        for _ in 0..MAX_PER_HOUR {
            assert!(b.allow("a", t));
        }
        assert!(!b.allow("a", t), "31st in the hour");
        assert!(b.allow("b", t), "per channel");
        assert!(b.allow("a", t + Duration::from_secs(3601)), "an hour later");
    }

    #[test]
    fn reads_chats_and_telegram_errors() {
        let updates = serde_json::json!([
            { "update_id": 1, "message": { "chat": { "id": 111, "type": "private", "first_name": "Ann", "username": "ann" } } },
            { "update_id": 2, "message": { "chat": { "id": -1002, "type": "supergroup", "title": "Ops" } } },
            { "update_id": 3, "message": { "chat": { "id": 111, "type": "private", "first_name": "Ann" } } },
            { "update_id": 4, "my_chat_member": { "chat": { "id": 333, "type": "private", "username": "bob" } } }
        ]);
        let c = parse_chats(&updates);
        assert_eq!(c.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["333", "111", "-1002"]);
        assert_eq!(c[0].name, "@bob");
        assert_eq!(c[2].name, "Ops");
        assert!(parse_chats(&serde_json::Value::Null).is_empty());
        let e = telegram_error(401, &serde_json::json!({ "ok": false, "description": "Unauthorized" })).to_string();
        assert!(e.contains("BotFather"));
        let e = telegram_error(400, &serde_json::json!({ "description": "Bad Request: chat not found" })).to_string();
        assert!(e.contains("Find chats"));
        assert!(check_bot_token(TOKEN).is_ok() && check_bot_token("123:abc").is_err());
    }

    #[test]
    fn quiet_hours_hold_all_but_problems() {
        let night = Quiet {
            from: "22:00".into(),
            to: "07:00".into(),
        };
        assert!(in_quiet(&night, 23 * 60) && in_quiet(&night, 3 * 60) && !in_quiet(&night, 7 * 60) && !in_quiet(&night, 12 * 60));
        let lunch = Quiet {
            from: "12:00".into(),
            to: "13:30".into(),
        };
        assert!(in_quiet(&lunch, 12 * 60 + 45) && !in_quiet(&lunch, 13 * 60 + 30));
        assert!(!in_quiet(
            &Quiet {
                from: "25:00".into(),
                to: "07:00".into()
            },
            60
        ));
        let db = Database::in_memory().unwrap();
        let mut i = tg("1", TOKEN);
        i.quiet = Some(night.clone());
        let c = save(&db, None, &i).unwrap();
        assert_eq!(c.quiet, Some(night));
        let a = |level| Alert {
            kind: AlertKind::Health,
            level,
            title: "x".into(),
            body: String::new(),
        };
        assert!(goes_now(&c, &a(Level::Bad), 23 * 60), "problems always go");
        assert!(!goes_now(&c, &a(Level::Ok), 23 * 60) && !goes_now(&c, &a(Level::Warn), 23 * 60));
        assert!(goes_now(&c, &a(Level::Ok), 9 * 60));
        let mut bad = tg("1", TOKEN);
        bad.quiet = Some(Quiet {
            from: "22:00".into(),
            to: "22:00".into(),
        });
        assert!(validate(&bad, false).is_err());
    }
}
