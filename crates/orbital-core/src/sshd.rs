//! A security review of the SSH server: the settings sshd actually uses
//! (`sshd -T`, after all includes and defaults), checked against common
//! hardening advice, plus who is logged in right now.
//!
//! Read-only. Changes go through the editor, where safe save runs `sshd -t`
//! and rolls back a config that would not load.

use crate::error::Result;
use crate::ssh::SshSession;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Bad,
    Warn,
    Ok,
    Info,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshdCheck {
    pub id: &'static str,
    pub title: &'static str,
    pub level: Level,
    /// The setting as sshd uses it, e.g. `PermitRootLogin`.
    pub setting: &'static str,
    pub value: String,
    /// What it means and what to change.
    pub advice: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoginSession {
    pub user: String,
    pub tty: String,
    pub since: String,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshdReport {
    pub version: Option<String>,
    /// `sshd -t` passed.
    pub test_ok: bool,
    pub test_output: String,
    /// The effective settings could be read (needs root).
    pub readable: bool,
    pub checks: Vec<SshdCheck>,
    /// Every effective setting, lower-case names as sshd prints them.
    pub settings: BTreeMap<String, String>,
    /// sshd_config and its drop-ins, in load order.
    pub config_files: Vec<String>,
    pub sessions: Vec<LoginSession>,
}

pub(crate) const SCRIPT: &str = r#"export LC_ALL=C
S=$(command -v sshd 2>/dev/null || echo /usr/sbin/sshd)
echo "@@version $(ssh -V 2>&1 | head -1)"
echo @@test
out=$("$S" -t 2>&1); code=$?
printf '%s\n' "$out"
echo "@@test_exit $code"
echo @@effective
"$S" -T 2>/dev/null
echo @@files
for f in /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf; do [ -f "$f" ] && echo "$f"; done
echo @@who
who 2>/dev/null
"#;

fn get<'a>(m: &'a BTreeMap<String, String>, k: &str) -> Option<&'a str> {
    m.get(k).map(String::as_str)
}

