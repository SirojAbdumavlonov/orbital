//! Move (or copy) a container to another server, with everything it needs:
//! its settings (from `docker inspect`), its image (pulled by exact digest
//! when it came from a registry, otherwise streamed with `docker save | load`),
//! its named volumes and bind-mounted files and folders (streamed with tar),
//! its network, and then `docker run` with the same options on the target.
//!
//! Data flows source → this computer → target over the two SSH connections.
//! Environment variables (which often hold passwords) go to the target over
//! stdin into a temporary env file, never onto a command line or into the log.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use russh::ChannelMsg;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MoveMode {
    /// The original keeps running; the target gets a copy.
    Copy,
    /// The original is stopped before its data is copied (consistent data) and
    /// stays stopped; it's started again if anything fails.
    Move,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MountCopy {
    /// Host path on the source (a volume's mount point, or a bind source).
    pub source: String,
    /// Named volume to create on the target; None for a bind mount (same path on the target).
    pub volume: Option<String>,
    pub dest: String,
}

/// Everything needed to recreate a container elsewhere.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MovePlan {
    pub name: String,
    pub image: String,
    pub image_id: String,
    /// `repo@sha256:…` when the image came from a registry: the target pulls exactly that.
    pub repo_digest: Option<String>,
    pub running: bool,
    /// `docker run` options and arguments, before quoting (no env values here).
    pub run_args: Vec<String>,
    /// Number of environment variables passed through the env file.
    pub env_count: usize,
    /// Host ports the container publishes (checked on the target first).
    pub host_ports: Vec<u16>,
    pub copies: Vec<MountCopy>,
    /// A user-defined network to create on the target if missing.
    pub network: Option<String>,
    /// Things worth knowing before moving.
    pub notes: Vec<String>,
    /// Published ports, editable before moving.
    pub ports: Vec<PortMap>,
    /// Arguments after the image (the entry point's own plus the command), editable.
    pub args: Vec<String>,
    /// Environment, editable; values that look like secrets are left out (`value: None`).
    pub env_vars: Vec<EnvVar>,
    /// Settings that may tie it to the old server (domains in arguments or variables).
    pub hints: Vec<String>,
    #[serde(skip)]
    env: Vec<(String, String)>,
    #[serde(skip)]
    options: Vec<String>,
    #[serde(skip)]
    entrypoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortMap {
    /// "" or "0.0.0.0": every address; "127.0.0.1": this server only.
    pub host_ip: String,
    /// None: Docker picks a free port.
    pub host_port: Option<u16>,
    /// e.g. "9000/tcp".
    pub container: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub key: String,
    /// None when the name looks like a secret (password, token, key): not shown, kept unless changed.
    pub value: Option<String>,
    pub secret: bool,
}

/// Changes made in the Move dialog.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveOptions {
    pub ports: Option<Vec<PortMap>>,
    pub args: Option<Vec<String>>,
    /// New or changed variables.
    #[serde(default)]
    pub env_set: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub env_remove: Vec<String>,
}

fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    [
        "PASSWORD",
        "PASSWD",
        "PASS",
        "SECRET",
        "TOKEN",
        "KEY",
        "PRIVATE",
        "CREDENTIAL",
        "AUTH",
        "SALT",
    ]
    .iter()
    .any(|w| k.contains(w))
}

static ENV_KEY: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]{0,127}$").unwrap());
static CPORT: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"^\d{1,5}(/(tcp|udp|sctp))?$").unwrap());
static DOMAIN: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?i)\b((?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,24})\b").unwrap());

/// Domain names mentioned in a value (file names like grafana.ini aren't domains).
pub fn domains_in(v: &str) -> Vec<String> {
    const FILES: &[&str] = &[
        "ini", "conf", "json", "yml", "yaml", "toml", "sh", "py", "js", "log", "txt", "xml", "env", "pem", "crt", "key", "db", "sock",
        "html", "php", "so", "jar",
    ];
    DOMAIN
        .captures_iter(v)
        .filter_map(|c| {
            let m = c.get(1)?;
            let d = m.as_str().to_ascii_lowercase();
            let tld = d.rsplit('.').next().unwrap_or("");
            // Part of a file path (…/grafana.ini), but not a URL's host (https://…).
            let before = &v[..m.start()];
            let after_slash = before.ends_with('/') && !before.ends_with("//");
            (!FILES.contains(&tld) && !after_slash && !d.ends_with(".local") && d != "localhost").then_some(d)
        })
        .collect()
}

