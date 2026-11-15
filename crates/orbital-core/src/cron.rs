//! Scheduled jobs: every cron job on the server (users' crontabs, /etc/crontab,
//! /etc/cron.d), what each schedule means in words, when it runs next and
//! when it last ran; plus systemd timers and the cron.daily/weekly scripts.
//!
//! Users' crontabs are changed through `crontab -u USER -`, which checks the
//! syntax and installs nothing when it is wrong. A change names the exact line
//! it replaces, so a crontab edited in the meantime is never overwritten
//! blindly. System files (/etc/crontab, /etc/cron.d) are edited in the editor
//! with safe save, like any other config.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDateTime, Timelike};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lines Orbital pauses are kept in the crontab behind this prefix.
pub const PAUSED: &str = "#orbital-paused# ";

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    /// `user` (a user's crontab) or `system` (/etc/crontab, /etc/cron.d).
    pub source: String,
    pub user: String,
    /// The file, for system jobs.
    pub file: Option<String>,
    pub line: usize,
    /// Five fields, or `@daily`-style shorthand.
    pub schedule: String,
    pub command: String,
    /// The comment line just above the job, if any.
    pub note: Option<String>,
    pub paused: bool,
    pub description: String,
    /// Next runs in the server's time (RFC 3339), up to three.
    pub next_runs: Vec<String>,
    pub last_run: Option<String>,
    /// The line as it is in the crontab; changes name it to find the job.
    pub raw: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemdTimer {
    pub unit: String,
    pub activates: String,
    pub calendar: Option<String>,
    pub next: Option<String>,
    pub last: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicScript {
    /// `hourly`, `daily`, `weekly` or `monthly`.
    pub period: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CronReport {
    pub jobs: Vec<CronJob>,
    pub timers: Vec<SystemdTimer>,
    pub periodic: Vec<PeriodicScript>,
    /// The server's clock when this was read (RFC 3339 with its offset).
    pub server_now: Option<String>,
    pub timezone: Option<String>,
    /// Every user's crontab could be read (needs root).
    pub complete: bool,
}

pub(crate) const SCRIPT: &str = r#"export LC_ALL=C
echo "@@now $(date +%Y-%m-%dT%H:%M:%S%z)"
echo "@@tz $(timedatectl show -p Timezone --value 2>/dev/null || cat /etc/timezone 2>/dev/null)"
if [ "$(id -u)" = 0 ]; then
  echo @@root
  for u in $(cut -d: -f1 /etc/passwd); do
    c=$(crontab -l -u "$u" 2>/dev/null) || continue
    echo "@@user $u"; printf '%s\n' "$c"
  done
else
  echo "@@user $(id -un)"; crontab -l 2>/dev/null
fi
for f in /etc/crontab /etc/cron.d/*; do
  [ -f "$f" ] && [ -r "$f" ] || continue
  echo "@@file $f"; cat "$f"
done
for p in hourly daily weekly monthly; do
  for f in /etc/cron.$p/*; do [ -f "$f" ] && [ -x "$f" ] && echo "@@periodic $p $f"; done
done
if command -v systemctl >/dev/null 2>&1; then
  for t in $(systemctl list-timers --all --no-legend --plain 2>/dev/null | awk '{print $(NF-1)}'); do
    case "$t" in *.timer) ;; *) continue;; esac
    printf '@@timer %s|' "$t"
    systemctl show -p Unit -p NextElapseUSecRealtime -p LastTriggerUSec -p Description -p TimersCalendar "$t" 2>/dev/null | tr '\n' '|'
    echo
  done
fi
if command -v journalctl >/dev/null 2>&1; then
  echo @@log
  journalctl -u cron -u crond --since -3d -o short-iso --no-pager 2>/dev/null | grep -F ' CMD (' | tail -n 2000
fi
"#;

// ---------- schedules ----------

const SHORTHANDS: &[(&str, &str)] = &[
    ("@yearly", "0 0 1 1 *"),
    ("@annually", "0 0 1 1 *"),
    ("@monthly", "0 0 1 * *"),
    ("@weekly", "0 0 * * 0"),
    ("@daily", "0 0 * * *"),
    ("@midnight", "0 0 * * *"),
    ("@hourly", "0 * * * *"),
];
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_NAMES: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// A parsed five-field schedule: which values each field allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    minutes: Vec<bool>,
    hours: Vec<bool>,
    doms: Vec<bool>,
    months: Vec<bool>,
    dows: Vec<bool>,
    dom_any: bool,
    dow_any: bool,
}

fn parse_field(f: &str, min: u32, max: u32, names: &[&str]) -> Result<Vec<bool>> {
    let bad = || Error::InvalidInput(format!("can't read schedule field {f:?}"));
    let mut set = vec![false; max as usize + 1];
    let value = |s: &str| -> Result<u32> {
        let lower = s.to_ascii_lowercase();
        if let Some(i) = names.iter().position(|n| *n == lower) {
            return Ok(i as u32 + min);
        }
        s.parse::<u32>().map_err(|_| bad())
    };
    for part in f.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(bad)?),
            None => (part, 1),
        };
        let (lo, hi) = match range {
            "*" => (min, max),
            r => match r.split_once('-') {
                Some((a, b)) => (value(a)?, value(b)?),
                None => {
                    let v = value(r)?;
                    (v, if part.contains('/') { max } else { v })
                }
            },
        };
        // Sunday may be written 7.
        let (lo, hi) = if max == 6 { (lo.min(7), hi.min(7)) } else { (lo, hi) };
        if lo < min || hi > max.max(if max == 6 { 7 } else { max }) || lo > hi {
            return Err(bad());
        }
        let mut v = lo;
        while v <= hi {
            set[(v % if max == 6 { 7 } else { u32::MAX }) as usize] = true;
            v += step;
        }
    }
    Ok(set)
}

/// The five fields a schedule stands for (`@daily` → `0 0 * * *`). `None` for `@reboot`.
pub fn expand(schedule: &str) -> Option<String> {
    let s = schedule.trim();
    if s == "@reboot" {
        return None;
    }
    Some(
        SHORTHANDS
            .iter()
            .find(|(k, _)| *k == s)
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| s.to_string()),
    )
}

pub fn parse_schedule(schedule: &str) -> Result<Option<Schedule>> {
    let Some(fields) = expand(schedule) else { return Ok(None) };
    let f: Vec<&str> = fields.split_whitespace().collect();
    if f.len() != 5 {
        return Err(Error::InvalidInput(format!(
            "a schedule has five fields (minute hour day month weekday) or is like @daily; got {schedule:?}"
        )));
    }
    Ok(Some(Schedule {
        minutes: parse_field(f[0], 0, 59, &[])?,
        hours: parse_field(f[1], 0, 23, &[])?,
        doms: parse_field(f[2], 1, 31, &[])?,
        months: parse_field(f[3], 1, 12, &MONTHS)?,
        dows: parse_field(f[4], 0, 6, &DAYS)?,
        dom_any: f[2] == "*",
        dow_any: f[4] == "*",
    }))
}

impl Schedule {
    fn day_matches(&self, t: &NaiveDateTime) -> bool {
        let dom = self.doms[t.day() as usize];
        let dow = self.dows[t.weekday().num_days_from_sunday() as usize];
        // Cron's rule: if both day fields are restricted, either one matching is enough.
        match (self.dom_any, self.dow_any) {
            (true, true) => true,
            (false, true) => dom,
            (true, false) => dow,
            (false, false) => dom || dow,
        }
    }

    /// The next `n` times after `now` (in the server's local time).
    pub fn next_runs(&self, now: NaiveDateTime, n: usize) -> Vec<NaiveDateTime> {
        let mut out = Vec::new();
        let mut t = now.with_second(0).unwrap_or(now) + Duration::minutes(1);
        let end = now + Duration::days(366 * 4);
        while out.len() < n && t < end {
            if !self.months[t.month() as usize] || !self.day_matches(&t) {
                // Skip to the next day.
                t = (t.date() + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap_or(t + Duration::days(1));
                continue;
            }
            if !self.hours[t.hour() as usize] {
                t = t.with_minute(0).unwrap_or(t) + Duration::hours(1);
                continue;
            }
            if self.minutes[t.minute() as usize] {
                out.push(t);
            }
            t += Duration::minutes(1);
        }
        out
    }
}

fn list(v: &[u32]) -> String {
    match v {
        [] => String::new(),
        [a] => a.to_string(),
        _ => {
            let (last, rest) = v.split_last().unwrap();
            format!("{} and {last}", rest.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "))
        }
    }
}

fn plain_number(f: &str) -> Option<u32> {
    f.parse().ok()
}

fn numbers(f: &str) -> Option<Vec<u32>> {
    f.split(',').map(|x| x.parse().ok()).collect()
}

/// A schedule in words, for the common shapes; anything else is described as custom.
pub fn describe(schedule: &str) -> String {
    let s = schedule.trim();
    match s {
        "@reboot" => return "When the server starts".into(),
        "@yearly" | "@annually" => return "Every year on 1 January at 00:00".into(),
        "@monthly" => return "Every month on the 1st at 00:00".into(),
        "@weekly" => return "Every Sunday at 00:00".into(),
        "@daily" | "@midnight" => return "Every day at 00:00".into(),
        "@hourly" => return "Every hour, on the hour".into(),
        _ => {}
    }
    let f: Vec<&str> = s.split_whitespace().collect();
    if f.len() != 5 {
        return format!("Custom: {s}");
    }
    let (mi, h, dom, mon, dow) = (f[0], f[1], f[2], f[3], f[4]);
    let times = || -> Option<String> {
        let m = plain_number(mi)?;
        let hs = numbers(h)?;
        Some(hs.iter().map(|h| format!("{h:02}:{m:02}")).collect::<Vec<_>>().join(", "))
    };
    if mon != "*" {
        return format!("Custom: {s}");
    }
    let days = || -> Option<String> {
        let mut names = Vec::new();
        for part in dow.split(',') {
            if let Some((a, b)) = part.split_once('-') {
                let (a, b) = (day_index(a)?, day_index(b)?);
                if (a, b) == (1, 5) {
                    names.push("weekday".to_string());
                    continue;
                }
                names.push(format!("{} to {}", DAY_NAMES[a % 7], DAY_NAMES[b % 7]));
            } else {
                names.push(DAY_NAMES[day_index(part)? % 7].to_string());
            }
        }
        Some(names.join(", "))
    };
    match (mi, h, dom, dow) {
        ("*", "*", "*", "*") => "Every minute".into(),
        (m, "*", "*", "*") if m.starts_with("*/") => format!("Every {} minutes", &m[2..]),
        (m, "*", "*", "*") if plain_number(m).is_some() => format!("Every hour at :{:02}", plain_number(m).unwrap()),
        ("0", hh, "*", "*") if hh.starts_with("*/") => format!("Every {} hours, on the hour", &hh[2..]),
        (_, _, "*", "*") if times().is_some() => format!("Every day at {}", times().unwrap()),
        (_, _, "*", _) if times().is_some() && days().is_some() => {
            let d = days().unwrap();
            if d == "weekday" {
                format!("Every weekday at {}", times().unwrap())
            } else {
                format!("Every {d} at {}", times().unwrap())
            }
        }
        (_, _, d, "*") if times().is_some() && numbers(d).is_some() => {
            let ds = numbers(d).unwrap();
            format!("Every month on day {} at {}", list(&ds), times().unwrap())
        }
        _ => format!("Custom: {s}"),
    }
}

