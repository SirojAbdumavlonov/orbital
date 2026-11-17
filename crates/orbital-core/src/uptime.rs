//! Uptime checks run from this Mac while Orbital is open: HTTP(S) URLs (an
//! expected status code, optionally a piece of text the page must contain)
//! and TCP `host:port` checks, every 1, 5 or 15 minutes.
//!
//! The pure parts live here and are unit-tested: validating a check,
//! evaluating a probe into a [`CheckResult`], the up/down state machine
//! ([`transition`]), the certificate-expiry warning and reading a
//! certificate's expiry date out of its DER. The engine owns the schedule and
//! the alerts (`Engine::run_uptime`, `Engine::run_uptime_check`).
//!
//! Checks, results (7 days) and state changes (90 days) are kept in SQLite.
//! A URL with a user name or password is refused: secrets never go into SQLite.

use crate::db::Database;
use crate::error::{Error, Result};
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// How often a check may run.
pub const INTERVALS: [u32; 3] = [1, 5, 15];
/// Warn when an HTTPS certificate expires within this many days.
pub const CERT_WARN_DAYS: i64 = 14;
/// Results kept for the sparkline and uptime percentages.
const KEEP_RESULTS_DAYS: i64 = 7;
const KEEP_CHANGES_DAYS: i64 = 90;
/// Read at most this much of a page when looking for the expected text.
const MAX_BODY: usize = 1 << 20;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const TCP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckKind {
    Http,
    Tcp,
}

impl CheckKind {
    fn as_str(self) -> &'static str {
        match self {
            CheckKind::Http => "http",
            CheckKind::Tcp => "tcp",
        }
    }
    fn parse(s: &str) -> Self {
        if s == "tcp" {
            CheckKind::Tcp
        } else {
            CheckKind::Http
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckInput {
    /// Shown in the list and in alerts; defaults to the target.
    #[serde(default)]
    pub name: String,
    pub kind: CheckKind,
    /// A URL (http) or `host:port` (tcp).
    pub target: String,
    /// HTTP only: the status code that means "up" (default 200).
    #[serde(default)]
    pub expected_status: Option<u16>,
    /// HTTP only: text that must appear in the response body.
    #[serde(default)]
    pub contains: Option<String>,
    /// 1, 5 or 15 minutes.
    pub interval_minutes: u32,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub name: String,
    pub kind: CheckKind,
    pub target: String,
    pub expected_status: u16,
    pub contains: Option<String>,
    pub interval_minutes: u32,
    pub paused: bool,
    pub created_at: String,
}

/// One run of a check.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub at: String,
    pub up: bool,
    pub latency_ms: Option<u64>,
    /// HTTP status code, when a response came back.
    pub status_code: Option<u16>,
    /// Why the check is down.
    pub error: Option<String>,
    /// HTTPS: when the server's certificate expires.
    pub cert_expires: Option<String>,
    /// HTTPS: set when the certificate expires within [`CERT_WARN_DAYS`].
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    /// Never ran (or paused).
    Unknown,
    Up,
    Down,
}

/// A change worth an alert.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    WentDown,
    Recovered,
}

/// The state after a result, and whether it's a change to alert about. The
/// first result after starting is only an alert when it's down: "it's up" is
/// not news, "it's down" is, even right after launch.
pub fn transition(prev: CheckState, up: bool) -> (CheckState, Option<Change>) {
    match (prev, up) {
        (CheckState::Down, true) => (CheckState::Up, Some(Change::Recovered)),
        (CheckState::Up | CheckState::Unknown, false) => (CheckState::Down, Some(Change::WentDown)),
        (_, true) => (CheckState::Up, None),
        (CheckState::Down, false) => (CheckState::Down, None),
    }
}

/// A recorded up/down change.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateChange {
    pub at: String,
    pub up: bool,
    pub error: Option<String>,
}

/// A check with what's known about it, for the Uptime page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckStatus {
    pub check: Check,
    pub state: CheckState,
    pub last: Option<CheckResult>,
    /// The latest results, oldest first (for the sparkline).
    pub recent: Vec<CheckResult>,
    /// Share of runs that were up, in percent; None without runs.
    pub uptime_24h: Option<f64>,
    pub uptime_7d: Option<f64>,
    /// The most recent failure, even if it's up again now.
    pub last_error: Option<StateChange>,
    /// When the current state began.
    pub since: Option<String>,
    /// Recent up/down changes, newest first.
    pub changes: Vec<StateChange>,
}