impl MovePlan {
    /// Rebuild the `docker run` arguments and the derived lists from the editable parts.
    fn rebuild(&mut self) {
        let mut a = self.options.clone();
        let mut ports = Vec::new();
        for p in &self.ports {
            a.push("-p".into());
            let ip = p.host_ip.trim();
            a.push(match (ip, p.host_port) {
                ("" | "0.0.0.0" | "::", None) => p.container.clone(),
                ("" | "0.0.0.0" | "::", Some(hp)) => format!("{hp}:{}", p.container),
                (ip, None) => format!("{ip}::{}", p.container),
                (ip, Some(hp)) => format!("{ip}:{hp}:{}", p.container),
            });
            if let Some(hp) = p.host_port {
                if !ports.contains(&hp) {
                    ports.push(hp);
                }
            }
        }
        if let Some(e) = &self.entrypoint {
            a.push("--entrypoint".into());
            a.push(e.clone());
        }
        a.push(self.image.clone());
        a.extend(self.args.iter().cloned());
        self.run_args = a;
        self.host_ports = ports;
        self.env_count = self.env.len();
        self.env_vars = self
            .env
            .iter()
            .map(|(k, v)| EnvVar {
                key: k.clone(),
                value: (!looks_secret(k)).then(|| v.clone()),
                secret: looks_secret(k),
            })
            .collect();
        let mut hints = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (what, text) in self.args.iter().map(|a| ("an argument".to_string(), a.clone())).chain(
            self.env
                .iter()
                .filter(|(k, _)| !looks_secret(k))
                .map(|(k, v)| (format!("variable {k}"), v.clone())),
        ) {
            for d in domains_in(&text) {
                if seen.insert(d.clone()) {
                    hints.push(format!("{what} mentions {d}: if that name points to the old server, change it here or update its DNS and web server after moving."));
                }
            }
        }
        self.hints = hints;
    }

    /// Apply the dialog's changes, checking each one.
    pub fn apply(&mut self, o: &MoveOptions) -> Result<()> {
        if let Some(ports) = &o.ports {
            for p in ports {
                let ip = p.host_ip.trim();
                if !ip.is_empty() && ip.parse::<std::net::IpAddr>().is_err() {
                    return Err(Error::InvalidInput(format!("{ip:?} isn't an IP address")));
                }
                if !CPORT.is_match(&p.container) {
                    return Err(Error::InvalidInput(format!(
                        "{:?} isn't a container port like 9000/tcp",
                        p.container
                    )));
                }
                if p.host_port == Some(0) {
                    return Err(Error::InvalidInput("port 0 can't be published".into()));
                }
            }
            self.ports = ports.clone();
        }
        if let Some(args) = &o.args {
            if args.iter().any(|a| a.contains('\0') || a.contains('\n')) {
                return Err(Error::InvalidInput("an argument can't contain a line break".into()));
            }
            self.args = args.clone();
        }
        for k in &o.env_remove {
            self.env.retain(|(key, _)| key != k);
        }
        for (k, v) in &o.env_set {
            if !ENV_KEY.is_match(k) {
                return Err(Error::InvalidInput(format!("{k:?} isn't a variable name")));
            }
            if v.contains('\n') || v.contains('\0') {
                return Err(Error::InvalidInput(format!("variable {k} can't have several lines")));
            }
            match self.env.iter_mut().find(|(key, _)| key == k) {
                Some(e) => e.1 = v.clone(),
                None => self.env.push((k.clone(), v.clone())),
            }
        }
        self.rebuild();
        Ok(())
    }
}

/// Mounts that belong to the host, not the container: used as they are on the target, never copied.
fn system_path(p: &str) -> bool {
    ["/proc", "/sys", "/dev", "/run", "/var/run", "/var/lib/docker/containers"]
        .iter()
        .any(|x| p == *x || p.starts_with(&format!("{x}/")))
        || matches!(
            p,
            "/etc/localtime" | "/etc/timezone" | "/etc/hosts" | "/etc/resolv.conf" | "/" | "/etc" | "/home" | "/root"
        )
}

