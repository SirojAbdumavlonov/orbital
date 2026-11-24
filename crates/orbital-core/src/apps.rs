//! Apps (Stacks tab → Apps): ready-made Docker Compose stacks for popular
//! self-hosted apps. A fixed catalogue: the only values that go into a
//! compose file are a validated stack name and a port number. Every app
//! listens on 127.0.0.1 only (open it through port forwarding, or publish it
//! with New website for HTTPS). Passwords and keys are generated here, written
//! to `/opt/stacks/<name>/.env` (0600) over stdin, and reach the compose file
//! only as `${NAME}` references, so they never land in the audit log or in
//! Orbital's own storage; the ones the user needs to sign in are shown once.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::stacks::{validate_name, STACK_DIR};
use serde::Serialize;

pub struct AppSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub hint: &'static str,
    /// Suggested port on the server (127.0.0.1).
    pub port: u16,
    /// The compose file, with `@PORT@` for the host port.
    compose: &'static str,
    /// Variables generated into `.env` (random letters and digits).
    secrets: &'static [&'static str],
    /// Which of them the user needs to see, with a label.
    shown: &'static [(&'static str, &'static str)],
    /// What to do the first time it's opened.
    pub first_steps: &'static str,
    /// Roughly how much memory it wants, in MB.
    pub memory_mb: u32,
}

