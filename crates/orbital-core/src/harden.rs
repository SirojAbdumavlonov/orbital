//! One-click SSH hardening: chosen settings go into one drop-in Orbital owns,
//! `/etc/ssh/sshd_config.d/10-orbital-hardening.conf`. sshd uses the first
//! value it reads for most settings, and `10-` sorts before the usual
//! `50-cloud-init.conf`, so these win over the drop-ins images ship with.
//!
//! Only fixed settings and values are written. The file is checked with
//! `sshd -t` and removed again if sshd wouldn't start; sshd is then reloaded,
//! which keeps open connections. The engine refuses changes that would lock
//! this connection out unless the user forces them.

use crate::error::{Error, Result};
use crate::shell::quote;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DROPIN: &str = "/etc/ssh/sshd_config.d/10-orbital-hardening.conf";

/// Settings Orbital can set, with the values it may write.
const ALLOWED: &[(&str, &[&str])] = &[
    ("PermitRootLogin", &["no", "prohibit-password"]),
    ("PasswordAuthentication", &["no"]),
    ("KbdInteractiveAuthentication", &["no"]),
    ("PermitEmptyPasswords", &["no"]),
    ("MaxAuthTries", &["3", "4"]),
    ("X11Forwarding", &["no"]),
    ("LoginGraceTime", &["30", "60"]),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HardenState {
    /// The main config includes sshd_config.d, so a drop-in is read.
    pub includes_dropins: bool,
    /// Settings in Orbital's drop-in now.
    pub current: BTreeMap<String, String>,
    pub dropin: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HardenResult {
    pub state: HardenState,
    /// Settings written that sshd doesn't use (an earlier file sets them):
    /// `(setting, value sshd uses)`.
    pub not_in_effect: Vec<(String, String)>,
}

/// What the lock-out check needs to know about this connection.
#[derive(Debug, Clone, Copy)]
pub struct Access {
    /// This profile signs in with a password.
    pub password_login: bool,
    /// The login user is root.
    pub root: bool,
    /// A key from this computer can sign in as the login user.
    pub key_from_here: bool,
}

pub fn validate(choices: &BTreeMap<String, String>) -> Result<()> {
    for (k, v) in choices {
        let ok = ALLOWED.iter().any(|(s, vals)| s == k && vals.contains(&v.as_str()));
        if !ok {
            return Err(Error::InvalidInput(format!("Orbital doesn't set {k} to {v:?}")));
        }
    }
    Ok(())
}

pub fn render(choices: &BTreeMap<String, String>) -> Result<String> {
    validate(choices)?;
    let mut out = String::from("# Written by Orbital (SSH server → Harden). Remove it there to undo.\n");
    for (k, v) in choices {
        out.push_str(&format!("{k} {v}\n"));
    }
    Ok(out)
}

/// Why these settings would lock this connection out, if they would.
pub fn lockout(choices: &BTreeMap<String, String>, a: Access) -> Option<String> {
    let no = |k: &str| choices.get(k).map(String::as_str) == Some("no");
    if (no("PasswordAuthentication") || no("KbdInteractiveAuthentication")) && a.password_login {
        return Some(
            "This connection signs in with a password. With password login off, Orbital (and you) couldn't sign in again. \
             Allow a key in the SSH keys tab and switch this server's profile to it first."
                .into(),
        );
    }
    if no("PasswordAuthentication") && !a.key_from_here {
        return Some(
            "No key from this computer can sign in to this account, so with password login off you'd have no way back in. \
             Allow one of your keys in the SSH keys tab first."
                .into(),
        );
    }
    if a.root {
        match choices.get("PermitRootLogin").map(String::as_str) {
            Some("no") => {
                return Some(
                    "This connection signs in as root. With root login off it couldn't sign in again. \
                     Create a user with sudo (Users tab), sign in as that user, then turn root login off."
                        .into(),
                )
            }
            Some("prohibit-password") if a.password_login => {
                return Some("This connection signs in as root with a password, which this setting turns off.".into())
            }
            _ => {}
        }
    }
    None
}

pub const STATE_SCRIPT: &str = "export LC_ALL=C\n\
grep -Eiq '^[[:space:]]*Include[[:space:]]+.*sshd_config\\.d' /etc/ssh/sshd_config 2>/dev/null && echo @@include\n\
[ -f /etc/ssh/sshd_config.d/10-orbital-hardening.conf ] && { echo @@dropin; cat /etc/ssh/sshd_config.d/10-orbital-hardening.conf; }\n\
true\n";

pub fn parse_state(out: &str) -> HardenState {
    let mut current = BTreeMap::new();
    let mut in_file = false;
    for l in out.lines() {
        match l.trim() {
            "@@include" => continue,
            "@@dropin" => in_file = true,
            t if in_file && !t.is_empty() && !t.starts_with('#') => {
                if let Some((k, v)) = t.split_once(char::is_whitespace) {
                    current.insert(k.to_string(), v.trim().to_string());
                }
            }
            _ => {}
        }
    }
    HardenState {
        includes_dropins: out.lines().any(|l| l.trim() == "@@include"),
        current,
        dropin: DROPIN.into(),
    }
}

/// Install the drop-in from stdin (or remove it with `remove`), check with
/// `sshd -t`, put the previous file back on failure, reload sshd, then print
/// the effective settings (`sshd -T`).
pub fn apply_script(remove: bool) -> String {
    let f = quote(DROPIN);
    let install = if remove {
        format!("cat >/dev/null; rm -f -- {f}")
    } else {
        format!("cat > \"$t\" && mkdir -p /etc/ssh/sshd_config.d && cp \"$t\" {f} && chmod 600 {f}")
    };
    format!(
        "export LC_ALL=C\n\
         t=$(mktemp) || exit 1; trap 'rm -f \"$t\" \"$t.old\"' EXIT\n\
         had=0; [ -e {f} ] && {{ cp -p {f} \"$t.old\"; had=1; }}\n\
         {install} || exit 1\n\
         S=$(command -v sshd || echo /usr/sbin/sshd)\n\
         mkdir -p /run/sshd 2>/dev/null\n\
         if ! out=$(\"$S\" -t 2>&1); then printf '%s\\n' \"$out\"; if [ $had = 1 ]; then cp -p \"$t.old\" {f}; else rm -f -- {f}; fi; echo @@restored; exit 3; fi\n\
         systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || service ssh reload 2>/dev/null || service sshd reload 2>/dev/null || echo @@noreload\n\
         echo @@effective\n\
         \"$S\" -T 2>/dev/null\n"
    )
}

/// Settings the drop-in sets that sshd -T reports differently.
pub fn not_in_effect(choices: &BTreeMap<String, String>, apply_out: &str) -> Vec<(String, String)> {
    let Some((_, eff)) = apply_out.split_once("@@effective") else {
        return vec![];
    };
    let effective: BTreeMap<String, String> = eff
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(k, v)| (k.to_lowercase(), v.trim().to_string())))
        .collect();
    choices
        .iter()
        .filter_map(|(k, v)| {
            let want = if k == "PermitRootLogin" && v == "prohibit-password" {
                "without-password"
            } else {
                v
            };
            match effective.get(&k.to_lowercase()) {
                Some(e) if e == v || e == want => None,
                // LoginGraceTime prints in seconds already; anything else is a mismatch.
                Some(e) => Some((k.clone(), e.clone())),
                None => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
    const SAFE: Access = Access {
        password_login: false,
        root: false,
        key_from_here: true,
    };

    #[test]
    fn writes_only_allowed_settings() {
        let text = render(&c(&[("PasswordAuthentication", "no"), ("MaxAuthTries", "3")])).unwrap();
        assert!(text.ends_with("MaxAuthTries 3\nPasswordAuthentication no\n"), "{text}");
        assert!(render(&c(&[("PasswordAuthentication", "yes")])).is_err());
        assert!(render(&c(&[("PermitRootLogin", "no\nPort 23")])).is_err());
        assert!(render(&c(&[("Port", "2222")])).is_err());
    }

    #[test]
    fn refuses_what_would_lock_this_connection_out() {
        let pw_off = c(&[("PasswordAuthentication", "no")]);
        assert!(lockout(&pw_off, SAFE).is_none());
        assert!(lockout(
            &pw_off,
            Access {
                password_login: true,
                ..SAFE
            }
        )
        .is_some());
        assert!(lockout(
            &pw_off,
            Access {
                key_from_here: false,
                ..SAFE
            }
        )
        .is_some());
        assert!(lockout(
            &c(&[("KbdInteractiveAuthentication", "no")]),
            Access {
                password_login: true,
                ..SAFE
            }
        )
        .is_some());
        let root_off = c(&[("PermitRootLogin", "no")]);
        assert!(lockout(&root_off, SAFE).is_none());
        assert!(lockout(&root_off, Access { root: true, ..SAFE }).is_some());
        assert!(lockout(&c(&[("PermitRootLogin", "prohibit-password")]), Access { root: true, ..SAFE }).is_none());
        assert!(lockout(
            &c(&[("PermitRootLogin", "prohibit-password")]),
            Access {
                root: true,
                password_login: true,
                key_from_here: true
            }
        )
        .is_some());
        assert!(lockout(
            &c(&[("X11Forwarding", "no")]),
            Access {
                password_login: true,
                root: true,
                key_from_here: false
            }
        )
        .is_none());
    }

    #[test]
    fn reads_state_and_effect() {
        let s = parse_state("@@include\n@@dropin\n# Written by Orbital\nMaxAuthTries 3\nX11Forwarding no\n");
        assert!(s.includes_dropins);
        assert_eq!(s.current, c(&[("MaxAuthTries", "3"), ("X11Forwarding", "no")]));
        assert!(!parse_state("").includes_dropins);
        let out = "ok\n@@effective\nmaxauthtries 6\nx11forwarding no\npermitrootlogin without-password\n";
        let chosen = c(&[
            ("MaxAuthTries", "3"),
            ("X11Forwarding", "no"),
            ("PermitRootLogin", "prohibit-password"),
        ]);
        assert_eq!(not_in_effect(&chosen, out), vec![("MaxAuthTries".to_string(), "6".to_string())]);
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn scripts_are_valid_sh() {
        for s in [apply_script(false), apply_script(true), STATE_SCRIPT.to_string()] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&s).output().unwrap();
            assert!(o.status.success(), "{s}\n{}", String::from_utf8_lossy(&o.stderr));
        }
    }
}
