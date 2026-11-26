//! Packages: what's installed (apt/dnf/yum), searching the repositories, and
//! installing or removing one package after a dry run shows everything that
//! would change. Names must match a strict pattern and are quoted; removing a
//! package the server needs to stay reachable (sshd, sudo, systemd, the
//! package manager, the kernel) is refused unless forced.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::updates::PackageManager;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub name: String,
    pub version: String,
    /// Installed size in bytes, when the manager says.
    pub size: Option<u64>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub name: String,
    pub summary: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PkgAction {
    Install,
    Remove,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PkgPlan {
    pub install: Vec<String>,
    pub remove: Vec<String>,
    /// What the package manager printed (for dnf, which has no simple format).
    pub text: String,
    /// Packages in `remove` the server needs to stay reachable.
    pub essential: Vec<String>,
}

static NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9+._\-]{0,127}$").unwrap());
static TERM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9+._\-]{1,63}$").unwrap());

/// Removing these (or anything that takes them along) can cut the server off.
const ESSENTIAL: &[&str] = &[
    "openssh-server",
    "ssh",
    "openssh",
    "sudo",
    "systemd",
    "systemd-sysv",
    "bash",
    "dash",
    "coreutils",
    "libc6",
    "glibc",
    "apt",
    "dpkg",
    "dnf",
    "yum",
    "rpm",
    "passwd",
    "login",
    "util-linux",
    "init",
    "udev",
    "grub-pc",
    "grub2",
    "grub-efi-amd64",
    "linux-image-generic",
    "linux-generic",
    "kernel",
    "kernel-core",
    "netplan.io",
    "ifupdown",
    "network-manager",
    "NetworkManager",
];

pub fn is_essential(name: &str) -> bool {
    ESSENTIAL.contains(&name) || name.starts_with("linux-image-") || name.starts_with("kernel-core-")
}

pub fn validate_name(n: &str) -> Result<&str> {
    if NAME.is_match(n) {
        Ok(n)
    } else {
        Err(Error::InvalidInput(format!("{n:?} isn't a package name")))
    }
}

pub fn list_command(pm: PackageManager) -> &'static str {
    match pm {
        PackageManager::Apt => "dpkg-query -W -f='${db:Status-Abbrev}\\t${Package}\\t${Version}\\t${Installed-Size}\\t${binary:Summary}\\n' 2>/dev/null | grep '^ii' | cut -f2-",
        _ => "rpm -qa --qf '%{NAME}\\t%{VERSION}-%{RELEASE}\\t%{SIZE}\\t%{SUMMARY}\\n' 2>/dev/null",
    }
}

pub fn parse_list(pm: PackageManager, out: &str) -> Vec<Package> {
    let mut v: Vec<Package> = out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 4 || f[0].is_empty() {
                return None;
            }
            // dpkg gives KiB, rpm bytes.
            let size = f[2]
                .trim()
                .parse::<u64>()
                .ok()
                .map(|n| if pm == PackageManager::Apt { n * 1024 } else { n });
            Some(Package {
                name: f[0].into(),
                version: f[1].into(),
                size,
                summary: f[3..].join("\t"),
            })
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v.dedup_by(|a, b| a.name == b.name);
    v
}

pub fn search_command(pm: PackageManager, term: &str) -> Result<String> {
    if !TERM.is_match(term) {
        return Err(Error::InvalidInput(
            "search for a package name: letters, digits, '-', '.', '+'".into(),
        ));
    }
    let t = quote(term);
    Ok(match pm {
        PackageManager::Apt => format!("apt-cache search --names-only -- {t} 2>/dev/null | head -n 150"),
        PackageManager::Dnf => format!("dnf -q search -- {t} 2>/dev/null | head -n 150"),
        PackageManager::Yum => format!("yum -q search -- {t} 2>/dev/null | head -n 150"),
    })
}

pub fn parse_search(pm: PackageManager, out: &str, installed: &[Package]) -> Vec<Found> {
    let has = |n: &str| installed.iter().any(|p| p.name == n);
    out.lines()
        .filter_map(|l| {
            let (name, summary) = match pm {
                PackageManager::Apt => l.split_once(" - ")?,
                _ => {
                    if l.starts_with('=') || l.starts_with("Last metadata") || l.starts_with("Matched") {
                        return None;
                    }
                    let (n, s) = l.split_once(" : ")?;
                    // name.arch
                    let n = n.trim();
                    let n = n
                        .rsplit_once('.')
                        .map(|(a, arch)| {
                            if ["x86_64", "noarch", "aarch64", "i686"].contains(&arch) {
                                a
                            } else {
                                n
                            }
                        })
                        .unwrap_or(n);
                    (n, s)
                }
            };
            let name = name.trim();
            NAME.is_match(name).then(|| Found {
                name: name.into(),
                summary: summary.trim().into(),
                installed: has(name),
            })
        })
        .collect()
}

/// A dry run: what installing or removing `name` would change.
pub fn plan_command(pm: PackageManager, name: &str, a: PkgAction) -> Result<String> {
    let n = quote(validate_name(name)?);
    Ok(match (pm, a) {
        (PackageManager::Apt, PkgAction::Install) => format!("LC_ALL=C apt-get -s install -y -- {n} 2>&1"),
        (PackageManager::Apt, PkgAction::Remove) => format!("LC_ALL=C apt-get -s remove -y -- {n} 2>&1"),
        (_, PkgAction::Install) => format!("LC_ALL=C {} --assumeno install -- {n} 2>&1; true", pm_bin(pm)),
        (_, PkgAction::Remove) => format!("LC_ALL=C {} --assumeno remove -- {n} 2>&1; true", pm_bin(pm)),
    })
}

