//! The server agent: a Telegram bot, backed by Claude or ChatGPT, that runs
//! on the server itself (so it answers 24/7) and helps the owner find out
//! what is going on. It looks freely; changes wait for the owner's Approve in
//! Telegram. It also messages the owner when a service fails, a disk fills up
//! or a container keeps restarting.
//!
//! The agent is `agent/orbital_agent.py` (standard library for Telegram, the
//! provider's official SDK for the AI, installed into a virtualenv here). The
//! bot token and the AI key are written to `/etc/orbital-agent/config.json`
//! (root only) through stdin, never on a command line, and Orbital doesn't
//! keep them. Everything Orbital runs here is fixed text.

use crate::error::{Error, Result};
use crate::ssh::SshSession;
use serde::{Deserialize, Serialize};

/// The agent itself, uploaded to `/opt/orbital-agent/agent.py`.
pub const AGENT_PY: &str = include_str!("agent/orbital_agent.py");
/// Its version, as the agent reports it (kept equal to `VERSION` in the script).
pub const VERSION: &str = "1.3.3";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Claude,
    Openai,
}

/// What the user typed to set the agent up. The token and the key go to the
/// server only.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSetup {
    pub telegram_token: String,
    pub provider: Provider,
    pub api_key: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "yes")]
    pub alerts: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub bot: Option<String>,
    pub alerts: bool,
    /// A pairing code is waiting to be used.
    pub pair_code_active: bool,
    /// The Telegram account it answers.
    pub owner: Option<String>,
    pub paired_at: Option<i64>,
    /// Problems it has told the owner about and that aren't fixed yet.
    pub alerting: Vec<String>,
    /// The last lines of its activity log (JSON lines, no secrets).
    pub recent: Vec<String>,
    /// The installed agent is older than this Orbital's (or its version can't be read).
    pub update_available: bool,
    /// Time of the daily summary ("09:00"), or none.
    pub daily_report: Option<String>,
    /// The agent version this Orbital installs.
    pub latest_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCode {
    pub code: String,
    /// Unix seconds.
    pub expires: i64,
    pub bot: Option<String>,
}

/// Python 3.8+ with venv, the folders, the virtualenv and both AI SDKs (so the
/// provider can change without a reinstall).
pub const PREPARE_SCRIPT: &str = r#"set -e
need=0
command -v python3 >/dev/null 2>&1 || need=1
[ $need -eq 0 ] && python3 -c 'import venv, ensurepip' >/dev/null 2>&1 || need=1
if [ $need -eq 1 ]; then
  if command -v apt-get >/dev/null 2>&1; then
    DEBIAN_FRONTEND=noninteractive apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3 python3-venv
  elif command -v dnf >/dev/null 2>&1; then dnf -y -q install python3
  elif command -v yum >/dev/null 2>&1; then yum -y -q install python3
  else echo "This server has no python3 and no apt, dnf or yum to install it."; exit 3; fi
fi
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 8) else 4)' || { echo "Python 3.8 or newer is needed."; exit 4; }
mkdir -p /opt/orbital-agent /etc/orbital-agent /var/lib/orbital-agent
chmod 700 /etc/orbital-agent /var/lib/orbital-agent
[ -x /opt/orbital-agent/venv/bin/python ] || python3 -m venv /opt/orbital-agent/venv
/opt/orbital-agent/venv/bin/pip install --quiet --disable-pip-version-check --upgrade anthropic openai
"#;

/// Writes the agent from stdin.
pub const WRITE_AGENT: &str = "cat > /opt/orbital-agent/agent.py.new && chmod 755 /opt/orbital-agent/agent.py.new && mv /opt/orbital-agent/agent.py.new /opt/orbital-agent/agent.py";

/// Writes the config (with the keys) from stdin, readable by root only.
pub const WRITE_CONFIG: &str = "umask 077 && cat > /etc/orbital-agent/config.json.new && chmod 600 /etc/orbital-agent/config.json.new && mv /etc/orbital-agent/config.json.new /etc/orbital-agent/config.json";

pub const UNIT: &str = "[Unit]
Description=Orbital agent (Telegram + AI assistant)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py run
Restart=always
RestartSec=10
User=root

[Install]
WantedBy=multi-user.target
";

