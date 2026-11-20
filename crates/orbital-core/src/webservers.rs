//! Apache and HAProxy, read the way nginx is: what the server actually loads.
//!
//! Apache: virtual hosts from `apachectl -S`, loaded modules from `-M`, the
//! configtest result, and on Debian/Ubuntu the sites in `sites-available`
//! with whether each is enabled (`a2ensite`/`a2dissite`).
//!
//! HAProxy: frontends, backends and their servers from haproxy.cfg, the
//! `haproxy -c` result, and, when the config has a stats socket, live server
//! status from `show stat`, so a server can be drained or put in maintenance.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------- Apache ----------

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApacheVhost {
    pub server_name: String,
    pub aliases: Vec<String>,
    pub port: u16,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApacheSite {
    /// Name as a2ensite takes it (file name without `.conf`).
    pub name: String,
    pub file: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApacheStatus {
    pub test_ok: bool,
    pub test_output: String,
    pub vhosts: Vec<ApacheVhost>,
    pub modules: Vec<String>,
    pub server_root: Option<String>,
    pub document_root: Option<String>,
    /// Debian-style site management is available.
    pub manages_sites: bool,
    pub sites: Vec<ApacheSite>,
}

pub(crate) const APACHE_SCRIPT: &str = r#"export LC_ALL=C
A=$(command -v apache2ctl || command -v apachectl || command -v httpd) || { echo @@missing; exit 0; }
echo @@test; out=$("$A" -t 2>&1); code=$?; printf '%s\n' "$out"; echo "@@test_exit $code"
echo @@vhosts; "$A" -S 2>&1
echo @@modules; "$A" -M 2>/dev/null
if [ -d /etc/apache2/sites-available ]; then
  echo @@available; ls -1 /etc/apache2/sites-available 2>/dev/null
  echo @@enabled; ls -1 /etc/apache2/sites-enabled 2>/dev/null
fi
"#;

static NAMEVHOST_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^port (\d+) namevhost (\S+) \((.+):(\d+)\)$").unwrap());
static SINGLE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\S+:(\d+)\s+(\S+) \((.+):(\d+)\)$").unwrap());

pub fn parse_apache(out: &str) -> Option<ApacheStatus> {
    let mut st = ApacheStatus::default();
    let mut section = "";
    let mut test = Vec::new();
    let (mut available, mut enabled) = (Vec::<String>::new(), Vec::<String>::new());
    for line in out.lines() {
        let l = line.trim();
        if l == "@@missing" {
            return None;
        }
        if let Some(code) = l.strip_prefix("@@test_exit") {
            st.test_ok = code.trim() == "0";
            section = "";
            continue;
        }
        if let Some(s) = l.strip_prefix("@@") {
            section = match s {
                "test" | "vhosts" | "modules" | "available" | "enabled" => s,
                _ => "",
            };
            if s == "available" {
                st.manages_sites = true;
            }
            continue;
        }
        match section {
            "test" if !l.is_empty() => test.push(l.to_string()),
            "vhosts" => {
                if let Some(c) = NAMEVHOST_RE.captures(l).or_else(|| SINGLE_RE.captures(l)) {
                    st.vhosts.push(ApacheVhost {
                        port: c[1].parse().unwrap_or(0),
                        server_name: c[2].to_string(),
                        aliases: Vec::new(),
                        file: c[3].to_string(),
                        line: c[4].parse().unwrap_or(0),
                    });
                } else if let Some(a) = l.strip_prefix("alias ").or_else(|| l.strip_prefix("wild alias ")) {
                    if let Some(v) = st.vhosts.last_mut() {
                        v.aliases.push(a.trim().to_string());
                    }
                } else if let Some(v) = l.strip_prefix("ServerRoot:") {
                    st.server_root = Some(v.trim().trim_matches('"').to_string());
                } else if let Some(v) = l.strip_prefix("Main DocumentRoot:") {
                    st.document_root = Some(v.trim().trim_matches('"').to_string());
                }
            }
            "modules" => {
                // " ssl_module (shared)"
                if let Some(m) = l.split_whitespace().next().filter(|m| m.ends_with("_module")) {
                    st.modules.push(m.trim_end_matches("_module").to_string());
                }
            }
            "available" if !l.is_empty() => available.push(l.to_string()),
            "enabled" if !l.is_empty() => enabled.push(l.to_string()),
            _ => {}
        }
    }
    st.test_output = test.join("\n");
    st.vhosts
        .dedup_by(|a, b| a.server_name == b.server_name && a.port == b.port && a.file == b.file);
    st.sites = available
        .iter()
        .filter(|f| f.ends_with(".conf"))
        .map(|f| ApacheSite {
            name: f.trim_end_matches(".conf").to_string(),
            file: format!("/etc/apache2/sites-available/{f}"),
            enabled: enabled.contains(f),
        })
        .collect();
    st.modules.sort();
    Some(st)
}

static SITE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._\-]{0,127}$").unwrap());

/// Enable or disable a Debian-style site, keeping it only if `configtest` still
/// passes. Prints `@@rolledback` and the test output when it had to undo.
pub fn apache_site_script(name: &str, enable: bool) -> Result<String> {
    if !SITE_RE.is_match(name) {
        return Err(Error::InvalidInput(format!("invalid site name: {name:?}")));
    }
    let (doit, undo) = if enable {
        ("a2ensite", "a2dissite")
    } else {
        ("a2dissite", "a2ensite")
    };
    let n = quote(name);
    Ok(format!(
        "export LC_ALL=C; {doit} -q {n} || exit 2; \
         if ! out=$(apache2ctl configtest 2>&1); then {undo} -q {n} >/dev/null 2>&1; echo @@rolledback; printf '%s\\n' \"$out\"; exit 3; fi; \
         echo ok"
    ))
}

// ---------- HAProxy ----------

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HaServer {
    pub name: String,
    pub address: String,
    pub options: String,
    /// From the stats socket: `UP`, `DOWN`, `MAINT`, `DRAIN`, `no check`, …
    pub status: Option<String>,
    pub sessions: Option<u64>,
    pub check: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HaSection {
    /// `frontend`, `backend` or `listen`.
    pub kind: String,
    pub name: String,
    pub line: usize,
    pub binds: Vec<String>,
    pub mode: Option<String>,
    pub balance: Option<String>,
    pub default_backend: Option<String>,
    /// `use_backend` targets, with their conditions.
    pub use_backends: Vec<String>,
    pub servers: Vec<HaServer>,
    /// Live status of the frontend/backend itself.
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HaproxyStatus {
    pub config_file: String,
    pub test_ok: bool,
    pub test_output: String,
    pub sections: Vec<HaSection>,
    /// The stats socket, when the config defines one that exists.
    pub socket: Option<String>,
}

pub(crate) const HAPROXY_SCRIPT: &str = r#"export LC_ALL=C
command -v haproxy >/dev/null 2>&1 || { echo @@missing; exit 0; }
CFG=/etc/haproxy/haproxy.cfg
echo "@@file $CFG"
echo @@test; out=$(haproxy -c -f "$CFG" 2>&1); code=$?; printf '%s\n' "$out"; echo "@@test_exit $code"
echo @@config; cat "$CFG" 2>&1
S=$(awk '$1 == "stats" && $2 == "socket" {print $3; exit}' "$CFG" 2>/dev/null)
if [ -n "$S" ] && [ -S "$S" ]; then
  echo "@@socket $S"; echo @@stat
  echo "show stat" | { socat stdio "$S" 2>/dev/null || nc -U "$S" 2>/dev/null; }
fi
"#;

pub fn parse_haproxy(out: &str) -> Option<HaproxyStatus> {
    let mut st = HaproxyStatus::default();
    let mut section = "";
    let mut test = Vec::new();
    let mut cfg_line = 0usize;
    let mut header: Vec<String> = Vec::new();
    let mut live: HashMap<(String, String), HashMap<String, String>> = HashMap::new();
    for line in out.lines() {
        if line.trim() == "@@missing" {
            return None;
        }
        if let Some(code) = line.strip_prefix("@@test_exit") {
            st.test_ok = code.trim() == "0";
            section = "";
            continue;
        }
        if let Some(f) = line.strip_prefix("@@file ") {
            st.config_file = f.trim().to_string();
            continue;
        }
        if let Some(sock) = line.strip_prefix("@@socket ") {
            st.socket = Some(sock.trim().to_string());
            continue;
        }
        if let Some(s) = line.strip_prefix("@@") {
            section = match s {
                "test" | "config" | "stat" => s,
                _ => "",
            };
            continue;
        }
        match section {
            "test" if !line.trim().is_empty() => test.push(line.trim().to_string()),
            "config" => {
                cfg_line += 1;
                let l = line.split('#').next().unwrap_or("").trim();
                if l.is_empty() {
                    continue;
                }
                let words: Vec<&str> = l.split_whitespace().collect();
                let starts_section = !line.starts_with([' ', '\t']);
                if starts_section {
                    if matches!(words[0], "frontend" | "backend" | "listen") {
                        st.sections.push(HaSection {
                            kind: words[0].to_string(),
                            name: words.get(1).unwrap_or(&"").to_string(),
                            line: cfg_line,
                            binds: words.get(2).map(|b| vec![b.to_string()]).unwrap_or_default(),
                            ..Default::default()
                        });
                    } else if matches!(words[0], "global" | "defaults" | "resolvers" | "peers" | "userlist" | "program") {
                        // Not a proxy: stop attaching directives to the previous section.
                        st.sections.push(HaSection {
                            kind: String::new(),
                            ..Default::default()
                        });
                    }
                    continue;
                }
                let Some(sec) = st.sections.last_mut().filter(|s| !s.kind.is_empty()) else {
                    continue;
                };
                let rest = words[1..].join(" ");
                match words[0] {
                    "bind" => sec.binds.push(words.get(1).unwrap_or(&"").to_string()),
                    "mode" => sec.mode = Some(rest),
                    "balance" => sec.balance = Some(rest),
                    "default_backend" => sec.default_backend = Some(rest),
                    "use_backend" => sec.use_backends.push(rest),
                    "server" if words.len() >= 3 => sec.servers.push(HaServer {
                        name: words[1].to_string(),
                        address: words[2].to_string(),
                        options: words[3..].join(" "),
                        ..Default::default()
                    }),
                    _ => {}
                }
            }
            "stat" => {
                let l = line.trim();
                if let Some(h) = l.strip_prefix("# ") {
                    header = h.split(',').map(String::from).collect();
                } else if !l.is_empty() && !header.is_empty() {
                    let f: Vec<&str> = l.split(',').collect();
                    let row: HashMap<String, String> = header.iter().cloned().zip(f.iter().map(|x| x.to_string())).collect();
                    if let (Some(px), Some(sv)) = (row.get("pxname").cloned(), row.get("svname").cloned()) {
                        live.insert((px, sv), row);
                    }
                }
            }
            _ => {}
        }
    }
    st.sections.retain(|s| !s.kind.is_empty());
    st.test_output = test.join("\n");
    for sec in &mut st.sections {
        let own = if sec.kind == "frontend" { "FRONTEND" } else { "BACKEND" };
        sec.status = live
            .get(&(sec.name.clone(), own.to_string()))
            .and_then(|r| r.get("status").cloned());
        for srv in &mut sec.servers {
            if let Some(r) = live.get(&(sec.name.clone(), srv.name.clone())) {
                srv.status = r.get("status").cloned().filter(|s| !s.is_empty());
                srv.sessions = r.get("scur").and_then(|v| v.parse().ok());
                srv.check = r.get("check_status").cloned().filter(|s| !s.is_empty());
            }
        }
    }
    Some(st)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HaServerState {
    /// Take traffic again.
    Ready,
    /// Finish current sessions, take no new ones.
    Drain,
    /// Out of rotation for maintenance.
    Maint,
}

static HA_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9_.:\-]{1,64}$").unwrap());
static SOCK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^/[A-Za-z0-9_./\-]{1,200}$").unwrap());

/// Change one server's state over the stats socket.
pub fn haproxy_state_script(socket: &str, backend: &str, server: &str, state: HaServerState) -> Result<String> {
    if !SOCK_RE.is_match(socket) || socket.contains("..") {
        return Err(Error::InvalidInput(format!("invalid stats socket path: {socket:?}")));
    }
    for n in [backend, server] {
        if !HA_NAME_RE.is_match(n) {
            return Err(Error::InvalidInput(format!("invalid HAProxy name: {n:?}")));
        }
    }
    let s = match state {
        HaServerState::Ready => "ready",
        HaServerState::Drain => "drain",
        HaServerState::Maint => "maint",
    };
    let line = format!("set server {backend}/{server} state {s}");
    Ok(format!(
        "echo {line} | {{ socat stdio {sock} 2>&1 || nc -U {sock} 2>&1; }}",
        line = quote(&line),
        sock = quote(socket)
    ))
}

impl SshSession {
    /// On the server, or inside `container` (the official httpd image).
    pub async fn apache_status(&self, container: Option<&str>) -> Result<ApacheStatus> {
        let o = self.inspect_in(container, APACHE_SCRIPT).await?;
        parse_apache(&o.stdout).ok_or_else(|| crate::databases::missing("Apache", container, &o))
    }

    pub async fn haproxy_status(&self) -> Result<HaproxyStatus> {
        let (o, _) = self.exec_inspect(HAPROXY_SCRIPT).await?;
        parse_haproxy(&o.stdout).ok_or_else(|| Error::NotFound("HAProxy is not installed on this server".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APACHE: &str = "@@test\nSyntax OK\n@@test_exit 0\n@@vhosts\nVirtualHost configuration:\n*:80                   is a NameVirtualHost\n         default server example.com (/etc/apache2/sites-enabled/000-default.conf:1)\n         port 80 namevhost example.com (/etc/apache2/sites-enabled/000-default.conf:1)\n                 alias www.example.com\n         port 80 namevhost blog.example.com (/etc/apache2/sites-enabled/blog.conf:1)\n*:443                  shop.example.com (/etc/apache2/sites-enabled/shop-ssl.conf:2)\nServerRoot: \"/etc/apache2\"\nMain DocumentRoot: \"/var/www/html\"\n@@modules\nLoaded Modules:\n core_module (static)\n ssl_module (shared)\n rewrite_module (shared)\n@@available\n000-default.conf\nblog.conf\nold.conf\nshop-ssl.conf\n@@enabled\n000-default.conf\nblog.conf\nshop-ssl.conf\n";

    #[test]
    fn parses_apache() {
        let st = parse_apache(APACHE).unwrap();
        assert!(st.test_ok && st.manages_sites);
        assert_eq!(st.vhosts.len(), 3);
        assert_eq!(st.vhosts[0].server_name, "example.com");
        assert_eq!(st.vhosts[0].aliases, ["www.example.com"]);
        assert_eq!((st.vhosts[2].port, st.vhosts[2].line), (443, 2));
        assert_eq!(st.modules, ["core", "rewrite", "ssl"]);
        assert_eq!(st.document_root.as_deref(), Some("/var/www/html"));
        let old = st.sites.iter().find(|s| s.name == "old").unwrap();
        assert!(!old.enabled && st.sites.iter().filter(|s| s.enabled).count() == 3);
        assert!(parse_apache("@@missing\n").is_none());
        let bad = parse_apache("@@test\nAH00526: Syntax error on line 3\n@@test_exit 1\n").unwrap();
        assert!(!bad.test_ok && bad.test_output.contains("line 3"));
    }

    #[test]
    fn apache_site_commands_roll_back() {
        let s = apache_site_script("blog", true).unwrap();
        assert!(s.contains("a2ensite -q blog") && s.contains("a2dissite -q blog >/dev/null"));
        assert!(apache_site_script("blog; reboot", true).is_err());
        assert!(apache_site_script("-x", false).is_err());
    }

    const HAPROXY: &str = "@@file /etc/haproxy/haproxy.cfg\n@@test\nConfiguration file is valid\n@@test_exit 0\n@@config\nglobal\n    stats socket /run/haproxy/admin.sock mode 660 level admin\n\ndefaults\n    mode http\n\nfrontend www # public\n    bind *:80\n    bind *:443 ssl crt /etc/ssl/site.pem\n    use_backend api if { path_beg /api }\n    default_backend web\n\nbackend web\n    balance roundrobin\n    server web1 10.0.0.11:8080 check\n    server web2 10.0.0.12:8080 check backup\n\nlisten stats\n    bind :8404\n    mode http\n@@socket /run/haproxy/admin.sock\n@@stat\n# pxname,svname,qcur,qmax,scur,smax,slim,stot,bin,bout,dreq,dresp,ereq,econ,eresp,wretr,wredis,status,weight,act,bck,chkfail,chkdown,lastchg,downtime,qlimit,pid,iid,sid,throttle,lbtot,tracked,type,rate,rate_lim,rate_max,check_status\nwww,FRONTEND,,,3,10,2000,100,0,0,0,0,0,,,,,OPEN,,,,,,,,,1,1,0,,,,0,0,0,5,\nweb,web1,0,0,2,5,,50,0,0,,0,,0,0,0,0,UP,1,1,0,0,0,100,0,,1,2,1,,50,,2,0,,3,L4OK\nweb,web2,0,0,0,0,,0,0,0,,0,,0,0,0,0,MAINT,1,0,1,0,0,100,0,,1,2,2,,0,,2,0,,0,\nweb,BACKEND,0,0,2,5,200,50,0,0,0,0,,0,0,0,0,UP,1,1,1,,0,100,0,,1,2,0,,50,,1,0,,3,\n";

    #[test]
    fn parses_haproxy_with_live_status() {
        let st = parse_haproxy(HAPROXY).unwrap();
        assert!(st.test_ok);
        assert_eq!(st.socket.as_deref(), Some("/run/haproxy/admin.sock"));
        let names: Vec<(&str, &str)> = st.sections.iter().map(|s| (s.kind.as_str(), s.name.as_str())).collect();
        assert_eq!(names, [("frontend", "www"), ("backend", "web"), ("listen", "stats")]);
        let www = &st.sections[0];
        assert_eq!(www.binds, ["*:80", "*:443"]);
        assert_eq!(www.default_backend.as_deref(), Some("web"));
        assert_eq!(www.use_backends, ["api if { path_beg /api }"]);
        assert_eq!(www.status.as_deref(), Some("OPEN"));
        let web = &st.sections[1];
        assert_eq!(web.balance.as_deref(), Some("roundrobin"));
        assert_eq!(web.servers[0].status.as_deref(), Some("UP"));
        assert_eq!(web.servers[0].sessions, Some(2));
        assert_eq!(web.servers[0].check.as_deref(), Some("L4OK"));
        assert_eq!(web.servers[1].status.as_deref(), Some("MAINT"));
        assert_eq!(web.servers[1].options, "check backup");
        assert_eq!(st.sections[2].binds, [":8404"]);
        assert!(parse_haproxy("@@missing\n").is_none());
    }

    #[test]
    fn haproxy_state_commands_validate() {
        let s = haproxy_state_script("/run/haproxy/admin.sock", "web", "web2", HaServerState::Drain).unwrap();
        assert_eq!(
            s,
            "echo 'set server web/web2 state drain' | { socat stdio /run/haproxy/admin.sock 2>&1 || nc -U /run/haproxy/admin.sock 2>&1; }"
        );
        assert!(haproxy_state_script("/run/x.sock", "web;reboot", "a", HaServerState::Ready).is_err());
        assert!(haproxy_state_script("relative.sock", "web", "a", HaServerState::Ready).is_err());
        assert!(haproxy_state_script("/run/../etc/x", "web", "a", HaServerState::Ready).is_err());
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn scripts_run_under_sh() {
        for s in [APACHE_SCRIPT, HAPROXY_SCRIPT] {
            let out = std::process::Command::new("sh").arg("-c").arg(s).output().unwrap();
            assert!(out.status.success());
        }
    }
}
