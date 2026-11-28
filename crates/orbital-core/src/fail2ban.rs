//! Fail2ban: which jails watch which logs, who is banned right now, and the
//! commands to ban and unban an address.
//!
//! Jail names reach the status script only as shell variables read from
//! `fail2ban-client` itself; names and addresses the user sends back are
//! validated before they go into a command.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::net::IpAddr;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Jail {
    pub name: String,
    pub currently_failed: u64,
    pub total_failed: u64,
    pub currently_banned: u64,
    pub total_banned: u64,
    pub banned: Vec<String>,
    /// Log files or journal matches the jail reads.
    pub watching: Vec<String>,
    /// Seconds an address stays banned (`-1` is forever).
    pub bantime: Option<String>,
    /// Window, in seconds, in which `maxretry` failures trigger a ban.
    pub findtime: Option<String>,
    pub maxretry: Option<String>,
    /// Addresses and networks this jail never bans.
    pub ignore_ip: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fail2banStatus {
    pub version: Option<String>,
    /// The server process answers.
    pub running: bool,
    /// What fail2ban-client said when it couldn't answer.
    pub error: Option<String>,
    pub jails: Vec<Jail>,
    /// The address this SSH connection comes from, so the UI can warn before banning it.
    pub client_ip: Option<String>,
}

pub(crate) const STATUS_SCRIPT: &str = r#"export LC_ALL=C
command -v fail2ban-client >/dev/null 2>&1 || { echo @@missing; exit 0; }
echo "@@version $(fail2ban-client --version 2>/dev/null | head -1)"
st=$(fail2ban-client status 2>&1) || { echo "@@down"; printf '%s\n' "$st"; exit 0; }
for j in $(printf '%s\n' "$st" | sed -n 's/.*Jail list:[[:space:]]*//p' | tr ',' ' '); do
  echo "@@jail $j"
  fail2ban-client status "$j" 2>&1
  for k in bantime findtime maxretry; do printf '%s=%s\n' "$k" "$(fail2ban-client get "$j" "$k" 2>/dev/null | head -1)"; done
  echo "@@ignoreip"
  fail2ban-client get "$j" ignoreip 2>/dev/null
done
"#;

fn strip_tree(line: &str) -> &str {
    line.trim_start_matches(|c: char| c == '|' || c == '`' || c == '-' || c.is_whitespace())
}

/// Parse the status script's output. `None` when fail2ban is not installed.
pub fn parse_status(out: &str) -> Option<Fail2banStatus> {
    let mut st = Fail2banStatus {
        running: true,
        ..Default::default()
    };
    let mut cur: Option<Jail> = None;
    let mut in_ignore = false;
    let mut down: Vec<&str> = Vec::new();
    let mut is_down = false;
    for line in out.lines() {
        let l = line.trim_end();
        if l == "@@missing" {
            return None;
        }
        if let Some(v) = l.strip_prefix("@@version") {
            let v = v.trim().trim_start_matches("Fail2Ban v").trim_start_matches('v');
            st.version = (!v.is_empty()).then(|| v.to_string());
            continue;
        }
        if l == "@@down" {
            is_down = true;
            st.running = false;
            continue;
        }
        if is_down {
            if !l.trim().is_empty() {
                down.push(l.trim());
            }
            continue;
        }
        if let Some(name) = l.strip_prefix("@@jail ") {
            st.jails.extend(cur.take());
            cur = Some(Jail {
                name: name.trim().to_string(),
                ..Default::default()
            });
            in_ignore = false;
            continue;
        }
        if l == "@@ignoreip" {
            in_ignore = true;
            continue;
        }
        let Some(j) = cur.as_mut() else { continue };
        if in_ignore {
            // "These IP addresses/networks are ignored:" then a tree of entries.
            if l.starts_with('|') || l.starts_with('`') {
                let v = strip_tree(l);
                if !v.is_empty() {
                    j.ignore_ip.push(v.to_string());
                }
            }
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            if matches!(k, "bantime" | "findtime" | "maxretry") {
                let v = v.trim();
                let v = (!v.is_empty()).then(|| v.to_string());
                match k {
                    "bantime" => j.bantime = v,
                    "findtime" => j.findtime = v,
                    _ => j.maxretry = v,
                }
                continue;
            }
        }
        let Some((k, v)) = strip_tree(l).split_once(':') else { continue };
        let v = v.trim();
        let n = || v.parse().unwrap_or(0);
        match k.trim() {
            "Currently failed" => j.currently_failed = n(),
            "Total failed" => j.total_failed = n(),
            "Currently banned" => j.currently_banned = n(),
            "Total banned" => j.total_banned = n(),
            "Banned IP list" => j.banned = v.split_whitespace().map(String::from).collect(),
            "File list" => j.watching = v.split_whitespace().map(String::from).collect(),
            "Journal matches" if !v.is_empty() => j.watching = vec![v.to_string()],
            _ => {}
        }
    }
    st.jails.extend(cur.take());
    if !st.running {
        st.error = Some(if down.is_empty() {
            "fail2ban is not answering".into()
        } else {
            down.join("\n")
        });
    }
    Some(st)
}

