//! Linux user accounts: list them with groups, shells, sign-in status and SSH
//! keys; create them with every common option; change, lock and delete them.
//!
//! Commands are built from validated input only. A new password goes to
//! `chpasswd` over stdin, so it is never on a command line or in the audit
//! log. The account used for the current connection, and root, are protected
//! from changes that would lock the user out.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_path};
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserAccount {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub primary_group: Option<String>,
    /// Supplementary groups.
    pub groups: Vec<String>,
    pub full_name: String,
    pub home: String,
    pub shell: String,
    /// A system account (below UID_MIN), not a person.
    pub system: bool,
    /// Member of sudo, wheel or admin.
    pub admin: bool,
    /// `P` (password set), `L` (locked), `NP` (no password), from `passwd -S`.
    pub password_status: Option<String>,
    pub password_changed: Option<String>,
    /// Lines in ~/.ssh/authorized_keys, when readable.
    pub ssh_keys: Option<u32>,
    /// The shell lets the user log in (not nologin/false).
    pub can_login: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsersReport {
    pub users: Vec<UserAccount>,
    pub groups: Vec<GroupInfo>,
    /// Login shells from /etc/shells.
    pub shells: Vec<String>,
    /// First UID for people (login.defs UID_MIN), usually 1000.
    pub uid_min: u32,
    /// The group that grants sudo here: `sudo` (Debian/Ubuntu) or `wheel` (RHEL).
    pub admin_group: Option<String>,
    /// Sign-in status and keys could be read (needs root).
    pub complete: bool,
}

pub(crate) const LIST_SCRIPT: &str = r#"export LC_ALL=C
echo @@passwd; getent passwd 2>/dev/null || cat /etc/passwd
echo @@group; getent group 2>/dev/null || cat /etc/group
echo @@shells; grep -v '^[[:space:]]*#' /etc/shells 2>/dev/null
echo "@@uidmin $(awk '$1 == "UID_MIN" {print $2}' /etc/login.defs 2>/dev/null)"
echo @@status
(getent passwd 2>/dev/null || cat /etc/passwd) | while IFS=: read -r n _ uid _ _ home _; do
  [ "$uid" -ge 1000 ] 2>/dev/null || [ "$uid" -eq 0 ] 2>/dev/null || continue
  passwd -S "$n" 2>/dev/null | head -1
  if [ -r "$home/.ssh/authorized_keys" ]; then
    echo "@@keys $n $(grep -c '^[a-z]' "$home/.ssh/authorized_keys" 2>/dev/null)"
  fi
done
true
"#;

fn admin_groups() -> [&'static str; 3] {
    ["sudo", "wheel", "admin"]
}

pub fn parse_list(out: &str) -> UsersReport {
    let mut r = UsersReport {
        uid_min: 1000,
        ..Default::default()
    };
    let mut section = "";
    let mut status: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut keys: HashMap<String, u32> = HashMap::new();
    let mut raw_users: Vec<Vec<String>> = Vec::new();
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("@@uidmin") {
            r.uid_min = v.trim().parse().unwrap_or(1000);
            continue;
        }
        if let Some(v) = line.strip_prefix("@@keys ") {
            if let Some((n, c)) = v.split_once(' ') {
                keys.insert(n.to_string(), c.trim().parse().unwrap_or(0));
            }
            continue;
        }
        if let Some(s) = line.strip_prefix("@@") {
            section = match s.trim() {
                "passwd" | "group" | "shells" | "status" => s.trim(),
                _ => "",
            };
            continue;
        }
        match section {
            "passwd" => {
                let f: Vec<String> = line.split(':').map(String::from).collect();
                if f.len() >= 7 && !f[0].starts_with('#') {
                    raw_users.push(f);
                }
            }
            "group" => {
                let f: Vec<&str> = line.split(':').collect();
                if f.len() >= 4 && !f[0].starts_with('#') {
                    r.groups.push(GroupInfo {
                        name: f[0].to_string(),
                        gid: f[2].parse().unwrap_or(0),
                        members: f[3].split(',').filter(|m| !m.is_empty()).map(String::from).collect(),
                    });
                }
            }
            "shells" => {
                let l = line.trim();
                if l.starts_with('/') && !r.shells.iter().any(|s| s == l) {
                    r.shells.push(l.to_string());
                }
            }
            "status" => {
                // Debian: "deploy P 2026-11-01 0 99999 7 -1"; RHEL: "deploy PS 2026-11-01 … (Password set…)".
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() >= 2 {
                    let st = match f[1] {
                        "PS" => "P",
                        "LK" => "L",
                        other => other,
                    };
                    status.insert(f[0].to_string(), (st.to_string(), f.get(2).map(|d| d.to_string())));
                }
            }
            _ => {}
        }
    }
    r.complete = !status.is_empty();
    r.admin_group = admin_groups()
        .iter()
        .find(|g| r.groups.iter().any(|x| &x.name == *g))
        .map(|g| g.to_string());
    for f in raw_users {
        let uid: u32 = f[2].parse().unwrap_or(0);
        let gid: u32 = f[3].parse().unwrap_or(0);
        let name = f[0].clone();
        let groups: Vec<String> = r
            .groups
            .iter()
            .filter(|g| g.members.iter().any(|m| m == &name))
            .map(|g| g.name.clone())
            .collect();
        let shell = f[6].clone();
        let (password_status, password_changed) = status.get(&name).cloned().map_or((None, None), |(s, d)| (Some(s), d));
        r.users.push(UserAccount {
            primary_group: r.groups.iter().find(|g| g.gid == gid).map(|g| g.name.clone()),
            admin: uid == 0 || groups.iter().any(|g| admin_groups().contains(&g.as_str())),
            system: uid != 0 && (uid < r.uid_min || uid >= 65534),
            can_login: !shell.ends_with("nologin") && !shell.ends_with("/false") && !shell.is_empty(),
            full_name: f[4].split(',').next().unwrap_or("").to_string(),
            home: f[5].clone(),
            ssh_keys: keys.get(&name).copied(),
            password_status,
            password_changed,
            groups,
            shell,
            uid,
            gid,
            name,
        });
    }
    r
}