fn day_index(s: &str) -> Option<usize> {
    let l = s.to_ascii_lowercase();
    DAYS.iter()
        .position(|d| *d == l)
        .or_else(|| l.parse::<usize>().ok().filter(|n| *n <= 7))
}

static FIELD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9A-Za-z*/,\-]+$").unwrap());

/// Check a schedule the user typed: five sensible fields or a known shorthand.
pub fn validate_schedule(s: &str) -> Result<String> {
    let s = s.trim();
    if s == "@reboot" || SHORTHANDS.iter().any(|(k, _)| *k == s) {
        return Ok(s.to_string());
    }
    let f: Vec<&str> = s.split_whitespace().collect();
    if f.len() != 5 || f.iter().any(|x| !FIELD_RE.is_match(x)) {
        return Err(Error::InvalidInput(
            "a schedule has five fields: minute hour day-of-month month day-of-week (for example 0 3 * * *)".into(),
        ));
    }
    parse_schedule(s)?;
    Ok(f.join(" "))
}

fn validate_line(what: &str, s: &str, max: usize) -> Result<String> {
    let s = s.trim();
    if s.is_empty() && what == "command" {
        return Err(Error::InvalidInput("the command is empty".into()));
    }
    if s.chars().count() > max || s.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
        return Err(Error::InvalidInput(format!(
            "the {what} must be one line of at most {max} characters"
        )));
    }
    Ok(s.to_string())
}

