//! Discover which well-known server software is present on a host, so the UI
//! can offer a tab for exactly what is there (nginx, Docker, PostgreSQL, …).
//!
//! Detection is one read-only script built from a fixed catalogue — nothing
//! from the user or the server is interpolated into it. A tool counts as
//! present if its binary is on PATH, systemd knows its unit, or its process
//! is running on the host. A process inside a container doesn't count by
//! itself (the host has no client for it); instead running containers whose
//! image is a known program (postgres, redis, httpd, …) are listed on the
//! tool, so its tab can work inside them.

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
    /// Installed on the server itself (binary, unit or a host process).
    pub on_host: bool,
    /// Running containers whose image is this program.
    pub containers: Vec<String>,
}

/// Image names (the last part, without tag) per tool, for databases and web
/// servers whose tab can work inside a container.
fn images(id: &str) -> &'static [&'static str] {
    match id {
        "postgresql" => &["postgres", "postgresql", "postgis", "timescaledb", "timescaledb-ha"],
        "mysql" => &["mysql", "mariadb", "percona-server", "mysql-server"],
        "redis" => &["redis", "valkey", "redis-stack", "redis-stack-server", "keydb"],
        "mongodb" => &["mongo", "mongodb", "mongodb-community-server"],
        "apache" => &["httpd", "apache"],
        _ => &[],
    }
}

/// Which tool an image runs: `docker.io/library/postgres:16-alpine` → postgresql.
pub fn image_tool(image: &str) -> Option<&'static str> {
    let repo = image.split('@').next().unwrap_or(image);
    let last = repo.rsplit('/').next().unwrap_or(repo);
    let name = last.split(':').next().unwrap_or(last).to_ascii_lowercase();
    CATALOG.iter().map(|t| t.id).find(|id| images(id).contains(&name.as_str()))
}

/// Add running containers to the tools they run, creating a tool that is only in containers.
pub fn add_containers(tools: &mut Vec<DetectedTool>, containers: &[crate::docker::Container]) {
    for c in containers.iter().filter(|c| c.state == "running") {
        let Some(id) = image_tool(&c.image) else { continue };
        let name = c.names.split(',').next().unwrap_or(&c.names).to_string();
        match tools.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.containers.push(name);
                t.running = true;
            }
            None => {
                let spec = CATALOG.iter().find(|t| t.id == id).expect("image ids come from the catalogue");
                tools.push(DetectedTool {
                    id: spec.id.into(),
                    name: spec.name.into(),
                    category: spec.category.into(),
                    running: true,
                    binary: None,
                    unit: None,
                    unit_state: None,
                    paths: Vec::new(),
                    on_host: false,
                    containers: vec![name],
                });
            }
        }
    }
    tools.sort_by_key(|t| !t.running);
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
                // With pgrep, a process counts only outside containers (its cgroup names no
                // container runtime); without pgrep (busybox), any process counts.
                "for n in {}; do c=; if command -v pgrep >/dev/null 2>&1; then for p in $(pgrep -x \"$n\" 2>/dev/null); do \
                 if grep -qsE 'docker|containerd|libpod|kubepods|lxc' \"/proc/$p/cgroup\"; then c=1; else c=2; break; fi; done; \
                 else {{ ps -Ao comm= | sed 's|.*/||; s/ *$//' | grep -qxF \"$n\" || ps -Ao ucomm= | sed 's/ *$//' | grep -qxF \"$n\"; }} >/dev/null 2>&1 && c=2; fi; \
                 [ \"$c\" = 2 ] && {{ echo \"proc=$n\"; break; }}; [ \"$c\" = 1 ] && echo \"cproc=$n\"; done\n",
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
                t.on_host = true;
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
                        on_host: false,
                        containers: Vec::new(),
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
        // As root when sudo works: config files in folders only root can read are found too.
        let (o, _) = self.exec_inspect(&detect_script()).await?;
        let mut tools = parse_detect(&o.stdout);
        if tools.iter().any(|t| t.id == "docker" && t.running) {
            if let Ok(cs) = self.list_containers().await {
                add_containers(&mut tools, &cs);
            }
        }
        Ok(tools)
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
    fn container_processes_alone_dont_count_but_containers_do() {
        // A redis-server seen only inside a container: no tab from the process itself.
        let mut tools = parse_detect("@@tool redis\ncproc=redis-server\n@@tool docker\nbin=/usr/bin/docker\nunit=docker.service active\n");
        assert_eq!(tools.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["docker"]);
        assert!(tools[0].on_host);

        let c = |name: &str, image: &str, state: &str| crate::docker::Container {
            id: format!("{name}-id"),
            names: name.into(),
            image: image.into(),
            state: state.into(),
            status: String::new(),
            ports: String::new(),
            created_at: String::new(),
            compose_project: None,
        };
        add_containers(
            &mut tools,
            &[
                c("cache", "redis:7-alpine", "running"),
                c("db", "docker.io/library/postgres:16", "running"),
                c("old-db", "postgres:15", "exited"),
                c("web", "ghcr.io/acme/shop:1.2", "running"),
            ],
        );
        let redis = tools.iter().find(|t| t.id == "redis").unwrap();
        assert!(redis.running && !redis.on_host && redis.binary.is_none());
        assert_eq!(redis.containers, ["cache"]);
        let pg = tools.iter().find(|t| t.id == "postgresql").unwrap();
        assert_eq!(pg.containers, ["db"]);
        assert_eq!(tools.len(), 3);
    }

    #[test]
    fn images_map_to_tools() {
        assert_eq!(image_tool("postgres:16-alpine"), Some("postgresql"));
        assert_eq!(image_tool("bitnami/postgresql:16"), Some("postgresql"));
        assert_eq!(image_tool("mariadb@sha256:abc"), Some("mysql"));
        assert_eq!(image_tool("registry.example.com:5000/valkey/valkey:8"), Some("redis"));
        assert_eq!(image_tool("httpd:2.4"), Some("apache"));
        assert_eq!(image_tool("Mongo:7"), Some("mongodb"));
        assert_eq!(image_tool("nginx:1.27"), None);
        assert_eq!(image_tool("acme/redis-exporter"), None);
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
    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(detect_script()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(stdout.matches("@@tool ").count(), CATALOG.len());
        let _ = parse_detect(&stdout);
    }
}