// ---------- validation ----------

static USER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z_][a-z0-9_\-]{0,31}$").unwrap());
static GROUP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_\-]{0,31}$").unwrap());
static DATE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());
static KEY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(256|384|521)|sk-ssh-ed25519@openssh\.com|sk-ecdsa-sha2-nistp256@openssh\.com) [A-Za-z0-9+/]+={0,3}( [^\r\n]{0,200})?$").unwrap()
});

pub fn validate_user(name: &str) -> Result<&str> {
    if USER_RE.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!(
            "{name:?} isn't a valid user name: use lowercase letters, digits, - and _, starting with a letter"
        )))
    }
}

fn validate_group(name: &str) -> Result<&str> {
    if GROUP_RE.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!("invalid group name: {name:?}")))
    }
}

fn validate_full_name(s: &str) -> Result<&str> {
    if s.chars().count() > 64 || s.chars().any(|c| c.is_control() || c == ':' || c == ',') {
        return Err(Error::InvalidInput("full name: at most 64 characters, without : or ,".into()));
    }
    Ok(s)
}

fn validate_shell(s: &str) -> Result<&str> {
    validate_path(s)?;
    if s.contains(char::is_whitespace) {
        return Err(Error::InvalidInput("invalid shell".into()));
    }
    Ok(s)
}

pub fn validate_key(k: &str) -> Result<&str> {
    let k = k.trim();
    if KEY_RE.is_match(k) {
        Ok(k)
    } else {
        Err(Error::InvalidInput(
            "that doesn't look like an SSH public key (it should start with ssh-ed25519, ssh-rsa or ecdsa-sha2-…, on one line)".into(),
        ))
    }
}

fn validate_password(p: &str) -> Result<&str> {
    if p.is_empty() || p.len() > 256 || p.contains(['\n', '\r', '\0']) {
        return Err(Error::InvalidInput("password: 1–256 characters on one line".into()));
    }
    Ok(p)
}