/// Sent to the app while checks run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum UptimeEvent {
    /// A check ran (the page refreshes).
    Result { check_id: String, result: CheckResult },
    /// It went down or came back: notify.
    Changed {
        check_id: String,
        name: String,
        target: String,
        change: Change,
        error: Option<String>,
    },
    /// Its certificate expires soon (once per check per launch).
    CertWarning { check_id: String, name: String, message: String },
}

/// Receives [`UptimeEvent`]s (the app turns them into notifications and toasts).
pub type Listener = std::sync::Arc<dyn Fn(UptimeEvent) + Send + Sync>;

/// What the engine remembers between runs (not stored).
#[derive(Default)]
pub struct Runtime {
    pub states: HashMap<String, CheckState>,
    pub last_run: HashMap<String, Instant>,
    pub running: HashSet<String>,
    pub cert_warned: HashSet<String>,
    pub last_prune: Option<Instant>,
}

impl Runtime {
    /// Forget a check's state, e.g. after it was paused, edited or deleted.
    pub fn forget(&mut self, id: &str) {
        self.states.remove(id);
        self.last_run.remove(id);
        self.cert_warned.remove(id);
    }
}

// ---------- validation ----------

/// Normalise and check what the user entered.
pub fn validate(input: &CheckInput) -> Result<CheckInput> {
    let target = input.target.trim().to_string();
    if target.is_empty() || target.len() > 2000 {
        return Err(Error::InvalidInput("enter the address to check".into()));
    }
    if !INTERVALS.contains(&input.interval_minutes) {
        return Err(Error::InvalidInput("check every 1, 5 or 15 minutes".into()));
    }
    let name = input.name.trim();
    if name.len() > 80 {
        return Err(Error::InvalidInput("the name is too long (up to 80 characters)".into()));
    }
    let mut out = CheckInput {
        name: if name.is_empty() { target.clone() } else { name.to_string() },
        target,
        ..input.clone()
    };
    match input.kind {
        CheckKind::Http => {
            let url = url::Url::parse(&out.target).map_err(|_| Error::InvalidInput(format!("{:?} is not a URL", out.target)))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
                return Err(Error::InvalidInput("the URL must start with http:// or https://".into()));
            }
            if !url.username().is_empty() || url.password().is_some() {
                return Err(Error::InvalidInput(
                    "leave the user name and password out of the URL: Orbital doesn't store them with checks".into(),
                ));
            }
            let status = input.expected_status.unwrap_or(200);
            if !(100..=599).contains(&status) {
                return Err(Error::InvalidInput("the expected status must be between 100 and 599".into()));
            }
            out.expected_status = Some(status);
            out.contains = input.contains.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(String::from);
            if out.contains.as_ref().is_some_and(|c| c.len() > 500) {
                return Err(Error::InvalidInput(
                    "the text to look for is too long (up to 500 characters)".into(),
                ));
            }
        }
        CheckKind::Tcp => {
            parse_host_port(&out.target)?;
            out.expected_status = None;
            out.contains = None;
        }
    }
    Ok(out)
}

/// `host:port` or `[v6]:port`.
pub fn parse_host_port(s: &str) -> Result<(String, u16)> {
    let bad = || Error::InvalidInput(format!("{s:?} is not host:port (for example db.example.com:5432)"));
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, p) = rest.split_once("]:").ok_or_else(bad)?;
        (h, p)
    } else {
        s.rsplit_once(':').ok_or_else(bad)?
    };
    let port: u16 = port.parse().map_err(|_| bad())?;
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'));
    if port == 0 || !host_ok || (host.contains(':') && !s.starts_with('[')) {
        return Err(bad());
    }
    Ok((host.to_string(), port))
}

// ---------- evaluation ----------

