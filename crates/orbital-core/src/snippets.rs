//! Snippets: saved commands, run on one server or many at once.
//!
//! A command may have parameters written `{{name}}`. Values are filled in at
//! run time and each one goes through `shell::quote`, so a value is always one
//! argument and can never become a second command. Every run is audited per
//! server with the exact command that was sent.

use crate::db::Database;
use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::ExecOutput;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

static PARAM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]{0,31})\s*\}\}").unwrap());

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    pub description: Option<String>,
    /// Run through sudo (as root).
    pub sudo: bool,
    /// The `{{…}}` names in the command, in order of first use.
    pub params: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnippetInput {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sudo: bool,
}

/// The result on one server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetRun {
    pub session_id: String,
    pub server: String,
    /// The command as sent (parameters filled in and quoted).
    pub command: String,
    pub output: Option<ExecOutput>,
    pub error: Option<String>,
}

pub fn params(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in PARAM.captures_iter(command) {
        let n = c[1].to_string();
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// Fill in the parameters, each value quoted as one shell word.
pub fn render(command: &str, values: &HashMap<String, String>) -> Result<String> {
    let mut missing = Vec::new();
    for p in params(command) {
        if values.get(&p).is_none_or(|v| v.is_empty()) {
            missing.push(p);
        }
    }
    if !missing.is_empty() {
        return Err(Error::InvalidInput(format!("fill in {}", missing.join(", "))));
    }
    if values.values().any(|v| v.contains('\0')) {
        return Err(Error::InvalidInput("a value contains a NUL character".into()));
    }
    Ok(PARAM.replace_all(command, |c: &regex::Captures| quote(&values[&c[1]])).into_owned())
}

pub fn validate(input: &SnippetInput) -> Result<SnippetInput> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 100 || name.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidInput("give the snippet a short name".into()));
    }
    let command = input.command.trim_end();
    if command.trim().is_empty() || command.len() > 10_000 || command.contains('\0') {
        return Err(Error::InvalidInput("the command is empty or too long".into()));
    }
    // An unclosed placeholder is almost certainly a typo.
    let stray = PARAM.replace_all(command, "");
    if stray.contains("{{") {
        return Err(Error::InvalidInput(
            "a {{parameter}} isn't written right: use letters, digits and _ between {{ and }}".into(),
        ));
    }
    Ok(SnippetInput {
        name: name.to_string(),
        command: command.to_string(),
        description: input.description.as_ref().map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
        sudo: input.sudo,
    })
}

/// Useful starting points, offered when there are no snippets yet.
pub fn examples() -> Vec<SnippetInput> {
    let s = |name: &str, command: &str, description: &str, sudo: bool| SnippetInput {
        name: name.into(),
        command: command.into(),
        description: Some(description.into()),
        sudo,
    };
    vec![
        s("Disk space", "df -h -x tmpfs -x devtmpfs -x overlay -x squashfs", "Free space on every real disk.", false),
        s("Biggest folders", "du -xh --max-depth=1 {{folder}} 2>/dev/null | sort -rh | head -15", "What takes the space in a folder.", true),
        s("Memory and swap", "free -h && echo && ps -eo pid,user,rss,comm --sort=-rss | head -11", "Memory use and the ten largest processes.", false),
        s("Failed services", "systemctl --failed --no-legend --plain", "systemd units that have failed.", false),
        s("Restart a service", "systemctl restart {{service}} && systemctl --no-pager status {{service}} | head -12", "Restart a unit and show its status.", true),
        s("Recent errors", "journalctl -p err --since {{since}} --no-pager | tail -40", "Errors in the system journal, e.g. since \"1 hour ago\".", true),
        s("Pending updates", "(command -v apt >/dev/null && apt list --upgradable 2>/dev/null) || (command -v dnf >/dev/null && dnf -q check-update) || echo 'no apt or dnf'", "Packages with updates available.", false),
        s("Who is logged in", "who && echo && last -n 10", "Current sessions and the last logins.", false),
    ]
}

// ---------- storage ----------