// ---------- creating ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewUser {
    pub name: String,
    pub full_name: Option<String>,
    /// Login shell, e.g. /bin/bash. `None` uses the system default.
    pub shell: Option<String>,
    pub create_home: bool,
    /// Home folder; `None` uses /home/<name>.
    pub home: Option<String>,
    /// Existing group to use as primary; `None` creates one named after the user.
    pub primary_group: Option<String>,
    /// Supplementary groups, e.g. sudo, docker, www-data.
    pub groups: Vec<String>,
    pub uid: Option<u32>,
    /// YYYY-MM-DD after which the account can't sign in.
    pub expires: Option<String>,
    /// A service account (low UID, no aging) rather than a person.
    pub system: bool,
    /// Public keys for ~/.ssh/authorized_keys.
    pub ssh_keys: Vec<String>,
    /// Ask for a new password at first sign-in.
    pub must_change_password: bool,
}

/// One root script that creates the account, installs its SSH keys and, when
/// `with_password`, reads `name:password` for chpasswd from stdin.
pub fn create_script(u: &NewUser, with_password: bool) -> Result<String> {
    let name = quote(validate_user(&u.name)?);
    let mut cmd = String::from("useradd");
    if u.system {
        cmd.push_str(" -r");
    }
    cmd.push_str(if u.create_home { " -m" } else { " -M" });
    if let Some(h) = u.home.as_deref().filter(|h| !h.is_empty()) {
        cmd.push_str(&format!(" -d {}", quote(validate_path(h)?)));
    }
    if let Some(s) = u.shell.as_deref().filter(|s| !s.is_empty()) {
        cmd.push_str(&format!(" -s {}", quote(validate_shell(s)?)));
    }
    if let Some(c) = u.full_name.as_deref().filter(|c| !c.is_empty()) {
        cmd.push_str(&format!(" -c {}", quote(validate_full_name(c)?)));
    }
    match u.primary_group.as_deref().filter(|g| !g.is_empty()) {
        Some(g) => cmd.push_str(&format!(" -g {}", quote(validate_group(g)?))),
        None => cmd.push_str(" -U"),
    }
    if !u.groups.is_empty() {
        let gs: Vec<&str> = u.groups.iter().map(|g| validate_group(g)).collect::<Result<_>>()?;
        cmd.push_str(&format!(" -G {}", quote(&gs.join(","))));
    }
    if let Some(id) = u.uid {
        if !(1..60000).contains(&id) {
            return Err(Error::InvalidInput("UID must be between 1 and 59999".into()));
        }
        cmd.push_str(&format!(" -u {id}"));
    }
    if let Some(e) = u.expires.as_deref().filter(|e| !e.is_empty()) {
        if !DATE_RE.is_match(e) {
            return Err(Error::InvalidInput("expiry date must be YYYY-MM-DD".into()));
        }
        cmd.push_str(&format!(" -e {e}"));
    }
    cmd.push_str(&format!(" -- {name} </dev/null"));

    let mut script = format!("set -e; export LC_ALL=C\n{cmd}\n");
    if !u.ssh_keys.is_empty() {
        let keys: Vec<&str> = u.ssh_keys.iter().map(|k| validate_key(k)).collect::<Result<_>>()?;
        script.push_str(&add_keys_snippet(&u.name, &keys));
    }
    if with_password {
        script.push_str("chpasswd\n");
        if u.must_change_password {
            script.push_str(&format!("chage -d 0 -- {name}\n"));
        }
    } else {
        // No password: SSH keys only. Lock the password so it can't be guessed.
        script.push_str(&format!("passwd -l {name} >/dev/null 2>&1 || true\n"));
    }
    script.push_str("echo ok\n");
    Ok(script)
}

/// Appends keys to the user's authorized_keys with the permissions sshd insists on.
fn add_keys_snippet(user: &str, keys: &[&str]) -> String {
    let u = quote(user);
    let lines: Vec<String> = keys.iter().map(|k| format!("printf '%s\\n' {}", quote(k))).collect();
    format!(
        "h=$(getent passwd {u} | cut -d: -f6); g=$(id -gn {u})\n\
         [ -n \"$h\" ] && [ -d \"$h\" ] || {{ echo 'the user has no home folder for SSH keys' >&2; exit 4; }}\n\
         mkdir -p \"$h/.ssh\" && chmod 700 \"$h/.ssh\"\n\
         {{ {} ; }} >> \"$h/.ssh/authorized_keys\"\n\
         chmod 600 \"$h/.ssh/authorized_keys\" && chown -R {u}:\"$g\" \"$h/.ssh\"\n",
        lines.join("; ")
    )
}