/// What came back from an HTTP request.
#[derive(Debug, Clone, Default)]
pub struct HttpProbe {
    pub status: u16,
    pub body: Option<String>,
    pub latency_ms: u64,
    pub cert_expires: Option<DateTime<Utc>>,
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

fn status_text(code: u16) -> String {
    match reason(code) {
        "" => code.to_string(),
        r => format!("{code} {r}"),
    }
}

/// The warning shown when a certificate expires within [`CERT_WARN_DAYS`].
pub fn cert_warning(expires: DateTime<Utc>, now: DateTime<Utc>) -> Option<String> {
    let left = expires - now;
    if left > ChronoDuration::days(CERT_WARN_DAYS) {
        return None;
    }
    Some(if left <= ChronoDuration::zero() {
        "the TLS certificate has expired".into()
    } else if left < ChronoDuration::days(1) {
        "the TLS certificate expires within a day".into()
    } else {
        let d = left.num_days();
        format!("the TLS certificate expires in {d} day{}", if d == 1 { "" } else { "s" })
    })
}

/// Turn an HTTP probe (or the error that stopped it) into a result.
pub fn evaluate_http(check: &Check, probe: std::result::Result<HttpProbe, String>, now: DateTime<Utc>) -> CheckResult {
    let at = now.to_rfc3339();
    let p = match probe {
        Ok(p) => p,
        Err(e) => {
            return CheckResult {
                at,
                up: false,
                latency_ms: None,
                status_code: None,
                error: Some(e),
                cert_expires: None,
                warning: None,
            }
        }
    };
    let error = if p.status != check.expected_status {
        Some(format!(
            "got {}, expected {}",
            status_text(p.status),
            status_text(check.expected_status)
        ))
    } else if let Some(want) = check.contains.as_deref() {
        match p.body.as_deref() {
            Some(body) if body.contains(want) => None,
            _ => Some(format!("the page doesn't contain {want:?}")),
        }
    } else {
        None
    };
    CheckResult {
        at,
        up: error.is_none(),
        latency_ms: Some(p.latency_ms),
        status_code: Some(p.status),
        error,
        cert_expires: p.cert_expires.map(|d| d.to_rfc3339()),
        warning: p.cert_expires.and_then(|d| cert_warning(d, now)),
    }
}

/// Turn a TCP connection attempt into a result.
pub fn evaluate_tcp(probe: std::result::Result<u64, String>, now: DateTime<Utc>) -> CheckResult {
    let at = now.to_rfc3339();
    match probe {
        Ok(ms) => CheckResult {
            at,
            up: true,
            latency_ms: Some(ms),
            status_code: None,
            error: None,
            cert_expires: None,
            warning: None,
        },
        Err(e) => CheckResult {
            at,
            up: false,
            latency_ms: None,
            status_code: None,
            error: Some(e),
            cert_expires: None,
            warning: None,
        },
    }
}

/// Percent of results that were up.
pub fn uptime_percent(ups: impl IntoIterator<Item = bool>) -> Option<f64> {
    let (mut n, mut up) = (0u32, 0u32);
    for u in ups {
        n += 1;
        up += u as u32;
    }
    (n > 0).then(|| f64::from(up) * 100.0 / f64::from(n))
}

/// Whether a check is due, given when it last ran.
pub fn is_due(check: &Check, last_run: Option<Instant>, now: Instant) -> bool {
    !check.paused && last_run.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(u64::from(check.interval_minutes) * 60))
}

// ---------- certificates ----------

/// Read one DER element: (tag, contents, rest).
fn der(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        (rest[..n].iter().fold(0usize, |a, &b| (a << 8) | b as usize), &rest[n..])
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

fn der_time(tag: u8, v: &[u8]) -> Option<DateTime<Utc>> {
    let s = std::str::from_utf8(v).ok()?.strip_suffix('Z')?;
    let (year, rest) = match tag {
        // UTCTime: YYMMDDHHMMSS, 50–99 → 19xx.
        0x17 if s.len() == 12 => {
            let yy: i32 = s[..2].parse().ok()?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &s[2..])
        }
        // GeneralizedTime: YYYYMMDDHHMMSS.
        0x18 if s.len() == 14 => (s[..4].parse().ok()?, &s[4..]),
        _ => return None,
    };
    let n = |i: usize| rest.get(i..i + 2)?.parse::<u32>().ok();
    Utc.with_ymd_and_hms(year, n(0)?, n(2)?, n(4)?, n(6)?, n(8)?).single()
}

