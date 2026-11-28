//! UFW, the firewall most Ubuntu and Debian servers use: read its rules, and
//! build the commands that add, change and delete them.
//!
//! Rules are never written from server text: add and insert commands are built
//! from a validated [`UfwRuleInput`], and deletes go by rule number after the
//! engine has checked the number still points at the rule the user saw.
//!
//! [`ssh_reachable`] answers "would this firewall still let me in?", so the
//! engine can refuse a change that would cut off the SSH connection in use.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UfwRule {
    /// Position in `ufw status numbered` (1-based). 0 for rules read from
    /// `ufw show added` while the firewall is off.
    pub number: u32,
    /// What the rule covers, as ufw prints it: `22/tcp`, `80,443/tcp`, `Nginx Full`, `Anywhere`.
    pub to: String,
    /// `ALLOW`, `DENY`, `REJECT` or `LIMIT`.
    pub action: String,
    /// `IN`, `OUT` or `FWD`.
    pub direction: String,
    /// Where traffic comes from: `Anywhere`, an address or a network.
    pub from: String,
    pub v6: bool,
    pub comment: Option<String>,
    /// The rule line without its number, whitespace collapsed. Used to check
    /// that a number still means the same rule before deleting it.
    pub raw: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UfwStatus {
    pub active: bool,
    /// Default policy for incoming traffic: `deny`, `allow` or `reject`.
    pub default_incoming: Option<String>,
    pub default_outgoing: Option<String>,
    pub logging: Option<String>,
    /// The live rules (only while the firewall is on).
    pub rules: Vec<UfwRule>,
    /// Rules that will apply once the firewall is turned on, as ufw commands.
    pub added: Vec<String>,
    /// Application profiles (`OpenSSH`, `Nginx Full`, …) usable in rules.
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UfwAction {
    Allow,
    Deny,
    Reject,
    /// Allow, but block an address that opens 6+ connections in 30 seconds.
    Limit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UfwDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UfwProtocol {
    Any,
    Tcp,
    Udp,
}

/// A rule as the user describes it in the form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UfwRuleInput {
    pub action: UfwAction,
    pub direction: UfwDirection,
    /// `22`, `80,443`, `6000:6007`. Either this or `app`.
    pub port: Option<String>,
    pub protocol: UfwProtocol,
    /// The other side: an address or network. `None` or `any` means anywhere.
    pub address: Option<String>,
    /// An application profile from `ufw app list`. Either this or `port`.
    pub app: Option<String>,
    pub comment: Option<String>,
}

/// A rule the user picked, as they saw it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UfwRuleRef {
    pub number: u32,
    pub raw: String,
}

// ---------- reading ----------

pub(crate) const STATUS_SCRIPT: &str = "export LC_ALL=C\n\
command -v ufw >/dev/null 2>&1 || { echo @@missing; exit 0; }\n\
echo @@verbose; ufw status verbose 2>&1\n\
echo @@numbered; ufw status numbered 2>&1\n\
echo @@added; ufw show added 2>&1\n\
echo @@apps; ufw app list 2>&1\n\
echo @@defaults; grep -E '^DEFAULT_(INPUT|OUTPUT)_POLICY=' /etc/default/ufw 2>/dev/null\n";

static NUMBERED_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\[\s*(\d+)\]\s+(.*)$").unwrap());
static RULE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(.*?)\s+(ALLOW|DENY|REJECT|LIMIT)(?:\s+(IN|OUT|FWD))?\s+(.*?)\s*$").unwrap());
static DEFAULT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\w+) \((incoming|outgoing|routed)\)").unwrap());

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse one rule line (without the `[ n]` prefix) from `ufw status`.
pub fn parse_rule_line(number: u32, line: &str) -> Option<UfwRule> {
    let (body, comment) = match line.split_once(" # ") {
        Some((b, c)) => (b, Some(c.trim().to_string())),
        None => (line, None),
    };
    let c = RULE_RE.captures(body.trim())?;
    let to = c[1].trim().to_string();
    let from = c[4].trim().to_string();
    if to.is_empty() || from.is_empty() || to == "To" {
        return None;
    }
    Some(UfwRule {
        number,
        v6: to.contains("(v6)") || from.contains("(v6)"),
        action: c[2].to_string(),
        direction: c.get(3).map(|m| m.as_str()).unwrap_or("IN").to_string(),
        to,
        from,
        comment,
        raw: collapse(line),
    })
}