fn s<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(v, |v, k| v.get(k).unwrap_or(&Value::Null))
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// Read `docker inspect` output into a plan. Pure; the engine runs it.
pub fn plan(inspect: &Value) -> Result<MovePlan> {
    let name = s(inspect, &["Name"]).as_str().unwrap_or("").trim_start_matches('/').to_string();
    crate::shell::validate_container(&name)?;
    let image = s(inspect, &["Config", "Image"]).as_str().unwrap_or("").to_string();
    let image_id = s(inspect, &["Image"]).as_str().unwrap_or("").to_string();
    if image.is_empty() || image_id.is_empty() {
        return Err(Error::InvalidInput(
            "docker inspect didn't say which image this container runs".into(),
        ));
    }
    let running = s(inspect, &["State", "Running"]).as_bool().unwrap_or(false);
    let hc = s(inspect, &["HostConfig"]);
    let cfg = s(inspect, &["Config"]);
    let mut a: Vec<String> = vec!["--name".into(), name.clone()];
    let mut notes = Vec::new();

    let restart = s(hc, &["RestartPolicy", "Name"]).as_str().unwrap_or("");
    match restart {
        "" | "no" => {}
        "on-failure" => {
            let n = s(hc, &["RestartPolicy", "MaximumRetryCount"]).as_u64().unwrap_or(0);
            a.push("--restart".into());
            a.push(if n > 0 { format!("on-failure:{n}") } else { "on-failure".into() });
        }
        r => {
            a.push("--restart".into());
            a.push(r.into());
        }
    }

    // Published ports: "3000/tcp": [{"HostIp": "127.0.0.1", "HostPort": "3000"}]
    let mut ports = Vec::new();
    if let Some(pb) = s(hc, &["PortBindings"]).as_object() {
        let mut keys: Vec<&String> = pb.keys().collect();
        keys.sort();
        for cport in keys {
            for b in pb[cport].as_array().into_iter().flatten() {
                let ip = b.get("HostIp").and_then(Value::as_str).unwrap_or("");
                let hp = b.get("HostPort").and_then(Value::as_str).unwrap_or("");
                ports.push(PortMap {
                    host_ip: if ip == "0.0.0.0" || ip == "::" {
                        String::new()
                    } else {
                        ip.to_string()
                    },
                    host_port: hp.parse().ok().filter(|p| *p > 0),
                    container: cport.clone(),
                });
            }
        }
    }

    // Mounts: volumes and binds are copied; tmpfs recreated; host paths used as they are.
    let mut copies = Vec::new();
    for m in s(inspect, &["Mounts"]).as_array().into_iter().flatten() {
        let kind = m.get("Type").and_then(Value::as_str).unwrap_or("");
        let src = m.get("Source").and_then(Value::as_str).unwrap_or("").to_string();
        let dest = m.get("Destination").and_then(Value::as_str).unwrap_or("").to_string();
        let ro = m.get("RW").and_then(Value::as_bool) == Some(false);
        let suffix = if ro { ":ro" } else { "" };
        match kind {
            "volume" => {
                let vol = m.get("Name").and_then(Value::as_str).unwrap_or("").to_string();
                if vol.is_empty() || src.is_empty() {
                    continue;
                }
                a.push("-v".into());
                a.push(format!("{vol}:{dest}{suffix}"));
                copies.push(MountCopy {
                    source: src,
                    volume: Some(vol),
                    dest,
                });
            }
            "bind" => {
                a.push("-v".into());
                a.push(format!("{src}:{dest}{suffix}"));
                if system_path(&src) {
                    notes.push(format!("{src} is a host path; the target's own {src} is used."));
                } else {
                    copies.push(MountCopy {
                        source: src,
                        volume: None,
                        dest,
                    });
                }
            }
            "tmpfs" => {
                a.push("--tmpfs".into());
                a.push(dest);
            }
            _ => {}
        }
    }

    // Network: host, none, or a user-defined one (compose's "project_default").
    let mode = s(hc, &["NetworkMode"]).as_str().unwrap_or("default");
    let mut network = None;
    match mode {
        "default" | "bridge" | "" => {}
        "host" | "none" => {
            a.push("--network".into());
            a.push(mode.into());
        }
        m if m.starts_with("container:") => {
            return Err(Error::InvalidInput(format!(
                "this container shares another container's network ({m}); move that one instead"
            )));
        }
        m => {
            a.push("--network".into());
            a.push(m.into());
            network = Some(m.to_string());
            notes.push(format!(
                "It joins network {m} on the target (created if missing); other containers on that network stay here."
            ));
        }
    }

    // Labels, except those Compose and image builders set.
    if let Some(labels) = s(cfg, &["Labels"]).as_object() {
        let mut ks: Vec<&String> = labels.keys().collect();
        ks.sort();
        for k in ks {
            if k.starts_with("com.docker.compose.") || k.starts_with("org.opencontainers.") || k.starts_with("maintainer") {
                continue;
            }
            if let Some(v) = labels[k].as_str() {
                a.push("--label".into());
                a.push(format!("{k}={v}"));
            }
        }
    }
    if let Some(project) = s(cfg, &["Labels", "com.docker.compose.project"]).as_str() {
        notes.push(format!(
            "It's part of Compose project {project}; on the target it runs as a plain container (docker run), not under Compose."
        ));
    }

    let user = s(cfg, &["User"]).as_str().unwrap_or("");
    if !user.is_empty() {
        a.push("--user".into());
        a.push(user.into());
    }
    let wd = s(cfg, &["WorkingDir"]).as_str().unwrap_or("");
    if !wd.is_empty() {
        a.push("--workdir".into());
        a.push(wd.into());
    }
    if s(hc, &["Privileged"]).as_bool() == Some(true) {
        a.push("--privileged".into());
        notes.push("It runs privileged (full access to the host).".into());
    }
    if s(hc, &["Init"]).as_bool() == Some(true) {
        a.push("--init".into());
    }
    for c in strs(s(hc, &["CapAdd"])) {
        a.push("--cap-add".into());
        a.push(c);
    }
    for h in strs(s(hc, &["ExtraHosts"])) {
        a.push("--add-host".into());
        a.push(h);
    }
    for d in s(hc, &["Devices"]).as_array().into_iter().flatten() {
        if let Some(p) = d.get("PathOnHost").and_then(Value::as_str) {
            a.push("--device".into());
            a.push(p.into());
            notes.push(format!("It uses device {p}; the target needs the same device."));
        }
    }
    if let Some(m) = s(hc, &["Memory"]).as_u64().filter(|m| *m > 0) {
        a.push("--memory".into());
        a.push(m.to_string());
    }
    if let Some(n) = s(hc, &["NanoCpus"]).as_u64().filter(|n| *n > 0) {
        a.push("--cpus".into());
        a.push(format!("{}", n as f64 / 1e9));
    }

    // Environment: passed through an env file (multi-line values can't be).
    let mut env = Vec::new();
    for e in strs(s(cfg, &["Env"])) {
        let (k, v) = e.split_once('=').unwrap_or((e.as_str(), ""));
        if v.contains('\n') {
            notes.push(format!(
                "Variable {k} has several lines and isn't copied; set it on the target yourself."
            ));
        } else {
            env.push((k.to_string(), v.to_string()));
        }
    }

    // Entry point and command, as the container has them.
    let ep = strs(s(cfg, &["Entrypoint"]));
    let mut args: Vec<String> = ep.iter().skip(1).cloned().collect();
    args.extend(strs(s(cfg, &["Cmd"])));

    let repo_digest = strs(s(inspect, &["ImageRepoDigests"])).into_iter().next();
    let mut p = MovePlan {
        name,
        image,
        image_id,
        repo_digest,
        running,
        run_args: Vec::new(),
        env_count: 0,
        host_ports: Vec::new(),
        copies,
        network,
        notes,
        ports,
        args,
        env_vars: Vec::new(),
        hints: Vec::new(),
        env,
        options: a,
        entrypoint: ep.first().cloned(),
    };
    p.rebuild();
    Ok(p)
}