pub const WRITE_UNIT: &str = "cat > /etc/systemd/system/orbital-agent.service && systemctl daemon-reload && systemctl enable orbital-agent >/dev/null 2>&1 && systemctl restart orbital-agent";

pub const RESTART: &str = "systemctl restart orbital-agent";
pub const STOP: &str = "systemctl stop orbital-agent";

/// Hands a new pairing code (stdin) to the agent.
pub const SET_PAIR_CODE: &str = "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py pair";

pub const ALERTS_ON: &str = "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py set alerts on";
pub const ALERTS_OFF: &str = "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py set alerts off";

/// The models the configured key can use, listed by the agent itself (Orbital
/// doesn't keep the key): JSON `[{"id", "name"}]`, or `{"error": …}`.
pub const LIST_MODELS: &str = "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py models";

/// Switch the model; the running agent uses it from the next message.
pub fn set_model_command(model: &str) -> Result<String> {
    let m = model.trim();
    if m.is_empty() || m.len() > 80 || !m.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._:/".contains(&b)) {
        return Err(Error::InvalidInput("a model name is letters, digits and - . _ : / only".into()));
    }
    Ok(format!(
        "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py set model {}",
        crate::shell::quote(m)
    ))
}

/// The agent's model list, or what went wrong (an old agent without the
/// command, or the provider's own error).
pub fn parse_model_list(stdout: &str, stderr: &str) -> Result<Vec<AiModel>> {
    let text = stdout.trim();
    if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(text) {
        let models: Vec<AiModel> = list
            .iter()
            .filter_map(|m| {
                let id = m.get("id")?.as_str()?.to_string();
                let name = m.get("name").and_then(|n| n.as_str()).unwrap_or(&id).to_string();
                Some(AiModel { id, name })
            })
            .collect();
        return Ok(models);
    }
    if let Some(e) = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
    {
        return Err(Error::Other(format!("the AI provider said: {e}")));
    }
    if stderr.contains("unknown command") {
        return Err(Error::InvalidInput(
            "this agent is too old to list models: click Update agent first".into(),
        ));
    }
    Err(Error::Other(format!(
        "couldn't list the models: {}",
        stderr.trim().lines().last().unwrap_or("no answer")
    )))
}

/// Message the paired account (no AI involved): JSON `{"ok": true}` or `{"error": …}`.
pub const TEST_COMMAND: &str = "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py test";

/// Daily summary at `time` ("HH:MM", server time), or off.
pub fn set_daily_command(time: Option<&str>) -> Result<String> {
    let value = match time.map(str::trim) {
        None | Some("") => "off".to_string(),
        Some(t) => {
            let ok = t.len() == 5
                && t.as_bytes()[2] == b':'
                && t[..2].parse::<u8>().is_ok_and(|h| h < 24)
                && t[3..].parse::<u8>().is_ok_and(|m| m < 60);
            if !ok {
                return Err(Error::InvalidInput("give the time as HH:MM, like 09:00".into()));
            }
            t.to_string()
        }
    };
    Ok(format!(
        "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py set daily {value}"
    ))
}

/// What the test command printed: who it reached, or why it couldn't.
pub fn parse_test(stdout: &str, stderr: &str) -> Result<Option<String>> {
    let v: serde_json::Value = match serde_json::from_str(stdout.trim()) {
        Ok(v) => v,
        Err(_) if stderr.contains("unknown command") => {
            return Err(Error::InvalidInput(
                "this agent is too old for a test message: click Update agent first".into(),
            ))
        }
        Err(_) => {
            return Err(Error::Other(format!(
                "the agent didn't answer: {}",
                stderr.trim().lines().last().unwrap_or("no output")
            )))
        }
    };
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        return Err(Error::Other(e.to_string()));
    }
    Ok(v.get("to").and_then(|t| t.as_str()).map(String::from))
}

pub const REMOVE: &str = "systemctl disable --now orbital-agent >/dev/null 2>&1; rm -f /etc/systemd/system/orbital-agent.service; systemctl daemon-reload; rm -rf /opt/orbital-agent /etc/orbital-agent /var/lib/orbital-agent /var/log/orbital-agent.log; echo removed";

pub const STATUS_SCRIPT: &str = r#"if [ -f /opt/orbital-agent/agent.py ]; then
  echo "@@installed"
  echo "@@active $(systemctl is-active orbital-agent 2>/dev/null)"
  /opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py status 2>/dev/null | sed 's/^/@@json /'
  tail -n 40 /var/log/orbital-agent.log 2>/dev/null | sed 's/^/@@log /'
