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
pub const VERSION: &str = "1.3.0";

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
    /// The installed agent is older than this Orbital's.
    pub update_available: bool,
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
  echo "Installing Python …"
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
echo "Installing the AI libraries (anthropic, openai) …"
/opt/orbital-agent/venv/bin/pip install --quiet --disable-pip-version-check --upgrade anthropic openai
echo "Python ready"
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
    st.update_available = st.installed && st.version.as_deref().is_some_and(|v| v != VERSION);
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

/// Whether the AI key works (lists models: free, nothing is generated).
pub async fn check_ai_key(provider: Provider, key: &str) -> Result<()> {
    let c = http()?;
    let req = match provider {
        Provider::Claude => c
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key.trim())
            .header("anthropic-version", "2023-06-01"),
        Provider::Openai => c.get("https://api.openai.com/v1/models").bearer_auth(key.trim()),
    };
    let r = req
        .send()
        .await
        .map_err(|e| Error::Other(format!("couldn't reach the AI provider: {}", crate::dns::net_error(&e))))?;
    match r.status().as_u16() {
        200 => Ok(()),
        401 | 403 => Err(Error::InvalidInput("the AI provider refused this API key".into())),
        429 => Ok(()), // the key works; it's just busy
        s => Err(Error::Other(format!("the AI provider answered {s}"))),
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
        assert_eq!(parse_status(""), AgentStatus::default());
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