/// Parse the output of [`STATUS_SCRIPT`]. `None` when ufw is not installed.
pub fn parse_status(out: &str) -> Option<UfwStatus> {
    let mut st = UfwStatus::default();
    let mut section = "";
    let mut missing = false;
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = match name.trim() {
                "missing" => {
                    missing = true;
                    ""
                }
                "verbose" => "verbose",
                "numbered" => "numbered",
                "added" => "added",
                "apps" => "apps",
                "defaults" => "defaults",
                _ => "",
            };
            continue;
        }
        match section {
            "verbose" => {
                if let Some(v) = line.strip_prefix("Status:") {
                    st.active = v.trim() == "active";
                } else if let Some(v) = line.strip_prefix("Logging:") {
                    st.logging = Some(v.trim().to_string());
                } else if let Some(v) = line.strip_prefix("Default:") {
                    for c in DEFAULT_RE.captures_iter(v) {
                        let policy = Some(c[1].to_string());
                        match &c[2] {
                            "incoming" => st.default_incoming = policy,
                            "outgoing" => st.default_outgoing = policy,
                            _ => {}
                        }
                    }
                }
            }
            "numbered" => {
                if let Some(c) = NUMBERED_RE.captures(line.trim()) {
                    if let Some(r) = c[1].parse().ok().and_then(|n| parse_rule_line(n, &c[2])) {
                        st.rules.push(r);
                    }
                }
            }
            "added" => {
                let l = line.trim();
                if let Some(cmd) = l.strip_prefix("ufw ") {
                    st.added.push(cmd.to_string());
                }
            }
            "apps" => {
                if line.starts_with("  ") && !line.trim().is_empty() {
                    st.apps.push(line.trim().to_string());
                }
            }
            "defaults" => {
                // Only needed while the firewall is off; `status verbose` wins otherwise.
                let policy = |v: &str| match v.trim_matches('"') {
                    "ACCEPT" => "allow",
                    "REJECT" => "reject",
                    _ => "deny",
                };
                if let Some(v) = line.strip_prefix("DEFAULT_INPUT_POLICY=") {
                    st.default_incoming.get_or_insert_with(|| policy(v).into());
                } else if let Some(v) = line.strip_prefix("DEFAULT_OUTPUT_POLICY=") {
                    st.default_outgoing.get_or_insert_with(|| policy(v).into());
                }
            }
            _ => {}
        }
    }
    (!missing).then_some(st)
}

// ---------- reachability ----------