/// The next runs, as RFC 3339 times in the server's offset.
pub fn next_runs(schedule: &str, now: &DateTime<FixedOffset>, n: usize) -> Vec<String> {
    match parse_schedule(schedule) {
        Ok(Some(s)) => s
            .next_runs(now.naive_local(), n)
            .into_iter()
            .filter_map(|t| t.and_local_timezone(*now.offset()).single())
            .map(|t| t.to_rfc3339())
            .collect(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CronPreview {
    pub description: String,
    pub next_runs: Vec<String>,
    /// Why the schedule isn't valid, if it isn't.
    pub error: Option<String>,
}

/// What a schedule means and when it would run, for the job form.
pub fn preview(schedule: &str, server_now: &str) -> Result<CronPreview> {
    let now = DateTime::parse_from_rfc3339(server_now).map_err(|e| Error::InvalidInput(format!("server time: {e}")))?;
    let error = validate_schedule(schedule).err().map(|e| match e {
        Error::InvalidInput(m) => m,
        other => other.to_string(),
    });
    Ok(CronPreview {
        description: if error.is_some() { String::new() } else { describe(schedule) },
        next_runs: if error.is_some() {
            Vec::new()
        } else {
            next_runs(schedule, &now, 3)
        },
        error,
    })
}

// ---------- reading ----------

/// Split a crontab line into schedule and the rest. System files have a user field.
fn split_job(line: &str, system: bool) -> Option<(String, Option<String>, String)> {
    let line = line.trim();
    let (schedule, rest) = if line.starts_with('@') {
        let (s, r) = line.split_once(char::is_whitespace)?;
        (s.to_string(), r.trim_start())
    } else {
        let mut it = line.splitn(6, char::is_whitespace).filter(|x| !x.is_empty());
        let fields: Vec<&str> = (0..5).map(|_| it.next()).collect::<Option<_>>()?;
        if fields.iter().any(|f| !FIELD_RE.is_match(f)) {
            return None;
        }
        // splitn on single spaces may leave runs of spaces; rebuild the remainder.
        let consumed = fields
            .iter()
            .fold(0usize, |pos, f| line[pos..].find(f).map(|i| pos + i + f.len()).unwrap_or(pos));
        (fields.join(" "), line[consumed..].trim_start())
    };
    if system {
        let (user, cmd) = rest.split_once(char::is_whitespace)?;
        Some((schedule, Some(user.to_string()), cmd.trim().to_string()))
    } else {
        Some((schedule, None, rest.trim().to_string()))
    }
}

static ENV_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[A-Za-z_][A-Za-z0-9_]*\s*="#).unwrap());
static LOG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(\S+)\s+\S+\s+\S+\[\d+\]:\s+\(([^)]+)\)\s+CMD\s+\((.*)\)\s*$").unwrap());

pub fn parse(out: &str) -> CronReport {
    let mut r = CronReport::default();
    let mut section: (&str, String) = ("", String::new());
    let mut line_no = 0usize;
    let mut note: Option<String> = None;
    let mut last: HashMap<(String, String), String> = HashMap::new();
    for raw in out.lines() {
        if let Some(v) = raw.strip_prefix("@@now ") {
            r.server_now = DateTime::parse_from_str(v.trim(), "%Y-%m-%dT%H:%M:%S%z")
                .ok()
                .map(|d| d.to_rfc3339());
            continue;
        }
        if let Some(v) = raw.strip_prefix("@@tz") {
            r.timezone = Some(v.trim().to_string()).filter(|v| !v.is_empty());
            continue;
        }
        if raw == "@@root" {
            r.complete = true;
            continue;
        }
        if let Some(u) = raw.strip_prefix("@@user ") {
            section = ("user", u.trim().to_string());
            line_no = 0;
            note = None;
            continue;
        }
        if let Some(f) = raw.strip_prefix("@@file ") {
            section = ("file", f.trim().to_string());
            line_no = 0;
            note = None;
            continue;
        }
        if let Some(p) = raw.strip_prefix("@@periodic ") {
            if let Some((period, path)) = p.split_once(' ') {
                r.periodic.push(PeriodicScript {
                    period: period.into(),
                    path: path.trim().into(),
                });
            }
            continue;
        }
        if let Some(t) = raw.strip_prefix("@@timer ") {
            let mut parts = t.split('|');
            let unit = parts.next().unwrap_or("").to_string();
            let mut timer = SystemdTimer {
                unit,
                ..Default::default()
            };
            let some = |v: &str| (!v.is_empty() && v != "n/a" && v != "0").then(|| v.to_string());
            for p in parts {
                if let Some((k, v)) = p.split_once('=') {
                    match k {
                        "Unit" => timer.activates = v.to_string(),
                        "NextElapseUSecRealtime" => timer.next = some(v),
                        "LastTriggerUSec" => timer.last = some(v),
                        "Description" => timer.description = some(v),
                        // "{ OnCalendar=*-*-* 06,18:00:00 ; next_elapse=… }"
                        "TimersCalendar" => {
                            timer.calendar = p
                                .split_once("OnCalendar=")
                                .and_then(|(_, c)| c.split(" ;").next())
                                .map(|c| c.trim().to_string())
                        }
                        _ => {}
                    }
                }
            }
            if !timer.unit.is_empty() {
                r.timers.push(timer);
            }
            continue;
        }
        if raw == "@@log" {
            section = ("log", String::new());
            continue;
        }
        match section.0 {
            "user" | "file" => {
                line_no += 1;
                let system = section.0 == "file";
                let mut text = raw.trim();
                let paused = text.starts_with(PAUSED.trim_end());
                if paused {
                    text = text[PAUSED.trim_end().len()..].trim_start();
                } else if text.starts_with('#') {
                    let n = text.trim_start_matches('#').trim();
                    note = (!n.is_empty()).then(|| n.to_string());
                    continue;
                }
                if text.is_empty() || ENV_RE.is_match(text) {
                    note = None;
                    continue;
                }
                let Some((schedule, user, command)) = split_job(text, system) else {
                    note = None;
                    continue;
                };
                r.jobs.push(CronJob {
                    source: if system { "system" } else { "user" }.into(),
                    user: user.unwrap_or_else(|| section.1.clone()),
                    file: system.then(|| section.1.clone()),
                    line: line_no,
                    description: describe(&schedule),
                    schedule,
                    command,
                    note: note.take(),
                    paused,
                    raw: raw.trim_end().to_string(),
                    ..Default::default()
                });
            }
            "log" => {
                if let Some(c) = LOG_RE.captures(raw.trim()) {
                    last.insert((c[2].to_string(), c[3].trim().to_string()), c[1].to_string());
                }
            }
            _ => {}
        }
    }
    let now = r.server_now.as_deref().and_then(|n| DateTime::parse_from_rfc3339(n).ok());
    for j in &mut r.jobs {
        if let (Some(now), false) = (now, j.paused) {
            j.next_runs = next_runs(&j.schedule, &now, 3);
        }
        j.last_run = last.get(&(j.user.clone(), j.command.clone())).cloned();
    }
    r
}

// ---------- changing ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum CronChange {
    Add {
        schedule: String,
        command: String,
        note: Option<String>,
    },
    /// Replace the job whose line is `raw`.
    Update {
        raw: String,
        schedule: String,
        command: String,
        note: Option<String>,
    },
    Delete {
        raw: String,
    },
    Pause {
        raw: String,
    },
    Resume {
        raw: String,
    },
}

impl CronChange {
    pub fn verb(&self) -> &'static str {
        match self {
            CronChange::Add { .. } => "add",
            CronChange::Update { .. } => "update",
            CronChange::Delete { .. } => "delete",
            CronChange::Pause { .. } => "pause",
            CronChange::Resume { .. } => "resume",
        }
    }
}

