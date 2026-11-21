//! "You may try": software Orbital has a tab for, installed from the server's
//! own package repositories (apt, dnf or yum) and started. A fixed list with
//! fixed package names; nothing from the user goes into the script.

use crate::error::{Error, Result};
use serde::Serialize;

pub struct Installable {
    /// The tool id in `tools::CATALOG` (the tab it unlocks).
    pub id: &'static str,
    pub hint: &'static str,
    apt: &'static [&'static str],
    dnf: &'static [&'static str],
    /// Run after the packages are in (first-time setup).
    after: &'static str,
    /// Services to enable and start; the first one that exists.
    services: &'static [&'static str],
    /// Needs port 80 (a second web server would fail to start).
    web: bool,
}

pub const INSTALLABLE: &[Installable] = &[
    Installable {
        id: "docker",
        hint: "Run apps in containers; manage them, their logs and Compose files here",
        apt: &["docker.io"],
        dnf: &["moby-engine"],
        after: "",
        services: &["docker"],
        web: false,
    },
    Installable {
        id: "nginx",
        hint: "Web server and reverse proxy; then New website and HTTPS in two clicks",
        apt: &["nginx"],
        dnf: &["nginx"],
        after: "",
        services: &["nginx"],
        web: true,
    },
    Installable {
        id: "postgresql",
        hint: "Database; see databases, sizes and connections, back them up",
        apt: &["postgresql"],
        dnf: &["postgresql-server"],
        after: "if command -v postgresql-setup >/dev/null 2>&1 && [ ! -d /var/lib/pgsql/data/base ]; then postgresql-setup --initdb; fi",
        services: &["postgresql"],
        web: false,
    },
    Installable {
        id: "mysql",
        hint: "MariaDB (MySQL-compatible) database, with one-click backups",
        apt: &["mariadb-server"],
        dnf: &["mariadb-server"],
        after: "",
        services: &["mariadb", "mysql"],
        web: false,
    },
    Installable {
        id: "redis",
        hint: "In-memory cache and queue; memory, keys and hit rate at a glance",
        apt: &["redis-server"],
        dnf: &["redis"],
        after: "",
        services: &["redis-server", "redis"],
        web: false,
    },
    Installable {
        id: "certbot",
        hint: "Free HTTPS certificates from Let's Encrypt, renewed automatically",
        apt: &["certbot", "python3-certbot-nginx"],
        dnf: &["certbot", "python3-certbot-nginx"],
        after: "",
        services: &[],
        web: false,
    },
    Installable {
        id: "fail2ban",
        hint: "Bans addresses that keep failing to sign in over SSH",
        apt: &["fail2ban"],
        dnf: &["fail2ban"],
        after: "",
        services: &["fail2ban"],
        web: false,
    },
    Installable {
        id: "ufw",
        hint: "A simple firewall; installed off, turn it on from its tab with a lock-out check",
        apt: &["ufw"],
        dnf: &["ufw"],
        after: "",
        services: &[],
        web: false,
    },
    Installable {
        id: "haproxy",
        hint: "Load balancer; backends, servers and their state",
        apt: &["haproxy"],
        dnf: &["haproxy"],
        after: "",
        services: &["haproxy"],
        web: false,
    },
    Installable {
        id: "apache",
        hint: "Apache web server; sites, virtual hosts and modules",
        apt: &["apache2"],
        dnf: &["httpd"],
        after: "",
        services: &["apache2", "httpd"],
        web: true,
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOption {
    pub id: String,
    pub name: String,
    pub category: String,
    pub hint: String,
    /// Package names for apt / dnf, to show before installing.
    pub apt: Vec<String>,
    pub dnf: Vec<String>,
}

/// The list for the "+" menu, names from the tool catalogue.
pub fn options() -> Vec<InstallOption> {
    INSTALLABLE
        .iter()
        .filter_map(|i| {
            let t = crate::tools::CATALOG.iter().find(|t| t.id == i.id)?;
            Some(InstallOption {
                id: i.id.into(),
                name: t.name.into(),
                category: t.category.into(),
                hint: i.hint.into(),
                apt: i.apt.iter().map(|s| s.to_string()).collect(),
                dnf: i.dnf.iter().map(|s| s.to_string()).collect(),
            })
        })
        .collect()
}

pub fn find(id: &str) -> Result<&'static Installable> {
    INSTALLABLE
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| Error::InvalidInput(format!("Orbital doesn't install {id:?}")))
}

/// The install script (run as root): packages, first-time setup, start the service.
pub fn script(i: &Installable) -> String {
    let mut s = format!(
        "set -e\nif command -v apt-get >/dev/null 2>&1; then\n  export DEBIAN_FRONTEND=noninteractive\n  apt-get update -q\n  apt-get install -y -q {apt}\nelif command -v dnf >/dev/null 2>&1; then\n  dnf install -y {dnf}\nelif command -v yum >/dev/null 2>&1; then\n  yum install -y {dnf}\nelse\n  echo 'This server has neither apt nor dnf; install it with its own package manager.' >&2; exit 2\nfi\n",
        apt = i.apt.join(" "),
        dnf = i.dnf.join(" ")
    );
    if !i.after.is_empty() {
        s.push_str(i.after);
        s.push('\n');
    }
    if !i.services.is_empty() {
        s.push_str(&format!(
            "for u in {}; do if systemctl list-unit-files \"$u.service\" >/dev/null 2>&1 && systemctl cat \"$u.service\" >/dev/null 2>&1; then systemctl enable --now \"$u\"; break; fi; done\n",
            i.services.join(" ")
        ));
    }
    s
}

/// Lock-out style check before a second web server: is port 80 taken? Prints the listener.
pub const PORT80_CHECK: &str = "ss -Hltnp 2>/dev/null | awk '$4 ~ /[:.]80$/' | head -3";

pub fn needs_port80(i: &Installable) -> bool {
    i.web
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_installable_has_a_tab_and_a_fixed_script() {
        assert_eq!(options().len(), INSTALLABLE.len(), "each id is in tools::CATALOG");
        for i in INSTALLABLE {
            let s = script(i);
            assert!(s.contains("apt-get install -y -q") && s.contains("dnf install -y"), "{s}");
            for p in i.apt.iter().chain(i.dnf) {
                assert!(p.chars().all(|c| c.is_ascii_alphanumeric() || "-.".contains(c)), "{p}");
            }
        }
        let pg = script(find("postgresql").unwrap());
        assert!(
            pg.contains("postgresql-setup --initdb") && pg.contains("for u in postgresql;"),
            "{pg}"
        );
        assert!(
            script(find("ufw").unwrap()).lines().all(|l| !l.contains("enable --now")),
            "ufw is installed off"
        );
        assert!(find("mongodb").is_err() && find("x; reboot").is_err());
        assert!(needs_port80(find("apache").unwrap()) && !needs_port80(find("redis").unwrap()));
    }

    #[test]
    fn script_is_valid_sh() {
        for i in INSTALLABLE {
            let out = std::process::Command::new("sh")
                .arg("-n")
                .arg("-c")
                .arg(script(i))
                .output()
                .unwrap();
            assert!(out.status.success(), "{}: {}", i.id, String::from_utf8_lossy(&out.stderr));
        }
    }
}
