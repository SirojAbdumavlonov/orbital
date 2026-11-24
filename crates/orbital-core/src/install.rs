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
        // Compose (the Stacks tab and Apps) is a separate package; try the usual names.
        after: COMPOSE_AFTER,
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
    Installable {
        id: "caddy",
        hint: "Web server with automatic HTTPS; one Caddyfile for every site",
        apt: &["caddy"],
        dnf: &["caddy"],
        after: "",
        services: &["caddy"],
        web: true,
    },
    Installable {
        id: "php-fpm",
        hint: "Runs PHP apps (WordPress, Laravel) behind nginx or Apache",
        apt: &["php-fpm", "php-mysql", "php-curl", "php-xml", "php-mbstring", "php-zip", "php-gd"],
        dnf: &["php-fpm", "php-mysqlnd", "php-xml", "php-mbstring", "php-gd"],
        // Debian names the unit after the version (php8.2-fpm); start whichever came.
        after: "for u in $(systemctl list-unit-files 'php*-fpm.service' --no-legend 2>/dev/null | awk '{print $1}'); do systemctl enable --now \"$u\" && break; done",
        services: &[],
        web: false,
    },
    Installable {
        id: "nodejs",
        hint: "JavaScript runtime and npm, for Node apps and Deploy builds",
        apt: &["nodejs", "npm"],
        dnf: &["nodejs", "npm"],
        after: "",
        services: &[],
        web: false,
    },
    Installable {
        id: "memcached",
        hint: "Simple in-memory cache for PHP and Python apps; listens on 127.0.0.1",
        apt: &["memcached"],
        dnf: &["memcached"],
        after: "",
        services: &["memcached"],
        web: false,
    },
    Installable {
        id: "supervisor",
        hint: "Keeps workers and queue consumers running; programs in /etc/supervisor/conf.d",
        apt: &["supervisor"],
        dnf: &["supervisor"],
        after: "",
        services: &["supervisor", "supervisord"],
        web: false,
    },
    Installable {
        id: "chrony",
        hint: "Keeps the clock right (certificates, logs and 2FA codes depend on it)",
        apt: &["chrony"],
        dnf: &["chrony"],
        after: "",
        services: &["chrony", "chronyd"],
        web: false,
    },
    Installable {
        id: "auto-updates",
        hint: "Installs security updates every day on its own (unattended-upgrades or dnf-automatic)",
        apt: &["unattended-upgrades"],
        dnf: &["dnf-automatic"],
        after: AUTO_UPDATES_AFTER,
        services: &["unattended-upgrades"],
        web: false,
    },
    Installable {
        id: "wireguard",
        hint: "Fast VPN; tools installed, write /etc/wireguard/wg0.conf, then start wg-quick@wg0",
        apt: &["wireguard-tools"],
        dnf: &["wireguard-tools"],
        after: "",
        services: &[],
        web: false,
    },
    Installable {
        id: "node-exporter",
        hint: "Server metrics for Prometheus on port 9100; keep that port closed except to Prometheus",
        apt: &["prometheus-node-exporter"],
        dnf: &["golang-github-prometheus-node-exporter"],
        after: "",
        services: &["prometheus-node-exporter", "node_exporter"],
        web: false,
    },
];

/// After Docker: the Compose plugin, under whichever name this system packages it.
const COMPOSE_AFTER: &str = "if ! docker compose version >/dev/null 2>&1; then\n  if command -v apt-get >/dev/null 2>&1; then apt-get install -y -q docker-compose-v2 || apt-get install -y -q docker-compose-plugin || true;\n  else dnf install -y docker-compose || dnf install -y docker-compose-plugin || true; fi\nfi";

/// Daily security updates on: apt's periodic settings, or dnf-automatic's timer.
const AUTO_UPDATES_AFTER: &str = "if [ -d /etc/apt/apt.conf.d ] && command -v unattended-upgrade >/dev/null 2>&1; then\n  printf 'APT::Periodic::Update-Package-Lists \"1\";\\nAPT::Periodic::Unattended-Upgrade \"1\";\\n' > /etc/apt/apt.conf.d/20auto-upgrades\nfi\nif systemctl list-unit-files dnf-automatic-install.timer >/dev/null 2>&1; then systemctl enable --now dnf-automatic-install.timer || true; fi";

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
/// It reports how far it got for the progress ring in the "+" window:
/// `@@progress N` lines between phases, apt's own percentages
/// (`APT::Status-Fd=1`: `dlstatus:` downloading, `pmstatus:` installing), and
/// dnf/yum's package counts; the UI reads them (src/lib/installProgress.ts).
pub fn script(i: &Installable) -> String {
    let mut s = format!(
        "set -e\necho '@@progress 2'\nif command -v apt-get >/dev/null 2>&1; then\n  export DEBIAN_FRONTEND=noninteractive\n  apt-get update -q\n  echo '@@progress 10'\n  apt-get install -y -q -o APT::Status-Fd=1 {apt}\nelif command -v dnf >/dev/null 2>&1; then\n  echo '@@progress 10'\n  dnf install -y {dnf}\nelif command -v yum >/dev/null 2>&1; then\n  echo '@@progress 10'\n  yum install -y {dnf}\nelse\n  echo 'This server has neither apt nor dnf; install it with its own package manager.' >&2; exit 2\nfi\necho '@@progress 92'\n",
        apt = i.apt.join(" "),
        dnf = i.dnf.join(" ")
    );
    if !i.after.is_empty() {
        s.push_str(i.after);
        s.push('\n');
    }
    if !i.services.is_empty() {
        s.push_str("echo '@@progress 96'\n");
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
            assert!(
                s.contains("APT::Status-Fd=1") && s.contains("@@progress 10") && s.contains("@@progress 92"),
                "progress for the ring: {s}"
            );
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
        let d = script(find("docker").unwrap());
        assert!(d.contains("docker-compose-v2") && d.contains("docker compose version"), "{d}");
        let a = script(find("auto-updates").unwrap());
        assert!(a.contains("20auto-upgrades") && a.contains("dnf-automatic-install.timer"), "{a}");
        assert!(script(find("php-fpm").unwrap()).contains("php*-fpm.service"));
        assert!(
            script(find("wireguard").unwrap()).lines().all(|l| !l.contains("enable --now")),
            "WireGuard waits for its config"
        );
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