fn job_lines(schedule: &str, command: &str, note: &Option<String>) -> Result<Vec<String>> {
    let schedule = validate_schedule(schedule)?;
    let command = validate_line("command", command, 2000)?;
    let mut out = Vec::new();
    if let Some(n) = note
        .as_deref()
        .map(|n| validate_line("note", n, 200))
        .transpose()?
        .filter(|n| !n.is_empty())
    {
        out.push(format!("# {n}"));
    }
    out.push(format!("{schedule} {command}"));
    Ok(out)
}

/// The crontab after `change`. Fails if the line to change isn't there (any more).
pub fn apply(current: &str, change: &CronChange) -> Result<String> {
    let mut lines: Vec<String> = current.lines().map(String::from).collect();
    let find = |lines: &[String], raw: &str| -> Result<usize> {
        lines
            .iter()
            .position(|l| l.trim_end() == raw.trim_end())
            .ok_or_else(|| Error::InvalidInput("the crontab changed since it was loaded; refresh and try again".into()))
    };
    match change {
        CronChange::Add { schedule, command, note } => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.extend(job_lines(schedule, command, note)?);
        }
        CronChange::Update {
            raw,
            schedule,
            command,
            note,
        } => {
            let i = find(&lines, raw)?;
            let paused = lines[i].trim_start().starts_with(PAUSED.trim_end());
            let mut new = job_lines(schedule, command, note)?;
            if paused {
                let last = new.len() - 1;
                new[last] = format!("{PAUSED}{}", new[last]);
            }
            // Replace the note above it too, when the job had one.
            let start = if i > 0 && lines[i - 1].trim_start().starts_with('#') && !lines[i - 1].contains(PAUSED.trim_end()) {
                i - 1
            } else {
                i
            };
            lines.splice(start..=i, new);
        }
        CronChange::Delete { raw } => {
            let i = find(&lines, raw)?;
            lines.remove(i);
        }
        CronChange::Pause { raw } => {
            let i = find(&lines, raw)?;
            if !lines[i].starts_with(PAUSED.trim_end()) {
                lines[i] = format!("{PAUSED}{}", lines[i]);
            }
        }
        CronChange::Resume { raw } => {
            let i = find(&lines, raw)?;
            lines[i] = lines[i]
                .trim_start_matches(PAUSED)
                .trim_start_matches(PAUSED.trim_end())
                .trim_start()
                .to_string();
        }
    }
    let mut s = lines.join("\n");
    s.push('\n');
    Ok(s)
}

