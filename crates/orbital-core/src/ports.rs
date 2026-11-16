//! Which ports are open on the server, which program and service listens on
//! each, whether it is reachable from the network, and what the firewall
//! does with it.
//!
//! Read-only. Uses `ss` (iproute2), falling back to `lsof` where `ss` is
//! missing (macOS, minimal images). Without root, other users' processes are
//! listed without their names.

use crate::error::{Error, Result};
use crate::firewall::{self, UfwRule};
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListeningPort {
    /// `tcp` or `udp`.
    pub protocol: String,
    /// The address it is bound to, as printed: `0.0.0.0`, `::`, `127.0.0.1`, `*`.
    pub address: String,
    pub port: u16,
    /// `all` (every interface), `local` (this server only) or `address` (one specific address).
    pub scope: String,
    pub process: Option<String>,
    pub pid: Option<u32>,
    /// The systemd service the process belongs to.
    pub unit: Option<String>,
    /// What ufw does with traffic from the internet to this port: `open`,
    /// `blocked`, `restricted` (only some addresses), `docker` (Docker
    /// publishes it past ufw), or `None` when there is no active firewall or
    /// the port is local-only.
    pub firewall: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortsReport {
    pub ports: Vec<ListeningPort>,
    /// `ss` or `lsof`.
    pub source: String,
    /// Read with root's view: every process name is known.
    pub complete: bool,
    /// ufw is installed and on.
    pub firewall_active: bool,
}

pub(crate) const SCRIPT: &str = r#"export LC_ALL=C
if command -v ss >/dev/null 2>&1; then
  echo @@ss
  out=$(ss -H -tulpn 2>/dev/null || ss -tulpn 2>/dev/null)
  printf '%s\n' "$out"
  echo @@units
  for p in $(printf '%s\n' "$out" | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u); do
    u=$(grep -o '[^/]*\.service' "/proc/$p/cgroup" 2>/dev/null | tail -1)
    [ -n "$u" ] && echo "$p $u"
  done
elif command -v lsof >/dev/null 2>&1; then
  echo @@lsof
  lsof -nP +c 0 -iTCP -sTCP:LISTEN -iUDP -F pcPn 2>/dev/null
else
  echo @@none
fi
# Without root, ufw refuses ("You need to be root"); the sections stay empty.
if command -v ufw >/dev/null 2>&1; then
  echo @@verbose; ufw status verbose 2>/dev/null
  echo @@numbered; ufw status numbered 2>/dev/null
fi
true
"#;

static PROC_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\("([^"]+)",pid=(\d+)"#).unwrap());

/// Split `0.0.0.0:80`, `[::]:22`, `*:22`, `127.0.0.53%lo:53` into address and port.
fn split_addr(s: &str) -> Option<(String, u16)> {
    let (addr, port) = s.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let addr = addr.trim_start_matches('[').trim_end_matches(']');
    let addr = addr.split('%').next().unwrap_or(addr);
    Some((addr.to_string(), port))
}

fn scope(addr: &str) -> &'static str {
    match addr {
        "*" | "0.0.0.0" | "::" | "" => "all",
        a => match a.trim_start_matches("::ffff:").parse::<IpAddr>() {
            Ok(ip) if ip.is_loopback() => "local",
            _ => "address",
        },
    }
}

fn port(protocol: &str, address: String, port: u16, process: Option<String>, pid: Option<u32>) -> ListeningPort {
    ListeningPort {
        protocol: protocol.to_string(),
        scope: scope(&address).to_string(),
        address,
        port,
        process,
        pid,
        unit: None,
        firewall: None,
    }
}

pub fn parse_ss(out: &str) -> Vec<ListeningPort> {
    let mut v = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 || f[0] == "Netid" {
            continue;
        }
        let protocol = match f[0] {
            "tcp" => "tcp",
            "udp" => "udp",
            _ => continue,
        };
        // UDP sockets show as UNCONN; connected UDP and non-listening TCP are skipped.
        if !matches!(f[1], "LISTEN" | "UNCONN") {
            continue;
        }
        let Some((addr, p)) = split_addr(f[4]) else { continue };
        let rest = f[5..].join(" ");
        let mut names: Vec<String> = Vec::new();
        let mut pid = None;
        for c in PROC_RE.captures_iter(&rest) {
            if !names.contains(&c[1].to_string()) {
                names.push(c[1].to_string());
            }
            pid = pid.or_else(|| c[2].parse().ok());
        }
        let process = (!names.is_empty()).then(|| names.join(", "));
        v.push(port(protocol, addr, p, process, pid));
    }
    v
}