fn row(r: &rusqlite::Row) -> rusqlite::Result<Snippet> {
    let command: String = r.get("command")?;
    Ok(Snippet {
        id: r.get("id")?,
        name: r.get("name")?,
        params: params(&command),
        command,
        description: r.get("description")?,
        sudo: r.get::<_, i64>("sudo")? != 0,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

pub fn list(db: &Database) -> Result<Vec<Snippet>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM snippets ORDER BY name COLLATE NOCASE")?;
    let rows = st.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get(db: &Database, id: &str) -> Result<Snippet> {
    use rusqlite::OptionalExtension;
    db.conn()
        .query_row("SELECT * FROM snippets WHERE id = ?1", [id], row)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("snippet {id}")))
}

/// Create (`id` None) or replace a snippet.
pub fn save(db: &Database, id: Option<&str>, input: &SnippetInput) -> Result<Snippet> {
    let i = validate(input)?;
    let now = chrono::Utc::now().to_rfc3339();
    let id = match id {
        Some(id) => {
            let n = db.conn().execute(
                "UPDATE snippets SET name = ?2, command = ?3, description = ?4, sudo = ?5, updated_at = ?6 WHERE id = ?1",
                rusqlite::params![id, i.name, i.command, i.description, i.sudo as i64, now],
            )?;
            if n == 0 {
                return Err(Error::NotFound(format!("snippet {id}")));
            }
            id.to_string()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            db.conn().execute(
                "INSERT INTO snippets (id, name, command, description, sudo, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                rusqlite::params![id, i.name, i.command, i.description, i.sudo as i64, now],
            )?;
            id
        }
    };
    get(db, &id)
}

pub fn delete(db: &Database, id: &str) -> Result<()> {
    db.conn().execute("DELETE FROM snippets WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(kv: &[(&str, &str)]) -> HashMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn finds_parameters_once_in_order() {
        assert_eq!(
            params("systemctl restart {{service}} && journalctl -u {{ service }} --since {{since}}"),
            ["service", "since"]
        );
        assert!(params("echo {{}} {{1x}} {not}").is_empty());
    }

    #[test]
    fn values_are_always_one_quoted_word() {
        let cmd = "systemctl restart {{service}}";
        assert_eq!(render(cmd, &vals(&[("service", "nginx")])).unwrap(), "systemctl restart nginx");
        let evil = render(cmd, &vals(&[("service", "x; rm -rf / #")])).unwrap();
        assert_eq!(evil, "systemctl restart 'x; rm -rf / #'");
        assert_eq!(render("echo {{a}}", &vals(&[("a", "it's")])).unwrap(), r#"echo 'it'\''s'"#);
        assert!(matches!(render(cmd, &vals(&[])), Err(Error::InvalidInput(m)) if m.contains("service")));
        assert!(render(cmd, &vals(&[("service", "")])).is_err());
        assert_eq!(render("uptime", &vals(&[])).unwrap(), "uptime");
    }

    #[test]
    fn validates_snippets() {
        let ok = SnippetInput {
            name: " Disk ".into(),
            command: "df -h  \n".into(),
            description: Some("  ".into()),
            sudo: false,
        };
        let v = validate(&ok).unwrap();
        assert_eq!((v.name.as_str(), v.command.as_str(), v.description), ("Disk", "df -h", None));
        assert!(validate(&SnippetInput {
            name: "".into(),
            ..ok.clone()
        })
        .is_err());
        assert!(validate(&SnippetInput {
            command: "  ".into(),
            ..ok.clone()
        })
        .is_err());
        assert!(validate(&SnippetInput {
            command: "echo {{bad name}}".into(),
            ..ok.clone()
        })
        .is_err());
        assert!(validate(&SnippetInput {
            command: "echo {{ok}}".into(),
            ..ok
        })
        .is_ok());
        for e in examples() {
            assert!(validate(&e).is_ok(), "{}", e.name);
        }
    }

    #[test]
    fn stores_snippets() {
        let db = Database::in_memory().unwrap();
        let input = SnippetInput {
            name: "Restart".into(),
            command: "systemctl restart {{service}}".into(),
            description: None,
            sudo: true,
        };
        let s = save(&db, None, &input).unwrap();
        assert_eq!(s.params, ["service"]);
        assert!(s.sudo);
        let changed = save(
            &db,
            Some(&s.id),
            &SnippetInput {
                name: "Restart a unit".into(),
                ..input.clone()
            },
        )
        .unwrap();
        assert_eq!(changed.name, "Restart a unit");
        assert_eq!(list(&db).unwrap().len(), 1);
        assert!(matches!(save(&db, Some("nope"), &input), Err(Error::NotFound(_))));
        delete(&db, &s.id).unwrap();
        assert!(list(&db).unwrap().is_empty());
    }
}