/// What `crontab -l` prints for a user, read as root (or as that user when it is the login user).
pub fn read_command(user: &str) -> Result<String> {
    Ok(format!(
        "crontab -l -u {} 2>/dev/null || true",
        quote(crate::users::validate_user(user)?)
    ))
}

pub fn write_command(user: &str) -> Result<String> {
    Ok(format!("crontab -u {} -", quote(crate::users::validate_user(user)?)))
}

/// The part of a cron command the shell runs: cron turns an unescaped `%` into
/// a newline and feeds what follows as stdin, which "Run now" leaves out.
pub fn runnable_command(cmd: &str) -> String {
    let mut out = String::new();
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'%') => {
                out.push('%');
                chars.next();
            }
            '%' => break,
            c => out.push(c),
        }
    }
    out
}

/// Run a job's command now, as its user, the way cron would (with /bin/sh).
pub fn run_now_command(user: &str, command: &str) -> Result<String> {
    let user = crate::users::validate_user(user)?;
    let cmd = runnable_command(command);
    Ok(format!(
        "cd ~{u} 2>/dev/null || cd /; if [ \"$(id -un)\" = {q} ]; then sh -c {c}; else runuser -u {q} -- sh -c {c} 2>&1 || sudo -n -u {q} -- sh -c {c}; fi 2>&1",
        u = user,
        q = quote(user),
        c = quote(&cmd)
    ))
}