/// `lsof -F pcPn`: `p<pid>`, `c<command>`, then per socket `f<fd>`, `P<proto>`, `n<name>`.
pub fn parse_lsof(out: &str) -> Vec<ListeningPort> {
    let mut v = Vec::new();
    let (mut pid, mut cmd, mut proto): (Option<u32>, Option<String>, Option<String>) = (None, None, None);
    for line in out.lines() {
        let (tag, val) = match line.char_indices().nth(1) {
            Some((i, _)) => (&line[..i], &line[i..]),
            None => continue,
        };
        match tag {
            "p" => {
                pid = val.parse().ok();
                cmd = None;
            }
            "c" => cmd = Some(val.to_string()),
            "f" => proto = None,
            "P" => proto = Some(val.to_ascii_lowercase()),
            "n" => {
                if val.contains("->") {
                    continue;
                }
                let Some(pr) = proto.as_deref().filter(|p| *p == "tcp" || *p == "udp") else {
                    continue;
                };
                if let Some((addr, p)) = split_addr(val) {
                    let lp = port(pr, addr, p, cmd.clone(), pid);
                    if !v.contains(&lp) {
                        v.push(lp);
                    }
                }
            }
            _ => {}
        }
    }
    v
}

/// What ufw does with a connection from the internet to `p`.
fn firewall_verdict(p: &ListeningPort, rules: &[UfwRule], default_incoming: Option<&str>) -> Option<String> {
    if p.scope == "local" {
        return None;
    }
    if p.process.as_deref().is_some_and(|n| n.contains("docker-proxy")) {
        return Some("docker".into());
    }
    let udp = p.protocol == "udp";
    let v6 = p.address.contains(':');
    let relevant: Vec<&UfwRule> = rules
        .iter()
        .filter(|r| r.direction == "IN" && r.v6 == v6 && firewall::covers_port(&r.to, p.port, udp))
        .collect();
    for r in &relevant {
        if firewall::from_matches(&r.from, None) {
            return Some(
                if matches!(r.action.as_str(), "ALLOW" | "LIMIT") {
                    "open"
                } else {
                    "blocked"
                }
                .into(),
            );
        }
    }
    let some_allowed = relevant.iter().any(|r| matches!(r.action.as_str(), "ALLOW" | "LIMIT"));
    Some(
        match (default_incoming, some_allowed) {
            (Some("allow"), _) => "open",
            (_, true) => "restricted",
            _ => "blocked",
        }
        .into(),
    )
}

pub fn parse(out: &str, complete: bool) -> Result<PortsReport> {
    let mut sections: HashMap<&str, String> = HashMap::new();
    let mut cur = "";
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            cur = match name.trim() {
                "ss" => "ss",
                "units" => "units",
                "lsof" => "lsof",
                "none" => "none",
                "verbose" => "verbose",
                "numbered" => "numbered",
                _ => "",
            };
            sections.entry(cur).or_default();
            continue;
        }
        if let Some(s) = sections.get_mut(cur) {
            s.push_str(line);
            s.push('\n');
        }
    }
    let (mut ports, source) = if let Some(ss) = sections.get("ss") {
        (parse_ss(ss), "ss")
    } else if let Some(l) = sections.get("lsof") {
        (parse_lsof(l), "lsof")
    } else {
        return Err(Error::NotFound(
            "neither ss nor lsof is installed, so open ports can't be listed".into(),
        ));
    };
    let units: HashMap<u32, String> = sections
        .get("units")
        .map(|u| {
            u.lines()
                .filter_map(|l| l.split_once(' '))
                .filter_map(|(p, u)| Some((p.parse().ok()?, u.trim().to_string())))
                .collect()
        })
        .unwrap_or_default();
    let ufw = match (sections.get("verbose"), sections.get("numbered")) {
        (Some(v), Some(n)) => firewall::parse_status(&format!("@@verbose\n{v}@@numbered\n{n}")),
        _ => None,
    }
    .filter(|st| st.active);
    for p in &mut ports {
        p.unit = p.pid.and_then(|pid| units.get(&pid).cloned());
        if let Some(st) = &ufw {
            p.firewall = firewall_verdict(p, &st.rules, st.default_incoming.as_deref());
        }
    }
    ports.sort_by(|a, b| (a.port, &a.protocol, &a.address).cmp(&(b.port, &b.protocol, &b.address)));
    Ok(PortsReport {
        ports,
        source: source.into(),
        complete,
        firewall_active: ufw.is_some(),
    })
}