static JAIL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$").unwrap());

pub fn validate_jail(name: &str) -> Result<&str> {
    if JAIL_RE.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!("invalid jail name: {name:?}")))
    }
}

pub fn validate_ip(ip: &str) -> Result<IpAddr> {
    ip.trim()
        .parse()
        .map_err(|_| Error::InvalidInput(format!("{ip:?} is not an IP address")))
}

/// `fail2ban-client` command to unban `ip` from one jail, or from all of them.
pub fn unban_command(jail: Option<&str>, ip: &str) -> Result<String> {
    let ip = validate_ip(ip)?.to_string();
    Ok(match jail {
        Some(j) => format!("fail2ban-client set {} unbanip {}", quote(validate_jail(j)?), quote(&ip)),
        None => format!("fail2ban-client unban {}", quote(&ip)),
    })
}

pub fn ban_command(jail: &str, ip: &str) -> Result<String> {
    let ip = validate_ip(ip)?.to_string();
    Ok(format!("fail2ban-client set {} banip {}", quote(validate_jail(jail)?), quote(&ip)))
}

// ---------- SSH protection (Fail2ban → Protection) ----------

/// Where Orbital keeps its sshd jail settings. `.local` files in jail.d win over
/// the distribution's `.conf` ones; the stock files are never changed.
pub const PROTECT_FILE: &str = "/etc/fail2ban/jail.d/zz-orbital-sshd.local";

/// How SSH logins are limited: `max_retry` failures within `find_time_mins`
/// ban the address for `ban_time_mins` (`-1` = for good).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProtection {
    pub max_retry: u32,
    pub find_time_mins: u32,
    pub ban_time_mins: i64,
    /// Each new ban of the same address lasts longer (doubling, up to a week).
    pub increment: bool,
    /// Never ban the address this computer connects from.
    pub ignore_self: bool,
    /// Ban addresses that were banned 5 times in a day for a week (the recidive jail).
    pub recidive: bool,
    /// More addresses or networks never to ban.
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl Default for SshProtection {
    fn default() -> Self {
        Self {
            max_retry: 5,
            find_time_mins: 10,
            ban_time_mins: 60,
            increment: true,
            ignore_self: true,
            recidive: true,
            ignore: Vec::new(),
        }
    }
}

/// An address or a network (`203.0.113.0/24`).
fn valid_net(s: &str) -> bool {
    let (ip, bits) = match s.split_once('/') {
        Some((ip, b)) => (ip, Some(b)),
        None => (s, None),
    };
    match ip.parse::<IpAddr>() {
        Ok(a) => bits.is_none_or(|b| b.parse::<u8>().is_ok_and(|n| n <= if a.is_ipv4() { 32 } else { 128 })),
        Err(_) => false,
    }
}