/// Judge the effective settings. `None` values mean sshd did not report them.
pub fn checks(m: &BTreeMap<String, String>) -> Vec<SshdCheck> {
    let mut out = Vec::new();
    let mut push = |id, title, setting, level, value: &str, advice: &str| {
        out.push(SshdCheck {
            id,
            title,
            setting,
            level,
            value: value.to_string(),
            advice: advice.to_string(),
        })
    };

    if let Some(v) = get(m, "permitrootlogin") {
        let (level, advice) = match v {
            "no" => (Level::Ok, "Nobody can log in as root directly; admins log in as themselves and use sudo."),
            "prohibit-password" | "without-password" => (
                Level::Ok,
                "root can log in with a key only. Setting it to no is stricter: log in as yourself and use sudo.",
            ),
            "forced-commands-only" => (Level::Ok, "root can only run commands fixed in authorized_keys."),
            _ => (
                Level::Bad,
                "Anyone who guesses root's password owns the server. Set PermitRootLogin to no (or prohibit-password) once you can log in as another user with sudo.",
            ),
        };
        push("root-login", "Root login", "PermitRootLogin", level, v, advice);
    }

    let password = get(m, "passwordauthentication");
    if let Some(v) = password {
        let (level, advice) = if v == "yes" {
            (
                Level::Warn,
                "Bots try passwords on every public server all day. Once your key works, set PasswordAuthentication to no. Make sure this profile logs in with a key first, or you will lock yourself out.",
            )
        } else {
            (Level::Ok, "Only keys are accepted, so password guessing can't work.")
        };
        push("password-login", "Password login", "PasswordAuthentication", level, v, advice);
    }

    let kbd = get(m, "kbdinteractiveauthentication").or_else(|| get(m, "challengeresponseauthentication"));
    if let (Some(v), Some("no")) = (kbd, password) {
        if v == "yes" && get(m, "usepam") == Some("yes") {
            push(
                "kbd-interactive",
                "Keyboard-interactive login",
                "KbdInteractiveAuthentication",
                Level::Warn,
                v,
                "With PAM this still asks for a password, even though PasswordAuthentication is off. Set KbdInteractiveAuthentication to no as well.",
            );
        }
    }

    if let Some(v) = get(m, "permitemptypasswords") {
        if v == "yes" {
            push(
                "empty-passwords",
                "Empty passwords",
                "PermitEmptyPasswords",
                Level::Bad,
                v,
                "Accounts without a password can log in with no password at all. Set PermitEmptyPasswords to no.",
            );
        }
    }

    if let Some(v) = get(m, "pubkeyauthentication") {
        if v == "no" {
            push(
                "pubkey",
                "Key login",
                "PubkeyAuthentication",
                Level::Warn,
                v,
                "Keys are turned off, so passwords are the only way in. Set PubkeyAuthentication to yes.",
            );
        }
    }

    if let Some(v) = get(m, "maxauthtries") {
        let n: u32 = v.parse().unwrap_or(6);
        let (level, advice) = if n > 4 {
            (
                Level::Warn,
                "Each connection may try this many passwords or keys. 3 or 4 slows guessing down.",
            )
        } else {
            (Level::Ok, "Few attempts per connection.")
        };
        push("max-auth-tries", "Attempts per connection", "MaxAuthTries", level, v, advice);
    }

    if let Some(v) = get(m, "x11forwarding") {
        if v == "yes" {
            push(
                "x11",
                "X11 forwarding",
                "X11Forwarding",
                Level::Warn,
                v,
                "Lets remote graphical apps reach your screen. Servers rarely need it: set X11Forwarding to no.",
            );
        }
    }

    if let Some(v) = get(m, "logingracetime") {
        let secs: u32 = v.parse().unwrap_or(120);
        if secs > 60 {
            push(
                "grace",
                "Time to log in",
                "LoginGraceTime",
                Level::Info,
                v,
                "Seconds an unauthenticated connection may stay open. 30 frees slots held by bots sooner.",
            );
        }
    }

    let users: Vec<&str> = ["allowusers", "allowgroups"].iter().filter_map(|k| get(m, k)).collect();
    push(
        "allow-list",
        "Who may log in",
        "AllowUsers / AllowGroups",
        Level::Info,
        &if users.is_empty() {
            "everyone with an account".to_string()
        } else {
            users.join(", ")
        },
        if users.is_empty() {
            "Any account with a valid key or password can log in. AllowUsers or AllowGroups limits SSH to the people who need it."
        } else {
            "Only these users or groups may log in over SSH."
        },
    );

    if let Some(v) = get(m, "port") {
        push(
            "port",
            "Port",
            "Port",
            Level::Info,
            v,
            if v == "22" {
                "The standard port. Moving it cuts log noise from bots but doesn't stop a real attacker; keys and fail2ban matter more."
            } else {
                "A non-standard port. Remember to allow it in the firewall."
            },
        );
    }

    out.sort_by_key(|c| c.level);
    out
}