/// The env file's content (sent on stdin, never logged).
pub fn env_file(p: &MovePlan) -> String {
    p.env.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

/// `docker run -d …` reading the env file from stdin into a private temp file.
pub fn run_command(p: &MovePlan) -> String {
    let args: Vec<String> = p.run_args.iter().map(|x| quote(x)).collect();
    format!(
        "f=$(mktemp) && chmod 600 \"$f\" && cat > \"$f\" && docker run -d --env-file \"$f\" {}; rc=$?; rm -f \"$f\"; exit $rc",
        args.join(" ")
    )
}

/// What the target must not already have: a container of that name, the
/// published ports, non-empty folders at the bind paths. Prints `@@name`,
/// `@@port N`, `@@path P` for each clash; `@@nodocker` if docker is missing.
pub fn preflight_script(p: &MovePlan) -> String {
    let mut s = String::from("command -v docker >/dev/null 2>&1 || { echo @@nodocker; exit 0; }\n");
    s.push_str(&format!(
        "docker ps -a --format '{{{{.Names}}}}' 2>/dev/null | grep -qxF {} && echo @@name\n",
        quote(&p.name)
    ));
    for port in &p.host_ports {
        s.push_str(&format!(
            "if command -v ss >/dev/null 2>&1; then ss -Hltnu 2>/dev/null | awk '{{print $5}}' | grep -qE '[:.]{port}$' && echo '@@port {port}'; fi\n"
        ));
    }
    for c in p.copies.iter().filter(|c| c.volume.is_none()) {
        let q = quote(&c.source);
        s.push_str(&format!(
            "if [ -d {q} ]; then [ -n \"$(ls -A {q} 2>/dev/null)\" ] && echo '@@path '{q}; elif [ -e {q} ]; then echo '@@path '{q}; fi\n"
        ));
    }
    s
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clashes {
    pub no_docker: bool,
    pub name: bool,
    pub ports: Vec<u16>,
    pub paths: Vec<String>,
}

pub fn parse_preflight(out: &str) -> Clashes {
    let mut c = Clashes::default();
    for l in out.lines().map(str::trim) {
        if l == "@@nodocker" {
            c.no_docker = true;
        } else if l == "@@name" {
            c.name = true;
        } else if let Some(p) = l.strip_prefix("@@port ") {
            if let Ok(p) = p.parse() {
                c.ports.push(p);
            }
        } else if let Some(p) = l.strip_prefix("@@path ") {
            c.paths.push(p.to_string());
        }
    }
    c
}

/// tar of one mount: a volume's contents, or a bind path by name from its parent.
pub fn tar_out(c: &MountCopy) -> String {
    match c.volume {
        Some(_) => format!("tar -C {} -cpf - .", quote(&c.source)),
        None => {
            let (parent, name) = split(&c.source);
            format!("tar -C {} -cpf - -- {}", quote(parent), quote(name))
        }
    }
}

/// Where a mount's tar is unpacked on the target. For a volume, after it's created.
pub fn tar_in(c: &MountCopy) -> String {
    match &c.volume {
        Some(v) => format!(
            "docker volume create {v} >/dev/null && d=$(docker volume inspect -f '{{{{.Mountpoint}}}}' {v}) && tar -C \"$d\" -xpf -",
            v = quote(v)
        ),
        None => {
            let (parent, _) = split(&c.source);
            format!("mkdir -p {p} && tar -C {p} -xpf -", p = quote(parent))
        }
    }
}

fn split(path: &str) -> (&str, &str) {
    let t = path.trim_end_matches('/');
    match t.rsplit_once('/') {
        Some(("", n)) => ("/", n),
        Some((p, n)) => (p, n),
        None => (".", t),
    }
}

/// What the move dialog shows before anything happens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePreview {
    pub plan: MovePlan,
    pub clashes: Clashes,
    /// The target's name for messages.
    pub target: String,
}

/// nginx sites on the source that send traffic to one of these host ports.
pub fn nginx_hints(cfg: &crate::nginx::NginxConfig, ports: &[u16], source: &str, target: &str) -> Vec<String> {
    let mut out = Vec::new();
    for site in &cfg.sites {
        for loc in &site.locations {
            let Some(pp) = &loc.proxy_pass else { continue };
            let hit = ports.iter().find(|p| {
                ["127.0.0.1", "localhost", "0.0.0.0", "[::1]"]
                    .iter()
                    .any(|h| pp.contains(&format!("{h}:{p}")))
            });
            if let Some(port) = hit {
                let names = if site.server_names.is_empty() {
                    "a catch-all site".to_string()
                } else {
                    site.server_names.join(", ")
                };
                out.push(format!(
                    "nginx on {source} sends {names} to port {port} ({}). That site, its certificate and the DNS record stay with {source}: on {target} use Nginx → New website and point the DNS there, or reach it another way.",
                    site.file
                ));
                break;
            }
        }
    }
    out
}

/// Whether `docker` needs sudo on this server: the login user can't talk to
/// the daemon (not in the docker group) but sudo is allowed. Decided by
/// whether a harmless `docker ps` works, not by the wording of the error,
/// which differs between Docker versions.
pub async fn docker_root(s: &SshSession) -> Result<bool> {
    if !s.elevation_needed() {
        return Ok(false);
    }
    Ok(!s.exec("docker ps -q >/dev/null 2>&1").await?.ok())
}

/// `docker inspect` plus the image's registry digests (an image property, not the container's).
pub async fn inspect_for_move(s: &SshSession, id: &str) -> Result<Value> {
    let mut v = s.inspect_container(id).await?;
    if let Some(img) = v.get("Image").and_then(Value::as_str).map(String::from) {
        let (_, o) = s
            .docker(&format!("image inspect --format '{{{{json .RepoDigests}}}}' -- {}", quote(&img)))
            .await?;
        if let Ok(d) = serde_json::from_str::<Value>(o.stdout.trim()) {
            v["ImageRepoDigests"] = d;
        }
    }
    Ok(v)
}

/// Start `cmd` (as root when asked; the sudo password goes first on stdin).
async fn open_channel(s: &SshSession, cmd: &str, root: bool) -> Result<russh::Channel<russh::client::Msg>> {
    let full = if root { s.privileged_command(cmd) } else { cmd.to_string() };
    let pre = if root && s.elevation_needed() {
        s.sudo_input()?
    } else {
        Default::default()
    };
    let ch = s.handle().channel_open_session().await?;
    ch.exec(true, full).await?;
    if !pre.is_empty() {
        ch.data(&pre[..]).await?;
    }
    Ok(ch)
}

/// Run `src_cmd` on `src` and feed its output to `dst_cmd` on `dst`, as root
/// where asked (the sudo password goes first on stdin). Reports bytes as a
/// transfer and can be stopped. Returns the bytes moved.
pub async fn pipe(
    src: &SshSession,
    src_cmd: &str,
    src_root: bool,
    dst: &SshSession,
    dst_cmd: &str,
    dst_root: bool,
    label: &str,
) -> Result<u64> {
    let mut a = open_channel(src, src_cmd, src_root).await?;
    a.eof().await?;
    let mut b = open_channel(dst, dst_cmd, dst_root).await?;
    let (mut total, mut last) = (0u64, std::time::Instant::now());
    let (mut a_err, mut a_code) = (Vec::new(), None);
    loop {
        let msg = tokio::select! {
            m = a.wait() => m,
            _ = crate::progress::cancelled() => {
                let _ = a.signal(russh::Sig::TERM).await;
                let _ = a.close().await;
                let _ = b.close().await;
                return Err(Error::Cancelled);
            }
        };
        let Some(msg) = msg else { break };
        match msg {
            ChannelMsg::Data { data } => {
                b.data(&data[..]).await?;
                total += data.len() as u64;
                if last.elapsed() >= std::time::Duration::from_millis(250) {
                    last = std::time::Instant::now();
                    crate::progress::transfer(1, total, label);
                }
            }
            ChannelMsg::ExtendedData { data, .. } if a_err.len() < 16_384 => a_err.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => a_code = Some(exit_status),
            _ => {}
        }
    }
    crate::progress::transfer(1, total, label);
    b.eof().await?;
    let (mut b_out, mut b_code) = (Vec::new(), None);
    while let Some(msg) = b.wait().await {
        match msg {
            ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } if b_out.len() < 16_384 => b_out.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => b_code = Some(exit_status),
            _ => {}
        }
    }
    let last_line = |v: &[u8]| String::from_utf8_lossy(v).trim().lines().last().unwrap_or("").to_string();
    // tar's 1 means "a file changed while being read": the copy is still usable.
    if !matches!(a_code, Some(0) | Some(1)) {
        return Err(Error::Remote {
            code: a_code.map(|c| c as i32).unwrap_or(-1),
            stderr: format!("on the source: {}", last_line(&a_err)),
        });
    }
    if b_code != Some(0) {
        return Err(Error::Remote {
            code: b_code.map(|c| c as i32).unwrap_or(-1),
            stderr: format!("on the target: {}", last_line(&b_out)),
        });
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grafana() -> Value {
        serde_json::json!({
            "Name": "/grafana",
            "Image": "sha256:abc",
            "ImageRepoDigests": ["grafana/grafana@sha256:def"],
            "State": { "Running": true },
            "Config": {
                "Image": "grafana/grafana:11.3.0",
                "Env": ["GF_SECURITY_ADMIN_PASSWORD=s3cret", "PATH=/usr/bin"],
                "Cmd": null,
                "Entrypoint": ["/run.sh"],
                "User": "472",
                "WorkingDir": "/usr/share/grafana",
                "Labels": { "com.docker.compose.project": "observability", "org.opencontainers.image.source": "x", "team": "ops" }
            },
            "HostConfig": {
                "RestartPolicy": { "Name": "unless-stopped" },
                "PortBindings": { "3000/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "3000" }], "9000/udp": [{ "HostIp": "", "HostPort": "" }] },
                "NetworkMode": "observability_default",
                "CapAdd": ["NET_ADMIN"],
                "Memory": 536870912
            },
            "Mounts": [
                { "Type": "volume", "Name": "grafana-data", "Source": "/var/lib/docker/volumes/grafana-data/_data", "Destination": "/var/lib/grafana", "RW": true },
                { "Type": "bind", "Source": "/srv/observability/grafana.ini", "Destination": "/etc/grafana/grafana.ini", "RW": false },
                { "Type": "bind", "Source": "/etc/localtime", "Destination": "/etc/localtime", "RW": false }
            ]
        })
    }

    #[test]
    fn plans_a_container_with_everything_it_needs() {
        let p = plan(&grafana()).unwrap();
        assert_eq!(
            (p.name.as_str(), p.image.as_str(), p.repo_digest.as_deref()),
            ("grafana", "grafana/grafana:11.3.0", Some("grafana/grafana@sha256:def"))
        );
        let a = p.run_args.join(" ");
        for want in [
            "--name grafana",
            "--restart unless-stopped",
            "-p 127.0.0.1:3000:3000/tcp",
            "-p 9000/udp",
            "-v grafana-data:/var/lib/grafana",
            "-v /srv/observability/grafana.ini:/etc/grafana/grafana.ini:ro",
            "-v /etc/localtime:/etc/localtime:ro",
            "--network observability_default",
            "--label team=ops",
            "--user 472",
            "--cap-add NET_ADMIN",
            "--memory 536870912",
            "--entrypoint /run.sh grafana/grafana:11.3.0",
        ] {
            assert!(a.contains(want), "{want} missing from {a}");
        }
        assert!(!a.contains("compose") && !a.contains("opencontainers"), "{a}");
        // Secrets never on the command line: env goes through the file.
        assert!(!a.contains("s3cret") && !run_command(&p).contains("s3cret"));
        assert_eq!(env_file(&p), "GF_SECURITY_ADMIN_PASSWORD=s3cret\nPATH=/usr/bin\n");
        assert_eq!(p.host_ports, [3000]);
        assert_eq!(p.copies.len(), 2, "localtime is the host's, not copied");
        assert_eq!(p.network.as_deref(), Some("observability_default"));
        assert!(p.notes.iter().any(|n| n.contains("Compose project observability")));
    }

    #[test]
    fn ports_args_and_env_can_be_adjusted() {
        let mut v = grafana();
        v["Config"]["Cmd"] = serde_json::json!(["--trusted-origins", "portainer.sayyah.uz"]);
        v["Config"]["Env"] = serde_json::json!([
            "GF_SECURITY_ADMIN_PASSWORD=s3cret",
            "GF_SERVER_ROOT_URL=https://grafana.example.com/",
            "PATH=/usr/bin"
        ]);
        let mut p = plan(&v).unwrap();
        // What the dialog shows: ports, arguments, variables (secrets hidden), and domain hints.
        assert_eq!(
            p.ports[0],
            PortMap {
                host_ip: "127.0.0.1".into(),
                host_port: Some(3000),
                container: "3000/tcp".into()
            }
        );
        assert_eq!(p.args, ["--trusted-origins", "portainer.sayyah.uz"]);
        let pw = p.env_vars.iter().find(|e| e.key == "GF_SECURITY_ADMIN_PASSWORD").unwrap();
        assert!(pw.secret && pw.value.is_none());
        assert_eq!(
            p.env_vars.iter().find(|e| e.key == "PATH").unwrap().value.as_deref(),
            Some("/usr/bin")
        );
        assert_eq!(p.hints.len(), 2, "{:?}", p.hints);
        assert!(p.hints[0].contains("portainer.sayyah.uz") && p.hints[1].contains("grafana.example.com"));

        // Public on 9000, no domain argument, root URL changed, one variable dropped.
        p.apply(&MoveOptions {
            ports: Some(vec![PortMap {
                host_ip: "".into(),
                host_port: Some(9000),
                container: "3000/tcp".into(),
            }]),
            args: Some(vec![]),
            env_set: [("GF_SERVER_ROOT_URL".to_string(), "http://203.0.113.5:9000/".to_string())]
                .into_iter()
                .collect(),
            env_remove: vec!["PATH".into()],
        })
        .unwrap();
        let a = p.run_args.join(" ");
        assert!(
            a.contains("-p 9000:3000/tcp") && !a.contains("127.0.0.1") && a.ends_with("grafana/grafana:11.3.0"),
            "{a}"
        );
        assert_eq!(p.host_ports, [9000]);
        assert_eq!(
            env_file(&p),
            "GF_SECURITY_ADMIN_PASSWORD=s3cret\nGF_SERVER_ROOT_URL=http://203.0.113.5:9000/\n"
        );
        assert!(p.hints.is_empty(), "{:?}", p.hints);
        // The secret is kept unless it's set again, and never ends up on the command line.
        assert!(!run_command(&p).contains("s3cret"));

        // Bad edits are refused.
        for bad in [
            MoveOptions {
                ports: Some(vec![PortMap {
                    host_ip: "everyone".into(),
                    host_port: Some(1),
                    container: "80/tcp".into(),
                }]),
                ..Default::default()
            },
            MoveOptions {
                ports: Some(vec![PortMap {
                    host_ip: "".into(),
                    host_port: Some(1),
                    container: "80; rm".into(),
                }]),
                ..Default::default()
            },
            MoveOptions {
                ports: Some(vec![PortMap {
                    host_ip: "".into(),
                    host_port: Some(0),
                    container: "80".into(),
                }]),
                ..Default::default()
            },
            MoveOptions {
                args: Some(vec!["a\nb".into()]),
                ..Default::default()
            },
            MoveOptions {
                env_set: [("BAD-NAME".to_string(), "x".to_string())].into_iter().collect(),
                ..Default::default()
            },
            MoveOptions {
                env_set: [("OK".to_string(), "two\nlines".to_string())].into_iter().collect(),
                ..Default::default()
            },
        ] {
            assert!(plan(&grafana()).unwrap().apply(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn nginx_sites_pointing_at_the_container_are_named() {
        let dump = "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\n# configuration file /etc/nginx/sites-enabled/portainer:\nserver {\n    listen 443 ssl;\n    server_name portainer.sayyah.uz;\n    location / {\n        proxy_pass http://127.0.0.1:9000;\n    }\n}\nserver {\n    server_name other.example.com;\n    location / { proxy_pass http://127.0.0.1:3000; }\n}\n";
        let cfg = crate::nginx::parse_dump(dump, true);
        let h = nginx_hints(&cfg, &[9000], "old", "new");
        assert_eq!(h.len(), 1, "{h:?}");
        assert!(
            h[0].contains("portainer.sayyah.uz") && h[0].contains("port 9000") && h[0].contains("on new use Nginx"),
            "{h:?}"
        );
        assert!(nginx_hints(&cfg, &[8080], "old", "new").is_empty());
    }

    #[test]
    fn domains_but_not_file_names() {
        assert_eq!(domains_in("--trusted-origins portainer.sayyah.uz"), ["portainer.sayyah.uz"]);
        assert_eq!(domains_in("https://grafana.example.com/"), ["grafana.example.com"]);
        assert!(domains_in("/etc/grafana/grafana.ini").is_empty());
        assert!(domains_in("config.yml settings.json localhost").is_empty());
        assert!(domains_in("/usr/bin:/bin").is_empty());
    }

    #[test]
    fn refuses_what_cant_move() {
        let mut v = grafana();
        v["HostConfig"]["NetworkMode"] = "container:abc".into();
        assert!(plan(&v).is_err());
        let mut v = grafana();
        v["Name"] = "/bad name".into();
        assert!(plan(&v).is_err());
        assert!(plan(&serde_json::json!({})).is_err());
    }

    #[test]
    fn commands_are_quoted_and_parsed() {
        let p = plan(&grafana()).unwrap();
        let run = run_command(&p);
        assert!(run.starts_with("f=$(mktemp) && chmod 600"), "{run}");
        assert!(run.contains("docker run -d --env-file \"$f\" --name grafana"), "{run}");
        let pre = preflight_script(&p);
        assert!(
            pre.contains("grep -qxF grafana") && pre.contains("[:.]3000$") && pre.contains("/srv/observability/grafana.ini"),
            "{pre}"
        );
        assert_eq!(
            parse_preflight("@@name\n@@port 3000\n@@path /srv/x\n"),
            Clashes {
                no_docker: false,
                name: true,
                ports: vec![3000],
                paths: vec!["/srv/x".into()]
            }
        );
        assert!(parse_preflight("@@nodocker\n").no_docker);
        let vol = &p.copies[0];
        assert_eq!(tar_out(vol), "tar -C /var/lib/docker/volumes/grafana-data/_data -cpf - .");
        assert!(tar_in(vol)
            .starts_with("docker volume create grafana-data >/dev/null && d=$(docker volume inspect -f '{{.Mountpoint}}' grafana-data)"));
        let bind = &p.copies[1];
        assert_eq!(tar_out(bind), "tar -C /srv/observability -cpf - -- grafana.ini");
        assert_eq!(tar_in(bind), "mkdir -p /srv/observability && tar -C /srv/observability -xpf -");
        assert_eq!(split("/srv"), ("/", "srv"));
    }
}