pub fn validate_protection(p: &SshProtection) -> Result<()> {
    if !(2..=20).contains(&p.max_retry) {
        return Err(Error::InvalidInput("allow 2 to 20 failed attempts".into()));
    }
    if !(1..=1440).contains(&p.find_time_mins) {
        return Err(Error::InvalidInput("count failures over 1 minute to 24 hours".into()));
    }
    if p.ban_time_mins != -1 && !(1..=525_600).contains(&p.ban_time_mins) {
        return Err(Error::InvalidInput("ban for 1 minute to a year, or for good".into()));
    }
    if p.ignore.len() > 20 {
        return Err(Error::InvalidInput("up to 20 addresses never to ban".into()));
    }
    if let Some(bad) = p.ignore.iter().find(|x| !valid_net(x.trim())) {
        return Err(Error::InvalidInput(format!(
            "{bad:?} isn't an address or a network like 203.0.113.0/24"
        )));
    }
    Ok(())
}

/// The jail file: only numbers and checked addresses go in.
pub fn render_protection(p: &SshProtection, client_ip: Option<&str>) -> Result<String> {
    validate_protection(p)?;
    let mut ignore = vec!["127.0.0.1/8".to_string(), "::1".to_string()];
    if p.ignore_self {
        if let Some(ip) = client_ip.filter(|ip| valid_net(ip)) {
            ignore.push(ip.to_string());
        }
    }
    for x in &p.ignore {
        let x = x.trim().to_string();
        if !ignore.contains(&x) {
            ignore.push(x);
        }
    }
    let ban = if p.ban_time_mins == -1 {
        "-1".to_string()
    } else {
        format!("{}m", p.ban_time_mins)
    };
    let mut s = format!(
        "# Written by Orbital (Fail2ban → Protection); change it there or remove this file.\n[sshd]\nenabled = true\nmaxretry = {}\nfindtime = {}m\nbantime = {ban}\nignoreip = {}\n",
        p.max_retry,
        p.find_time_mins,
        ignore.join(" ")
    );
    if p.increment && p.ban_time_mins != -1 {
        s.push_str("bantime.increment = true\nbantime.factor = 2\nbantime.maxtime = 1w\n");
    }
    if p.recidive {
        s.push_str("\n[recidive]\nenabled = true\nfindtime = 1d\nmaxretry = 5\nbantime = 1w\n");
    }
    Ok(s)
}

/// Applies the file on stdin: keeps the old one, checks the whole config with
/// `fail2ban-client -t`, puts the old one back if that fails, then reloads.
/// On systems without auth.log (journal only) the sshd jail reads the journal.
pub const PROTECT_APPLY: &str = r#"set -e
f=/etc/fail2ban/jail.d/zz-orbital-sshd.local
tmp=$(mktemp)
cat > "$tmp"
if [ ! -e /var/log/auth.log ] && [ ! -e /var/log/secure ] && command -v journalctl >/dev/null 2>&1; then
  awk '{ print } $0 == "[sshd]" { print "backend = systemd" }' "$tmp" > "$tmp.b" && mv "$tmp.b" "$tmp"
fi
[ -e "$f" ] && cp -p "$f" "$f.orbital-prev" || rm -f "$f.orbital-prev"
install -m 0644 "$tmp" "$f"; rm -f "$tmp"
if ! out=$(fail2ban-client -t 2>&1); then
  if [ -e "$f.orbital-prev" ]; then mv "$f.orbital-prev" "$f"; else rm -f "$f"; fi
  echo "fail2ban rejected the settings, so the old ones were kept:" >&2
  printf '%s
' "$out" | tail -n 15 >&2
  exit 3
fi
rm -f "$f.orbital-prev"
if systemctl is-active --quiet fail2ban 2>/dev/null; then fail2ban-client reload >/dev/null; else systemctl enable --now fail2ban >/dev/null 2>&1 || fail2ban-client start >/dev/null; fi
echo ok"#;

/// Reads the file back (to fill the form), or None when Orbital hasn't written one.
pub const PROTECT_READ: &str = "cat /etc/fail2ban/jail.d/zz-orbital-sshd.local 2>/dev/null || true";