// ---------- changing ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum UserChange {
    /// Replace the supplementary groups.
    Groups {
        groups: Vec<String>,
    },
    Shell {
        shell: String,
    },
    FullName {
        full_name: String,
    },
    /// Block every sign-in (password and keys) without deleting anything.
    Lock,
    Unlock,
    /// Set a new password; it is sent over stdin.
    Password {
        must_change: bool,
    },
    AddKey {
        key: String,
    },
    Delete {
        remove_home: bool,
    },
}

impl UserChange {
    pub fn verb(&self) -> &'static str {
        match self {
            UserChange::Groups { .. } => "groups",
            UserChange::Shell { .. } => "shell",
            UserChange::FullName { .. } => "full_name",
            UserChange::Lock => "lock",
            UserChange::Unlock => "unlock",
            UserChange::Password { .. } => "password",
            UserChange::AddKey { .. } => "add_key",
            UserChange::Delete { .. } => "delete",
        }
    }
}

pub fn change_script(user: &str, c: &UserChange) -> Result<String> {
    let n = quote(validate_user(user)?);
    Ok(match c {
        UserChange::Groups { groups } => {
            let gs: Vec<&str> = groups.iter().map(|g| validate_group(g)).collect::<Result<_>>()?;
            format!("usermod -G {} -- {n}", quote(&gs.join(",")))
        }
        UserChange::Shell { shell } => format!("usermod -s {} -- {n}", quote(validate_shell(shell)?)),
        UserChange::FullName { full_name } => format!("usermod -c {} -- {n}", quote(validate_full_name(full_name)?)),
        // -e 1: the account expired in 1970, which also stops key sign-in.
        UserChange::Lock => format!("usermod -L -e 1 -- {n}"),
        UserChange::Unlock => format!("usermod -e '' -- {n} && {{ usermod -U -- {n} 2>/dev/null || true; }}"),
        UserChange::Password { must_change } => {
            let mut s = "chpasswd".to_string();
            if *must_change {
                s.push_str(&format!(" && chage -d 0 -- {n}"));
            }
            s
        }
        UserChange::AddKey { key } => format!("set -e\n{}echo ok", add_keys_snippet(user, &[validate_key(key)?])),
        UserChange::Delete { remove_home } => format!("userdel {}-- {n}", if *remove_home { "-r " } else { "" }),
    })
}

/// Changes that would cut the user off from the server through the account
/// they are connected as, or from root. `admin_group` is this server's sudo group.
pub fn lockout_reason(target: &str, login_user: &str, c: &UserChange, admin_group: Option<&str>) -> Option<String> {
    let me = target == login_user;
    match c {
        UserChange::Delete { .. } if me => Some(format!("{target} is the account this connection uses.")),
        UserChange::Delete { .. } | UserChange::Lock if target == "root" => Some("root can't be deleted or locked from here.".into()),
        UserChange::Lock if me => Some(format!(
            "Locking {target} would end every way back into this server through this account."
        )),
        UserChange::Groups { groups } if me && admin_group.is_some_and(|g| !groups.iter().any(|x| x == g)) => Some(format!(
            "{target} would leave the {} group and lose sudo, so Orbital couldn't change the server any more through this account.",
            admin_group.unwrap_or("sudo")
        )),
        UserChange::Shell { shell } if me && (shell.ends_with("nologin") || shell.ends_with("/false")) => {
            Some(format!("That shell would stop {target} from signing in."))
        }
        _ => None,
    }
}

pub fn password_input(user: &str, password: &str) -> Result<Vec<u8>> {
    Ok(format!("{}:{}\n", validate_user(user)?, validate_password(password)?).into_bytes())
}