/// Named ports ufw accepts in rules, and the app profiles that mean SSH.
fn named_port(name: &str) -> Option<u16> {
    match name.to_ascii_lowercase().as_str() {
        "ssh" | "openssh" => Some(22),
        "http" | "www" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

/// Does the `to` column of a rule cover `port` over TCP (or UDP when `udp`)?
pub fn covers_port(to: &str, port: u16, udp: bool) -> bool {
    let spec = to.replace("(v6)", "");
    let spec = spec.split(" on ").next().unwrap_or("").trim();
    if spec == "Anywhere" || spec.is_empty() {
        return true;
    }
    // "Anywhere 22" never appears; an address target looks like "10.0.0.1 22/tcp".
    let spec = spec.rsplit(' ').next().unwrap_or(spec);
    if let Some(p) = named_port(spec) {
        return p == port && !udp;
    }
    let (ports, proto) = match spec.split_once('/') {
        Some((p, proto)) => (p, Some(proto)),
        None => (spec, None),
    };
    if matches!((proto, udp), (Some("tcp"), true) | (Some("udp"), false)) {
        return false;
    }
    ports.split(',').any(|part| match part.split_once(':') {
        Some((a, b)) => matches!((a.parse::<u16>(), b.parse::<u16>()), (Ok(a), Ok(b)) if a <= port && port <= b),
        None => part.parse::<u16>().map(|p| p == port).unwrap_or(false),
    })
}

/// Is `client` inside the `from` column (`Anywhere`, an address or a CIDR)?
/// With no known client, only rules for anywhere count.
pub fn from_matches(from: &str, client: Option<IpAddr>) -> bool {
    let f = from.replace("(v6)", "");
    let f = f.split_whitespace().next().unwrap_or("");
    if f == "Anywhere" || f.is_empty() || f == "any" {
        return true;
    }
    let Some(client) = client else { return false };
    let (addr, prefix) = match f.split_once('/') {
        Some((a, p)) => (a, p.parse::<u32>().ok()),
        None => (f, None),
    };
    let Ok(net) = addr.parse::<IpAddr>() else { return false };
    match (net, client) {
        (IpAddr::V4(n), IpAddr::V4(c)) => {
            let bits = prefix.unwrap_or(32).min(32);
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            u32::from(n) & mask == u32::from(c) & mask
        }
        (IpAddr::V6(n), IpAddr::V6(c)) => {
            let bits = prefix.unwrap_or(128).min(128);
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            u128::from(n) & mask == u128::from(c) & mask
        }
        _ => false,
    }
}

/// What the firewall does with a new incoming TCP connection to `port` from
/// `client`: the first matching rule decides, else the default policy.
/// Returns `true` when it is let in.
pub fn admits(rules: &[UfwRule], default_incoming: Option<&str>, port: u16, client: Option<IpAddr>) -> bool {
    let v6 = matches!(client, Some(IpAddr::V6(_)));
    for r in rules {
        // Live rules are listed once per IP version; added rules cover both.
        if r.direction != "IN" || (client.is_some() && r.number > 0 && r.v6 != v6) {
            continue;
        }
        if covers_port(&r.to, port, false) && from_matches(&r.from, client) {
            return matches!(r.action.as_str(), "ALLOW" | "LIMIT");
        }
    }
    default_incoming == Some("allow")
}

/// Would the SSH connection in use still get through `rules` once the firewall is on?
pub fn ssh_reachable(rules: &[UfwRule], default_incoming: Option<&str>, port: u16, client: Option<IpAddr>) -> bool {
    admits(rules, default_incoming, port, client)
}

/// Turn a `ufw show added` command (`allow 22/tcp`, `limit from 10.0.0.0/8 to any port 22`)
/// into the shape `ufw status` prints, for the same reachability check.
pub fn rule_from_added(cmd: &str) -> Option<UfwRule> {
    let t: Vec<&str> = cmd.split_whitespace().collect();
    let action = t.first()?.to_ascii_uppercase();
    if !matches!(action.as_str(), "ALLOW" | "DENY" | "REJECT" | "LIMIT") {
        return None;
    }
    let mut i = 1;
    let mut direction = "IN".to_string();
    if let Some(d @ ("in" | "out")) = t.get(i).copied() {
        direction = d.to_ascii_uppercase();
        i += 1;
    }
    let (mut from, mut to, mut port, mut proto, mut app) = ("Anywhere".to_string(), None::<String>, None, None, None);
    if !t[i..].iter().any(|w| matches!(*w, "from" | "to" | "proto" | "on")) {
        // Simple form: `allow 22/tcp`, `allow OpenSSH`, `allow 'Nginx Full'`.
        let rest = t[i..].join(" ");
        let rest = rest.split(" comment ").next().unwrap_or("").trim_matches(['\'', '"']).to_string();
        to = Some(rest);
    } else {
        while i < t.len() {
            let next = t.get(i + 1).map(|s| s.to_string());
            match t[i] {
                "from" => from = next.filter(|v| v != "any").unwrap_or_else(|| "Anywhere".into()),
                "to" => to = next.filter(|v| v != "any"),
                "port" => port = next,
                "proto" => proto = next,
                "app" => {
                    app = Some(
                        t[i + 1..]
                            .join(" ")
                            .split(" comment ")
                            .next()
                            .unwrap_or("")
                            .trim_matches(['\'', '"'])
                            .to_string(),
                    )
                }
                "comment" => break,
                _ => {
                    i += 1;
                    continue;
                }
            }
            i += 2;
        }
        let target = match (app, port) {
            (Some(a), _) => a,
            (None, Some(p)) => match proto {
                Some(pr) => format!("{p}/{pr}"),
                None => p,
            },
            (None, None) => "Anywhere".into(),
        };
        to = Some(match to {
            Some(addr) => format!("{addr} {target}"),
            None => target,
        });
    }
    Some(UfwRule {
        number: 0,
        to: to.unwrap_or_else(|| "Anywhere".into()),
        action,
        direction,
        from,
        v6: false,
        comment: None,
        raw: collapse(cmd),
    })
}

impl UfwStatus {
    /// The rules that are, or will be, in force: live ones while active,
    /// otherwise the added ones in order.
    pub fn effective_rules(&self) -> Vec<UfwRule> {
        if self.active {
            self.rules.clone()
        } else {
            self.added.iter().filter_map(|c| rule_from_added(c)).collect()
        }
    }
}

// ---------- building commands ----------

static PORT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{1,5}(?::\d{1,5})?(?:,\d{1,5}(?::\d{1,5})?)*$").unwrap());
static APP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9 ._()+-]{0,63}$").unwrap());

fn validate_ports(spec: &str, protocol: UfwProtocol) -> Result<()> {
    if !PORT_RE.is_match(spec) {
        return Err(Error::InvalidInput(format!(
            "port must look like 22, 80,443 or 6000:6007 (got {spec:?})"
        )));
    }
    for part in spec.split(',') {
        let nums: Vec<u32> = part.split(':').map(|n| n.parse().unwrap_or(0)).collect();
        if nums.iter().any(|&n| n == 0 || n > 65535) {
            return Err(Error::InvalidInput(format!("port out of range in {spec:?}")));
        }
        if nums.len() == 2 && nums[0] >= nums[1] {
            return Err(Error::InvalidInput(format!("port range {part} must go from low to high")));
        }
    }
    if (spec.contains(',') || spec.contains(':')) && protocol == UfwProtocol::Any {
        return Err(Error::InvalidInput("a port list or range needs TCP or UDP chosen, not both".into()));
    }
    Ok(())
}

/// Validate an address or network (`203.0.113.7`, `10.0.0.0/8`, `2001:db8::/32`).
pub fn validate_address(a: &str) -> Result<()> {
    let (addr, prefix) = match a.split_once('/') {
        Some((x, p)) => (x, Some(p)),
        None => (a, None),
    };
    let ip: IpAddr = addr
        .parse()
        .map_err(|_| Error::InvalidInput(format!("{a:?} is not an IP address or network")))?;
    if let Some(p) = prefix {
        let max = if ip.is_ipv4() { 32 } else { 128 };
        match p.parse::<u32>() {
            Ok(n) if n <= max => {}
            _ => return Err(Error::InvalidInput(format!("network prefix in {a:?} must be 0–{max}"))),
        }
    }
    Ok(())
}

/// The arguments after `ufw` (or `ufw insert N`) for a rule. Every value is
/// validated and quoted; nothing comes from the server.
pub fn rule_args(r: &UfwRuleInput) -> Result<String> {
    let action = match r.action {
        UfwAction::Allow => "allow",
        UfwAction::Deny => "deny",
        UfwAction::Reject => "reject",
        UfwAction::Limit => "limit",
    };
    if r.action == UfwAction::Limit && r.direction == UfwDirection::Out {
        return Err(Error::InvalidInput("limit only works for incoming traffic".into()));
    }
    let dir = match r.direction {
        UfwDirection::In => "in",
        UfwDirection::Out => "out",
    };
    let port = r.port.as_deref().map(str::trim).filter(|p| !p.is_empty());
    let app = r.app.as_deref().map(str::trim).filter(|a| !a.is_empty());
    let addr = r
        .address
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case("any") && !a.eq_ignore_ascii_case("anywhere"));
    if let Some(a) = addr {
        validate_address(a)?;
    }
    let mut out = format!("{action} {dir}");
    let target = match (port, app) {
        (Some(_), Some(_)) => return Err(Error::InvalidInput("choose a port or an application, not both".into())),
        (None, None) => return Err(Error::InvalidInput("choose a port or an application".into())),
        (Some(p), None) => {
            validate_ports(p, r.protocol)?;
            format!("port {p}")
        }
        (None, Some(a)) => {
            if !APP_RE.is_match(a) {
                return Err(Error::InvalidInput(format!("invalid application name {a:?}")));
            }
            if r.protocol != UfwProtocol::Any {
                return Err(Error::InvalidInput("an application rule sets its own protocol".into()));
            }
            format!("app {}", quote(a))
        }
    };
    match r.protocol {
        UfwProtocol::Tcp => out.push_str(" proto tcp"),
        UfwProtocol::Udp => out.push_str(" proto udp"),
        UfwProtocol::Any => {}
    }
    match r.direction {
        // Incoming: the address is where traffic comes from; the port is ours.
        UfwDirection::In => out.push_str(&format!(" from {} to any {target}", addr.unwrap_or("any"))),
        // Outgoing: the address is where traffic goes.
        UfwDirection::Out => out.push_str(&format!(" to {} {target}", addr.unwrap_or("any"))),
    }
    if let Some(c) = r.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        if c.chars().count() > 120 || c.chars().any(|ch| ch.is_control()) {
            return Err(Error::InvalidInput("comment must be one line of at most 120 characters".into()));
        }
        out.push_str(&format!(" comment {}", quote(c)));
    }
    Ok(out)
}