fi
true
"#;

/// The config file's contents. The bot's name is kept for display.
pub fn config_json(setup: &AgentSetup, bot_username: &str, server_name: &str) -> Result<String> {
    validate_setup(setup)?;
    Ok(serde_json::json!({
        "telegram_token": setup.telegram_token.trim(),
        "provider": match setup.provider { Provider::Claude => "claude", Provider::Openai => "openai" },
        "api_key": setup.api_key.trim(),
        "model": setup.model.as_deref().map(str::trim).filter(|m| !m.is_empty()),
        "bot_username": bot_username,
        "server_name": server_name,
        "alerts": setup.alerts,
    })
    .to_string())
}

pub fn validate_setup(s: &AgentSetup) -> Result<()> {
    let token = s.telegram_token.trim();
    // BotFather tokens: digits, a colon, then 30+ URL-safe characters.
    let ok_token = token.split_once(':').is_some_and(|(id, rest)| {
        !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_digit())
            && rest.len() >= 30
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    });
    if !ok_token {
        return Err(Error::InvalidInput(
            "that doesn't look like a bot token from @BotFather (123456789:AA…)".into(),
        ));
    }
    let key = s.api_key.trim();
    if key.len() < 20 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::InvalidInput("paste the whole API key".into()));
    }
    match s.provider {
        Provider::Claude if key.starts_with("sk-ant-oat") => {
            return Err(Error::InvalidInput(
                "this is a Claude subscription (OAuth) token, like the one Claude Code uses, not an API key. Create an API key at console.anthropic.com → API keys (it starts with sk-ant-api) and paste that".into(),
            ))
        }
        Provider::Claude if !key.starts_with("sk-ant-") => return Err(Error::InvalidInput("Claude API keys start with sk-ant-".into())),
        Provider::Openai if !key.starts_with("sk-") => return Err(Error::InvalidInput("OpenAI API keys start with sk-".into())),
        _ => {}
    }
    if let Some(m) = &s.model {
        let m = m.trim();
        if !m.is_empty() && (m.len() > 80 || !m.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._:/".contains(&b))) {
            return Err(Error::InvalidInput("a model name is letters, digits and - . _ : / only".into()));
        }
    }
    Ok(())
}

/// A pairing code: 8 characters without look-alikes (no 0/O, 1/I).
pub fn new_pair_code() -> String {
    use chacha20poly1305::aead::rand_core::RngCore;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 8];
    chacha20poly1305::aead::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect()
}

pub fn parse_status(out: &str) -> AgentStatus {
    let mut st = AgentStatus::default();
    for line in out.lines() {
        if line == "@@installed" {
            st.installed = true;
        } else if let Some(a) = line.strip_prefix("@@active ") {
            st.running = a.trim() == "active";
        } else if let Some(j) = line.strip_prefix("@@json ") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(j) {
                let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
                st.version = s("version");
                st.provider = s("provider");
                st.model = s("model");
                st.bot = s("bot");
                st.owner = s("owner");
                st.alerts = v.get("alerts").and_then(|x| x.as_bool()).unwrap_or(true);
                st.pair_code_active = v.get("pairCodeActive").and_then(|x| x.as_bool()).unwrap_or(false);
                st.paired_at = v.get("pairedAt").and_then(|x| x.as_i64());
                st.daily_report = s("dailyReport");
                st.alerting = v
                    .get("alerting")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
            }
        } else if let Some(l) = line.strip_prefix("@@log ") {
            st.recent.push(l.to_string());
        }
    }
    st.latest_version = VERSION.to_string();
    // An agent that doesn't say its version is older than any that does.
    st.update_available = st.installed && st.version.as_deref() != Some(VERSION);
    st
}

// ---------- checks from this computer, before anything goes to the server ----------

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| Error::Other(e.to_string()))
}

/// The bot's @username, or why the token doesn't work.
pub async fn telegram_bot_name(token: &str) -> Result<String> {
    let url = format!("https://api.telegram.org/bot{}/getMe", token.trim());
    let r = http()?
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Other(format!("couldn't reach Telegram: {}", crate::dns::net_error(&e))))?;
    let body = r.bytes().await.map_err(|e| Error::Other(e.to_string()))?;
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| Error::Other(e.to_string()))?;
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return Err(Error::InvalidInput(
            "Telegram doesn't accept this bot token: copy it again from @BotFather".into(),
        ));
    }
    v.pointer("/result/username")
        .and_then(|x| x.as_str())
        .map(String::from)
        .ok_or_else(|| Error::Other("Telegram didn't say the bot's name".into()))
}

