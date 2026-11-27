//! Engine calls for alerts beyond Orbital's window (notify.rs): channels,
//! Telegram chat lookup, sending and the test button. A child module of
//! `engine`, so it uses the same audit plumbing.

use super::*;
use crate::notify::{self as nt, Alert, AlertKind, Channel, ChannelConfig, ChannelInput, SendResult, TelegramBot};

impl Engine {
    pub fn set_telegram_base(&self, base: &str) {
        *self.telegram_base.write().unwrap_or_else(|p| p.into_inner()) = base.to_string();
    }

    fn telegram_base(&self) -> String {
        self.telegram_base.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn record_notify(&self, action: &str, target: &str, outcome: Outcome, details: Option<serde_json::Value>) {
        self.record(
            None,
            NewEvent {
                category: Some(Category::Notify),
                action: action.into(),
                target: Some(target.into()),
                outcome: Some(outcome),
                details,
                ..Default::default()
            },
        );
    }

    pub fn notify_channels(&self) -> Result<Vec<Channel>> {
        nt::list(&self.db)
    }

    /// The bot's name and the chats that wrote to it, to pick where alerts go.
    pub async fn telegram_lookup(&self, token: &str) -> Result<TelegramBot> {
        nt::telegram_bot(&self.telegram_base(), token).await
    }

    /// Add (`id` None) or change a channel. A new token or password replaces
    /// the saved one; an empty one keeps it.
    pub fn save_notify_channel(&self, id: Option<&str>, input: &ChannelInput) -> Result<Channel> {
        let had = match id {
            Some(id) => self.secrets.exists(&nt::secret_key(id))?,
            None => false,
        };
        if let Err(e) = nt::validate(input, had) {
            self.record_notify(
                "notify.save",
                input.name.trim(),
                Outcome::Denied,
                Some(serde_json::json!({ "error": e.to_string() })),
            );
            return Err(e);
        }
        let c = nt::save(&self.db, id, input)?;
        let secret = input.secret.trim();
        if !c.config.has_secret() {
            let _ = self.secrets.delete(&nt::secret_key(&c.id));
        } else if !secret.is_empty() {
            if let Err(e) = self.secrets.set(&nt::secret_key(&c.id), secret) {
                if id.is_none() {
                    let _ = nt::delete(&self.db, &c.id);
                }
                return Err(e);
            }
        }
        self.record_notify(
            if id.is_some() { "notify.change" } else { "notify.add" },
            &c.name,
            Outcome::Success,
            Some(serde_json::json!({ "kind": c.config.kind(), "events": c.events, "enabled": c.enabled })),
        );
        Ok(c)
    }

    pub fn delete_notify_channel(&self, id: &str) -> Result<()> {
        let c = nt::get(&self.db, id)?;
        let _ = self.secrets.delete(&nt::secret_key(id));
        nt::delete(&self.db, id)?;
        self.record_notify("notify.remove", &c.name, Outcome::Success, None);
        Ok(())
    }

    /// One message on one channel. Errors never carry the token or password.
    async fn deliver(&self, c: &Channel, a: &Alert) -> Result<()> {
        let secret = if c.config.has_secret() {
            self.secrets.get(&nt::secret_key(&c.id))?
        } else {
            None
        };
        match &c.config {
            ChannelConfig::Telegram { chat_id, .. } => {
                let token =
                    secret.ok_or_else(|| Error::InvalidInput("the bot token is missing: edit the channel and paste it again".into()))?;
                nt::telegram_send(&self.telegram_base(), token.as_str(), chat_id, &nt::text(a)).await
            }
            ChannelConfig::Email { .. } => {
                let (subject, body) = nt::email(a);
                nt::email_send(&c.config, secret.as_ref().map(|s| s.as_str()), &subject, &body).await
            }
            ChannelConfig::Agent { profile_id, server } => {
                let s = {
                    let sessions = self.sessions.read().await;
                    sessions.values().find(|s| &s.profile.id == profile_id).cloned()
                };
                let s = s.ok_or_else(|| Error::Other(format!("{server} isn't connected, so its AI agent can't send this")))?;
                let o = s
                    .exec_privileged_with(
                        nt::AGENT_SAY_COMMAND,
                        Some(nt::text(a).as_bytes()),
                        std::time::Duration::from_secs(40),
                    )
                    .await?;
                self.record_exec(
                    &s,
                    Category::Notify,
                    "notify.agent",
                    "Telegram",
                    &s.privileged_command(nt::AGENT_SAY_COMMAND),
                    &o,
                );
                crate::agent::parse_test(&o.stdout, &o.stderr).map(|_| ())
            }
        }
    }

    /// Send to every enabled channel that takes this kind of alert. Failures
    /// are recorded, never raised: an alert must not break what found it.
    pub async fn send_alert(&self, a: &Alert) -> Vec<SendResult> {
        let channels = match nt::list(&self.db) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("couldn't read notification channels: {e}");
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for c in channels.into_iter().filter(|c| c.enabled && c.events.contains(&a.kind)) {
            if !self.notify_brake.allow(&c.id, std::time::Instant::now()) {
                self.record_notify(
                    "notify.send",
                    &c.name,
                    Outcome::Denied,
                    Some(serde_json::json!({ "error": "more than 30 alerts in an hour; held back", "title": a.title })),
                );
                out.push(SendResult {
                    channel_id: c.id.clone(),
                    channel: c.name.clone(),
                    ok: false,
                    error: Some("held back: more than 30 alerts in the last hour".into()),
                });
                continue;
            }
            let r = self.deliver(&c, a).await;
            let error = r.as_ref().err().map(|e| e.to_string());
            self.record_notify(
                "notify.send",
                &c.name,
                if r.is_ok() { Outcome::Success } else { Outcome::Failure },
                Some(serde_json::json!({ "kind": c.config.kind(), "title": a.title, "error": error })),
            );
            out.push(SendResult {
                channel_id: c.id.clone(),
                channel: c.name.clone(),
                ok: r.is_ok(),
                error,
            });
        }
        out
    }

    /// The test button: one message on this channel now, whatever it's subscribed to.
    pub async fn test_notify_channel(&self, id: &str) -> Result<()> {
        let c = nt::get(&self.db, id)?;
        let a = Alert {
            kind: AlertKind::Health,
            level: nt::Level::Ok,
            title: "Test from Orbital".into(),
            body: format!("Alerts from Orbital will arrive here ({}).", c.name),
        };
        let r = self.deliver(&c, &a).await;
        self.record_notify(
            "notify.test",
            &c.name,
            if r.is_ok() { Outcome::Success } else { Outcome::Failure },
            r.as_ref().err().map(|e| serde_json::json!({ "error": e.to_string() })),
        );
        r
    }
}