/// The rules after `ufw <rule>` as ufw really applies it: a rule for the same
/// target (same `to`, `from` and direction, whatever its action) is *updated in
/// place* ("Rule updated"), IPv6 twin included; only a new target is appended.
/// So `deny 22/tcp` where `allow 22/tcp` exists replaces the allow, it doesn't
/// land after it. Pure; tested.
pub fn after_add(rules: &[UfwRule], new: &UfwRule) -> Vec<UfwRule> {
    let bare = |s: &str| s.trim_end_matches(" (v6)").trim().to_string();
    let same = |r: &UfwRule| bare(&r.to) == bare(&new.to) && bare(&r.from) == bare(&new.from) && r.direction == new.direction;
    let mut out: Vec<UfwRule> = rules.to_vec();
    let mut replaced = false;
    for r in out.iter_mut().filter(|r| same(r)) {
        r.action = new.action.clone();
        r.comment = new.comment.clone();
        replaced = true;
    }
    if !replaced {
        out.push(new.clone());
    }
    out
}

/// The rule a [`UfwRuleInput`] will become, in `ufw status` shape, for the
/// reachability check before it exists.
pub fn preview_rule(r: &UfwRuleInput) -> Result<UfwRule> {
    let args = rule_args(r)?;
    rule_from_added(&args.replace('\'', "")).ok_or_else(|| Error::InvalidInput("unsupported rule".into()))
}