/// A model the key can use, for the setup's model list.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiModel {
    pub id: String,
    pub name: String,
}

/// What Check found: the bot's @name and the models the key can use.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCheck {
    pub bot: String,
    pub models: Vec<AiModel>,
}

/// The provider's own words for an error reply (`{"error": {"message": …}}`
/// on both APIs), so "400" becomes something a person can act on.
pub fn provider_error(status: u16, body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| {
        v.pointer("/error/message")
            .or_else(|| v.get("message"))
            .and_then(|m| m.as_str())
            .map(str::to_string)
    });
    let what = match status {
        401 => "the AI provider refused this API key",
        403 => "this API key isn't allowed to do that",
        404 => "the AI provider doesn't know that address",
        429 => "the AI provider is rate-limiting this key, or it has no credit left",
        s if s >= 500 => "the AI provider had a problem on its side",
        _ => "the AI provider rejected the request",
    };
    match msg {
        Some(m) if !m.trim().is_empty() => format!("{what} ({status}): {}", m.trim()),
        _ => format!("{what} ({status})"),
    }
}

/// Models worth offering, newest first. OpenAI lists everything (embeddings,
/// audio, images); only chat models can run the agent.
pub fn parse_models(provider: Provider, body: &str) -> Vec<AiModel> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return vec![];
    };
    let Some(items) = v.get("data").and_then(|d| d.as_array()) else {
        return vec![];
    };
    let mut out: Vec<(String, AiModel)> = items
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?.to_string();
            match provider {
                Provider::Claude => {
                    let name = m.get("display_name").and_then(|x| x.as_str()).unwrap_or(&id).to_string();
                    let when = m.get("created_at").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    Some((when, AiModel { id, name }))
                }
                Provider::Openai => {
                    const NOT_CHAT: [&str; 12] = [
                        "embedding",
                        "audio",
                        "realtime",
                        "transcribe",
                        "tts",
                        "image",
                        "dall-e",
                        "whisper",
                        "moderation",
                        "search",
                        "instruct",
                        "codex",
                    ];
                    let chat = (id.starts_with("gpt-")
                        || id.starts_with("chatgpt-")
                        || (id.starts_with('o') && id[1..].starts_with(|c: char| c.is_ascii_digit())))
                        && !NOT_CHAT.iter().any(|w| id.contains(w));
                    let when = m.get("created").and_then(|x| x.as_i64()).unwrap_or(0);
                    chat.then(|| (format!("{when:020}"), AiModel { name: id.clone(), id }))
                }
            }
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    out.into_iter().map(|(_, m)| m).collect()
}

/// Whether the AI key works, and which models it can use (listing models is
/// free: nothing is generated).
pub async fn check_ai_key(provider: Provider, key: &str) -> Result<Vec<AiModel>> {
    let c = http()?;
    let req = match provider {
        Provider::Claude => c
            .get("https://api.anthropic.com/v1/models?limit=100")
            .header("x-api-key", key.trim())
            .header("anthropic-version", "2026-10-21"),
        Provider::Openai => c.get("https://api.openai.com/v1/models").bearer_auth(key.trim()),
    };
    let r = req
        .send()
        .await
        .map_err(|e| Error::Other(format!("couldn't reach the AI provider: {}", crate::dns::net_error(&e))))?;
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    match status {
        200 => {
            let models = parse_models(provider, &body);
            if models.is_empty() {
                return Err(Error::Other("the key works, but the provider lists no chat models for it".into()));
            }
            Ok(models)
        }
        401 | 403 => Err(Error::InvalidInput(provider_error(status, &body))),
        s => Err(Error::Other(provider_error(s, &body))),
    }
}

