//! Kernel tuning: well-known `sysctl` settings with their current value and
//! a recommendation, and one file Orbital owns, `/etc/sysctl.d/90-orbital.conf`.
//!
//! Applying writes that file from stdin, loads it with `sysctl -p` and puts
//! the previous file back if the kernel refuses a value. Keys and values
//! match strict patterns and every key must exist on the server first.
//! Removing a key from the file keeps its current value until the next boot.

use crate::error::{Error, Result};
use crate::shell::quote;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;

pub const FILE: &str = "/etc/sysctl.d/90-orbital.conf";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Cmp {
    /// Should be exactly the recommendation.
    Eq,
    /// At least this number.
    AtLeast,
    /// At most this number.
    AtMost,
}

struct Known {
    key: &'static str,
    group: &'static str,
    title: &'static str,
    recommended: &'static str,
    cmp: Cmp,
    why: &'static str,
    /// Only worth it when this tool runs on the server (e.g. Redis).
    only_with: Option<&'static str>,
}

const KNOWN: &[Known] = &[
    Known {
        key: "net.ipv4.tcp_syncookies",
        group: "Network security",
        title: "SYN flood protection",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "Keeps the server answering when it's flooded with half-open connections.",
        only_with: None,
    },
    Known {
        key: "net.ipv4.conf.all.accept_redirects",
        group: "Network security",
        title: "Ignore ICMP redirects (IPv4)",
        recommended: "0",
        cmp: Cmp::Eq,
        why: "Redirects let another machine on the network reroute this server's traffic. Servers don't need them.",
        only_with: None,
    },
    Known {
        key: "net.ipv4.conf.default.accept_redirects",
        group: "Network security",
        title: "Ignore ICMP redirects on new interfaces",
        recommended: "0",
        cmp: Cmp::Eq,
        why: "The same for interfaces that appear later (Docker, VPNs).",
        only_with: None,
    },
    Known {
        key: "net.ipv6.conf.all.accept_redirects",
        group: "Network security",
        title: "Ignore ICMP redirects (IPv6)",
        recommended: "0",
        cmp: Cmp::Eq,
        why: "As for IPv4.",
        only_with: None,
    },
    Known {
        key: "net.ipv4.conf.all.send_redirects",
        group: "Network security",
        title: "Don't send redirects",
        recommended: "0",
        cmp: Cmp::Eq,
        why: "Only routers should send them. Leave it on if this server routes traffic for others (a VPN gateway).",
        only_with: None,
    },
    Known {
        key: "net.ipv4.conf.all.accept_source_route",
        group: "Network security",
        title: "Refuse source-routed packets",
        recommended: "0",
        cmp: Cmp::Eq,
        why: "Source routing lets a sender choose the path, which is used to get around filters.",
        only_with: None,
    },
    Known {
        key: "net.ipv4.conf.all.log_martians",
        group: "Network security",
        title: "Log impossible addresses",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "Packets with addresses that can't be real are logged, which shows spoofing attempts.",
        only_with: None,
    },
    Known {
        key: "net.ipv4.icmp_echo_ignore_broadcasts",
        group: "Network security",
        title: "Ignore broadcast pings",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "Stops the server taking part in smurf amplification attacks.",
        only_with: None,
    },
    Known {
        key: "kernel.dmesg_restrict",
        group: "Kernel hardening",
        title: "Kernel log for root only",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "The kernel log can reveal addresses that help exploits; ordinary users don't need it.",
        only_with: None,
    },
    Known {
        key: "kernel.kptr_restrict",
        group: "Kernel hardening",
        title: "Hide kernel addresses",
        recommended: "1",
        cmp: Cmp::AtLeast,
        why: "Kernel pointers in /proc are hidden from users, which makes exploits harder.",
        only_with: None,
    },
    Known {
        key: "fs.protected_hardlinks",
        group: "Kernel hardening",
        title: "Protected hard links",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "Users can't hard-link files they don't own, a classic way to trick root programs.",
        only_with: None,
    },
    Known {
        key: "fs.protected_symlinks",
        group: "Kernel hardening",
        title: "Protected symbolic links",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "Links in shared folders like /tmp are followed only when it's safe.",
        only_with: None,
    },
    Known {
        key: "fs.suid_dumpable",
        group: "Kernel hardening",
        title: "No core dumps from setuid programs",
        recommended: "0",
        cmp: Cmp::Eq,
        why: "A crash dump of a privileged program can contain passwords or keys.",
        only_with: None,
    },
    Known {
        key: "vm.swappiness",
        group: "Performance",
        title: "Swap only when memory runs out",
        recommended: "10",
        cmp: Cmp::AtMost,
        why: "The default (60) swaps early. Servers stay faster when they keep programs in memory.",
        only_with: None,
    },
    Known {
        key: "net.core.somaxconn",
        group: "Performance",
        title: "Connection queue",
        recommended: "4096",
        cmp: Cmp::AtLeast,
        why: "How many new connections may wait for a busy web server or database before they're refused.",
        only_with: None,
    },
    Known {
        key: "fs.inotify.max_user_watches",
        group: "Performance",
        title: "File watches",
        recommended: "524288",
        cmp: Cmp::AtLeast,
        why: "Programs that watch files (Node.js, IDE servers, log shippers) fail with \"no space left\" when this runs out.",
        only_with: None,
    },
    Known {
        key: "vm.overcommit_memory",
        group: "Performance",
        title: "Memory overcommit for Redis",
        recommended: "1",
        cmp: Cmp::Eq,
        why: "Redis needs it to save snapshots reliably (it warns about it in its log).",
        only_with: Some("redis"),
    },
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KernelSetting {
    pub key: String,
    pub group: String,
    pub title: String,
    pub why: String,
    pub recommended: String,
    pub cmp: Cmp,
    pub only_with: Option<String>,
    /// What the kernel uses now; `None` if the key doesn't exist here.
    pub current: Option<String>,
    /// The value in Orbital's file, if it sets this key.
    pub ours: Option<String>,
    /// The current value meets the recommendation.
    pub ok: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KernelReport {
    pub settings: Vec<KernelSetting>,
    /// Keys in Orbital's file that aren't in the list above, with their values.
    pub custom: BTreeMap<String, String>,
    /// Current values of `custom` keys.
    pub custom_current: BTreeMap<String, String>,
    pub file_exists: bool,
    pub file: String,
}

static KEY: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9_]+(\.[A-Za-z0-9_\-]+)+$").unwrap());
static VALUE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9 _.,:/\-]{1,100}$").unwrap());