/// When an X.509 certificate (DER) stops being valid.
pub fn cert_not_after(cert: &[u8]) -> Option<DateTime<Utc>> {
    let (0x30, cert, _) = der(cert)? else { return None };
    let (0x30, mut tbs, _) = der(cert)? else { return None };
    // version [0] is optional
    if tbs.first() == Some(&0xa0) {
        tbs = der(tbs)?.2;
    }
    let (_serial, _, rest) = der(tbs)?;
    let (_sig_alg, _, rest) = der(rest)?;
    let (_issuer, _, rest) = der(rest)?;
    let (0x30, validity, _) = der(rest)? else { return None };
    let (_, _, after) = der(validity)?;
    let (tag, v, _) = der(after)?;
    der_time(tag, v)
}

// ---------- probes ----------

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .tls_info(true)
        .user_agent(concat!("Orbital-Uptime/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("HTTP client")
}

fn describe_reqwest(e: &reqwest::Error) -> String {
    let mut msg = if e.is_timeout() {
        "timed out".to_string()
    } else if e.is_connect() {
        "couldn't connect".to_string()
    } else if e.is_redirect() {
        "too many redirects".to_string()
    } else {
        "request failed".to_string()
    };
    // The innermost cause says why (DNS, refused, certificate, …).
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(e);
    let mut last = None;
    while let Some(s) = src {
        last = Some(s.to_string());
        src = s.source();
    }
    if let Some(l) = last {
        msg.push_str(": ");
        msg.push_str(&l);
    }
    msg
}

pub async fn probe_http(client: &reqwest::Client, check: &Check) -> std::result::Result<HttpProbe, String> {
    let t = Instant::now();
    let mut resp = client.get(&check.target).send().await.map_err(|e| describe_reqwest(&e))?;
    let latency_ms = t.elapsed().as_millis() as u64;
    let cert_expires = resp
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(|i| i.peer_certificate())
        .and_then(cert_not_after);
    let status = resp.status().as_u16();
    let body = if check.contains.is_some() {
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| describe_reqwest(&e))? {
            buf.extend_from_slice(&chunk);
            if buf.len() >= MAX_BODY {
                buf.truncate(MAX_BODY);
                break;
            }
        }
        Some(String::from_utf8_lossy(&buf).into_owned())
    } else {
        None
    };
    Ok(HttpProbe {
        status,
        body,
        latency_ms,
        cert_expires,
    })
}

pub async fn probe_tcp(target: &str) -> std::result::Result<u64, String> {
    let (host, port) = parse_host_port(target).map_err(|e| e.to_string())?;
    let t = Instant::now();
    match tokio::time::timeout(TCP_TIMEOUT, tokio::net::TcpStream::connect((host.as_str(), port))).await {
        Ok(Ok(_)) => Ok(t.elapsed().as_millis() as u64),
        Ok(Err(e)) => Err(format!("couldn't connect: {e}")),
        Err(_) => Err("timed out".into()),
    }
}

/// Run a check once.
pub async fn probe(client: &reqwest::Client, check: &Check) -> CheckResult {
    match check.kind {
        CheckKind::Http => {
            let p = probe_http(client, check).await;
            evaluate_http(check, p, Utc::now())
        }
        CheckKind::Tcp => {
            let p = probe_tcp(&check.target).await;
            evaluate_tcp(p, Utc::now())
        }
    }
}

// ---------- storage ----------

fn check_row(r: &Row) -> rusqlite::Result<Check> {
    Ok(Check {
        id: r.get("id")?,
        name: r.get("name")?,
        kind: CheckKind::parse(&r.get::<_, String>("kind")?),
        target: r.get("target")?,
        expected_status: r.get("expected_status")?,
        contains: r.get("contains")?,
        interval_minutes: r.get("interval_minutes")?,
        paused: r.get("paused")?,
        created_at: r.get("created_at")?,
    })
}

fn result_row(r: &Row) -> rusqlite::Result<CheckResult> {
    Ok(CheckResult {
        at: r.get("at")?,
        up: r.get("up")?,
        latency_ms: r.get::<_, Option<i64>>("latency_ms")?.map(|v| v as u64),
        status_code: r.get("status_code")?,
        error: r.get("error")?,
        cert_expires: r.get("cert_expires")?,
        warning: r.get("warning")?,
    })
}

