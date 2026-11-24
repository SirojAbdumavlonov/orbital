//! Sign-ins and attacks (Logins tab): who is signed in now (`who`), recent
//! sign-ins (`last`), and failed SSH attempts over the last 24 hours from the
//! journal (or auth.log / secure), counted by address and by user name. One
//! fixed, read-only script; parsed here.

use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub user: String,
    pub tty: String,
    /// The address they came from ("" for the console).
    pub from: String,
    /// As `who`/`last` print it.
    pub when: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    pub key: String,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginsReport {
    pub now: Vec<Session>,
    pub recent: Vec<Session>,
    /// Failed SSH attempts in the last 24 hours.
    pub failed: u32,
    /// Busiest addresses and tried user names, most first (10 each).
    pub top_addresses: Vec<Count>,
    pub top_users: Vec<Count>,
    /// Where the failures were read from: "journal", "auth.log" or "" (nothing readable).
    pub source: String,
}

pub const SCRIPT: &str = r#"export LC_ALL=C
echo "@@who"
who 2>/dev/null
echo "@@last"
last -n 40 -w 2>/dev/null | grep -v -E '^(reboot|wtmp|btmp)|^$' | head -40
echo "@@failed"
if command -v journalctl >/dev/null 2>&1 && journalctl -q -n 1 _COMM=sshd >/dev/null 2>&1; then
  echo "@@source journal"
  journalctl _COMM=sshd _COMM=sshd-session --since "-24h" --no-pager -o cat 2>/dev/null
else
  for f in /var/log/auth.log /var/log/secure; do
    if [ -r "$f" ]; then echo "@@source auth.log"; tail -n 50000 "$f" | grep sshd; break; fi
  done
fi | grep -E 'Failed password|Invalid user|authentication failure|Failed publickey|maximum authentication attempts' | tail -n 50000
true"#;

fn word_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let i = line.find(marker)? + marker.len();
    line[i..].split_whitespace().next()
}

fn looks_like_address(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':') && (s.contains('.') || s.contains(':'))
}

/// The address and user name of one failed-attempt line, when it has them.
pub fn failed_line(line: &str) -> Option<(String, Option<String>)> {
    let addr = word_after(line, " from ")
        .filter(|a| looks_like_address(a))
        .or_else(|| word_after(line, "rhost=").filter(|a| looks_like_address(a)))?;
    let user = word_after(line, "invalid user ")
        .or_else(|| word_after(line, "Invalid user "))
        .or_else(|| word_after(line, " for ").filter(|u| *u != "invalid"))
        .or_else(|| word_after(line, " user="))
        .map(|u| u.to_string());
    Some((addr.to_string(), user))
}

fn top(map: HashMap<String, u32>) -> Vec<Count> {
    let mut v: Vec<Count> = map.into_iter().map(|(key, count)| Count { key, count }).collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
    v.truncate(10);
    v
}

pub fn parse(out: &str) -> LoginsReport {
    let mut r = LoginsReport::default();
    let mut section = "";
    let mut addrs: HashMap<String, u32> = HashMap::new();
    let mut users: HashMap<String, u32> = HashMap::new();
    for line in out.lines() {
        if let Some(s) = line.strip_prefix("@@") {
            match s.split_whitespace().next() {
                Some("source") => r.source = s.trim_start_matches("source").trim().to_string(),
                Some(x) => {
                    section = match x {
                        "who" => "who",
                        "last" => "last",
                        "failed" => "failed",
                        _ => section,
                    }
                }
                None => {}
            }
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        match section {
            // alice  pts/0  2026-11-25 09:12 (203.0.113.5)
            "who" if f.len() >= 3 => {
                let from = f
                    .iter()
                    .find(|w| w.starts_with('('))
                    .map(|w| w.trim_matches(|c| c == '(' || c == ')').to_string())
                    .unwrap_or_default();
                let when = f[2..].iter().filter(|w| !w.starts_with('(')).copied().collect::<Vec<_>>().join(" ");
                r.now.push(Session {
                    user: f[0].into(),
                    tty: f[1].into(),
                    from,
                    when,
                });
            }
            // alice  pts/0  203.0.113.5  Tue Nov 25 09:12   still logged in
            "last" if f.len() >= 4 => {
                let (from, rest) = if looks_like_address(f[2]) || f[2].contains('.') {
                    (f[2].to_string(), &f[3..])
                } else {
                    (String::new(), &f[2..])
                };
                r.recent.push(Session {
                    user: f[0].into(),
                    tty: f[1].into(),
                    from,
                    when: rest.join(" "),
                });
            }
            "failed" => {
                if let Some((a, u)) = failed_line(line) {
                    r.failed += 1;
                    *addrs.entry(a).or_default() += 1;
                    if let Some(u) = u {
                        *users.entry(u).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    r.top_addresses = top(addrs);
    r.top_users = top(users);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@who\nalice    pts/0        2026-11-25 09:12 (203.0.113.5)\nroot     tty1         2026-11-24 22:01\n@@last\nalice    pts/0        203.0.113.5      Tue Nov 25 09:12   still logged in\nbob      pts/1        198.51.100.7     Mon Nov 24 18:01 - 18:40  (00:39)\n@@failed\n@@source journal\nFailed password for root from 203.0.113.99 port 51122 ssh2\nFailed password for invalid user admin from 203.0.113.99 port 51130 ssh2\nInvalid user oracle from 198.51.100.200 port 40022\nFailed password for root from 203.0.113.99 port 51140 ssh2\npam_unix(sshd:auth): authentication failure; logname= uid=0 euid=0 tty=ssh ruser= rhost=2001:db8::7  user=ubuntu\nsomething without an address\n";

    #[test]
    fn reads_who_last_and_failed_attempts() {
        let r = parse(OUT);
        assert_eq!(r.now.len(), 2);
        assert_eq!(
            r.now[0],
            Session {
                user: "alice".into(),
                tty: "pts/0".into(),
                from: "203.0.113.5".into(),
                when: "2026-11-25 09:12".into()
            }
        );
        assert_eq!(r.now[1].from, "");
        assert_eq!(r.recent.len(), 2);
        assert_eq!(r.recent[1].from, "198.51.100.7");
        assert!(r.recent[1].when.starts_with("Mon Nov 24 18:01"));
        assert_eq!(r.source, "journal");
        assert_eq!(r.failed, 5);
        assert_eq!(
            r.top_addresses[0],
            Count {
                key: "203.0.113.99".into(),
                count: 3
            }
        );
        assert!(r.top_addresses.iter().any(|c| c.key == "2001:db8::7"));
        assert_eq!(
            r.top_users[0],
            Count {
                key: "root".into(),
                count: 2
            }
        );
        assert!(
            r.top_users.iter().any(|c| c.key == "admin")
                && r.top_users.iter().any(|c| c.key == "oracle")
                && r.top_users.iter().any(|c| c.key == "ubuntu")
        );
    }

    #[test]
    fn empty_and_the_script() {
        assert_eq!(parse(""), LoginsReport::default());
        let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(o.status.success());
    }
}