impl SshSession {
    /// The agent's state on this server (root's view: its files are root-only).
    pub async fn agent_status(&self) -> Result<AgentStatus> {
        let (o, _) = self.exec_inspect(STATUS_SCRIPT).await?;
        Ok(parse_status(&o.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> AgentSetup {
        AgentSetup {
            telegram_token: "123456789:AAH-abcdefghijklmnopqrstuvwxyz012345".into(),
            provider: Provider::Claude,
            api_key: "sk-ant-api03-abcdefghijklmnopqrstuvwxyz".into(),
            model: None,
            alerts: true,
        }
    }

    #[test]
    fn version_matches_the_script() {
        assert!(
            AGENT_PY.contains(&format!("VERSION = \"{VERSION}\"")),
            "bump VERSION in both places"
        );
        assert!(AGENT_PY.contains("CLAUDE_MODEL = \"claude-opus-5-5\""));
    }

    #[test]
    fn validates_tokens_keys_and_models() {
        assert!(validate_setup(&setup()).is_ok());
        let bad = |f: &dyn Fn(&mut AgentSetup)| {
            let mut s = setup();
            f(&mut s);
            validate_setup(&s).is_err()
        };
        assert!(bad(&|s| s.telegram_token = "hello".into()));
        assert!(bad(&|s| s.telegram_token = "12ab:AAH-abcdefghijklmnopqrstuvwxyz012345".into()));
        assert!(
            bad(&|s| s.api_key = "sk-proj-abcdefghijklmnopqrstuvwxyz".into()),
            "an OpenAI key for Claude"
        );
        assert!(bad(&|s| s.model = Some("x; rm -rf /".into())));
        let mut s = setup();
        s.provider = Provider::Openai;
        s.api_key = "sk-proj-abcdefghijklmnopqrstuvwxyz".into();
        s.model = Some("gpt-5".into());
        assert!(validate_setup(&s).is_ok());
    }

    #[test]
    fn config_carries_the_keys_only_in_json() {
        let j: serde_json::Value = serde_json::from_str(&config_json(&setup(), "orbital_test_bot", "prod-web-01").unwrap()).unwrap();
        assert_eq!(j["provider"], "claude");
        assert_eq!(j["bot_username"], "orbital_test_bot");
        assert!(j["model"].is_null(), "the agent's default model is used");
        // The scripts that run on the server never contain a key.
        for s in [
            PREPARE_SCRIPT,
            WRITE_AGENT,
            WRITE_CONFIG,
            WRITE_UNIT,
            SET_PAIR_CODE,
            REMOVE,
            STATUS_SCRIPT,
        ] {
            assert!(!s.contains("sk-ant") && !s.contains("AAH-"));
        }
        assert!(WRITE_CONFIG.starts_with("umask 077") && WRITE_CONFIG.contains("chmod 600"));
    }

    #[test]
    fn subscription_tokens_are_explained() {
        let mut s = setup();
        s.api_key = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz".into();
        let e = validate_setup(&s).unwrap_err().to_string();
        assert!(e.contains("subscription") && e.contains("console.anthropic.com"), "{e}");
    }

    #[test]
    fn provider_errors_say_why() {
        let anthropic =
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"OAuth authentication is currently not supported."}}"#;
        assert_eq!(
            provider_error(400, anthropic),
            "the AI provider rejected the request (400): OAuth authentication is currently not supported."
        );
        let openai = r#"{"error":{"message":"Incorrect API key provided: sk-proj-****abcd.","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert!(provider_error(401, openai).starts_with("the AI provider refused this API key (401): Incorrect API key"));
        assert_eq!(
            provider_error(500, "<html>oops</html>"),
            "the AI provider had a problem on its side (500)"
        );
    }

    #[test]
    fn lists_models_newest_first_and_only_chat_ones() {
        let claude = r#"{"data":[{"id":"claude-haiku-5-5","display_name":"Claude Haiku 5.5","created_at":"2026-09-01T00:00:00Z"},{"id":"claude-opus-5-5","display_name":"Claude Opus 5.5","created_at":"2026-10-01T00:00:00Z"}]}"#;
        let m = parse_models(Provider::Claude, claude);
        assert_eq!(
            m.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            ["claude-opus-5-5", "claude-haiku-5-5"]
        );
        assert_eq!(m[0].name, "Claude Opus 5.5");
        let openai = r#"{"data":[{"id":"gpt-5","created":300},{"id":"text-embedding-3-large","created":400},{"id":"gpt-4o-audio-preview","created":350},{"id":"o3","created":200},{"id":"gpt-image-1","created":500},{"id":"whisper-1","created":100},{"id":"omni-moderation-latest","created":450}]}"#;
        assert_eq!(
            parse_models(Provider::Openai, openai)
                .iter()
                .map(|x| x.id.as_str())
                .collect::<Vec<_>>(),
            ["gpt-5", "o3"]
        );
        assert!(parse_models(Provider::Claude, "not json").is_empty());
    }

    #[test]
    fn daily_summary_and_test_message() {
        assert!(set_daily_command(Some("09:00")).unwrap().ends_with("set daily 09:00"));
        assert!(set_daily_command(None).unwrap().ends_with("set daily off"));
        for bad in ["9:00", "24:00", "09:60", "nine", "09:00; reboot"] {
            assert!(set_daily_command(Some(bad)).is_err(), "{bad}");
        }
        assert_eq!(parse_test(r#"{"ok": true, "to": "siroj"}"#, "").unwrap().as_deref(), Some("siroj"));
        assert!(parse_test(r#"{"error": "not paired yet"}"#, "")
            .unwrap_err()
            .to_string()
            .contains("not paired"));
        assert!(parse_test("", "unknown command test")
            .unwrap_err()
            .to_string()
            .contains("Update agent"));
    }

    #[test]
    fn an_agent_without_a_version_is_outdated() {
        let st = parse_status("@@installed\n@@active active\n");
        assert!(st.installed && st.update_available);
        let current = parse_status(&format!("@@installed\n@@json {{\"version\": \"{VERSION}\"}}\n"));
        assert!(!current.update_available);
    }

    #[test]
    fn changing_the_model() {
        assert_eq!(
            set_model_command(" claude-sonnet-5-5 ").unwrap(),
            "/opt/orbital-agent/venv/bin/python /opt/orbital-agent/agent.py set model claude-sonnet-5-5"
        );
        assert!(set_model_command("x; rm -rf /").is_err());
        assert!(set_model_command("").is_err());
        let ok = parse_model_list(
            r#"[{"id":"claude-opus-5-5","name":"Claude Opus 5.5"},{"id":"claude-sonnet-5-5"}]"#,
            "",
        )
        .unwrap();
        assert_eq!(
            ok.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["Claude Opus 5.5", "claude-sonnet-5-5"]
        );
        let e = parse_model_list(r#"{"error": "AuthenticationError: invalid x-api-key"}"#, "")
            .unwrap_err()
            .to_string();
        assert!(e.contains("invalid x-api-key"), "{e}");
        let old = parse_model_list("", "unknown command models").unwrap_err().to_string();
        assert!(old.contains("Update agent"), "{old}");
    }

    #[test]
    fn pair_codes_are_readable_and_random() {
        let a = new_pair_code();
        assert_eq!(a.len(), 8);
        assert!(a.bytes().all(|b| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&b)));
        assert_ne!(a, new_pair_code());
    }

    #[test]
    fn parses_status() {
        let out = "@@installed\n@@active active\n@@json {\"version\": \"1.2.9\", \"provider\": \"claude\", \"model\": \"claude-opus-5-5\", \"bot\": \"orbital_test_bot\", \"alerts\": true, \"pairCodeActive\": false, \"owner\": \"siroj\", \"pairedAt\": 1763300000, \"alerting\": [\"disk:/\"]}\n@@log {\"event\": \"start\"}\n";
        let st = parse_status(out);
        assert!(st.installed && st.running && st.update_available);
        assert_eq!(st.owner.as_deref(), Some("siroj"));
        assert_eq!(st.alerting, ["disk:/"]);
        assert_eq!(st.recent.len(), 1);
        let empty = parse_status("");
        assert!(!empty.installed && !empty.update_available && empty.latest_version == VERSION);
    }

    /// The agent's own checks (secret paths refused, names validated, tool
    /// schemas strict), when python3 is here to run them. Unix only: the agent
    /// runs on Linux servers, and its path checks are about Unix paths.
    #[cfg(unix)]
    #[test]
    fn the_agent_passes_its_selftest() {
        let dir = std::env::temp_dir().join(format!("orbital-agent-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("agent.py");
        std::fs::write(&file, AGENT_PY).unwrap();
        match std::process::Command::new("python3").arg(&file).arg("selftest").output() {
            Ok(o) => assert!(
                o.status.success(),
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(_) => eprintln!("python3 isn't installed here; skipped"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
