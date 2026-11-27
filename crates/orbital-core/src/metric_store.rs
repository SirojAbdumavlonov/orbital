//! Monitoring history: one point a minute per server (CPU, memory, fullest
//! disk, load, network, uptime) from the metrics Orbital reads while it's
//! connected, kept 30 days in SQLite (`metric_history`). Read back for any
//! date range, averaged into at most `max` buckets so a month draws as fast as
//! an hour. Beside it, `metric_detail` keeps that minute's busiest programs
//! (names only: `ps -o comm`, never arguments, which can hold passwords) and
//! every disk's use, for "what was running then" and per-disk growth.
//! `events` reads what Orbital changed on the server (the audit log) and the
//! restarts seen in the history, to mark them on the charts.
//! Measurements only: not audited, nothing secret.

use crate::db::Database;
use crate::error::Result;
use crate::metrics::Metrics;
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// How long points are kept.
pub const KEEP_SECS: i64 = 30 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    /// Start of the minute (or bucket), Unix seconds.
    pub t: i64,
    pub cpu: Option<f64>,
    /// % used.
    pub mem: f64,
    /// % used on the fullest real disk.
    pub disk: Option<f64>,
    /// 1-minute load average.
    pub load: f64,
    /// Bytes per second.
    pub rx: Option<f64>,
    pub tx: Option<f64>,
    /// Seconds since the server booted.
    pub uptime: Option<i64>,
}