impl SshSession {
    pub async fn listening_ports(&self) -> Result<PortsReport> {
        let (o, root) = self.exec_inspect(SCRIPT).await?;
        parse(&o.stdout, root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: &str = "@@ss
tcp   LISTEN 0      511          0.0.0.0:80        0.0.0.0:*    users:((\"nginx\",pid=812,fd=6),(\"nginx\",pid=811,fd=6))
tcp   LISTEN 0      4096         0.0.0.0:22        0.0.0.0:*    users:((\"sshd\",pid=900,fd=3))
tcp   LISTEN 0      4096            [::]:22           [::]:*    users:((\"sshd\",pid=900,fd=4))
tcp   LISTEN 0      244        127.0.0.1:5432      0.0.0.0:*    users:((\"postgres\",pid=1200,fd=5))
udp   UNCONN 0      0      127.0.0.53%lo:53        0.0.0.0:*    users:((\"systemd-resolve\",pid=500,fd=13))
tcp   LISTEN 0      4096         0.0.0.0:8080      0.0.0.0:*    users:((\"docker-proxy\",pid=3001,fd=4))
tcp   LISTEN 0      128        10.0.0.5:9100      0.0.0.0:*
tcp   LISTEN 0      511          0.0.0.0:3000      0.0.0.0:*    users:((\"node\",pid=4000,fd=20))
tcp   ESTAB  0      0          10.0.0.5:22    203.0.113.9:51515 users:((\"sshd\",pid=901,fd=4))
@@units
812 nginx.service
900 ssh.service
1200 postgresql@16-main.service
@@verbose
Status: active
Default: deny (incoming), allow (outgoing), disabled (routed)
@@numbered
[ 1] 22/tcp                     ALLOW IN    Anywhere
[ 2] 80/tcp                     ALLOW IN    Anywhere
[ 3] 3000                       ALLOW IN    10.0.0.0/8
[ 4] 22/tcp (v6)                ALLOW IN    Anywhere (v6)
";

    #[test]
    fn parses_ss_with_units_and_firewall() {
        let r = parse(SS, true).unwrap();
        assert_eq!(r.source, "ss");
        assert!(r.complete && r.firewall_active);
        let find = |port: u16, v6: bool| r.ports.iter().find(|p| p.port == port && p.address.contains(':') == v6).unwrap();
        assert_eq!(r.ports.len(), 8, "established connection skipped");

        let web = find(80, false);
        assert_eq!(
            (web.process.as_deref(), web.pid, web.unit.as_deref()),
            (Some("nginx"), Some(812), Some("nginx.service"))
        );
        assert_eq!((web.scope.as_str(), web.firewall.as_deref()), ("all", Some("open")));
        assert_eq!(find(22, true).firewall.as_deref(), Some("open"));
        let pg = find(5432, false);
        assert_eq!((pg.scope.as_str(), pg.firewall.as_deref()), ("local", None));
        assert_eq!(pg.unit.as_deref(), Some("postgresql@16-main.service"));
        let dns = find(53, false);
        assert_eq!(
            (dns.protocol.as_str(), dns.address.as_str(), dns.scope.as_str()),
            ("udp", "127.0.0.53", "local")
        );
        assert_eq!(find(8080, false).firewall.as_deref(), Some("docker"));
        let exporter = find(9100, false);
        assert_eq!((exporter.scope.as_str(), exporter.process.as_deref()), ("address", None));
        assert_eq!(exporter.firewall.as_deref(), Some("blocked"));
        assert_eq!(find(3000, false).firewall.as_deref(), Some("restricted"));
        // Sorted by port.
        assert!(r.ports.windows(2).all(|w| w[0].port <= w[1].port));
    }

    #[test]
    fn parses_lsof_without_firewall() {
        let out = "@@lsof\np812\ncnginx\nf6\nPTCP\nn*:80\nf7\nPTCP\nn[::]:80\np95\ncmDNSResponder\nf9\nPUDP\nn*:5353\nf10\nPUDP\nn192.168.1.4:5353->224.0.0.251:5353\np2001\ncsshd\nf3\nPTCP\nn127.0.0.1:2222\n";
        let r = parse(out, false).unwrap();
        assert_eq!(r.source, "lsof");
        assert!(!r.complete && !r.firewall_active);
        assert_eq!(r.ports.len(), 4);
        let ssh = r.ports.iter().find(|p| p.port == 2222).unwrap();
        assert_eq!(
            (ssh.process.as_deref(), ssh.pid, ssh.scope.as_str()),
            (Some("sshd"), Some(2001), "local")
        );
        assert!(r.ports.iter().all(|p| p.firewall.is_none()));
        assert!(r.ports.iter().any(|p| p.port == 80 && p.address == "::"));
    }

    #[test]
    fn missing_tools() {
        assert!(parse("@@none\n", false).is_err());
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(out.status.success());
        let _ = parse(&String::from_utf8_lossy(&out.stdout), false);
    }
}
