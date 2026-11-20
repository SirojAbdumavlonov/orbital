//! Network check: is something reachable *from the server*? Ping, DNS
//! lookup, a TCP port, an HTTP(S) request and a traceroute, each a fixed
//! command with the target validated and quoted. Read-only.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NetCheck {
    Ping,
    Dns,
    Port,
    Http,
    Trace,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetResult {
    pub ok: bool,
    /// One line for the result list ("reachable, 12.3 ms", "HTTP 200 in 0.21 s", …).
    pub summary: String,
    /// The command's own output.
    pub output: String,
    pub command: String,
}

/// A host name or an IPv4/IPv6 address; never starting with "-".
pub fn validate_host(h: &str) -> Result<&str> {
    let h = h.trim();
    let ok = !h.is_empty()
        && h.len() <= 253
        && !h.starts_with('-')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'));
    if ok {
        Ok(h)
    } else {
        Err(Error::InvalidInput(format!("not a host name or address: {h:?}")))
    }
}

fn validate_url(u: &str) -> Result<&str> {
    let u = u.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    let ok = rest.is_some_and(|r| !r.is_empty() && !r.starts_with('-'))
        && u.len() <= 2000
        && !u.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(u)
    } else {
        Err(Error::InvalidInput("give a full address starting with http:// or https://".into()))
    }
}

/// The command for one check. `target` is a host (or a URL for Http); `port` is for Port.
pub fn command(check: NetCheck, target: &str, port: Option<u16>) -> Result<String> {
    Ok(match check {
        NetCheck::Ping => format!("ping -c 4 -- {} 2>&1", quote(validate_host(target)?)),
        NetCheck::Dns => {
            let h = quote(validate_host(target)?);
            format!(
                "if command -v getent >/dev/null 2>&1; then getent ahosts {h} | awk '{{print $1}}' | sort -u; \
                 elif command -v host >/dev/null 2>&1; then host {h}; else nslookup {h}; fi 2>&1"
            )
        }
        NetCheck::Port => {
            let h = quote(validate_host(target)?);
            let p = port
                .filter(|p| *p > 0)
                .ok_or_else(|| Error::InvalidInput("give a port between 1 and 65535".into()))?;
            format!(
                "if command -v nc >/dev/null 2>&1; then nc -z -w 5 {h} {p} 2>&1 && echo @@open || echo @@closed; \
                 elif command -v bash >/dev/null 2>&1; then timeout 5 bash -c \"</dev/tcp/\"{h}\"/{p}\" 2>&1 && echo @@open || echo @@closed; \
                 else echo 'neither nc nor bash is installed'; echo @@closed; fi"
            )
        }
        NetCheck::Http => format!(
            "curl -sS -o /dev/null --max-time 15 -w '@@http %{{http_code}} %{{time_total}} %{{remote_ip}}\\n' -- {} 2>&1",
            quote(validate_url(target)?)
        ),
        NetCheck::Trace => {
            let h = quote(validate_host(target)?);
            format!(
                "if command -v traceroute >/dev/null 2>&1; then traceroute -n -q 1 -w 2 -m 20 {h}; \
                 elif command -v tracepath >/dev/null 2>&1; then tracepath -n -m 20 {h}; \
                 else echo 'traceroute is not installed (apt install traceroute)'; fi 2>&1"
            )
        }
    })
}

/// Turn a check's output into a one-line verdict.
pub fn summarize(check: NetCheck, out: &str, code: i32) -> (bool, String) {
    match check {
        NetCheck::Ping => {
            let loss = out
                .lines()
                .find(|l| l.contains("packet loss"))
                .and_then(|l| l.split(',').find(|p| p.contains("packet loss")))
                .map(|p| p.trim().to_string());
            let avg = out
                .lines()
                .find(|l| l.contains("min/avg/max"))
                .and_then(|l| l.split('=').nth(1))
                .and_then(|v| v.trim().split('/').nth(1))
                .map(|a| format!("{a} ms average"));
            let ok = code == 0;
            let what = [Some(if ok { "reachable".to_string() } else { "no answer".to_string() }), avg, loss];
            (ok, what.into_iter().flatten().collect::<Vec<_>>().join(", "))
        }
        NetCheck::Dns => {
            let addrs: Vec<&str> = out
                .lines()
                .map(str::trim)
                .filter(|l| {
                    !l.is_empty() && (l.parse::<std::net::IpAddr>().is_ok() || l.contains(" has address") || l.contains(" has IPv6"))
                })
                .collect();
            if addrs.is_empty() {
                (false, "the name didn't resolve".into())
            } else {
                let shown: Vec<&str> = addrs.iter().map(|l| l.rsplit(' ').next().unwrap_or(l)).take(4).collect();
                (true, format!("resolves to {}", shown.join(", ")))
            }
        }
        NetCheck::Port => {
            if out.contains("@@open") {
                (true, "the port is open".into())
            } else {
                (false, "closed, filtered or nothing listening".into())
            }
        }
        NetCheck::Http => match out.lines().find_map(|l| l.strip_prefix("@@http ")) {
            Some(rest) => {
                let mut it = rest.split_whitespace();
                let status = it.next().unwrap_or("000");
                let secs = it.next().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0);
                let ip = it.next().unwrap_or("");
                if status == "000" {
                    let why = out.lines().find(|l| l.starts_with("curl:")).unwrap_or("no answer").trim();
                    (false, why.to_string())
                } else {
                    let ok = status.starts_with('2') || status.starts_with('3');
                    (
                        ok,
                        format!(
                            "HTTP {status} in {secs:.2} s{}",
                            if ip.is_empty() { String::new() } else { format!(" from {ip}") }
                        ),
                    )
                }
            }
            None => (false, out.lines().last().unwrap_or("curl didn't run").trim().to_string()),
        },
        NetCheck::Trace => {
            let hops = out
                .lines()
                .filter(|l| l.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit()))
                .count();
            (
                code == 0 && hops > 0,
                if hops > 0 {
                    format!("{hops} hops")
                } else {
                    "no route shown".into()
                },
            )
        }
    }
}