fn pm_bin(pm: PackageManager) -> &'static str {
    match pm {
        PackageManager::Yum => "yum",
        _ => "dnf",
    }
}

pub fn parse_plan(pm: PackageManager, out: &str) -> PkgPlan {
    let mut p = PkgPlan {
        text: out.trim().to_string(),
        ..Default::default()
    };
    if pm == PackageManager::Apt {
        for l in out.lines() {
            let mut f = l.split_whitespace();
            match (f.next(), f.next()) {
                (Some("Inst"), Some(n)) => p.install.push(n.into()),
                (Some("Remv"), Some(n)) => p.remove.push(n.into()),
                _ => {}
            }
        }
    } else {
        // dnf/yum: a table; package lines start with a space and a name.
        let mut mode = "";
        for l in out.lines() {
            let t = l.trim();
            if t.starts_with("Installing") || t.starts_with("Upgrading") {
                mode = "i";
            } else if t.starts_with("Removing") || t.starts_with("Erasing") {
                mode = "r";
            } else if t.starts_with("Transaction Summary") || t.is_empty() {
                mode = "";
            } else if l.starts_with(' ') && !mode.is_empty() {
                if let Some(n) = t.split_whitespace().next().filter(|n| NAME.is_match(n)) {
                    if mode == "i" {
                        p.install.push(n.into())
                    } else {
                        p.remove.push(n.into())
                    }
                }
            }
        }
    }
    p.essential = p.remove.iter().filter(|n| is_essential(n)).cloned().collect();
    p
}

pub fn run_command(pm: PackageManager, name: &str, a: PkgAction) -> Result<String> {
    let n = quote(validate_name(name)?);
    Ok(match (pm, a) {
        (PackageManager::Apt, PkgAction::Install) => format!("DEBIAN_FRONTEND=noninteractive apt-get install -y -- {n} 2>&1"),
        (PackageManager::Apt, PkgAction::Remove) => format!("DEBIAN_FRONTEND=noninteractive apt-get remove -y -- {n} 2>&1"),
        (_, PkgAction::Install) => format!("{} -y install -- {n} 2>&1", pm_bin(pm)),
        (_, PkgAction::Remove) => format!("{} -y remove -- {n} 2>&1", pm_bin(pm)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_installed_packages() {
        let apt = parse_list(
            PackageManager::Apt,
            "nginx\t1.24.0-2ubuntu7\t1234\tsmall, powerful web server\ncurl\t8.5.0\t500\tcommand line tool\tfor URLs\n\n",
        );
        assert_eq!(apt.len(), 2);
        assert_eq!(apt[0].name, "curl");
        assert_eq!(apt[0].summary, "command line tool\tfor URLs");
        assert_eq!(apt[1].size, Some(1234 * 1024));
        let rpm = parse_list(PackageManager::Dnf, "bash\t5.2.15-3.fc39\t8000000\tThe GNU Bourne Again shell\n");
        assert_eq!(rpm[0].size, Some(8_000_000));
    }

    #[test]
    fn searches() {
        let inst = parse_list(PackageManager::Apt, "htop\t3.3.0\t300\tviewer\n");
        let f = parse_search(
            PackageManager::Apt,
            "htop - interactive processes viewer\nbtop - resource monitor\nbad name - x\n",
            &inst,
        );
        assert_eq!(f.len(), 2);
        assert!(f[0].installed && !f[1].installed);
        let d = parse_search(
            PackageManager::Dnf,
            "=== Name Matched: htop ===\nhtop.x86_64 : Interactive process viewer\nLast metadata expiration check: 0:01:00 ago\n",
            &[],
        );
        assert_eq!(
            d,
            [Found {
                name: "htop".into(),
                summary: "Interactive process viewer".into(),
                installed: false
            }]
        );
        assert!(search_command(PackageManager::Apt, "ht").is_ok());
        assert!(search_command(PackageManager::Apt, "x; rm -rf /").is_err());
        assert!(search_command(PackageManager::Apt, "-o").is_err(), "no options");
    }

    #[test]
    fn plans_and_flags_essential_removals() {
        let apt = parse_plan(PackageManager::Apt, "NOTE: This is only a simulation!\nRemv openssh-server [1:9.6p1-3ubuntu13]\nRemv ssh [1:9.6p1]\nInst htop (3.3.0 Ubuntu:24.04)\nConf htop\n");
        assert_eq!(apt.remove, ["openssh-server", "ssh"]);
        assert_eq!(apt.install, ["htop"]);
        assert_eq!(apt.essential, ["openssh-server", "ssh"]);
        let dnf = parse_plan(PackageManager::Dnf, "Dependencies resolved.\n================\n Package  Arch  Version  Repo  Size\n================\nRemoving:\n htop     x86_64 3.3.0   @fedora 400 k\nRemoving unused dependencies:\n hwloc-libs x86_64 2.10 @fedora 2 M\n\nTransaction Summary\n");
        assert_eq!(dnf.remove, ["htop", "hwloc-libs"]);
        assert!(is_essential("linux-image-6.8.0-45-generic"));
        assert!(!is_essential("htop"));
    }

    #[test]
    fn commands_quote_names_and_end_options() {
        assert_eq!(
            run_command(PackageManager::Apt, "htop", PkgAction::Install).unwrap(),
            "DEBIAN_FRONTEND=noninteractive apt-get install -y -- htop 2>&1"
        );
        assert!(run_command(PackageManager::Dnf, "htop;id", PkgAction::Remove).is_err());
        assert!(plan_command(PackageManager::Apt, "-y", PkgAction::Remove).is_err());
        assert!(plan_command(PackageManager::Yum, "htop", PkgAction::Install)
            .unwrap()
            .starts_with("LC_ALL=C yum --assumeno"));
    }
}