/// Duration text from fail2ban (`10m`, `1h`, `600`, `-1`) in minutes.
fn minutes_of(v: &str) -> Option<i64> {
    let v = v.trim();
    if v == "-1" {
        return Some(-1);
    }
    let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len()));
    let n: i64 = num.parse().ok()?;
    Some(match unit.trim() {
        "" | "s" => (n + 59) / 60,
        "m" => n,
        "h" => n * 60,
        "d" => n * 1440,
        "w" => n * 10080,
        _ => return None,
    })
}

pub fn parse_protection(file: &str) -> Option<SshProtection> {
    if !file.contains("[sshd]") {
        return None;
    }
    let mut p = SshProtection {
        increment: false,
        recidive: file.contains("[recidive]"),
        ignore_self: false,
        ..Default::default()
    };
    let mut section = "";
    for l in file.lines().map(str::trim) {
        if l.starts_with('[') {
            section = l;
            continue;
        }
        if section != "[sshd]" {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "maxretry" => p.max_retry = v.parse().unwrap_or(p.max_retry),
            "findtime" => p.find_time_mins = minutes_of(v).map(|m| m.max(1) as u32).unwrap_or(p.find_time_mins),
            "bantime" => p.ban_time_mins = minutes_of(v).unwrap_or(p.ban_time_mins),
            "bantime.increment" => p.increment = v == "true",
            "ignoreip" => {
                p.ignore = v
                    .split_whitespace()
                    .filter(|x| *x != "127.0.0.1/8" && *x != "::1")
                    .map(String::from)
                    .collect()
            }
            _ => {}
        }
    }
    Some(p)
}

