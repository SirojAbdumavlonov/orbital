//! Discover which well-known server software is present on a host, so the UI
//! can offer a tab for exactly what is there (nginx, Docker, PostgreSQL, …).
//!
//! Detection is one read-only script built from a fixed catalogue — nothing
//! from the user or the server is interpolated into it. A tool counts as
//! present if its binary is on PATH, systemd knows its unit, or its process
//! is running.

use crate::shell::quote;
use crate::ssh::SshSession;
use serde::Serialize;

pub struct ToolSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    bins: &'static [&'static str],
    units: &'static [&'static str],
    procs: &'static [&'static str],
    /// Config files and folders worth showing, if they exist.
    paths: &'static [&'static str],
}

pub const CATALOG: &[ToolSpec] = &[
    ToolSpec {
        id: "nginx",
        name: "nginx",
        category: "Web server",
        bins: &["nginx", "openresty"],
        units: &["nginx.service", "openresty.service"],
        procs: &["nginx"],
        paths: &["/etc/nginx", "/usr/local/openresty/nginx/conf", "/var/log/nginx"],
    },
    ToolSpec {
        id: "apache",
        name: "Apache",
        category: "Web server",
        bins: &["apache2", "httpd"],
        units: &["apache2.service", "httpd.service"],
        procs: &["apache2", "httpd"],
        paths: &["/etc/apache2", "/etc/httpd", "/var/log/apache2", "/var/log/httpd"],
    },
    ToolSpec {
        id: "caddy",
        name: "Caddy",
        category: "Web server",
        bins: &["caddy"],
        units: &["caddy.service"],
        procs: &["caddy"],
        paths: &["/etc/caddy/Caddyfile", "/etc/caddy"],
    },
    ToolSpec {
        id: "haproxy",
        name: "HAProxy",
        category: "Load balancer",
        bins: &["haproxy"],
        units: &["haproxy.service"],
        procs: &["haproxy"],
        paths: &["/etc/haproxy/haproxy.cfg", "/etc/haproxy"],
    },
    ToolSpec {
        id: "docker",
        name: "Docker",
        category: "Containers",
        bins: &["docker"],
        units: &["docker.service"],
        procs: &["dockerd"],
        paths: &["/etc/docker/daemon.json", "/etc/docker"],
    },
    ToolSpec {
        id: "postgresql",
        name: "PostgreSQL",
        category: "Database",
        bins: &["psql", "postgres"],
        units: &["postgresql.service"],
        procs: &["postgres"],
        paths: &["/etc/postgresql", "/var/lib/pgsql/data/postgresql.conf", "/var/log/postgresql"],
    },
    ToolSpec {
        id: "mysql",
        name: "MySQL / MariaDB",
        category: "Database",
        bins: &["mysqld", "mariadbd", "mysql", "mariadb"],
        units: &["mysql.service", "mariadb.service", "mysqld.service"],
        procs: &["mysqld", "mariadbd"],
        paths: &["/etc/mysql", "/etc/my.cnf", "/etc/my.cnf.d", "/var/log/mysql"],
    },
    ToolSpec {
        id: "redis",
        name: "Redis",
        category: "Database",
        bins: &["redis-server", "valkey-server"],
        units: &["redis-server.service", "redis.service", "valkey-server.service", "valkey.service"],
        procs: &["redis-server", "valkey-server"],
        paths: &["/etc/redis/redis.conf", "/etc/redis.conf", "/etc/redis", "/etc/valkey"],
    },
    ToolSpec {
        id: "mongodb",
        name: "MongoDB",
        category: "Database",
        bins: &["mongod"],
        units: &["mongod.service", "mongodb.service"],
        procs: &["mongod"],
        paths: &["/etc/mongod.conf", "/var/log/mongodb"],
    },
    ToolSpec {
        id: "php-fpm",
        name: "PHP-FPM",
        category: "Runtime",
        bins: &["php-fpm"],
        units: &["php-fpm.service"],
        procs: &["php-fpm"],
        paths: &["/etc/php", "/etc/php-fpm.conf", "/etc/php-fpm.d"],
    },
    ToolSpec {
        id: "sshd",
        name: "SSH server",
        category: "Access",
        bins: &["sshd"],
        units: &["ssh.service", "sshd.service"],
        procs: &["sshd"],
        paths: &["/etc/ssh/sshd_config", "/etc/ssh/sshd_config.d"],
    },
    ToolSpec {
        id: "fail2ban",
        name: "Fail2ban",
        category: "Access",
        bins: &["fail2ban-server"],
        units: &["fail2ban.service"],
        procs: &["fail2ban-server"],
        paths: &["/etc/fail2ban/jail.local", "/etc/fail2ban", "/var/log/fail2ban.log"],
    },
    ToolSpec {
        id: "ufw",
        name: "UFW firewall",
        category: "Access",
        bins: &["ufw"],
        units: &["ufw.service"],
        procs: &[],
        paths: &["/etc/ufw", "/etc/default/ufw"],
    },
    ToolSpec {
        id: "certbot",
        name: "HTTPS certificates",
        category: "Certificates",
        bins: &["certbot"],
        units: &["certbot.timer", "snap.certbot.renew.timer"],
        procs: &[],
        paths: &["/etc/letsencrypt/renewal", "/etc/letsencrypt/cli.ini", "/var/log/letsencrypt"],
    },
    ToolSpec {
        id: "cron",
        name: "Cron",
        category: "Scheduling",
        bins: &["cron", "crond"],
        units: &["cron.service", "crond.service"],
        procs: &["cron", "crond"],
        paths: &["/etc/crontab", "/etc/cron.d"],
    },
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPath {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTool {
    pub id: String,
    pub name: String,
    pub category: String,
    pub running: bool,
    /// Path of the binary, when it is on PATH.
    pub binary: Option<String>,
    /// The systemd unit that manages it, when systemd knows one.
    pub unit: Option<String>,
    /// `active`, `inactive`, `failed`, … for that unit.
    pub unit_state: Option<String>,
    pub paths: Vec<ToolPath>,
}

fn words(items: &[&str]) -> String {
    items.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
}

/// The detection script: prints one `@@tool <id>` section per catalogue entry.
pub fn detect_script() -> String {
    let mut s = String::from("export LC_ALL=C\n");
    for t in CATALOG {
        s.push_str(&format!("echo {}\n", quote(&format!("@@tool {}", t.id))));
        if !t.bins.is_empty() {
            s.push_str(&format!(
                "for b in {}; do p=$(command -v \"$b\" 2>/dev/null) && {{ echo \"bin=$p\"; break; }}; done\n",
                words(t.bins)
            ));
        }
        if !t.units.is_empty() {
            // First unit systemd has loaded wins; prints "unit=<name> <ActiveState>".
            s.push_str(&format!(
                "for u in {}; do st=$(systemctl show -p LoadState -p ActiveState \"$u\" 2>/dev/null | tr '\\n' ' '); \
                 case \"$st\" in *LoadState=loaded*) a=${{st##*ActiveState=}}; echo \"unit=$u ${{a%% *}}\"; break;; esac; done\n",
                words(t.units)
            ));
        }
        if !t.procs.is_empty() {
            s.push_str(&format!(
                "for n in {}; do {{ pgrep -x \"$n\" || ps -Ao comm= | sed 's|.*/||; s/ *$//' | grep -qxF \"$n\" || ps -Ao ucomm= | sed 's/ *$//' | grep -qxF \"$n\"; }} >/dev/null 2>&1 && {{ echo \"proc=$n\"; break; }}; done\n",
                words(t.procs)
            ));
        }
        if !t.paths.is_empty() {
            s.push_str(&format!(
                "for f in {}; do if [ -d \"$f\" ]; then echo \"dir=$f\"; elif [ -e \"$f\" ]; then echo \"file=$f\"; fi; done\n",
                words(t.paths)
            ));
        }
    }
    s
}

/// Parse the script's output. Unknown ids and stray lines are ignored; tools
/// with no sign of being installed are left out. Running tools come first.
pub fn parse_detect(out: &str) -> Vec<DetectedTool> {
    let mut found: Vec<DetectedTool> = Vec::new();
    let mut cur: Option<(DetectedTool, bool)> = None; // (tool, process seen)
    let finish = |cur: Option<(DetectedTool, bool)>, found: &mut Vec<DetectedTool>| {
        if let Some((mut t, proc_seen)) = cur {
            let present = t.binary.is_some() || t.unit.is_some() || proc_seen;
            if present {
                t.running = proc_seen || matches!(t.unit_state.as_deref(), Some("active" | "reloading" | "activating"));
                found.push(t);
            }
        }
    };
    for line in out.lines() {
        let line = line.trim_end();
        if let Some(id) = line.strip_prefix("@@tool ") {
            finish(cur.take(), &mut found);
            cur = CATALOG.iter().find(|t| t.id == id).map(|spec| {
                (
                    DetectedTool {
                        id: spec.id.into(),
                        name: spec.name.into(),
                        category: spec.category.into(),
                        running: false,
                        binary: None,
                        unit: None,
                        unit_state: None,
                        paths: Vec::new(),
                    },
                    false,
                )
            });
            continue;
        }
        let Some((tool, proc_seen)) = cur.as_mut() else { continue };
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "bin" if value.starts_with('/') => tool.binary = Some(value.to_string()),
            "unit" => {
                let mut it = value.split_whitespace();
                if let Some(unit) = it.next() {
                    tool.unit = Some(unit.to_string());
                    tool.unit_state = it.next().map(String::from);
                }
            }
            "proc" => *proc_seen = true,
            "dir" | "file" if value.starts_with('/') => tool.paths.push(ToolPath {
                path: value.to_string(),
                is_dir: key == "dir",
            }),
            _ => {}
        }
    }
    finish(cur.take(), &mut found);
    found.sort_by_key(|t| !t.running);
    found
}

impl SshSession {
    pub async fn detect_tools(&self) -> crate::Result<Vec<DetectedTool>> {
        Ok(parse_detect(&self.exec(&detect_script()).await?.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@tool nginx\nbin=/usr/sbin/nginx\nunit=nginx.service active\nproc=nginx\ndir=/etc/nginx\ndir=/var/log/nginx\n\
@@tool apache\n\
@@tool docker\nbin=/usr/bin/docker\nunit=docker.service active\nproc=dockerd\nfile=/etc/docker/daemon.json\n\
@@tool redis\nbin=/usr/bin/redis-server\nunit=redis-server.service inactive\nfile=/etc/redis/redis.conf\n\
@@tool mysql\nunit=mysql.service failed\n\
@@tool sshd\nproc=sshd\nfile=/etc/ssh/sshd_config\n\
@@tool made-up\nbin=/usr/bin/evil\n\
garbage line\n";

    #[test]
    fn parses_detection_output() {
        let tools = parse_detect(OUT);
        let ids: Vec<&str> = tools.iter().map(|t| t.id.as_str()).collect();
        // Running first (stable within each group); absent and unknown tools dropped.
        assert_eq!(ids, ["nginx", "docker", "sshd", "redis", "mysql"]);

        let nginx = &tools[0];
        assert!(nginx.running);
        assert_eq!(nginx.unit.as_deref(), Some("nginx.service"));
        assert_eq!(nginx.unit_state.as_deref(), Some("active"));
        assert_eq!(nginx.binary.as_deref(), Some("/usr/sbin/nginx"));
        assert_eq!(nginx.paths.len(), 2);
        assert!(nginx.paths[0].is_dir);

        // Process without a unit (container, init script) still counts as running.
        let sshd = tools.iter().find(|t| t.id == "sshd").unwrap();
        assert!(sshd.running && sshd.unit.is_none());
        assert!(!sshd.paths[0].is_dir);

        let redis = tools.iter().find(|t| t.id == "redis").unwrap();
        assert!(!redis.running);
        let mysql = tools.iter().find(|t| t.id == "mysql").unwrap();
        assert!(!mysql.running);
        assert_eq!(mysql.unit_state.as_deref(), Some("failed"));

        assert!(parse_detect("").is_empty());
    }

    #[test]
    fn script_is_built_from_the_catalogue_only() {
        let s = detect_script();
        for t in CATALOG {
            assert!(s.contains(&format!("@@tool {}", t.id)));
        }
        // Ids are unique and every entry can be detected somehow.
        let mut ids: Vec<&str> = CATALOG.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        assert!(CATALOG.iter().all(|t| !t.bins.is_empty() || !t.units.is_empty()));
    }

    /// The script must be valid POSIX sh and behave on a machine where most
    /// tools are missing.
    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(detect_script()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(stdout.matches("@@tool ").count(), CATALOG.len());
        let _ = parse_detect(&stdout);
    }
}
