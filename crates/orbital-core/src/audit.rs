//! Tamper-evident audit trail. Every state-changing action (and every
//! connection) is recorded with who/what/where/result. Rows are hash-chained:
//! `hash = SHA-256(prev_hash || canonical(row))`, and SQLite triggers reject
//! UPDATE/DELETE. `verify()` re-computes the chain and pin-points the first
//! altered row.

use crate::db::Database;
use crate::error::Result;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Connection,
    File,
    Config,
    Service,
    Docker,
    Terminal,
    Command,
    Security,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Success,
    Failure,
    Denied,
}

#[derive(Debug, Clone, Default)]
pub struct NewEvent {
    pub profile_id: Option<String>,
    pub server: Option<String>,
    pub category: Option<Category>,
    pub action: String,
    pub target: Option<String>,
    pub command: Option<String>,
    pub outcome: Option<Outcome>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub seq: i64,
    pub id: String,
    pub ts: String,
    pub profile_id: Option<String>,
    pub server: Option<String>,
    pub actor: String,
    pub category: Category,
    pub action: String,
    pub target: Option<String>,
    pub command: Option<String>,
    pub outcome: Outcome,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub details: Option<serde_json::Value>,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFilter {
    pub profile_id: Option<String>,
    pub category: Option<Category>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub before_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub ok: bool,
    pub checked: i64,
    pub first_broken_seq: Option<i64>,
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// The local user performing the action (shown as "actor").
pub fn local_actor() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

fn canonical(e: &AuditEvent) -> String {
    // Stable, explicit field order — never depends on serde map ordering.
    serde_json::json!([
        e.id,
        e.ts,
        e.profile_id,
        e.server,
        e.actor,
        enum_str(&e.category),
        e.action,
        e.target,
        e.command,
        enum_str(&e.outcome),
        e.exit_code,
        e.duration_ms,
        e.details.as_ref().map(|d| d.to_string()),
    ])
    .to_string()
}

fn compute_hash(prev: &str, e: &AuditEvent) -> String {
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(canonical(e).as_bytes());
    hex::encode(h.finalize())
}

pub fn record(db: &Database, ev: NewEvent) -> Result<AuditEvent> {
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    let prev: String = tx
        .query_row("SELECT hash FROM audit_log ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
        .optional()?
        .unwrap_or_else(|| GENESIS.to_string());
    let mut e = AuditEvent {
        seq: 0,
        id: uuid::Uuid::new_v4().to_string(),
        ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        profile_id: ev.profile_id,
        server: ev.server,
        actor: local_actor(),
        category: ev.category.unwrap_or(Category::Command),
        action: ev.action,
        target: ev.target,
        command: ev.command,
        outcome: ev.outcome.unwrap_or(Outcome::Success),
        exit_code: ev.exit_code,
        duration_ms: ev.duration_ms,
        details: ev.details,
        prev_hash: prev.clone(),
        hash: String::new(),
    };
    e.hash = compute_hash(&prev, &e);
    tx.execute(
        "INSERT INTO audit_log (id, ts, profile_id, server, actor, category, action, target, command, outcome, exit_code, duration_ms, details, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            e.id,
            e.ts,
            e.profile_id,
            e.server,
            e.actor,
            enum_str(&e.category),
            e.action,
            e.target,
            e.command,
            enum_str(&e.outcome),
            e.exit_code,
            e.duration_ms,
            e.details.as_ref().map(|d| d.to_string()),
            e.prev_hash,
            e.hash
        ],
    )?;
    e.seq = tx.last_insert_rowid();
    tx.commit()?;
    Ok(e)
}

fn parse_enum<T: for<'de> Deserialize<'de>>(s: String) -> std::result::Result<T, serde_json::Error> {
    serde_json::from_value(serde_json::Value::String(s))
}

fn from_row(r: &Row) -> rusqlite::Result<AuditEvent> {
    let details: Option<String> = r.get("details")?;
    Ok(AuditEvent {
        seq: r.get("seq")?,
        id: r.get("id")?,
        ts: r.get("ts")?,
        profile_id: r.get("profile_id")?,
        server: r.get("server")?,
        actor: r.get("actor")?,
        category: parse_enum(r.get("category")?).unwrap_or(Category::Command),
        action: r.get("action")?,
        target: r.get("target")?,
        command: r.get("command")?,
        outcome: parse_enum(r.get("outcome")?).unwrap_or(Outcome::Failure),
        exit_code: r.get("exit_code")?,
        duration_ms: r.get("duration_ms")?,
        details: details.and_then(|d| serde_json::from_str(&d).ok()),
        prev_hash: r.get("prev_hash")?,
        hash: r.get("hash")?,
    })
}

pub fn query(db: &Database, f: &AuditFilter) -> Result<Vec<AuditEvent>> {
    let mut sql = String::from("SELECT * FROM audit_log WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(p) = &f.profile_id {
        args.push(Box::new(p.clone()));
        sql.push_str(&format!(" AND profile_id = ?{}", args.len()));
    }
    if let Some(c) = &f.category {
        args.push(Box::new(enum_str(c)));
        sql.push_str(&format!(" AND category = ?{}", args.len()));
    }
    if let Some(s) = f.search.as_ref().filter(|s| !s.trim().is_empty()) {
        args.push(Box::new(format!("%{}%", s.trim())));
        let n = args.len();
        sql.push_str(&format!(
            " AND (action LIKE ?{n} OR target LIKE ?{n} OR command LIKE ?{n} OR server LIKE ?{n})"
        ));
    }
    if let Some(b) = f.before_seq {
        args.push(Box::new(b));
        sql.push_str(&format!(" AND seq < ?{}", args.len()));
    }
    args.push(Box::new(f.limit.unwrap_or(200).clamp(1, 5000)));
    sql.push_str(&format!(" ORDER BY seq DESC LIMIT ?{}", args.len()));
    let conn = db.conn();
    let mut st = conn.prepare(&sql)?;
    let rows = st
        .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn verify(db: &Database) -> Result<VerifyReport> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM audit_log ORDER BY seq ASC")?;
    let mut rows = st.query([])?;
    let mut prev = GENESIS.to_string();
    let mut checked = 0;
    while let Some(r) = rows.next()? {
        let e = from_row(r)?;
        checked += 1;
        if e.prev_hash != prev || compute_hash(&prev, &e) != e.hash {
            return Ok(VerifyReport {
                ok: false,
                checked,
                first_broken_seq: Some(e.seq),
            });
        }
        prev = e.hash;
    }
    Ok(VerifyReport {
        ok: true,
        checked,
        first_broken_seq: None,
    })
}

pub fn export_json(db: &Database) -> Result<String> {
    let all = query(
        db,
        &AuditFilter {
            limit: Some(5000),
            ..Default::default()
        },
    )?;
    Ok(serde_json::to_string_pretty(&all.into_iter().rev().collect::<Vec<_>>())?)
}

pub fn export_csv(db: &Database) -> Result<String> {
    fn esc(s: &str) -> String {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let mut out = String::from("seq,ts,actor,server,category,action,target,command,outcome,exit_code,duration_ms,hash\n");
    for e in query(
        db,
        &AuditFilter {
            limit: Some(5000),
            ..Default::default()
        },
    )?
    .into_iter()
    .rev()
    {
        out.push_str(
            &[
                e.seq.to_string(),
                e.ts,
                e.actor,
                e.server.unwrap_or_default(),
                enum_str(&e.category),
                e.action,
                e.target.unwrap_or_default(),
                e.command.unwrap_or_default(),
                enum_str(&e.outcome),
                e.exit_code.map(|c| c.to_string()).unwrap_or_default(),
                e.duration_ms.map(|c| c.to_string()).unwrap_or_default(),
                e.hash,
            ]
            .iter()
            .map(|s| esc(s))
            .collect::<Vec<_>>()
            .join(","),
        );
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(action: &str) -> NewEvent {
        NewEvent {
            server: Some("deploy@10.0.0.5".into()),
            category: Some(Category::Service),
            action: action.into(),
            target: Some("nginx.service".into()),
            command: Some("systemctl restart -- nginx.service".into()),
            ..Default::default()
        }
    }

    #[test]
    fn chain_and_append_only() {
        let db = Database::in_memory().unwrap();
        let a = record(&db, ev("service.restart")).unwrap();
        let b = record(&db, ev("service.stop")).unwrap();
        assert_eq!(a.prev_hash, GENESIS);
        assert_eq!(b.prev_hash, a.hash);
        assert!(verify(&db).unwrap().ok);

        // UPDATE and DELETE are blocked by triggers.
        assert!(db.conn().execute("UPDATE audit_log SET action='x' WHERE seq=1", []).is_err());
        assert!(db.conn().execute("DELETE FROM audit_log", []).is_err());

        // Simulate an attacker bypassing triggers: verification catches it.
        db.conn()
            .execute_batch("DROP TRIGGER audit_no_update; UPDATE audit_log SET target='apache' WHERE seq=1;")
            .unwrap();
        let r = verify(&db).unwrap();
        assert!(!r.ok);
        assert_eq!(r.first_broken_seq, Some(1));
    }

    #[test]
    fn filter_and_export() {
        let db = Database::in_memory().unwrap();
        record(&db, ev("service.restart")).unwrap();
        record(
            &db,
            NewEvent {
                action: "file.write".into(),
                category: Some(Category::File),
                ..Default::default()
            },
        )
        .unwrap();
        let only = query(
            &db,
            &AuditFilter {
                category: Some(Category::File),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(only.len(), 1);
        let s = query(
            &db,
            &AuditFilter {
                search: Some("nginx".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.len(), 1);
        assert!(export_csv(&db).unwrap().lines().count() == 3);
        assert!(export_json(&db).unwrap().contains("service.restart"));
    }
}