pub fn list_checks(db: &Database) -> Result<Vec<Check>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM uptime_checks ORDER BY created_at, name")?;
    let rows = st.query_map([], check_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_check(db: &Database, id: &str) -> Result<Check> {
    db.conn()
        .query_row("SELECT * FROM uptime_checks WHERE id = ?1", [id], check_row)
        .optional()?
        .ok_or_else(|| Error::NotFound("that uptime check no longer exists".into()))
}

/// Create (id None) or replace a check. Returns it as stored.
pub fn save_check(db: &Database, id: Option<&str>, input: &CheckInput) -> Result<Check> {
    let v = validate(input)?;
    let conn = db.conn();
    let id = match id {
        Some(id) => {
            let n = conn.execute(
                "UPDATE uptime_checks SET name=?2, kind=?3, target=?4, expected_status=?5, contains=?6, interval_minutes=?7, paused=?8 WHERE id=?1",
                params![id, v.name, v.kind.as_str(), v.target, v.expected_status.unwrap_or(0), v.contains, v.interval_minutes, v.paused],
            )?;
            if n == 0 {
                return Err(Error::NotFound("that uptime check no longer exists".into()));
            }
            id.to_string()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO uptime_checks(id, name, kind, target, expected_status, contains, interval_minutes, paused, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, v.name, v.kind.as_str(), v.target, v.expected_status.unwrap_or(0), v.contains, v.interval_minutes, v.paused, Utc::now().to_rfc3339()],
            )?;
            id
        }
    };
    drop(conn);
    get_check(db, &id)
}

pub fn set_paused(db: &Database, id: &str, paused: bool) -> Result<Check> {
    let n = db
        .conn()
        .execute("UPDATE uptime_checks SET paused=?2 WHERE id=?1", params![id, paused])?;
    if n == 0 {
        return Err(Error::NotFound("that uptime check no longer exists".into()));
    }
    get_check(db, id)
}

pub fn delete_check(db: &Database, id: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute("DELETE FROM uptime_results WHERE check_id = ?1", [id])?;
    conn.execute("DELETE FROM uptime_changes WHERE check_id = ?1", [id])?;
    conn.execute("DELETE FROM uptime_checks WHERE id = ?1", [id])?;
    Ok(())
}

pub fn store_result(db: &Database, check_id: &str, r: &CheckResult) -> Result<()> {
    db.conn().execute(
        "INSERT INTO uptime_results(check_id, at, up, latency_ms, status_code, error, cert_expires, warning) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![check_id, r.at, r.up, r.latency_ms.map(|v| v as i64), r.status_code, r.error, r.cert_expires, r.warning],
    )?;
    Ok(())
}

pub fn store_change(db: &Database, check_id: &str, at: &str, up: bool, error: Option<&str>) -> Result<()> {
    db.conn().execute(
        "INSERT INTO uptime_changes(check_id, at, up, error) VALUES (?1,?2,?3,?4)",
        params![check_id, at, up, error],
    )?;
    Ok(())
}

/// The state the last stored result implies (so a restart doesn't re-alert).
pub fn last_state(db: &Database, check_id: &str) -> Result<CheckState> {
    let up: Option<bool> = db
        .conn()
        .query_row(
            "SELECT up FROM uptime_results WHERE check_id = ?1 ORDER BY at DESC, rowid DESC LIMIT 1",
            [check_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match up {
        Some(true) => CheckState::Up,
        Some(false) => CheckState::Down,
        None => CheckState::Unknown,
    })
}

/// Drop results and changes past their keep time.
pub fn prune(db: &Database, now: DateTime<Utc>) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "DELETE FROM uptime_results WHERE at < ?1",
        [(now - ChronoDuration::days(KEEP_RESULTS_DAYS)).to_rfc3339()],
    )?;
    conn.execute(
        "DELETE FROM uptime_changes WHERE at < ?1",
        [(now - ChronoDuration::days(KEEP_CHANGES_DAYS)).to_rfc3339()],
    )?;
    Ok(())
}