impl SshSession {
    pub async fn net_check(&self, check: NetCheck, target: &str, port: Option<u16>) -> Result<NetResult> {
        let cmd = command(check, target, port)?;
        let o = self.exec(&cmd).await?;
        let output = format!("{}{}", o.stdout, o.stderr);
        let (ok, summary) = summarize(check, &output, o.exit_code);
        let output = output.replace("@@open\n", "").replace("@@closed\n", "");
        Ok(NetResult {
            ok,
            summary,
            output,
            command: cmd,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_quote_and_refuse_bad_targets() {
        assert_eq!(
            command(NetCheck::Ping, "db.internal", None).unwrap(),
            "ping -c 4 -- db.internal 2>&1"
        );
        assert!(command(NetCheck::Port, "10.0.0.5", Some(5432))
            .unwrap()
            .contains("nc -z -w 5 10.0.0.5 5432"));
        assert!(command(NetCheck::Http, "https://example.com/a?b=c&d", None)
            .unwrap()
            .ends_with("-- 'https://example.com/a?b=c&d' 2>&1"));
        assert!(command(NetCheck::Dns, "::1", None).unwrap().contains("getent ahosts ::1"));
        for bad in ["", "-oProxyCommand=x", "a b", "x;reboot", "$(id)", "a'b"] {
            assert!(command(NetCheck::Ping, bad, None).is_err(), "{bad}");
        }
        assert!(command(NetCheck::Port, "db", None).is_err());
        assert!(command(NetCheck::Port, "db", Some(0)).is_err());
        for bad in ["example.com", "ftp://x", "https://", "https://-x", "https://a b"] {
            assert!(command(NetCheck::Http, bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn summaries() {
        let ping = "4 packets transmitted, 4 received, 0% packet loss, time 3004ms\nrtt min/avg/max/mdev = 0.031/0.045/0.061/0.010 ms\n";
        assert_eq!(
            summarize(NetCheck::Ping, ping, 0),
            (true, "reachable, 0.045 ms average, 0% packet loss".into())
        );
        assert!(!summarize(NetCheck::Ping, "4 packets transmitted, 0 received, 100% packet loss\n", 1).0);
        assert_eq!(
            summarize(NetCheck::Dns, "93.184.215.14\n2606:2800:21f:cb07::1\n", 0),
            (true, "resolves to 93.184.215.14, 2606:2800:21f:cb07::1".into())
        );
        assert_eq!(
            summarize(NetCheck::Dns, "example.com has address 93.184.215.14\n", 0).1,
            "resolves to 93.184.215.14"
        );
        assert!(!summarize(NetCheck::Dns, "", 2).0);
        assert!(summarize(NetCheck::Port, "@@open\n", 0).0);
        assert!(!summarize(NetCheck::Port, "@@closed\n", 0).0);
        assert_eq!(
            summarize(NetCheck::Http, "@@http 200 0.213 93.184.215.14\n", 0),
            (true, "HTTP 200 in 0.21 s from 93.184.215.14".into())
        );
        assert_eq!(
            summarize(NetCheck::Http, "curl: (6) Could not resolve host: nope\n@@http 000 0.0 \n", 6),
            (false, "curl: (6) Could not resolve host: nope".into())
        );
        assert!(!summarize(NetCheck::Http, "@@http 502 0.1 1.2.3.4\n", 0).0);
        assert_eq!(
            summarize(NetCheck::Trace, "traceroute to x\n 1  10.0.0.1  0.3 ms\n 2  1.1.1.1  4 ms\n", 0).1,
            "2 hops"
        );
    }
}