pub fn parse(out: &str) -> SshdReport {
    let mut r = SshdReport::default();
    let mut section = "";
    let mut test: Vec<&str> = Vec::new();
    for line in out.lines() {
        let l = line.trim_end();
        if let Some(v) = l.strip_prefix("@@version") {
            let v = v.trim();
            r.version = (!v.is_empty()).then(|| v.split(',').next().unwrap_or(v).trim().to_string());
            continue;
        }
        if let Some(code) = l.strip_prefix("@@test_exit") {
            r.test_ok = code.trim() == "0";
            section = "";
            continue;
        }
        if let Some(s) = l.strip_prefix("@@") {
            section = match s {
                "test" => "test",
                "effective" => "effective",
                "files" => "files",
                "who" => "who",
                _ => "",
            };
            continue;
        }
        match section {
            "test" if !l.trim().is_empty() => test.push(l),
            "effective" => {
                if let Some((k, v)) = l.split_once(' ') {
                    // Repeated keys (port, listenaddress, …) are joined.
                    r.settings
                        .entry(k.to_string())
                        .and_modify(|e| {
                            e.push_str(", ");
                            e.push_str(v.trim());
                        })
                        .or_insert_with(|| v.trim().to_string());
                }
            }
            "files" if l.starts_with('/') => r.config_files.push(l.to_string()),
            "who" => {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() >= 3 {
                    let from = l
                        .rfind('(')
                        .and_then(|i| l[i + 1..].strip_suffix(')'))
                        .map(String::from)
                        .filter(|s| !s.is_empty());
                    let end = if from.is_some() { f.len() - 1 } else { f.len() };
                    r.sessions.push(LoginSession {
                        user: f[0].to_string(),
                        tty: f[1].to_string(),
                        since: f[2..end.max(2)].join(" "),
                        from,
                    });
                }
            }
            _ => {}
        }
    }
    r.test_output = test.join("\n");
    r.readable = !r.settings.is_empty();
    r.checks = checks(&r.settings);
    r
}

impl SshSession {
    pub async fn sshd_report(&self) -> Result<SshdReport> {
        let (o, _) = self.exec_inspect(SCRIPT).await?;
        Ok(parse(&o.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@version OpenSSH_9.6p1 Ubuntu-3ubuntu13.5, OpenSSL 3.0.13 30 Jan 2024
@@test
@@test_exit 0
@@effective
port 22
port 2222
permitrootlogin yes
passwordauthentication no
kbdinteractiveauthentication yes
usepam yes
pubkeyauthentication yes
permitemptypasswords no
maxauthtries 6
x11forwarding yes
logingracetime 120
@@files
/etc/ssh/sshd_config
/etc/ssh/sshd_config.d/50-cloud-init.conf
@@who
deploy   pts/0        2026-11-15 10:01 (203.0.113.9)
root     tty1         2026-11-14 08:00
";

    #[test]
    fn parses_and_judges_settings() {
        let r = parse(OUT);
        assert!(r.test_ok && r.readable);
        assert_eq!(r.version.as_deref(), Some("OpenSSH_9.6p1 Ubuntu-3ubuntu13.5"));
        assert_eq!(r.settings["port"], "22, 2222");
        assert_eq!(r.config_files.len(), 2);
        assert_eq!(r.sessions.len(), 2);
        assert_eq!(r.sessions[0].from.as_deref(), Some("203.0.113.9"));
        assert_eq!(r.sessions[0].since, "2026-11-15 10:01");
        assert_eq!(r.sessions[1].from, None);
        assert_eq!(r.sessions[1].since, "2026-11-14 08:00");

        let level = |id: &str| r.checks.iter().find(|c| c.id == id).map(|c| c.level);
        assert_eq!(level("root-login"), Some(Level::Bad));
        assert_eq!(level("password-login"), Some(Level::Ok));
        assert_eq!(level("kbd-interactive"), Some(Level::Warn));
        assert_eq!(level("max-auth-tries"), Some(Level::Warn));
        assert_eq!(level("x11"), Some(Level::Warn));
        assert_eq!(level("empty-passwords"), None);
        assert_eq!(level("grace"), Some(Level::Info));
        // Worst first.
        assert_eq!(r.checks[0].level, Level::Bad);
        assert!(r.checks.windows(2).all(|w| w[0].level <= w[1].level));
    }

    #[test]
    fn failed_test_and_unreadable_settings() {
        let r = parse("@@test\n/etc/ssh/sshd_config line 12: Bad configuration option: PermitRootLgin\n@@test_exit 255\n@@effective\n@@files\n@@who\n");
        assert!(!r.test_ok && !r.readable);
        assert!(r.test_output.contains("line 12"));
        // Only the always-present allow-list note remains.
        assert_eq!(r.checks.len(), 1);
    }

    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(out.status.success());
        let _ = parse(&String::from_utf8_lossy(&out.stdout));
    }
}
