//! System updates: which packages can be upgraded (security updates marked),
//! refreshing the package list, installing the updates, and rebooting when
//! the server says it needs it. apt (Debian, Ubuntu) and dnf/yum (Fedora,
//! RHEL, Rocky, Alma). Every script here is fixed text: nothing from the user
//! or the server is put into it.

use crate::error::{Error, Result};
use crate::ssh::SshSession;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PackageManager {
    Apt,
    Dnf,
    Yum,
}

impl PackageManager {
    fn from_marker(s: &str) -> Option<Self> {
        match s {
            "apt" => Some(Self::Apt),
            "dnf" => Some(Self::Dnf),
            "yum" => Some(Self::Yum),
            _ => None,
        }
    }

    /// Refresh the list of available versions.
    pub fn refresh_command(self) -> &'static str {
        match self {
            Self::Apt => "DEBIAN_FRONTEND=noninteractive apt-get update",
            Self::Dnf => "dnf -y makecache",
            Self::Yum => "yum -y makecache",
        }
    }

    /// Install every available update. Config files the admin changed are
    /// kept (no questions asked that nobody can answer).
    pub fn upgrade_command(self) -> &'static str {
        match self {
            Self::Apt => {
                "DEBIAN_FRONTEND=noninteractive apt-get -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade"
            }
            Self::Dnf => "dnf -y upgrade",
            Self::Yum => "yum -y update",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Upgradable {
    pub name: String,
    /// The installed version, when the package manager says.
    pub current: Option<String>,
    pub available: String,
    /// From a security repository (apt's `-security` pockets).
    pub security: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesReport {
    pub manager: Option<PackageManager>,
    pub packages: Vec<Upgradable>,
    pub reboot_required: bool,
    /// When the package list was last refreshed (Unix seconds), when known.
    pub refreshed_at: Option<i64>,
}

/// Lists upgradable packages without changing anything (apt's list is as
/// fresh as the last `apt-get update`).
pub const LIST_SCRIPT: &str = r#"
if command -v apt-get >/dev/null 2>&1; then
  echo "@@pm apt"
  for f in /var/lib/apt/periodic/update-success-stamp /var/lib/apt/lists/partial; do [ -e "$f" ] && { echo "@@refreshed $(stat -c %Y "$f" 2>/dev/null)"; break; }; done
  apt list --upgradable 2>/dev/null
elif command -v dnf >/dev/null 2>&1; then
  echo "@@pm dnf"; dnf -q check-update 2>/dev/null
elif command -v yum >/dev/null 2>&1; then
  echo "@@pm yum"; yum -q check-update 2>/dev/null
fi
[ -f /var/run/reboot-required ] && echo "@@reboot"
if command -v needs-restarting >/dev/null 2>&1; then needs-restarting -r >/dev/null 2>&1; [ $? -eq 1 ] && echo "@@reboot"; fi
true
"#;

/// Reboot a moment after the command returns, so its channel closes cleanly
/// instead of being cut off.
pub const REBOOT_COMMAND: &str = "nohup sh -c 'sleep 2; systemctl reboot || reboot' >/dev/null 2>&1 &";

pub fn parse(out: &str) -> UpdatesReport {
    let mut r = UpdatesReport::default();
    for line in out.lines() {
        let line = line.trim_end();
        if let Some(pm) = line.strip_prefix("@@pm ") {
            r.manager = PackageManager::from_marker(pm.trim());
        } else if let Some(t) = line.strip_prefix("@@refreshed ") {
            r.refreshed_at = t.trim().parse().ok();
        } else if line == "@@reboot" {
            r.reboot_required = true;
        } else if let Some(p) = match r.manager {
            Some(PackageManager::Apt) => parse_apt(line),
            Some(_) => parse_dnf(line),
            None => None,
        } {
            if !r.packages.iter().any(|x| x.name == p.name) {
                r.packages.push(p);
            }
        }
    }
    // Security updates first, then by name.
    r.packages.sort_by(|a, b| b.security.cmp(&a.security).then(a.name.cmp(&b.name)));
    r
}

/// `openssl/noble-updates,noble-security 3.0.13-0ubuntu3.4 amd64 [upgradable from: 3.0.13-0ubuntu3.1]`
fn parse_apt(line: &str) -> Option<Upgradable> {
    let (name, rest) = line.split_once('/')?;
    if name.is_empty() || name.contains(' ') || !line.contains("[upgradable from:") {
        return None;
    }
    let mut words = rest.split_whitespace();
    let suites = words.next()?;
    let available = words.next()?.to_string();
    let current = line
        .split("[upgradable from: ")
        .nth(1)
        .and_then(|s| s.strip_suffix(']'))
        .map(String::from);
    Some(Upgradable {
        name: name.to_string(),
        current,
        available,
        security: suites.split(',').any(|s| s.ends_with("-security")),
    })
}

/// `openssl.x86_64     1:3.0.7-28.el9_4     baseos`
fn parse_dnf(line: &str) -> Option<Upgradable> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() != 3 || line.starts_with(' ') || words[0].ends_with(':') {
        return None;
    }
    let (name, arch) = words[0].rsplit_once('.')?;
    if name.is_empty() || arch.is_empty() || !words[1].chars().next()?.is_ascii_digit() {
        return None;
    }
    Some(Upgradable {
        name: name.to_string(),
        current: None,
        available: words[1].to_string(),
        security: false,
    })
}

impl SshSession {
    /// What can be upgraded (read-only; root's view when sudo works).
    pub async fn updates(&self) -> Result<UpdatesReport> {
        let (o, _) = self.exec_inspect(LIST_SCRIPT).await?;
        Ok(parse(&o.stdout))
    }

    /// The package manager, or an explanation of why updates can't be managed here.
    pub async fn package_manager(&self) -> Result<PackageManager> {
        self.updates()
            .await?
            .manager
            .ok_or_else(|| Error::NotFound("no apt, dnf or yum on this server".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apt() {
        let out = "@@pm apt\n@@refreshed 1763300000\nListing... Done\nopenssl/noble-updates,noble-security 3.0.13-0ubuntu3.4 amd64 [upgradable from: 3.0.13-0ubuntu3.1]\nnginx/noble-updates 1.24.0-2ubuntu7.2 amd64 [upgradable from: 1.24.0-2ubuntu7.1]\nlibssl3t64/noble-security 3.0.13-0ubuntu3.4 amd64 [upgradable from: 3.0.13-0ubuntu3.1]\n@@reboot\n";
        let r = parse(out);
        assert_eq!(r.manager, Some(PackageManager::Apt));
        assert_eq!(r.refreshed_at, Some(1763300000));
        assert!(r.reboot_required);
        let names: Vec<_> = r.packages.iter().map(|p| (p.name.as_str(), p.security)).collect();
        assert_eq!(names, [("libssl3t64", true), ("openssl", true), ("nginx", false)]);
        assert_eq!(r.packages[2].current.as_deref(), Some("1.24.0-2ubuntu7.1"));
        assert_eq!(r.packages[2].available, "1.24.0-2ubuntu7.2");
    }

    #[test]
    fn parses_dnf_and_skips_headers() {
        let out = "@@pm dnf\n\nopenssl.x86_64                1:3.0.7-28.el9_4          baseos\nkernel.x86_64   5.14.0-427.el9   baseos\nObsoleting Packages:\ngrub2-tools.x86_64  1:2.06-80.el9  baseos\n    grub2-tools.x86_64 1:2.06-70.el9 @baseos\n";
        let r = parse(out);
        assert_eq!(r.manager, Some(PackageManager::Dnf));
        let names: Vec<_> = r.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["grub2-tools", "kernel", "openssl"]);
        assert!(!r.reboot_required);
    }

    #[test]
    fn nothing_to_do_and_unknown_systems() {
        let r = parse("@@pm apt\nListing... Done\n");
        assert!(r.packages.is_empty() && r.manager == Some(PackageManager::Apt));
        assert_eq!(parse("").manager, None);
    }

    #[test]
    fn commands_are_fixed_and_non_interactive() {
        for pm in [PackageManager::Apt, PackageManager::Dnf, PackageManager::Yum] {
            assert!(pm.upgrade_command().contains("-y"));
        }
        assert!(
            PackageManager::Apt.upgrade_command().contains("--force-confold"),
            "keeps changed config files"
        );
        assert!(REBOOT_COMMAND.starts_with("nohup ") && REBOOT_COMMAND.ends_with('&'));
    }
}