impl SshSession {
    /// Read the firewall. Needs root; with no sudo it reports what ufw allows.
    pub async fn ufw_status(&self) -> Result<UfwStatus> {
        let (o, _) = self.exec_inspect(STATUS_SCRIPT).await?;
        if o.stdout.contains("need to be root") {
            return Err(Error::PermissionDenied(
                "reading the firewall needs root: turn on sudo in this server's settings (with the sudo password if it asks for one), then reconnect".into(),
            ));
        }
        parse_status(&o.stdout).ok_or_else(|| Error::NotFound("ufw is not installed on this server".into()))
    }

    /// Where this SSH connection comes from, and the port it reached, as the
    /// server sees them (they differ from the profile behind NAT, port
    /// forwarding or a jump host).
    pub async fn ssh_connection(&self) -> (Option<IpAddr>, Option<u16>) {
        let Ok(o) = self.exec("printf '%s' \"$SSH_CONNECTION\"").await else {
            return (None, None);
        };
        let f: Vec<&str> = o.stdout.split_whitespace().collect();
        (f.first().and_then(|s| s.parse().ok()), f.get(3).and_then(|s| s.parse().ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: &str = "@@verbose
Status: active
Logging: on (low)
Default: deny (incoming), allow (outgoing), disabled (routed)
New profiles: skip

To                         Action      From
--                         ------      ----
22/tcp                     ALLOW IN    Anywhere
@@numbered
Status: active

     To                         Action      From
     --                         ------      ----
[ 1] 22/tcp                     LIMIT IN    Anywhere
[ 2] 80,443/tcp                 ALLOW IN    Anywhere                   # web
[ 3] Nginx Full                 ALLOW IN    Anywhere
[ 4] 3306                       DENY IN     10.0.0.0/8
[ 5] Anywhere                   ALLOW IN    203.0.113.7
[ 6] 53                         ALLOW OUT   Anywhere                   (out)
[ 7] 8080 on eth0               ALLOW IN    Anywhere
[ 8] 22/tcp (v6)                LIMIT IN    Anywhere (v6)
@@added
Added user rules (see 'ufw status' for running firewall):
ufw limit 22/tcp
ufw allow 80,443/tcp comment 'web'
@@apps
Available applications:
  Nginx Full
  OpenSSH
@@defaults
DEFAULT_INPUT_POLICY=\"DROP\"
DEFAULT_OUTPUT_POLICY=\"ACCEPT\"
";

    #[test]
    fn parses_active_status() {
        let st = parse_status(ACTIVE).unwrap();
        assert!(st.active);
        assert_eq!(st.default_incoming.as_deref(), Some("deny"));
        assert_eq!(st.default_outgoing.as_deref(), Some("allow"));
        assert_eq!(st.logging.as_deref(), Some("on (low)"));
        assert_eq!(st.rules.len(), 8);
        let r = &st.rules[1];
        assert_eq!(
            (r.number, r.to.as_str(), r.action.as_str(), r.direction.as_str()),
            (2, "80,443/tcp", "ALLOW", "IN")
        );
        assert_eq!(r.comment.as_deref(), Some("web"));
        assert_eq!(r.raw, "80,443/tcp ALLOW IN Anywhere # web");
        assert_eq!(st.rules[2].to, "Nginx Full");
        assert_eq!(st.rules[3].from, "10.0.0.0/8");
        assert_eq!(st.rules[5].direction, "OUT");
        assert_eq!(st.rules[6].to, "8080 on eth0");
        assert!(st.rules[7].v6 && !st.rules[0].v6);
        assert_eq!(st.apps, ["Nginx Full", "OpenSSH"]);
        assert_eq!(st.added, ["limit 22/tcp", "allow 80,443/tcp comment 'web'"]);
        assert!(parse_status("@@missing\n").is_none());
    }

    #[test]
    fn inactive_status_uses_defaults_file_and_added_rules() {
        let out = "@@verbose\nStatus: inactive\n@@numbered\nStatus: inactive\n@@added\nAdded user rules (see 'ufw status' for running firewall):\nufw allow from 10.0.0.0/8 to any port 22 proto tcp\n@@apps\n@@defaults\nDEFAULT_INPUT_POLICY=\"DROP\"\n";
        let st = parse_status(out).unwrap();
        assert!(!st.active && st.rules.is_empty());
        assert_eq!(st.default_incoming.as_deref(), Some("deny"));
        let eff = st.effective_rules();
        assert_eq!(eff[0].to, "22/tcp");
        assert_eq!(eff[0].from, "10.0.0.0/8");
    }

    #[test]
    fn port_and_address_matching() {
        assert!(covers_port("22/tcp", 22, false));
        assert!(!covers_port("22/udp", 22, false));
        assert!(covers_port("22", 22, true));
        assert!(covers_port("80,443/tcp", 443, false));
        assert!(covers_port("2000:2300/tcp", 2222, false));
        assert!(!covers_port("2000:2300/tcp", 2400, false));
        assert!(covers_port("OpenSSH", 22, false));
        assert!(covers_port("22/tcp (v6)", 22, false));
        assert!(covers_port("8080 on eth0", 8080, false));
        assert!(covers_port("Anywhere", 5432, false));
        assert!(covers_port("10.0.0.5 22/tcp", 22, false));
        assert!(!covers_port("Nginx Full", 22, false));

        let c: IpAddr = "10.1.2.3".parse().unwrap();
        assert!(from_matches("Anywhere", Some(c)));
        assert!(from_matches("10.0.0.0/8", Some(c)));
        assert!(!from_matches("192.168.0.0/16", Some(c)));
        assert!(from_matches("10.1.2.3", Some(c)));
        assert!(!from_matches("10.0.0.0/8", None));
        assert!(from_matches("2001:db8::/32", Some("2001:db8::9".parse().unwrap())));
    }

    #[test]
    fn first_matching_rule_decides_reachability() {
        let st = parse_status(ACTIVE).unwrap();
        let me: IpAddr = "198.51.100.4".parse().unwrap();
        assert!(ssh_reachable(&st.rules, st.default_incoming.as_deref(), 22, Some(me)));
        // Without rule 1 and its v6 twin, only 203.0.113.7 gets in (rule 5: all ports).
        let without: Vec<UfwRule> = st
            .rules
            .iter()
            .filter(|r| r.to != "22/tcp" && r.to != "22/tcp (v6)")
            .cloned()
            .collect();
        assert!(!ssh_reachable(&without, Some("deny"), 22, Some(me)));
        assert!(ssh_reachable(&without, Some("deny"), 22, Some("203.0.113.7".parse().unwrap())));
        assert!(ssh_reachable(&without, Some("allow"), 22, Some(me)));
        // A deny before the allow wins.
        let mut deny_first = vec![parse_rule_line(1, "22/tcp DENY IN Anywhere").unwrap()];
        deny_first.extend(st.rules.clone());
        assert!(!ssh_reachable(&deny_first, Some("deny"), 22, Some(me)));
        // Custom SSH port.
        assert!(!ssh_reachable(&st.rules, Some("deny"), 2222, Some(me)));
    }

    #[test]
    fn a_rule_for_the_same_target_replaces_the_old_one() {
        // `ufw deny 2222/tcp` with `allow 2222/tcp` in place: ufw updates the allow, so SSH on 2222 is cut.
        let rules = vec![
            parse_rule_line(1, "22/tcp ALLOW IN Anywhere").unwrap(),
            parse_rule_line(2, "2222/tcp ALLOW IN Anywhere").unwrap(),
            parse_rule_line(3, "22/tcp (v6) ALLOW IN Anywhere (v6)").unwrap(),
            parse_rule_line(4, "2222/tcp (v6) ALLOW IN Anywhere (v6)").unwrap(),
        ];
        let me: IpAddr = "198.51.100.4".parse().unwrap();
        let deny = rule_from_added("deny 2222/tcp").unwrap();
        let after = after_add(&rules, &deny);
        assert_eq!(after.len(), 4, "updated in place, not appended");
        assert_eq!(after[1].action, "DENY");
        assert_eq!(after[3].action, "DENY", "the IPv6 twin too");
        assert!(!ssh_reachable(&after, Some("deny"), 2222, Some(me)), "the check sees the lock-out");
        // Appending (the old model) would have hidden it.
        let mut appended = rules.clone();
        appended.push(deny);
        assert!(ssh_reachable(&appended, Some("deny"), 2222, Some(me)));
        // A rule for another target is still appended.
        let web = rule_from_added("allow 80/tcp").unwrap();
        assert_eq!(after_add(&rules, &web).len(), 5);
        // From a specific address is another target.
        let one = rule_from_added("deny from 203.0.113.7 to any port 2222 proto tcp").unwrap();
        assert_eq!(after_add(&rules, &one).len(), 5);
    }

    #[test]
    fn added_commands_become_rules() {
        let r = rule_from_added("limit 22/tcp").unwrap();
        assert_eq!((r.action.as_str(), r.to.as_str(), r.from.as_str()), ("LIMIT", "22/tcp", "Anywhere"));
        let r = rule_from_added("allow 'Nginx Full'").unwrap();
        assert_eq!(r.to, "Nginx Full");
        let r = rule_from_added("allow in proto tcp from 203.0.113.7 to any port 2222 comment 'office'").unwrap();
        assert_eq!((r.to.as_str(), r.from.as_str()), ("2222/tcp", "203.0.113.7"));
        let r = rule_from_added("allow in from any to any app OpenSSH").unwrap();
        assert_eq!(r.to, "OpenSSH");
        let r = rule_from_added("allow out to any port 53").unwrap();
        assert_eq!(r.direction, "OUT");
        assert!(rule_from_added("default deny").is_none());
    }

    fn input(port: Option<&str>, app: Option<&str>) -> UfwRuleInput {
        UfwRuleInput {
            action: UfwAction::Allow,
            direction: UfwDirection::In,
            port: port.map(String::from),
            protocol: UfwProtocol::Tcp,
            address: None,
            app: app.map(String::from),
            comment: None,
        }
    }

    #[test]
    fn builds_rule_commands_and_rejects_bad_input() {
        assert_eq!(
            rule_args(&input(Some("22"), None)).unwrap(),
            "allow in proto tcp from any to any port 22"
        );
        let r = UfwRuleInput {
            action: UfwAction::Limit,
            address: Some("10.0.0.0/8".into()),
            comment: Some("office VPN's range".into()),
            ..input(Some("2222"), None)
        };
        assert_eq!(
            rule_args(&r).unwrap(),
            "limit in proto tcp from 10.0.0.0/8 to any port 2222 comment 'office VPN'\\''s range'"
        );
        let app = UfwRuleInput {
            protocol: UfwProtocol::Any,
            ..input(None, Some("Nginx Full"))
        };
        assert_eq!(rule_args(&app).unwrap(), "allow in from any to any app 'Nginx Full'");
        let out = UfwRuleInput {
            direction: UfwDirection::Out,
            protocol: UfwProtocol::Udp,
            ..input(Some("53"), None)
        };
        assert_eq!(rule_args(&out).unwrap(), "allow out proto udp to any port 53");

        for bad in ["22; reboot", "0", "70000", "80:70", "$(id)", "22 80"] {
            assert!(rule_args(&input(Some(bad), None)).is_err(), "{bad}");
        }
        let any_range = UfwRuleInput {
            protocol: UfwProtocol::Any,
            ..input(Some("6000:6007"), None)
        };
        assert!(rule_args(&any_range).is_err());
        assert!(rule_args(&input(None, None)).is_err());
        assert!(rule_args(&input(Some("22"), Some("OpenSSH"))).is_err());
        assert!(rule_args(&input(None, Some("x'; reboot"))).is_err());
        for addr in ["10.0.0.0/33", "not-an-ip", "1.2.3.4; id"] {
            let r = UfwRuleInput {
                address: Some(addr.into()),
                ..input(Some("22"), None)
            };
            assert!(rule_args(&r).is_err(), "{addr}");
        }
        let nl = UfwRuleInput {
            comment: Some("a\nb".into()),
            ..input(Some("22"), None)
        };
        assert!(rule_args(&nl).is_err());
    }

    #[test]
    fn preview_matches_what_ufw_will_list() {
        let r = UfwRuleInput {
            action: UfwAction::Deny,
            ..input(Some("22"), None)
        };
        let p = preview_rule(&r).unwrap();
        assert_eq!((p.action.as_str(), p.to.as_str(), p.from.as_str()), ("DENY", "22/tcp", "Anywhere"));
        assert!(!ssh_reachable(&[p], Some("allow"), 22, None));
    }
}