fn real_disk(fs_type: &str, mount: &str, total: u64) -> bool {
    !matches!(
        fs_type,
        "tmpfs" | "devtmpfs" | "overlay" | "squashfs" | "iso9660" | "udf" | "efivarfs" | "ramfs" | "nsfs"
    ) && !mount.starts_with("/snap/")
        && total >= 64 * 1024 * 1024
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The minute's point from a metrics sample.
pub fn point(m: &Metrics) -> Point {
    let s = &m.snapshot;
    let disk = s
        .disks
        .iter()
        .filter(|d| real_disk(&d.fs_type, &d.mount, d.total_bytes))
        .map(|d| d.used_percent)
        .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
    Point {
        t: (m.sampled_at_ms / 60_000) * 60,
        cpu: m.cpu_percent.map(round1),
        mem: if s.mem_total > 0 {
            round1((1.0 - s.mem_available as f64 / s.mem_total as f64) * 100.0)
        } else {
            0.0
        },
        disk: disk.map(round1),
        load: (s.load[0] * 100.0).round() / 100.0,
        rx: m.net_rx_per_sec.map(f64::round),
        tx: m.net_tx_per_sec.map(f64::round),
        uptime: Some(s.uptime_secs as i64),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcSample {
    /// The program's name (no arguments).
    pub name: String,
    pub user: String,
    pub pid: u32,
    pub cpu: f64,
    pub mem: f64,
    pub rss_kb: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSample {
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

/// One minute in detail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moment {
    pub t: i64,
    pub procs: Vec<ProcSample>,
    pub disks: Vec<DiskSample>,
}

/// The minute's busiest programs (top 5 by CPU and top 5 by memory) and its real disks.
pub fn moment(m: &Metrics) -> Moment {
    let s = &m.snapshot;
    let mut by_cpu: Vec<&crate::metrics::Process> = s.processes.iter().collect();
    by_cpu.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    let mut by_mem: Vec<&crate::metrics::Process> = s.processes.iter().collect();
    by_mem.sort_by(|a, b| b.mem_percent.total_cmp(&a.mem_percent));
    let mut procs: Vec<ProcSample> = Vec::new();
    for p in by_cpu.iter().take(5).chain(by_mem.iter().take(5)) {
        if procs.iter().any(|x| x.pid == p.pid) {
            continue;
        }
        // `comm` is already just the name; keep only its first word, to be sure.
        let name: String = p.command.split_whitespace().next().unwrap_or("").chars().take(64).collect();
        procs.push(ProcSample {
            name,
            user: p.user.chars().take(32).collect(),
            pid: p.pid,
            cpu: round1(p.cpu_percent),
            mem: round1(p.mem_percent),
            rss_kb: p.rss_kb,
        });
    }
    let disks = s
        .disks
        .iter()
        .filter(|d| real_disk(&d.fs_type, &d.mount, d.total_bytes))
        .take(20)
        .map(|d| DiskSample {
            mount: d.mount.clone(),
            used: d.used_bytes,
            total: d.total_bytes,
        })
        .collect();
    Moment {
        t: (m.sampled_at_ms / 60_000) * 60,
        procs,
        disks,
    }
}

pub fn record_moment(db: &Database, profile_id: &str, m: &Moment) -> Result<()> {
    let procs = serde_json::to_string(&m.procs).unwrap_or_else(|_| "[]".into());
    let disks = serde_json::to_string(&m.disks).unwrap_or_else(|_| "[]".into());
    let conn = db.conn();
    conn.execute(
        "INSERT OR REPLACE INTO metric_detail (profile_id, t, procs, disks) VALUES (?1, ?2, ?3, ?4)",
        params![profile_id, m.t, procs, disks],
    )?;
    if m.t % 3600 == 0 {
        conn.execute("DELETE FROM metric_detail WHERE t < ?1", [m.t - KEEP_SECS])?;
    }
    Ok(())
}

fn moment_row(r: &rusqlite::Row) -> rusqlite::Result<Moment> {
    let procs: String = r.get(1)?;
    let disks: String = r.get(2)?;
    Ok(Moment {
        t: r.get(0)?,
        procs: serde_json::from_str(&procs).unwrap_or_default(),
        disks: serde_json::from_str(&disks).unwrap_or_default(),
    })
}

/// The recorded minute nearest `t`, within `within` seconds.
pub fn moment_at(db: &Database, profile_id: &str, t: i64, within: i64) -> Result<Option<Moment>> {
    use rusqlite::OptionalExtension;
    Ok(db
        .conn()
        .query_row(
            "SELECT t, procs, disks FROM metric_detail WHERE profile_id = ?1 AND t BETWEEN ?2 AND ?3 ORDER BY ABS(t - ?4) LIMIT 1",
            params![profile_id, t - within, t + within, t],
            moment_row,
        )
        .optional()?)
}

/// One disk over a range: (time, bytes used), and how fast it grows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSeries {
    pub mount: String,
    pub total: u64,
    pub points: Vec<(i64, u64)>,
    /// Bytes a day (negative when it shrinks), from a straight line through the points; None under 6 hours of data.
    pub per_day: Option<f64>,
    /// When it fills at that pace (Unix seconds), within a year.
    pub full_at: Option<i64>,
}

/// Least-squares slope (y per second) through (t, y).
pub fn slope(points: &[(i64, f64)]) -> Option<f64> {
    let n = points.len() as f64;
    if points.len() < 3 {
        return None;
    }
    let mx = points.iter().map(|p| p.0 as f64).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (t, y) in points {
        num += (*t as f64 - mx) * (y - my);
        den += (*t as f64 - mx).powi(2);
    }
    (den > 0.0).then(|| num / den)
}

/// Every disk's use over a range, one sample per bucket, with its growth.
pub fn disk_series(db: &Database, profile_id: &str, from: i64, to: i64, max: u32) -> Result<Vec<DiskSeries>> {
    let b = bucket(from, to, max);
    let conn = db.conn();
    let mut st = conn.prepare(
        "SELECT MIN(t), '[]', disks FROM metric_detail WHERE profile_id = ?1 AND t BETWEEN ?2 AND ?3 GROUP BY t / ?4 ORDER BY 1",
    )?;
    let rows = st
        .query_map(params![profile_id, from, to, b], moment_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out: Vec<DiskSeries> = Vec::new();
    for m in rows {
        for d in m.disks {
            match out.iter_mut().find(|s| s.mount == d.mount) {
                Some(s) => {
                    s.total = d.total;
                    s.points.push((m.t, d.used));
                }
                None => out.push(DiskSeries {
                    mount: d.mount,
                    total: d.total,
                    points: vec![(m.t, d.used)],
                    per_day: None,
                    full_at: None,
                }),
            }
        }
    }
    for s in &mut out {
        let span = s.points.last().map(|p| p.0).unwrap_or(0) - s.points.first().map(|p| p.0).unwrap_or(0);
        if span < 6 * 3600 {
            continue;
        }
        let pts: Vec<(i64, f64)> = s.points.iter().map(|(t, u)| (*t, *u as f64)).collect();
        if let Some(k) = slope(&pts) {
            s.per_day = Some((k * 86_400.0).round());
            let (t_last, used_last) = *s.points.last().unwrap();
            if k > 0.0 && s.total > used_last {
                let secs = (s.total - used_last) as f64 / k;
                if secs < 365.0 * 86_400.0 {
                    s.full_at = Some(t_last + secs as i64);
                }
            }
        }
    }
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    Ok(out)
}

/// Something that happened on a server, to mark on its charts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub t: i64,
    /// "restart", or the audit category ("service", "docker", "config", …).
    pub kind: String,
    pub action: String,
    pub target: Option<String>,
    pub ok: bool,
}

/// Changes Orbital made on this server (from the audit log: services, containers,
/// config files, packages, updates, deploys…) and the restarts seen in the history.
pub fn events(db: &Database, profile_id: &str, from: i64, to: i64) -> Result<Vec<Event>> {
    // The audit log's own format (UTC, milliseconds, "Z"), so text order is time order.
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_default()
    };
    let conn = db.conn();
    let mut st = conn.prepare(
        "SELECT ts, category, action, target, outcome FROM audit_log
         WHERE profile_id = ?1 AND ts BETWEEN ?2 AND ?3
           AND category IN ('config', 'service', 'docker', 'security', 'command')
           AND outcome != 'denied'
           AND action NOT LIKE '%.read%' AND action NOT IN ('command.run', 'notify.agent')
         ORDER BY ts DESC LIMIT 300",
    )?;
    let mut out: Vec<Event> = st
        .query_map(params![profile_id, iso(from), iso(to)], |r| {
            let ts: String = r.get(0)?;
            let outcome: String = r.get(4)?;
            Ok(Event {
                t: chrono::DateTime::parse_from_rfc3339(&ts).map(|d| d.timestamp()).unwrap_or(0),
                kind: r.get::<_, String>(1)?.trim_matches('"').to_string(),
                action: r.get(2)?,
                target: r.get(3)?,
                ok: outcome.trim_matches('"') == "success",
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Restarts: the uptime went down between two minutes.
    let mut st = conn
        .prepare("SELECT t, uptime FROM metric_history WHERE profile_id = ?1 AND t BETWEEN ?2 AND ?3 AND uptime IS NOT NULL ORDER BY t")?;
    let ups = st
        .query_map(params![profile_id, from, to], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for w in ups.windows(2) {
        if w[1].1 + 120 < w[0].1 {
            out.push(Event {
                t: w[1].0 - w[1].1.min(w[1].0 - w[0].0),
                kind: "restart".into(),
                action: "server.restart".into(),
                target: None,
                ok: true,
            });
        }
    }
    out.sort_by_key(|e| e.t);
    Ok(out)
}

/// Saves a point (replacing that minute's), and once an hour drops what's older than 30 days.
pub fn record(db: &Database, profile_id: &str, p: &Point) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT OR REPLACE INTO metric_history (profile_id, t, cpu, mem, disk, load, rx, tx, uptime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![profile_id, p.t, p.cpu, p.mem, p.disk, p.load, p.rx, p.tx, p.uptime],
    )?;
    if p.t % 3600 == 0 {
        conn.execute("DELETE FROM metric_history WHERE t < ?1", [p.t - KEEP_SECS])?;
    }
    Ok(())
}

/// The bucket size (seconds, a whole number of minutes) that fits `from..to` into at most `max` points.
pub fn bucket(from: i64, to: i64, max: u32) -> i64 {
    let span = (to - from).max(60);
    let per = (span as f64 / max.max(1) as f64 / 60.0).ceil() as i64;
    per.max(1) * 60
}

/// Points between `from` and `to` (Unix seconds), averaged per bucket (the
/// disk takes the bucket's highest, so a nearly full disk isn't smoothed away).
pub fn range(db: &Database, profile_id: &str, from: i64, to: i64, max: u32) -> Result<(i64, Vec<Point>)> {
    let b = bucket(from, to, max);
    let conn = db.conn();
    let mut st = conn.prepare(
        "SELECT (t / ?4) * ?4 AS b, AVG(cpu), AVG(mem), MAX(disk), AVG(load), AVG(rx), AVG(tx), MIN(uptime)
         FROM metric_history WHERE profile_id = ?1 AND t >= ?2 AND t <= ?3 GROUP BY b ORDER BY b",
    )?;
    let v = st
        .query_map(params![profile_id, from, to, b], |r| {
            Ok(Point {
                t: r.get(0)?,
                cpu: r.get::<_, Option<f64>>(1)?.map(round1),
                mem: round1(r.get::<_, Option<f64>>(2)?.unwrap_or(0.0)),
                disk: r.get::<_, Option<f64>>(3)?.map(round1),
                load: (r.get::<_, Option<f64>>(4)?.unwrap_or(0.0) * 100.0).round() / 100.0,
                rx: r.get::<_, Option<f64>>(5)?.map(f64::round),
                tx: r.get::<_, Option<f64>>(6)?.map(f64::round),
                uptime: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((b, v))
}

/// Brings in points kept by older versions (the browser's local storage), once.
pub fn import(db: &Database, profile_id: &str, points: &[Point]) -> Result<usize> {
    let conn = db.conn();
    let mut n = 0;
    for p in points.iter().take(50_000) {
        n += conn.execute(
            "INSERT OR IGNORE INTO metric_history (profile_id, t, cpu, mem, disk, load, rx, tx, uptime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![profile_id, p.t - p.t % 60, p.cpu, p.mem, p.disk, p.load, p.rx, p.tx, p.uptime],
        )?;
    }
    Ok(n)
}

pub fn forget(db: &Database, profile_id: &str) -> Result<()> {
    db.conn()
        .execute("DELETE FROM metric_history WHERE profile_id = ?1", [profile_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: i64, cpu: f64, disk: f64) -> Point {
        Point {
            t,
            cpu: Some(cpu),
            mem: 50.0,
            disk: Some(disk),
            load: 0.5,
            rx: Some(1000.0),
            tx: None,
            uptime: Some(t),
        }
    }

    #[test]
    fn buckets_fit_the_range() {
        assert_eq!(bucket(0, 3600, 720), 60, "an hour: every minute");
        assert_eq!(bucket(0, 86_400, 720), 120);
        assert_eq!(bucket(0, 30 * 86_400, 720), 3600);
        assert_eq!(bucket(100, 100, 720), 60);
    }

    #[test]
    fn records_reads_back_averaged_and_prunes() {
        let db = Database::in_memory().unwrap();
        let base: i64 = 1_800_000_000 / 3600 * 3600; // on the hour
        for i in 0..10 {
            record(&db, "a", &p(base + 60 + i * 60, i as f64 * 10.0, 40.0 + i as f64)).unwrap();
        }
        record(&db, "b", &p(base + 60, 99.0, 99.0)).unwrap();
        // The same minute again replaces it.
        record(&db, "a", &p(base + 60, 5.0, 40.0)).unwrap();
        let (b, all) = range(&db, "a", base, base + 3600, 720).unwrap();
        assert_eq!(b, 60);
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].cpu, Some(5.0));
        let (b, five) = range(&db, "a", base, base + 3600, 12).unwrap();
        assert_eq!(b, 300);
        assert_eq!(five[0].t, base);
        assert_eq!(five[0].disk, Some(43.0), "a bucket keeps its fullest disk");
        assert_eq!(five[1].cpu, Some(60.0), "minutes 5-9: (40+50+60+70+80)/5");
        // A point on the hour drops what's older than 30 days.
        record(&db, "a", &p(base + KEEP_SECS + 3600, 1.0, 1.0)).unwrap();
        let (_, left) = range(&db, "a", 0, i64::MAX, 720).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(range(&db, "b", 0, i64::MAX, 720).unwrap().1.len(), 0, "pruning covers every server");
    }

    #[test]
    fn imports_old_points_once() {
        let db = Database::in_memory().unwrap();
        let pts = vec![p(600, 1.0, 1.0), p(661, 2.0, 2.0)];
        assert_eq!(import(&db, "a", &pts).unwrap(), 2);
        assert_eq!(import(&db, "a", &pts).unwrap(), 0);
        assert_eq!(range(&db, "a", 0, 10_000, 720).unwrap().1[1].t, 660, "aligned to the minute");
        forget(&db, "a").unwrap();
        assert!(range(&db, "a", 0, 10_000, 720).unwrap().1.is_empty());
    }

    #[test]
    fn keeps_moments_and_disk_growth() {
        let db = Database::in_memory().unwrap();
        let gb = 1u64 << 30;
        for h in 0..13 {
            let m = Moment {
                t: h * 3600,
                procs: vec![ProcSample {
                    name: "postgres".into(),
                    user: "postgres".into(),
                    pid: 42,
                    cpu: 30.0,
                    mem: 12.5,
                    rss_kb: 1000,
                }],
                disks: vec![
                    DiskSample {
                        mount: "/".into(),
                        used: 50 * gb + h as u64 * gb,
                        total: 100 * gb,
                    },
                    DiskSample {
                        mount: "/data".into(),
                        used: 10 * gb,
                        total: 100 * gb,
                    },
                ],
            };
            record_moment(&db, "a", &m).unwrap();
        }
        let near = moment_at(&db, "a", 7200 + 100, 300).unwrap().unwrap();
        assert_eq!(near.t, 7200);
        assert_eq!(near.procs[0].name, "postgres");
        assert!(moment_at(&db, "a", 7200 + 1800, 300).unwrap().is_none(), "nothing that close");
        let series = disk_series(&db, "a", 0, 13 * 3600, 720).unwrap();
        assert_eq!(series.len(), 2);
        let root = &series[0];
        assert_eq!(root.mount, "/");
        assert_eq!(root.per_day, Some(24.0 * gb as f64), "1 GB an hour");
        // 62 GB used at 12:00, 38 GB left at 1 GB an hour: full 38 hours later.
        assert_eq!(root.full_at, Some(12 * 3600 + 38 * 3600));
        assert_eq!(series[1].per_day, Some(0.0));
        assert!(series[1].full_at.is_none());
    }

    #[test]
    fn events_come_from_the_audit_log_and_restarts() {
        use crate::audit::{record as audit, Category, NewEvent, Outcome};
        let db = Database::in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let ev = |cat, action: &str, outcome| NewEvent {
            profile_id: Some("a".into()),
            category: Some(cat),
            action: action.into(),
            target: Some("nginx".into()),
            outcome: Some(outcome),
            ..Default::default()
        };
        audit(&db, ev(Category::Service, "service.restart", Outcome::Success)).unwrap();
        audit(&db, ev(Category::Config, "config.save", Outcome::Failure)).unwrap();
        audit(&db, ev(Category::File, "file.upload", Outcome::Success)).unwrap();
        audit(&db, ev(Category::Service, "service.restart", Outcome::Denied)).unwrap();
        audit(
            &db,
            NewEvent {
                profile_id: Some("b".into()),
                ..ev(Category::Docker, "docker.restart", Outcome::Success)
            },
        )
        .unwrap();
        // Uptime 5000 s, then 60 s a minute later: it restarted.
        record(
            &db,
            "a",
            &Point {
                uptime: Some(5000),
                ..p(now - now % 60 - 120, 1.0, 1.0)
            },
        )
        .unwrap();
        record(
            &db,
            "a",
            &Point {
                uptime: Some(60),
                ..p(now - now % 60 - 60, 1.0, 1.0)
            },
        )
        .unwrap();
        let e = events(&db, "a", now - 3600, now + 60).unwrap();
        let kinds: Vec<(&str, &str, bool)> = e.iter().map(|x| (x.kind.as_str(), x.action.as_str(), x.ok)).collect();
        assert!(kinds.contains(&("service", "service.restart", true)), "{kinds:?}");
        assert!(kinds.contains(&("config", "config.save", false)));
        assert!(kinds.contains(&("restart", "server.restart", true)));
        assert!(
            !kinds.iter().any(|k| k.1 == "file.upload" || k.1 == "docker.restart"),
            "files and other servers left out"
        );
        assert_eq!(
            kinds.iter().filter(|k| k.1 == "service.restart").count(),
            1,
            "refused ones left out"
        );
        assert!(events(&db, "a", 0, 100).unwrap().is_empty());
    }

    #[test]
    fn a_moment_keeps_program_names_only() {
        let snap = crate::metrics::parse_snapshot(
            "@@ps\n 812 www-data 40.0 2.0 9000 nginx\n 900 app 1.0 30.0 500000 mysqld --password=hunter2\n 901 app 0.5 0.1 100 sleep\n",
        );
        let m = crate::metrics::derive(None, snap, 1_800_000_030_000);
        let mo = moment(&m);
        assert_eq!(mo.t, 1_800_000_000);
        assert_eq!(mo.procs[0].name, "nginx", "busiest first");
        assert!(mo.procs.iter().any(|p| p.name == "mysqld"), "and the biggest by memory");
        let json = serde_json::to_string(&mo).unwrap();
        assert!(!json.contains("hunter2"), "never arguments: {json}");
    }
}