impl SshSession {
    pub async fn list_users(&self) -> Result<UsersReport> {
        let (o, root) = self.exec_inspect(LIST_SCRIPT).await?;
        let mut r = parse_list(&o.stdout);
        r.complete = r.complete && root;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@passwd\nroot:x:0:0:root:/root:/bin/bash\ndaemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\nwww-data:x:33:33:www-data:/var/www:/usr/sbin/nologin\ndeploy:x:1000:1000:Deploy Bot,,,:/home/deploy:/bin/bash\nanna:x:1001:1001:Anna K:/home/anna:/bin/zsh\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n@@group\nroot:x:0:\nsudo:x:27:deploy\ndocker:x:998:deploy,anna\ndeploy:x:1000:\nanna:x:1001:\n@@shells\n/bin/sh\n/bin/bash\n/usr/bin/zsh\n/bin/zsh\n@@uidmin 1000\n@@status\nroot L 2026-01-01 0 99999 7 -1\ndeploy P 2026-11-01 0 99999 7 -1\n@@keys deploy 2\nanna LK 2026-10-01 0 99999 7 -1 (Password locked.)\n";

    #[test]
    fn parses_accounts() {
        let r = parse_list(OUT);
        assert!(r.complete);
        assert_eq!(r.admin_group.as_deref(), Some("sudo"));
        assert_eq!(r.shells, ["/bin/sh", "/bin/bash", "/usr/bin/zsh", "/bin/zsh"]);
        let deploy = r.users.iter().find(|u| u.name == "deploy").unwrap();
        assert!(deploy.admin && !deploy.system && deploy.can_login);
        assert_eq!(deploy.full_name, "Deploy Bot");
        assert_eq!(deploy.groups, ["sudo", "docker"]);
        assert_eq!(deploy.primary_group.as_deref(), Some("deploy"));
        assert_eq!((deploy.password_status.as_deref(), deploy.ssh_keys), (Some("P"), Some(2)));
        let anna = r.users.iter().find(|u| u.name == "anna").unwrap();
        assert_eq!(anna.password_status.as_deref(), Some("L"));
        assert!(!anna.admin && anna.ssh_keys.is_none());
        assert!(r.users.iter().find(|u| u.name == "www-data").unwrap().system);
        assert!(r.users.iter().find(|u| u.name == "nobody").unwrap().system);
        let root = r.users.iter().find(|u| u.name == "root").unwrap();
        assert!(root.admin && !root.system);
    }

    fn new(name: &str) -> NewUser {
        NewUser {
            name: name.into(),
            create_home: true,
            ..Default::default()
        }
    }

    #[test]
    fn create_script_has_every_option_quoted() {
        let u = NewUser {
            full_name: Some("Anna K".into()),
            shell: Some("/bin/zsh".into()),
            groups: vec!["sudo".into(), "docker".into()],
            uid: Some(1500),
            expires: Some("2027-06-30".into()),
            ssh_keys: vec!["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl anna@mac".into()],
            must_change_password: true,
            ..new("anna")
        };
        let s = create_script(&u, true).unwrap();
        assert!(
            s.contains("useradd -m -s /bin/zsh -c 'Anna K' -U -G sudo,docker -u 1500 -e 2027-06-30 -- anna </dev/null"),
            "{s}"
        );
        assert!(s.contains("chmod 700 \"$h/.ssh\"") && s.contains("'ssh-ed25519 AAAAC3"));
        assert!(s.contains("chpasswd\nchage -d 0 -- anna"));
        let keys_only = create_script(&new("svc"), false).unwrap();
        assert!(keys_only.contains("passwd -l svc") && !keys_only.contains("chpasswd"));
        let system = create_script(
            &NewUser {
                system: true,
                create_home: false,
                ..new("app")
            },
            false,
        )
        .unwrap();
        assert!(system.contains("useradd -r -M -U -- app"));
    }

    #[test]
    fn create_script_rejects_bad_input() {
        for bad in [
            new("Root"),
            new("a b"),
            new("-x"),
            new("x;reboot"),
            NewUser {
                full_name: Some("a:b".into()),
                ..new("ok")
            },
            NewUser {
                shell: Some("bash".into()),
                ..new("ok")
            },
            NewUser {
                groups: vec!["sudo;id".into()],
                ..new("ok")
            },
            NewUser { uid: Some(0), ..new("ok") },
            NewUser {
                expires: Some("next year".into()),
                ..new("ok")
            },
            NewUser {
                ssh_keys: vec!["not a key".into()],
                ..new("ok")
            },
            NewUser {
                ssh_keys: vec!["ssh-ed25519 AAAA x\nssh-rsa BBBB".into()],
                ..new("ok")
            },
            NewUser {
                home: Some("home/x".into()),
                ..new("ok")
            },
        ] {
            assert!(create_script(&bad, false).is_err(), "{bad:?}");
        }
        assert!(password_input("anna", "line\nbreak").is_err());
        assert!(password_input("anna", "").is_err());
        assert_eq!(password_input("anna", "p:ss word").unwrap(), b"anna:p:ss word\n");
    }

    #[test]
    fn changes_and_lockout_guard() {
        assert_eq!(
            change_script(
                "anna",
                &UserChange::Groups {
                    groups: vec!["docker".into()]
                }
            )
            .unwrap(),
            "usermod -G docker -- anna"
        );
        assert_eq!(
            change_script("anna", &UserChange::Groups { groups: vec![] }).unwrap(),
            "usermod -G '' -- anna"
        );
        assert_eq!(change_script("anna", &UserChange::Lock).unwrap(), "usermod -L -e 1 -- anna");
        assert_eq!(
            change_script("anna", &UserChange::Delete { remove_home: true }).unwrap(),
            "userdel -r -- anna"
        );
        assert!(change_script("anna", &UserChange::Password { must_change: true })
            .unwrap()
            .starts_with("chpasswd && chage -d 0"));
        assert!(change_script("anna", &UserChange::AddKey { key: "nope".into() }).is_err());

        let sudo = Some("sudo");
        assert!(lockout_reason("deploy", "deploy", &UserChange::Delete { remove_home: false }, sudo).is_some());
        assert!(lockout_reason("deploy", "deploy", &UserChange::Lock, sudo).is_some());
        assert!(lockout_reason(
            "deploy",
            "deploy",
            &UserChange::Groups {
                groups: vec!["docker".into()]
            },
            sudo
        )
        .is_some());
        assert!(lockout_reason(
            "deploy",
            "deploy",
            &UserChange::Groups {
                groups: vec!["sudo".into()]
            },
            sudo
        )
        .is_none());
        assert!(lockout_reason(
            "deploy",
            "deploy",
            &UserChange::Shell {
                shell: "/usr/sbin/nologin".into()
            },
            sudo
        )
        .is_some());
        assert!(lockout_reason("root", "deploy", &UserChange::Delete { remove_home: true }, sudo).is_some());
        assert!(lockout_reason("anna", "deploy", &UserChange::Lock, sudo).is_none());
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn list_script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(LIST_SCRIPT).output().unwrap();
        assert!(out.status.success());
        let r = parse_list(&String::from_utf8_lossy(&out.stdout));
        assert!(r.users.iter().any(|u| u.uid == 0));
    }
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    #[test]
    fn changes_arrive_from_the_ui_in_camel_case() {
        let c: UserChange = serde_json::from_str(r#"{"type":"password","mustChange":true}"#).unwrap();
        assert_eq!(c, UserChange::Password { must_change: true });
        let c: UserChange = serde_json::from_str(r#"{"type":"delete","removeHome":false}"#).unwrap();
        assert_eq!(c, UserChange::Delete { remove_home: false });
        let c: UserChange = serde_json::from_str(r#"{"type":"fullName","fullName":"A"}"#).unwrap();
        assert_eq!(c, UserChange::FullName { full_name: "A".into() });
        let u: NewUser = serde_json::from_str(r#"{"name":"a","createHome":true,"groups":[],"system":false,"sshKeys":[],"mustChangePassword":false,"fullName":null,"shell":null,"home":null,"primaryGroup":null,"uid":null,"expires":null}"#).unwrap();
        assert!(u.create_home);
    }
}