pub const APPS: &[AppSpec] = &[
    AppSpec {
        id: "uptime-kuma",
        name: "Uptime Kuma",
        category: "Monitoring",
        hint: "Status pages and uptime checks with alerts to Telegram, Slack, email and more",
        port: 3001,
        compose: "services:
  uptime-kuma:
    image: louislam/uptime-kuma:1
    restart: unless-stopped
    ports:
      - \"127.0.0.1:@PORT@:3001\"
    volumes:
      - data:/app/data
volumes:
  data:
",
        secrets: &[],
        shown: &[],
        first_steps: "Create the admin account on the first page.",
        memory_mb: 150,
    },
    AppSpec {
        id: "portainer",
        name: "Portainer",
        category: "Containers",
        hint: "A web UI for this server's Docker: containers, images, volumes, stacks",
        port: 9000,
        compose: "services:
  portainer:
    image: portainer/portainer-ce:lts
    restart: unless-stopped
    ports:
      - \"127.0.0.1:@PORT@:9000\"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - data:/data
volumes:
  data:
",
        secrets: &[],
        shown: &[],
        first_steps: "Create the admin user within 5 minutes of starting (after that Portainer locks setup until it's restarted).",
        memory_mb: 100,
    },
    AppSpec {
        id: "vaultwarden",
        name: "Vaultwarden",
        category: "Security",
        hint: "Password manager that works with the Bitwarden apps and browser extensions",
        port: 8081,
        compose: "services:
  vaultwarden:
    image: vaultwarden/server:latest
    restart: unless-stopped
    environment:
      ADMIN_TOKEN: ${ADMIN_TOKEN}
      SIGNUPS_ALLOWED: \"true\"
    ports:
      - \"127.0.0.1:@PORT@:80\"
    volumes:
      - data:/data
volumes:
  data:
",
        secrets: &["ADMIN_TOKEN"],
        shown: &[("Admin page token (at /admin)", "ADMIN_TOKEN")],
        first_steps:
            "Create your account, then turn sign-ups off on the /admin page. Phones and extensions need HTTPS: publish it with New website.",
        memory_mb: 60,
    },
    AppSpec {
        id: "gitea",
        name: "Gitea",
        category: "Developer tools",
        hint: "Your own Git hosting: repositories, issues, pull requests and CI",
        port: 3002,
        compose: "services:
  gitea:
    image: gitea/gitea:latest
    restart: unless-stopped
    environment:
      USER_UID: \"1000\"
      USER_GID: \"1000\"
    ports:
      - \"127.0.0.1:@PORT@:3000\"
    volumes:
      - data:/data
volumes:
  data:
",
        secrets: &[],
        shown: &[],
        first_steps: "Keep SQLite on the setup page and create the admin account there.",
        memory_mb: 200,
    },
    AppSpec {
        id: "n8n",
        name: "n8n",
        category: "Automation",
        hint: "Workflow automation: connect APIs, webhooks and AI without writing a service",
        port: 5678,
        compose: "services:
  n8n:
    image: docker.n8n.io/n8nio/n8n:latest
    restart: unless-stopped
    environment:
      N8N_ENCRYPTION_KEY: ${N8N_ENCRYPTION_KEY}
    ports:
      - \"127.0.0.1:@PORT@:5678\"
    volumes:
      - data:/home/node/.n8n
volumes:
  data:
",
        secrets: &["N8N_ENCRYPTION_KEY"],
        shown: &[],
        first_steps: "Create the owner account on the first page. Webhooks from outside need it published with New website.",
        memory_mb: 300,
    },
    AppSpec {
        id: "grafana",
        name: "Grafana",
        category: "Monitoring",
        hint: "Dashboards for Prometheus, databases and logs",
        port: 3003,
        compose: "services:
  grafana:
    image: grafana/grafana-oss:latest
    restart: unless-stopped
    environment:
      GF_SECURITY_ADMIN_PASSWORD: ${ADMIN_PASSWORD}
    ports:
      - \"127.0.0.1:@PORT@:3000\"
    volumes:
      - data:/var/lib/grafana
volumes:
  data:
",
        secrets: &["ADMIN_PASSWORD"],
        shown: &[("Password for admin", "ADMIN_PASSWORD")],
        first_steps: "Sign in as admin with the password below.",
        memory_mb: 150,
    },
    AppSpec {
        id: "wordpress",
        name: "WordPress",
        category: "Websites",
        hint: "Blog or website with its own MariaDB database",
        port: 8082,
        compose: "services:
  wordpress:
    image: wordpress:latest
    restart: unless-stopped
    depends_on:
      - db
    environment:
      WORDPRESS_DB_HOST: db
      WORDPRESS_DB_USER: wordpress
      WORDPRESS_DB_PASSWORD: ${DB_PASSWORD}
      WORDPRESS_DB_NAME: wordpress
    ports:
      - \"127.0.0.1:@PORT@:80\"
    volumes:
      - site:/var/www/html
  db:
    image: mariadb:11
    restart: unless-stopped
    environment:
      MYSQL_DATABASE: wordpress
      MYSQL_USER: wordpress
      MYSQL_PASSWORD: ${DB_PASSWORD}
      MYSQL_ROOT_PASSWORD: ${DB_ROOT_PASSWORD}
    volumes:
      - db:/var/lib/mysql
volumes:
  site:
  db:
",
        secrets: &["DB_PASSWORD", "DB_ROOT_PASSWORD"],
        shown: &[],
        first_steps: "Publish it with New website first: WordPress remembers the address it was set up on.",
        memory_mb: 400,
    },
    AppSpec {
        id: "nextcloud",
        name: "Nextcloud",
        category: "Files",
        hint: "Files, calendar and contacts in your own cloud, with a MariaDB database",
        port: 8084,
        compose: "services:
  nextcloud:
    image: nextcloud:latest
    restart: unless-stopped
    depends_on:
      - db
    environment:
      MYSQL_HOST: db
      MYSQL_DATABASE: nextcloud
      MYSQL_USER: nextcloud
      MYSQL_PASSWORD: ${DB_PASSWORD}
      NEXTCLOUD_ADMIN_USER: admin
      NEXTCLOUD_ADMIN_PASSWORD: ${ADMIN_PASSWORD}
      NEXTCLOUD_TRUSTED_DOMAINS: localhost 127.0.0.1
    ports:
      - \"127.0.0.1:@PORT@:80\"
    volumes:
      - data:/var/www/html
  db:
    image: mariadb:11
    restart: unless-stopped
    environment:
      MYSQL_DATABASE: nextcloud
      MYSQL_USER: nextcloud
      MYSQL_PASSWORD: ${DB_PASSWORD}
      MYSQL_ROOT_PASSWORD: ${DB_ROOT_PASSWORD}
    volumes:
      - db:/var/lib/mysql
volumes:
  data:
  db:
",
        secrets: &["DB_PASSWORD", "DB_ROOT_PASSWORD", "ADMIN_PASSWORD"],
        shown: &[("Password for admin", "ADMIN_PASSWORD")],
        first_steps:
            "Sign in as admin. Setup takes a minute on the first start. For a domain, add it to trusted_domains in config/config.php.",
        memory_mb: 512,
    },
    AppSpec {
        id: "metabase",
        name: "Metabase",
        category: "Analytics",
        hint: "Charts and dashboards over your PostgreSQL or MySQL data, no SQL needed",
        port: 3004,
        compose: "services:
  metabase:
    image: metabase/metabase:latest
    restart: unless-stopped
    environment:
      MB_DB_FILE: /metabase-data/metabase.db
    ports:
      - \"127.0.0.1:@PORT@:3000\"
    volumes:
      - data:/metabase-data
    extra_hosts:
      - \"host.docker.internal:host-gateway\"
volumes:
  data:
",
        secrets: &[],
        shown: &[],
        first_steps: "Create the admin account. Databases on this server are at host.docker.internal.",
        memory_mb: 1024,
    },
    AppSpec {
        id: "adminer",
        name: "Adminer",
        category: "Databases",
        hint: "One-page database browser for MySQL, PostgreSQL and SQLite",
        port: 8083,
        compose: "services:
  adminer:
    image: adminer:latest
    restart: unless-stopped
    ports:
      - \"127.0.0.1:@PORT@:8080\"
    extra_hosts:
      - \"host.docker.internal:host-gateway\"
",
        secrets: &[],
        shown: &[],
        first_steps:
            "Sign in with a database user. Databases on this server are at host.docker.internal; keep it private (no New website).",
        memory_mb: 30,
    },
];

