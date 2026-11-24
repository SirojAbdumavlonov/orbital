//! Search logs (Log search tab): a word or error across the systemd journal
//! or the files in /var/log changed in the time window, case-insensitive and
//! as a fixed string (never a pattern). Read-only; at most `MAX_LINES` lines,
//! newest last, with a note when there were more.

use crate::error::{Error, Result};
use crate::shell::quote;
use serde::{Deserialize, Serialize};

pub const MAX_LINES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogSource {
    Journal,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogHit {
    /// The file for /var/log hits; empty for the journal.
    pub file: String,
    pub line: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSearch {
    pub hits: Vec<LogHit>,
    /// More matched than are shown (the newest are kept).
    pub truncated: bool,
    /// Lines that matched in all.
    pub total: usize,
}

pub fn validate(term: &str, hours: u32) -> Result<(&str, u32)> {
    let t = term.trim();
    if t.chars().count() < 2 || t.chars().count() > 200 || t.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidInput("search for 2 to 200 characters on one line".into()));
    }
    if !(1..=24 * 14).contains(&hours) {
        return Err(Error::InvalidInput("search the last 1 hour to 14 days".into()));
    }
    Ok((t, hours))
}

/// The command; prints `@@total N`, then the matching lines (newest last).
pub fn command(term: &str, hours: u32, source: LogSource) -> Result<String> {
    let (t, h) = validate(term, hours)?;
    let q = quote(t);
    let keep = MAX_LINES;
    Ok(match source {
        LogSource::Journal => format!(
            "export LC_ALL=C; tmp=$(mktemp); journalctl --since '-{h}h' --no-pager -o short-iso 2>/dev/null | grep -F -i -- {q} > \"$tmp\"; echo \"@@total $(wc -l < \"$tmp\" | tr -d ' ')\"; tail -n {keep} \"$tmp\"; rm -f \"$tmp\""
        ),
        LogSource::Files => format!(
            "export LC_ALL=C; tmp=$(mktemp); find /var/log -type f \\( -name '*.log' -o -name syslog -o -name messages -o -name secure -o -name '*.err' \\) -mmin -{m} -size -500M 2>/dev/null | while IFS= read -r f; do grep -F -i -H -- {q} \"$f\" 2>/dev/null; done > \"$tmp\"; echo \"@@total $(wc -l < \"$tmp\" | tr -d ' ')\"; tail -n {keep} \"$tmp\"; rm -f \"$tmp\"",
            m = h * 60
        ),
    })
}

pub fn parse(out: &str, source: LogSource) -> LogSearch {
    let mut r = LogSearch::default();
    for line in out.lines() {
        if let Some(n) = line.strip_prefix("@@total ") {
            r.total = n.trim().parse().unwrap_or(0);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let hit = match source {
            LogSource::Journal => LogHit {
                file: String::new(),
                line: line.to_string(),
            },
            LogSource::Files => match line.split_once(':') {
                Some((f, l)) if f.starts_with("/var/log/") => LogHit {
                    file: f.to_string(),
                    line: l.to_string(),
                },
                _ => LogHit {
                    file: String::new(),
                    line: line.to_string(),
                },
            },
        };
        r.hits.push(hit);
    }
    r.truncated = r.total > r.hits.len();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_quotes_the_term() {
        assert!(
            validate("x", 24).is_err()
                && validate("ab\ncd", 24).is_err()
                && validate("error", 0).is_err()
                && validate("error", 24 * 15).is_err()
        );
        let c = command("can't connect; rm -rf /", 24, LogSource::Journal).unwrap();
        assert!(c.contains("grep -F -i -- 'can'\\''t connect; rm -rf /'"), "{c}");
        assert!(c.contains("--since '-24h'"));
        let f = command("timeout", 2, LogSource::Files).unwrap();
        assert!(f.contains("-mmin -120") && f.contains("grep -F -i -H -- timeout"));
        for c in [c, f] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
            assert!(o.status.success(), "{c}");
        }
    }

    #[test]
    fn reads_hits_and_says_when_some_were_cut() {
        let j = parse(
            "@@total 812\n2026-11-25T09:00:01+0000 web nginx[812]: upstream timed out\n",
            LogSource::Journal,
        );
        assert_eq!(j.total, 812);
        assert!(j.truncated && j.hits.len() == 1 && j.hits[0].file.is_empty());
        let f = parse(
            "@@total 2\n/var/log/nginx/error.log:2026/11/25 upstream timed out: x\n/var/log/app.log:timed out\n",
            LogSource::Files,
        );
        assert!(!f.truncated);
        assert_eq!(
            f.hits[0],
            LogHit {
                file: "/var/log/nginx/error.log".into(),
                line: "2026/11/25 upstream timed out: x".into()
            }
        );
    }
}