pub fn validate(key: &str, value: &str) -> Result<()> {
    if key.len() > 120 || !KEY.is_match(key) {
        return Err(Error::InvalidInput(format!("{key:?} isn't a sysctl name (like vm.swappiness)")));
    }
    if !VALUE.is_match(value.trim()) || value.trim() != value {
        return Err(Error::InvalidInput(format!("{value:?} isn't a value Orbital will write for {key}")));
    }
    Ok(())
}

/// Collapse tabs and repeated spaces the way `sysctl -n` prints values.
fn norm(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn meets(current: &str, recommended: &str, cmp: Cmp) -> bool {
    let (c, r) = (norm(current), norm(recommended));
    match cmp {
        Cmp::Eq => c == r,
        Cmp::AtLeast | Cmp::AtMost => match (c.parse::<i64>(), r.parse::<i64>()) {
            (Ok(c), Ok(r)) => (cmp == Cmp::AtLeast && c >= r) || (cmp == Cmp::AtMost && c <= r),
            _ => c == r,
        },
    }
}

/// Read every known key and Orbital's file. Nothing interpolated but fixed names.
pub fn read_script() -> String {
    let keys: Vec<&str> = KNOWN.iter().map(|k| k.key).collect();
    format!(
        "export LC_ALL=C\nfor k in {}; do v=$(sysctl -n \"$k\" 2>/dev/null) && printf '@@cur %s=%s\\n' \"$k\" \"$v\"; done\n\
         if [ -f {f} ]; then echo @@file; cat {f}; echo; echo @@end\n\
         for k in $(sed -n 's/^[[:space:]]*\\([a-z0-9_.-]*\\)[[:space:]]*=.*/\\1/p' {f}); do v=$(sysctl -n \"$k\" 2>/dev/null) && printf '@@cur %s=%s\\n' \"$k\" \"$v\"; done; fi\n",
        keys.join(" "),
        f = quote(FILE)
    )
}

/// `key = value` lines of a sysctl.d file (comments and blanks skipped).
pub fn parse_file(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            Some((k.trim().trim_start_matches('-').to_string(), v.trim().to_string()))
        })
        .collect()
}