impl SshSession {
    pub async fn fail2ban_status(&self) -> Result<Fail2banStatus> {
        let (o, _) = self.exec_inspect(STATUS_SCRIPT).await?;
        let mut st = parse_status(&o.stdout).ok_or_else(|| Error::NotFound("fail2ban is not installed on this server".into()))?;
        if !st.running && st.error.as_deref().is_some_and(|e| e.contains("Permission denied")) {
            return Err(Error::PermissionDenied(
                "reading fail2ban needs root: turn on sudo in this server's settings (with the sudo password if it asks for one), then reconnect".into(),
            ));
        }
        st.client_ip = self.ssh_connection().await.0.map(|ip| ip.to_string());
        Ok(st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@version 1.0.2
@@jail sshd
Status for the jail: sshd
|- Filter
|  |- Currently failed:\t3
|  |- Total failed:\t42
|  `- Journal matches:\t_SYSTEMD_UNIT=sshd.service + _COMM=sshd
`- Actions
   |- Currently banned:\t2
   |- Total banned:\t9
   `- Banned IP list:\t203.0.113.7 2001:db8::5
bantime=600
findtime=600
maxretry=5
@@ignoreip
These IP addresses/networks are ignored:
|- 127.0.0.0/8
`- ::1
@@jail nginx-http-auth
Status for the jail: nginx-http-auth
|- Filter
|  |- Currently failed:\t0
|  |- Total failed:\t0
|  `- File list:\t/var/log/nginx/error.log
`- Actions
   |- Currently banned:\t0
   |- Total banned:\t0
   `- Banned IP list:\t
bantime=-1
findtime=
maxretry=3
@@ignoreip
No IP address/network is ignored
";

    #[test]
    fn parses_jails() {
        let st = parse_status(OUT).unwrap();
        assert!(st.running && st.error.is_none());
        assert_eq!(st.version.as_deref(), Some("1.0.2"));
        assert_eq!(st.jails.len(), 2);
        let ssh = &st.jails[0];
        assert_eq!(ssh.name, "sshd");
        assert_eq!(
            (ssh.currently_failed, ssh.total_failed, ssh.currently_banned, ssh.total_banned),
            (3, 42, 2, 9)
        );
        assert_eq!(ssh.banned, ["203.0.113.7", "2001:db8::5"]);
        assert_eq!(ssh.watching, ["_SYSTEMD_UNIT=sshd.service + _COMM=sshd"]);
        assert_eq!(ssh.ignore_ip, ["127.0.0.0/8", "::1"]);
        assert_eq!((ssh.bantime.as_deref(), ssh.maxretry.as_deref()), (Some("600"), Some("5")));
        let web = &st.jails[1];
        assert!(web.banned.is_empty() && web.ignore_ip.is_empty());
        assert_eq!(web.watching, ["/var/log/nginx/error.log"]);
        assert_eq!(web.findtime, None);
        assert_eq!(web.bantime.as_deref(), Some("-1"));
    }

    #[test]
    fn not_running_and_not_installed() {
        assert!(parse_status("@@missing\n").is_none());
        let st = parse_status(
            "@@version 0.11.2\n@@down\nERROR   Failed to access socket path: /var/run/fail2ban/fail2ban.sock. Is fail2ban running?\n",
        )
        .unwrap();
        assert!(!st.running);
        assert!(st.error.unwrap().contains("Is fail2ban running"));
        assert!(st.jails.is_empty());
    }

    #[test]
    fn commands_validate_their_input() {
        assert_eq!(
            unban_command(Some("sshd"), "203.0.113.7").unwrap(),
            "fail2ban-client set sshd unbanip 203.0.113.7"
        );
        assert_eq!(unban_command(None, "2001:db8::5").unwrap(), "fail2ban-client unban 2001:db8::5");
        assert_eq!(
            ban_command("nginx-http-auth", "198.51.100.1").unwrap(),
            "fail2ban-client set nginx-http-auth banip 198.51.100.1"
        );
        assert!(unban_command(Some("sshd; reboot"), "1.2.3.4").is_err());
        assert!(unban_command(Some("-x"), "1.2.3.4").is_err());
        assert!(unban_command(None, "1.2.3.4 && id").is_err());
        assert!(ban_command("sshd", "10.0.0.0/8").is_err());
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(STATUS_SCRIPT).output().unwrap();
        assert!(out.status.success());
        let _ = parse_status(&String::from_utf8_lossy(&out.stdout));
    }

    #[test]
    fn ssh_protection_renders_checks_and_reads_back() {
        let p = SshProtection {
            max_retry: 3,
            find_time_mins: 15,
            ban_time_mins: 1440,
            ignore: vec!["198.51.100.0/24".into()],
            ..Default::default()
        };
        let f = render_protection(&p, Some("203.0.113.9")).unwrap();
        assert!(
            f.contains("[sshd]\nenabled = true\nmaxretry = 3\nfindtime = 15m\nbantime = 1440m\n"),
            "{f}"
        );
        assert!(f.contains("ignoreip = 127.0.0.1/8 ::1 203.0.113.9 198.51.100.0/24"), "{f}");
        assert!(f.contains("bantime.increment = true") && f.contains("[recidive]"));
        let back = parse_protection(&f).unwrap();
        assert_eq!(
            (
                back.max_retry,
                back.find_time_mins,
                back.ban_time_mins,
                back.increment,
                back.recidive
            ),
            (3, 15, 1440, true, true)
        );
        assert_eq!(back.ignore, ["203.0.113.9", "198.51.100.0/24"]);
        let forever = render_protection(
            &SshProtection {
                ban_time_mins: -1,
                recidive: false,
                ignore_self: false,
                ..Default::default()
            },
            Some("203.0.113.9"),
        )
        .unwrap();
        assert!(forever.contains("bantime = -1") && !forever.contains("increment") && !forever.contains("203.0.113.9"));
        for bad in [
            SshProtection {
                max_retry: 1,
                ..Default::default()
            },
            SshProtection {
                find_time_mins: 0,
                ..Default::default()
            },
            SshProtection {
                ban_time_mins: 0,
                ..Default::default()
            },
            SshProtection {
                ignore: vec!["evil; rm -rf /".into()],
                ..Default::default()
            },
            SshProtection {
                ignore: vec!["10.0.0.0/40".into()],
                ..Default::default()
            },
        ] {
            assert!(render_protection(&bad, None).is_err(), "{bad:?}");
        }
        assert_eq!(minutes_of("1h"), Some(60));
        assert_eq!(minutes_of("600"), Some(10));
        assert_eq!(minutes_of("1w"), Some(10080));
        assert!(parse_protection("").is_none());
        let o = std::process::Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(PROTECT_APPLY)
            .output()
            .unwrap();
        assert!(o.status.success());
    }
}