impl SshSession {
    pub async fn cron_report(&self) -> Result<CronReport> {
        let (o, _) = self.exec_inspect(SCRIPT).await?;
        Ok(parse(&o.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn describes_common_schedules() {
        assert_eq!(describe("* * * * *"), "Every minute");
        assert_eq!(describe("*/5 * * * *"), "Every 5 minutes");
        assert_eq!(describe("15 * * * *"), "Every hour at :15");
        assert_eq!(describe("0 */6 * * *"), "Every 6 hours, on the hour");
        assert_eq!(describe("0 3 * * *"), "Every day at 03:00");
        assert_eq!(describe("30 6,18 * * *"), "Every day at 06:30, 18:30");
        assert_eq!(describe("0 9 * * 1-5"), "Every weekday at 09:00");
        assert_eq!(describe("0 2 * * sun"), "Every Sunday at 02:00");
        assert_eq!(describe("0 2 * * 1,3"), "Every Monday, Wednesday at 02:00");
        assert_eq!(describe("0 4 1,15 * *"), "Every month on day 1 and 15 at 04:00");
        assert_eq!(describe("@reboot"), "When the server starts");
        assert_eq!(describe("@daily"), "Every day at 00:00");
        assert_eq!(describe("0 0 1 1 *"), "Custom: 0 0 1 1 *");
    }

    #[test]
    fn computes_next_runs() {
        let s = parse_schedule("0 3 * * *").unwrap().unwrap();
        assert_eq!(
            s.next_runs(at("2026-11-15 12:00"), 2),
            [at("2026-11-16 03:00"), at("2026-11-17 03:00")]
        );
        let s = parse_schedule("*/15 * * * *").unwrap().unwrap();
        assert_eq!(
            s.next_runs(at("2026-11-15 12:07"), 2),
            [at("2026-11-15 12:15"), at("2026-11-15 12:30")]
        );
        // Weekdays only: Friday evening → Monday.
        let s = parse_schedule("0 9 * * 1-5").unwrap().unwrap();
        assert_eq!(s.next_runs(at("2026-11-13 18:00"), 1), [at("2026-11-16 09:00")]);
        // Sunday as 7, and month names.
        let s = parse_schedule("0 0 * jan 7").unwrap().unwrap();
        assert_eq!(s.next_runs(at("2026-11-15 12:00"), 1), [at("2027-01-03 00:00")]);
        // Both day fields restricted: either matches (the 1st, or any Monday).
        let s = parse_schedule("0 0 1 * 1").unwrap().unwrap();
        assert_eq!(
            s.next_runs(at("2026-11-15 12:00"), 2),
            [at("2026-11-16 00:00"), at("2026-11-23 00:00")]
        );
        assert_eq!(parse_schedule("@reboot").unwrap(), None);
        for bad in ["61 * * * *", "* * * *", "a b c d e", "*/0 * * * *", "5-1 * * * *"] {
            assert!(parse_schedule(bad).is_err(), "{bad}");
        }
        let now = DateTime::parse_from_rfc3339("2026-11-15T12:00:00+05:00").unwrap();
        assert_eq!(next_runs("0 3 * * *", &now, 1), ["2026-11-16T03:00:00+05:00"]);
    }

    const OUT: &str = "@@now 2026-11-15T12:00:00+0500
@@tz Asia/Tashkent
@@root
@@user deploy
MAILTO=ops@example.com
# Nightly database backup
0 3 * * * /usr/local/bin/backup.sh >> /var/log/backup.log 2>&1
#orbital-paused# */5 * * * * curl -fsS https://hc.example.com/ping
@reboot /home/deploy/start-worker.sh
@@user root
@daily /usr/sbin/logrotate /etc/logrotate.conf
@@file /etc/cron.d/certbot
SHELL=/bin/sh
0 */12 * * * root test -x /usr/bin/certbot && certbot -q renew
@@periodic daily /etc/cron.daily/apt-compat
@@periodic weekly /etc/cron.weekly/man-db
@@timer apt-daily.timer|Unit=apt-daily.service|NextElapseUSecRealtime=Sun 2026-11-15 18:22:00 +05|LastTriggerUSec=Sun 2026-11-15 06:10:00 +05|Description=Daily apt download activities|TimersCalendar={ OnCalendar=*-*-* 06,18:00:00 ; next_elapse=Sun 2026-11-15 18:00:00 +05 }|
@@log
2026-11-15T03:00:01+0500 web CRON[812]: (deploy) CMD (/usr/local/bin/backup.sh >> /var/log/backup.log 2>&1)
2026-11-15T00:00:01+0500 web CRON[700]: (root) CMD (test -x /usr/bin/certbot && certbot -q renew)
";

    #[test]
    fn parses_every_source() {
        let r = parse(OUT);
        assert!(r.complete);
        assert_eq!(r.timezone.as_deref(), Some("Asia/Tashkent"));
        assert_eq!(r.jobs.len(), 5);
        let backup = &r.jobs[0];
        assert_eq!((backup.user.as_str(), backup.source.as_str()), ("deploy", "user"));
        assert_eq!(backup.schedule, "0 3 * * *");
        assert_eq!(backup.command, "/usr/local/bin/backup.sh >> /var/log/backup.log 2>&1");
        assert_eq!(backup.note.as_deref(), Some("Nightly database backup"));
        assert_eq!(backup.description, "Every day at 03:00");
        assert_eq!(backup.next_runs[0], "2026-11-16T03:00:00+05:00");
        assert_eq!(backup.last_run.as_deref(), Some("2026-11-15T03:00:01+0500"));
        let ping = &r.jobs[1];
        assert!(ping.paused && ping.next_runs.is_empty());
        assert_eq!(ping.raw, "#orbital-paused# */5 * * * * curl -fsS https://hc.example.com/ping");
        assert_eq!(r.jobs[2].description, "When the server starts");
        let certbot = r.jobs.iter().find(|j| j.source == "system").unwrap();
        assert_eq!(
            (certbot.user.as_str(), certbot.file.as_deref()),
            ("root", Some("/etc/cron.d/certbot"))
        );
        assert_eq!(certbot.line, 2);
        assert!(certbot.last_run.is_some());
        assert_eq!(r.periodic.len(), 2);
        let t = &r.timers[0];
        assert_eq!((t.unit.as_str(), t.activates.as_str()), ("apt-daily.timer", "apt-daily.service"));
        assert_eq!(t.calendar.as_deref(), Some("*-*-* 06,18:00:00"));
    }

    #[test]
    fn changes_find_their_line_and_validate() {
        let tab = "MAILTO=x\n# Nightly\n0 3 * * * /backup.sh\n";
        let added = apply(
            tab,
            &CronChange::Add {
                schedule: "*/10 * * * *".into(),
                command: "/ping.sh".into(),
                note: Some("Health check".into()),
            },
        )
        .unwrap();
        assert_eq!(
            added,
            "MAILTO=x\n# Nightly\n0 3 * * * /backup.sh\n\n# Health check\n*/10 * * * * /ping.sh\n"
        );
        let updated = apply(
            tab,
            &CronChange::Update {
                raw: "0 3 * * * /backup.sh".into(),
                schedule: "30 2 * * *".into(),
                command: "/backup.sh --full".into(),
                note: Some("Nightly full".into()),
            },
        )
        .unwrap();
        assert_eq!(updated, "MAILTO=x\n# Nightly full\n30 2 * * * /backup.sh --full\n");
        let paused = apply(
            tab,
            &CronChange::Pause {
                raw: "0 3 * * * /backup.sh".into(),
            },
        )
        .unwrap();
        assert!(paused.contains("#orbital-paused# 0 3 * * * /backup.sh"));
        let resumed = apply(
            &paused,
            &CronChange::Resume {
                raw: "#orbital-paused# 0 3 * * * /backup.sh".into(),
            },
        )
        .unwrap();
        assert_eq!(resumed, tab);
        let deleted = apply(
            tab,
            &CronChange::Delete {
                raw: "0 3 * * * /backup.sh".into(),
            },
        )
        .unwrap();
        assert_eq!(deleted, "MAILTO=x\n# Nightly\n");
        assert!(
            apply(
                tab,
                &CronChange::Delete {
                    raw: "0 4 * * * /other".into()
                }
            )
            .is_err(),
            "stale line"
        );
        for bad in [
            CronChange::Add {
                schedule: "0 3 * *".into(),
                command: "x".into(),
                note: None,
            },
            CronChange::Add {
                schedule: "0 3 * * * ; reboot".into(),
                command: "x".into(),
                note: None,
            },
            CronChange::Add {
                schedule: "0 3 * * *".into(),
                command: "a\n* * * * * evil".into(),
                note: None,
            },
            CronChange::Add {
                schedule: "0 3 * * *".into(),
                command: " ".into(),
                note: None,
            },
        ] {
            assert!(apply(tab, &bad).is_err(), "{bad:?}");
        }
        assert!(write_command("Bad User").is_err());
    }

    #[test]
    fn run_now_uses_the_part_cron_runs() {
        assert_eq!(runnable_command("date +\\%F > /tmp/d"), "date +%F > /tmp/d");
        assert_eq!(runnable_command("mail -s hi root%body text"), "mail -s hi root");
        let c = run_now_command("deploy", "echo 'hi'").unwrap();
        assert!(c.contains("runuser -u deploy -- sh -c 'echo '\\''hi'\\'''"));
        assert!(run_now_command("x;y", "true").is_err());
    }

    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(out.status.success());
        let r = parse(&String::from_utf8_lossy(&out.stdout));
        assert!(r.server_now.is_some());
    }
}