pub fn parse(out: &str) -> KernelReport {
    let mut current = BTreeMap::new();
    let mut file = String::new();
    let mut in_file = false;
    let mut file_exists = false;
    for line in out.lines() {
        if line == "@@file" {
            in_file = true;
            file_exists = true;
            continue;
        }
        if line == "@@end" {
            in_file = false;
            continue;
        }
        if in_file {
            file.push_str(line);
            file.push('\n');
        } else if let Some((k, v)) = line.strip_prefix("@@cur ").and_then(|r| r.split_once('=')) {
            current.insert(k.to_string(), norm(v));
        }
    }
    let ours = parse_file(&file);
    let settings = KNOWN
        .iter()
        .map(|k| {
            let cur = current.get(k.key).cloned();
            KernelSetting {
                key: k.key.into(),
                group: k.group.into(),
                title: k.title.into(),
                why: k.why.into(),
                recommended: k.recommended.into(),
                cmp: k.cmp,
                only_with: k.only_with.map(String::from),
                ok: cur.as_deref().is_some_and(|c| meets(c, k.recommended, k.cmp)),
                ours: ours.get(k.key).cloned(),
                current: cur,
            }
        })
        .collect();
    let custom: BTreeMap<String, String> = ours
        .iter()
        .filter(|(k, _)| !KNOWN.iter().any(|n| n.key == k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let custom_current = custom
        .keys()
        .filter_map(|k| current.get(k).map(|v| (k.clone(), v.clone())))
        .collect();
    KernelReport {
        settings,
        custom,
        custom_current,
        file_exists,
        file: FILE.into(),
    }
}

/// The file Orbital writes for `values` (sorted, with a header).
pub fn render(values: &BTreeMap<String, String>) -> Result<String> {
    let mut out = String::from("# Written by Orbital (Kernel tuning). Edit it there, or here; sysctl reads it at boot.\n");
    for (k, v) in values {
        validate(k, v)?;
        out.push_str(&format!("{k} = {v}\n"));
    }
    Ok(out)
}

/// Install the file from stdin, load it, and put the previous one back if
/// the kernel refuses it. Every key must exist first. With no keys, the
/// file is removed (values stay until the next boot).
pub fn apply_script(keys: &[&str]) -> Result<String> {
    for k in keys {
        validate(k, "0")?;
    }
    let f = quote(FILE);
    if keys.is_empty() {
        return Ok(format!("cat >/dev/null; rm -f -- {f}; echo @@removed\n"));
    }
    let list: Vec<String> = keys.iter().map(|k| quote(k)).collect();
    Ok(format!(
        "export LC_ALL=C\n\
         for k in {list}; do sysctl -n \"$k\" >/dev/null 2>&1 || {{ echo \"@@unknown $k\"; cat >/dev/null; exit 4; }}; done\n\
         t=$(mktemp) || exit 1; trap 'rm -f \"$t\" \"$t.old\"' EXIT\n\
         cat > \"$t\"\n\
         had=0; [ -e {f} ] && {{ cp -p {f} \"$t.old\"; had=1; }}\n\
         mkdir -p /etc/sysctl.d && cp \"$t\" {f} && chmod 644 {f} || exit 1\n\
         if out=$(sysctl -p {f} 2>&1); then printf '%s\\n' \"$out\"; echo @@applied; else\n\
           printf '%s\\n' \"$out\"\n\
           if [ $had = 1 ]; then cp -p \"$t.old\" {f}; sysctl -p {f} >/dev/null 2>&1; else rm -f {f}; fi\n\
           echo @@restored; exit 3\n\
         fi\n",
        list = list.join(" ")
    ))
}

/// The unknown key a failed apply named, if that's why it failed.
pub fn unknown_key(out: &str) -> Option<String> {
    out.lines().find_map(|l| l.strip_prefix("@@unknown ").map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@cur net.ipv4.tcp_syncookies=1
@@cur vm.swappiness=60
@@cur net.core.somaxconn=4096
@@cur kernel.kptr_restrict=2
@@cur fs.suid_dumpable=0
@@file
# Written by Orbital
vm.swappiness = 10
net.ipv4.ip_local_port_range = 10000	65000

@@end
@@cur net.ipv4.ip_local_port_range=10000	65000
";

    #[test]
    fn reads_current_values_and_our_file() {
        let r = parse(OUT);
        assert!(r.file_exists);
        let get = |k: &str| r.settings.iter().find(|s| s.key == k).unwrap().clone();
        assert!(get("net.ipv4.tcp_syncookies").ok);
        let sw = get("vm.swappiness");
        assert_eq!((sw.current.as_deref(), sw.ours.as_deref(), sw.ok), (Some("60"), Some("10"), false));
        assert!(get("kernel.kptr_restrict").ok, "2 is at least 1");
        assert!(get("net.core.somaxconn").ok);
        assert_eq!(get("vm.overcommit_memory").current, None, "not on this kernel");
        assert_eq!(get("vm.overcommit_memory").only_with.as_deref(), Some("redis"));
        assert_eq!(
            r.custom.get("net.ipv4.ip_local_port_range").map(String::as_str),
            Some("10000\t65000")
        );
        assert_eq!(
            r.custom_current.get("net.ipv4.ip_local_port_range").map(String::as_str),
            Some("10000 65000")
        );
    }

    #[test]
    fn compares_like_numbers_when_asked() {
        assert!(meets("10", "10", Cmp::AtMost));
        assert!(meets("1", "10", Cmp::AtMost));
        assert!(!meets("60", "10", Cmp::AtMost));
        assert!(meets("65535", "4096", Cmp::AtLeast));
        assert!(meets("10000\t65000", "10000 65000", Cmp::Eq));
    }

    #[test]
    fn refuses_bad_keys_and_values() {
        assert!(validate("vm.swappiness", "10").is_ok());
        assert!(validate("net.ipv4.conf.eth0-1.rp_filter", "2").is_ok());
        for k in ["vm", "vm.swappiness;id", "../etc", "VM.Swap", "vm..x"] {
            assert!(validate(k, "1").is_err(), "{k}");
        }
        for v in ["", "1; reboot", "$(id)", " 1", "a\nb"] {
            assert!(validate("vm.swappiness", v).is_err(), "{v:?}");
        }
        assert!(render(&[("vm.swappiness".to_string(), "1;x".to_string())].into()).is_err());
    }

    #[test]
    fn renders_and_parses_back() {
        let v: BTreeMap<String, String> = [("vm.swappiness", "10"), ("fs.protected_symlinks", "1")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let text = render(&v).unwrap();
        assert_eq!(parse_file(&text), v);
        assert!(text.starts_with("# Written by Orbital"));
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn scripts_are_valid_sh() {
        for s in [read_script(), apply_script(&["vm.swappiness"]).unwrap(), apply_script(&[]).unwrap()] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&s).output().unwrap();
            assert!(o.status.success(), "{}\n{}", s, String::from_utf8_lossy(&o.stderr));
        }
        assert!(apply_script(&["bad;key"]).is_err());
        assert_eq!(unknown_key("x\n@@unknown vm.nope\n").as_deref(), Some("vm.nope"));
    }
}