/// Everything the Uptime page shows about a check.
pub fn status(db: &Database, check: Check, now: DateTime<Utc>) -> Result<CheckStatus> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM uptime_results WHERE check_id = ?1 ORDER BY at DESC, rowid DESC LIMIT 60")?;
    let mut recent = st.query_map([&check.id], result_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    recent.reverse();
    let since = |days: i64| -> Result<Option<f64>> {
        let mut st = conn.prepare_cached("SELECT up FROM uptime_results WHERE check_id = ?1 AND at >= ?2")?;
        let ups = st
            .query_map(params![check.id, (now - ChronoDuration::days(days)).to_rfc3339()], |r| {
                r.get::<_, bool>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(uptime_percent(ups))
    };
    let (uptime_24h, uptime_7d) = (since(1)?, since(7)?);
    let mut st = conn.prepare("SELECT at, up, error FROM uptime_changes WHERE check_id = ?1 ORDER BY at DESC, rowid DESC LIMIT 20")?;
    let changes = st
        .query_map([&check.id], |r| {
            Ok(StateChange {
                at: r.get(0)?,
                up: r.get(1)?,
                error: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let last_error = conn
        .query_row(
            "SELECT at, error FROM uptime_results WHERE check_id = ?1 AND up = 0 ORDER BY at DESC, rowid DESC LIMIT 1",
            [&check.id],
            |r| {
                Ok(StateChange {
                    at: r.get(0)?,
                    up: false,
                    error: r.get(1)?,
                })
            },
        )
        .optional()?;
    let last = recent.last().cloned();
    let state = match (&last, check.paused) {
        (_, true) | (None, _) => CheckState::Unknown,
        (Some(r), _) if r.up => CheckState::Up,
        _ => CheckState::Down,
    };
    let since = changes.first().map(|c| c.at.clone());
    Ok(CheckStatus {
        check,
        state,
        last,
        recent,
        uptime_24h,
        uptime_7d,
        last_error,
        since,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_check() -> Check {
        Check {
            id: "c1".into(),
            name: "shop".into(),
            kind: CheckKind::Http,
            target: "https://shop.example.com/".into(),
            expected_status: 200,
            contains: None,
            interval_minutes: 5,
            paused: false,
            created_at: "2026-11-16T00:00:00Z".into(),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap()
    }

    fn input(kind: CheckKind, target: &str) -> CheckInput {
        CheckInput {
            name: String::new(),
            kind,
            target: target.into(),
            expected_status: None,
            contains: None,
            interval_minutes: 5,
            paused: false,
        }
    }

    #[test]
    fn transitions_alert_only_on_real_changes() {
        use CheckState::*;
        assert_eq!(transition(Unknown, true), (Up, None), "the first good result is not news");
        assert_eq!(
            transition(Unknown, false),
            (Down, Some(Change::WentDown)),
            "down right after launch is"
        );
        assert_eq!(transition(Up, true), (Up, None));
        assert_eq!(transition(Up, false), (Down, Some(Change::WentDown)));
        assert_eq!(transition(Down, false), (Down, None), "no repeat alerts while it stays down");
        assert_eq!(transition(Down, true), (Up, Some(Change::Recovered)));
    }

    #[test]
    fn http_evaluation() {
        let c = http_check();
        let ok = HttpProbe {
            status: 200,
            body: None,
            latency_ms: 42,
            cert_expires: None,
        };
        let r = evaluate_http(&c, Ok(ok.clone()), now());
        assert!(r.up && r.error.is_none());
        assert_eq!((r.latency_ms, r.status_code), (Some(42), Some(200)));

        let r = evaluate_http(&c, Ok(HttpProbe { status: 503, ..ok.clone() }), now());
        assert!(!r.up);
        assert_eq!(r.error.as_deref(), Some("got 503 Service Unavailable, expected 200 OK"));
        assert_eq!(r.status_code, Some(503), "the status is kept even when down");

        // A redirect is only up when that's what's expected.
        let r = evaluate_http(
            &Check {
                expected_status: 301,
                ..c.clone()
            },
            Ok(HttpProbe { status: 301, ..ok.clone() }),
            now(),
        );
        assert!(r.up);

        let with_text = Check {
            contains: Some("Add to cart".into()),
            ..c.clone()
        };
        let r = evaluate_http(
            &with_text,
            Ok(HttpProbe {
                body: Some("<button>Add to cart</button>".into()),
                ..ok.clone()
            }),
            now(),
        );
        assert!(r.up);
        let r = evaluate_http(
            &with_text,
            Ok(HttpProbe {
                body: Some("Maintenance".into()),
                ..ok.clone()
            }),
            now(),
        );
        assert_eq!(r.error.as_deref(), Some("the page doesn't contain \"Add to cart\""));
        // A wrong status is the more useful error.
        let r = evaluate_http(
            &with_text,
            Ok(HttpProbe {
                status: 500,
                body: Some("oops".into()),
                ..ok
            }),
            now(),
        );
        assert!(r.error.unwrap().starts_with("got 500"));

        let r = evaluate_http(&c, Err("couldn't connect: Connection refused".into()), now());
        assert!(!r.up && r.latency_ms.is_none());
        assert_eq!(r.error.as_deref(), Some("couldn't connect: Connection refused"));
    }

    #[test]
    fn certificate_expiry_warns_within_14_days() {
        let n = now();
        assert_eq!(cert_warning(n + ChronoDuration::days(30), n), None);
        assert_eq!(cert_warning(n + ChronoDuration::days(15), n), None);
        assert_eq!(
            cert_warning(n + ChronoDuration::days(14), n).as_deref(),
            Some("the TLS certificate expires in 14 days")
        );
        assert_eq!(
            cert_warning(n + ChronoDuration::hours(36), n).as_deref(),
            Some("the TLS certificate expires in 1 day")
        );
        assert_eq!(
            cert_warning(n + ChronoDuration::hours(3), n).as_deref(),
            Some("the TLS certificate expires within a day")
        );
        assert_eq!(
            cert_warning(n - ChronoDuration::days(1), n).as_deref(),
            Some("the TLS certificate has expired")
        );

        let c = http_check();
        let p = HttpProbe {
            status: 200,
            body: None,
            latency_ms: 10,
            cert_expires: Some(n + ChronoDuration::days(3)),
        };
        let r = evaluate_http(&c, Ok(p), n);
        assert!(r.up, "a certificate about to expire is a warning, not an outage");
        assert_eq!(r.warning.as_deref(), Some("the TLS certificate expires in 3 days"));
        assert!(r.cert_expires.unwrap().starts_with("2026-11-19"));
    }

    #[test]
    fn reads_the_expiry_date_from_a_certificate() {
        let der = include_bytes!("../tests/fixtures/uptime-cert.der");
        assert_eq!(cert_not_after(der), Some(Utc.with_ymd_and_hms(2036, 11, 12, 20, 4, 46).unwrap()));
        assert_eq!(cert_not_after(&der[..40]), None, "truncated");
        assert_eq!(cert_not_after(b"not a certificate"), None);
        assert_eq!(
            der_time(0x17, b"991231235959Z"),
            Utc.with_ymd_and_hms(1999, 12, 31, 23, 59, 59).single()
        );
        assert_eq!(
            der_time(0x18, b"20500101000000Z"),
            Utc.with_ymd_and_hms(2050, 1, 1, 0, 0, 0).single()
        );
    }

    #[test]
    fn tcp_evaluation_and_host_port() {
        assert!(evaluate_tcp(Ok(7), now()).up);
        let r = evaluate_tcp(Err("timed out".into()), now());
        assert!(!r.up && r.error.as_deref() == Some("timed out"));
        assert_eq!(parse_host_port("db.example.com:5432").unwrap(), ("db.example.com".into(), 5432));
        assert_eq!(parse_host_port("[2001:db8::1]:443").unwrap(), ("2001:db8::1".into(), 443));
        for bad in [
            "db.example.com",
            ":80",
            "host:0",
            "host:99999",
            "2001:db8::1:443",
            "a b:80",
            "host:80; rm -rf /",
        ] {
            assert!(parse_host_port(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validation() {
        let v = validate(&input(CheckKind::Http, "  https://shop.example.com/health ")).unwrap();
        assert_eq!(v.target, "https://shop.example.com/health");
        assert_eq!(v.name, v.target, "the name defaults to the address");
        assert_eq!(v.expected_status, Some(200));
        assert!(validate(&input(CheckKind::Http, "ftp://example.com")).is_err());
        assert!(validate(&input(CheckKind::Http, "example.com")).is_err());
        let e = validate(&input(CheckKind::Http, "https://admin:hunter2@example.com/")).unwrap_err();
        assert!(e.to_string().contains("password"), "no secrets in the database");
        assert!(validate(&CheckInput {
            interval_minutes: 2,
            ..input(CheckKind::Http, "https://example.com")
        })
        .is_err());
        assert!(validate(&CheckInput {
            expected_status: Some(700),
            ..input(CheckKind::Http, "https://example.com")
        })
        .is_err());
        let t = validate(&CheckInput {
            contains: Some("x".into()),
            expected_status: Some(200),
            ..input(CheckKind::Tcp, "db:5432")
        })
        .unwrap();
        assert_eq!((t.contains, t.expected_status), (None, None), "HTTP options don't apply to TCP");
        assert!(validate(&input(CheckKind::Tcp, "https://example.com")).is_err());
    }

    #[test]
    fn schedule_and_uptime_percent() {
        let c = http_check();
        let t = Instant::now();
        assert!(is_due(&c, None, t));
        assert!(!is_due(&c, Some(t), t + Duration::from_secs(299)));
        assert!(is_due(&c, Some(t), t + Duration::from_secs(300)));
        assert!(!is_due(&Check { paused: true, ..c }, None, t));
        assert_eq!(uptime_percent([]), None);
        assert_eq!(uptime_percent([true, true, true, false]), Some(75.0));
    }

    #[test]
    fn storage_round_trip_and_status() {
        let db = Database::in_memory().unwrap();
        let c = save_check(
            &db,
            None,
            &CheckInput {
                name: "Shop".into(),
                contains: Some("cart".into()),
                ..input(CheckKind::Http, "https://shop.example.com/")
            },
        )
        .unwrap();
        assert_eq!(
            (c.name.as_str(), c.expected_status, c.contains.as_deref()),
            ("Shop", 200, Some("cart"))
        );
        assert_eq!(last_state(&db, &c.id).unwrap(), CheckState::Unknown);

        let n = Utc::now();
        let r = |mins: i64, up: bool| CheckResult {
            at: (n - ChronoDuration::minutes(mins)).to_rfc3339(),
            up,
            latency_ms: Some(50),
            status_code: Some(if up { 200 } else { 502 }),
            error: (!up).then(|| "got 502 Bad Gateway, expected 200 OK".into()),
            cert_expires: None,
            warning: None,
        };
        for (m, up) in [(30, true), (20, false), (10, true), (0, true)] {
            store_result(&db, &c.id, &r(m, up)).unwrap();
        }
        store_change(&db, &c.id, &r(20, false).at, false, Some("got 502")).unwrap();
        store_change(&db, &c.id, &r(10, true).at, true, None).unwrap();
        // An old result outside both windows, pruned below.
        store_result(&db, &c.id, &r(60 * 24 * 8, false)).unwrap();

        let s = status(&db, c.clone(), n).unwrap();
        assert_eq!(s.state, CheckState::Up);
        assert_eq!(s.recent.len(), 5);
        assert!(s.recent.windows(2).all(|w| w[0].at <= w[1].at), "oldest first");
        assert_eq!(s.uptime_24h, Some(75.0));
        assert_eq!(s.last_error.unwrap().error.as_deref(), Some("got 502 Bad Gateway, expected 200 OK"));
        assert_eq!(s.changes.len(), 2);
        assert!(s.changes[0].up, "newest first");
        assert_eq!(s.since, Some(r(10, true).at));
        assert_eq!(last_state(&db, &c.id).unwrap(), CheckState::Up);

        prune(&db, n).unwrap();
        assert_eq!(status(&db, c.clone(), n).unwrap().recent.len(), 4);

        let p = set_paused(&db, &c.id, true).unwrap();
        assert!(p.paused);
        assert_eq!(status(&db, p, n).unwrap().state, CheckState::Unknown, "paused checks show no state");

        let edited = save_check(&db, Some(&c.id), &input(CheckKind::Tcp, "shop.example.com:443")).unwrap();
        assert_eq!((edited.kind, edited.expected_status, edited.contains), (CheckKind::Tcp, 0, None));
        delete_check(&db, &c.id).unwrap();
        assert!(get_check(&db, &c.id).is_err());
        assert!(list_checks(&db).unwrap().is_empty());
        assert!(save_check(&db, Some("missing"), &input(CheckKind::Tcp, "a:1")).is_err());
    }
}