pub fn find(id: &str) -> Result<&'static AppSpec> {
    APPS.iter()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::InvalidInput(format!("Orbital has no app {id:?}")))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppOption {
    pub id: String,
    pub name: String,
    pub category: String,
    pub hint: String,
    pub port: u16,
    pub first_steps: String,
    pub memory_mb: u32,
    /// Images it pulls, to show before installing.
    pub images: Vec<String>,
}

pub fn images(a: &AppSpec) -> Vec<String> {
    a.compose
        .lines()
        .filter_map(|l| l.trim().strip_prefix("image: "))
        .map(str::to_string)
        .collect()
}

pub fn options() -> Vec<AppOption> {
    APPS.iter()
        .map(|a| AppOption {
            id: a.id.into(),
            name: a.name.into(),
            category: a.category.into(),
            hint: a.hint.into(),
            port: a.port,
            first_steps: a.first_steps.into(),
            memory_mb: a.memory_mb,
            images: images(a),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub name: String,
    pub app: String,
    pub port: u16,
    /// `docker compose up` succeeded.
    pub started: bool,
    /// Compose's output (no secrets: they only exist in .env).
    pub output: String,
    /// Generated passwords the user signs in with, shown once.
    pub shown: Vec<Shown>,
    pub first_steps: String,
}

pub fn validate_port(port: u16) -> Result<u16> {
    if port < 1024 {
        return Err(Error::InvalidInput("pick a port from 1024 to 65535".into()));
    }
    Ok(port)
}

/// The compose file for this app on `port`, headed with what it is.
pub fn compose(a: &AppSpec, name: &str, port: u16) -> Result<String> {
    validate_name(name)?;
    validate_port(port)?;
    Ok(format!(
        "# {app} ({name}), installed from Orbital's Apps.\n# Listens on 127.0.0.1:{port} only. Passwords and keys are in .env beside this file.\n{body}",
        app = a.name,
        body = a.compose.replace("@PORT@", &port.to_string())
    ))
}

/// Random letters and digits (about 190 bits for 32), safe anywhere in YAML or a URL.
pub fn secret(len: usize) -> String {
    use chacha20poly1305::aead::rand_core::RngCore;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(len);
    while out.len() < len {
        let mut b = [0u8; 64];
        chacha20poly1305::aead::OsRng.fill_bytes(&mut b);
        // 62 * 4 = 248: drop bytes above that so every character is equally likely.
        out.extend(b.iter().filter(|x| **x < 248).map(|x| ALPHABET[(*x as usize) % 62] as char));
    }
    out.truncate(len);
    out
}

/// The `.env` file's contents (sent on stdin) and the values to show the user.
pub fn env_file(a: &AppSpec) -> (String, Vec<(String, String)>) {
    let values: Vec<(&str, String)> = a.secrets.iter().map(|k| (*k, secret(32))).collect();
    let mut env = String::from("# Generated by Orbital. Keep this file private.\n");
    for (k, v) in &values {
        env.push_str(&format!("{k}={v}\n"));
    }
    let shown = a
        .shown
        .iter()
        .filter_map(|(label, key)| values.iter().find(|(k, _)| k == key).map(|(_, v)| (label.to_string(), v.clone())))
        .collect();
    (env, shown)
}

/// Writes stdin to the stack's `.env`, readable only by the login user (who
/// may run Compose without sudo) and root.
pub fn write_env_command(name: &str) -> Result<String> {
    validate_name(name)?;
    let f = quote(&format!("{STACK_DIR}/{name}/.env"));
    Ok(format!(
        "umask 077 && cat > {f} && chmod 600 {f} && chown \"${{SUDO_USER:-$(id -un)}}\" {f}"
    ))
}

/// Read-only checks before installing: Docker and Compose there, the name
/// free, the port free, memory. Prints `@@` lines for [`parse_preflight`].
pub fn preflight_script(name: &str, port: u16) -> Result<String> {
    validate_name(name)?;
    validate_port(port)?;
    let dir = quote(&format!("{STACK_DIR}/{name}"));
    Ok(format!(
        "export LC_ALL=C\ncommand -v docker >/dev/null 2>&1 || echo '@@nodocker'\ndocker compose version >/dev/null 2>&1 || echo '@@nocompose'\n[ -e {dir} ] && echo '@@exists'\n(ss -Hltn 2>/dev/null || netstat -ltn 2>/dev/null) | awk '{{print $4}}' | grep -Eq '[:.]{port}$' && echo '@@port'\nawk '/^MemAvailable:/ {{print \"@@mem \" int($2/1024)}}' /proc/meminfo 2>/dev/null\ntrue"
    ))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preflight {
    pub no_docker: bool,
    pub no_compose: bool,
    pub exists: bool,
    pub port_taken: bool,
    pub mem_available_mb: Option<u32>,
}

pub fn parse_preflight(out: &str) -> Preflight {
    let mut p = Preflight::default();
    for l in out.lines().map(str::trim) {
        match l {
            "@@nodocker" => p.no_docker = true,
            "@@nocompose" => p.no_compose = true,
            "@@exists" => p.exists = true,
            "@@port" => p.port_taken = true,
            _ => {
                if let Some(m) = l.strip_prefix("@@mem ") {
                    p.mem_available_mb = m.trim().parse().ok();
                }
            }
        }
    }
    p
}

/// What stops the install: the refusal, and whether `force` may override it
/// (only for low memory; never for a missing Docker, a taken name or port).
pub fn blocker(a: &AppSpec, name: &str, port: u16, p: &Preflight) -> Option<(String, bool)> {
    if p.no_docker {
        return Some(("Docker isn't installed here; install it from the \"+\" menu first".into(), false));
    }
    if p.no_compose {
        return Some((
            "Docker Compose isn't installed (apt: docker-compose-v2 or docker-compose-plugin; dnf: docker-compose)".into(),
            false,
        ));
    }
    if p.exists {
        return Some((format!("{STACK_DIR}/{name} already exists; pick another name"), false));
    }
    if p.port_taken {
        return Some((format!("port {port} is already in use on this server; pick another"), false));
    }
    match p.mem_available_mb {
        Some(m) if m < a.memory_mb => Some((
            format!(
                "{} wants about {} MB of memory and this server has {m} MB free",
                a.name, a.memory_mb
            ),
            true,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_app_is_private_and_keeps_secrets_out_of_the_file() {
        let mut ids: Vec<&str> = APPS.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), APPS.len());
        for a in APPS {
            assert!(validate_name(a.id).is_ok(), "{} is a stack name", a.id);
            let c = compose(a, a.id, a.port).unwrap();
            assert!(!c.contains("@PORT@"));
            assert!(c.contains(&format!("\"127.0.0.1:{}:", a.port)), "{}: 127.0.0.1 only", a.id);
            // Every published port is bound to 127.0.0.1.
            for l in c.lines().filter(|l| l.trim_start().starts_with("- \"") && l.contains(':')) {
                let v = l.trim().trim_start_matches("- ").trim_matches('"');
                if v.split(':').count() == 3 && !v.starts_with('/') {
                    assert!(v.starts_with("127.0.0.1:"), "{}: {l}", a.id);
                }
            }
            for k in a.secrets {
                assert!(c.contains(&format!("${{{k}}}")), "{}: {k} is used", a.id);
            }
            for (_, k) in a.shown {
                assert!(a.secrets.contains(k));
            }
            let (env, shown) = env_file(a);
            assert_eq!(shown.len(), a.shown.len());
            for line in env.lines().filter(|l| !l.starts_with('#')) {
                let (k, v) = line.split_once('=').unwrap();
                assert!(a.secrets.contains(&k) && v.len() == 32 && !c.contains(v));
            }
            assert!(!images(a).is_empty() && images(a).iter().all(|i| !i.contains(' ')));
        }
        assert_eq!(options().len(), APPS.len());
        assert!(find("grafana").is_ok() && find("x; reboot").is_err());
    }

    #[test]
    fn names_and_ports_are_checked() {
        let g = find("grafana").unwrap();
        assert!(compose(g, "Bad Name", 3003).is_err() && compose(g, "g", 80).is_err());
        assert!(compose(g, "dash", 4000).unwrap().contains("127.0.0.1:4000:3000"));
        assert!(write_env_command("a;b").is_err() && preflight_script("a b", 3000).is_err());
        assert_eq!(
            write_env_command("dash").unwrap(),
            "umask 077 && cat > /opt/stacks/dash/.env && chmod 600 /opt/stacks/dash/.env && chown \"${SUDO_USER:-$(id -un)}\" /opt/stacks/dash/.env"
        );
        for c in [write_env_command("dash").unwrap(), preflight_script("dash", 4000).unwrap()] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
            assert!(o.status.success(), "{c}");
        }
    }

    #[test]
    fn secrets_are_random_letters_and_digits() {
        let a = secret(32);
        let b = secret(32);
        assert!(a.len() == 32 && a != b && a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(secret(100).len(), 100);
    }

    #[test]
    fn preflight_blocks_what_cant_work() {
        let g = find("grafana").unwrap();
        let p = parse_preflight("@@port\n@@mem 2048\n");
        assert_eq!(
            p,
            Preflight {
                port_taken: true,
                mem_available_mb: Some(2048),
                ..Default::default()
            }
        );
        assert!(matches!(blocker(g, "g", 3003, &p), Some((m, false)) if m.contains("port 3003")));
        assert!(matches!(blocker(g, "g", 3003, &parse_preflight("@@nocompose")), Some((_, false))));
        assert!(matches!(blocker(g, "g", 3003, &parse_preflight("@@exists")), Some((_, false))));
        let m = find("metabase").unwrap();
        assert!(matches!(blocker(m, "m", 3004, &parse_preflight("@@mem 300")), Some((_, true))));
        assert!(blocker(g, "g", 3003, &parse_preflight("@@mem 4000")).is_none());
        // The script runs where nothing is installed and prints only markers.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(preflight_script("orbital-test-none", 65001).unwrap())
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).lines().all(|l| l.starts_with("@@")));
    }
}

#[cfg(test)]
mod compose_check {
    /// Every app's file is valid for Compose itself, when Docker is on this machine.
    #[test]
    fn docker_compose_accepts_every_app() {
        let ok = std::process::Command::new("docker")
            .args(["compose", "version"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("docker compose isn't here; skipped");
            return;
        }
        for a in super::APPS {
            let dir = std::env::temp_dir().join(format!("orbital-app-{}-{}", a.id, std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("compose.yaml"), super::compose(a, a.id, a.port).unwrap()).unwrap();
            std::fs::write(dir.join(".env"), super::env_file(a).0).unwrap();
            let o = std::process::Command::new("docker")
                .args(["compose", "-f"])
                .arg(dir.join("compose.yaml"))
                .args(["config", "-q"])
                .output()
                .unwrap();
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            std::fs::remove_dir_all(&dir).ok();
            // Another test may point HOME elsewhere meanwhile, hiding the Compose plugin.
            if err.contains("unknown shorthand flag") || err.contains("is not a docker command") {
                eprintln!("docker compose went missing; skipped");
                return;
            }
            assert!(o.status.success() && !err.contains("not set"), "{}: {err}", a.id);
        }
    }
}
