//! Deploying an app that lives in a git checkout on the server: fetch and
//! fast-forward its branch, run its build commands, restart it. Every run is
//! kept (who, from which commit to which, outcome, output), and any earlier
//! commit can be deployed again (rollback).
//!
//! Paths, branches and commits are validated and quoted. Build and restart
//! commands are the user's own and run as written, in the app's folder, as the
//! login user (restarting a service uses sudo like the Services tab).

use crate::db::Database;
use crate::error::{Error, Result};
use crate::shell::{quote, validate_path, validate_unit};
use crate::ssh::{ExecOutput, SshSession};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RestartKind {
    None,
    /// `systemctl restart <unit>`, with the Services tab's pre-flight checks.
    Service,
    /// `docker compose up -d` in the app's folder.
    Compose,
    /// Any command, run in the app's folder.
    Command,
}

impl RestartKind {
    fn as_str(self) -> &'static str {
        match self {
            RestartKind::None => "none",
            RestartKind::Service => "service",
            RestartKind::Compose => "compose",
            RestartKind::Command => "command",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "service" => RestartKind::Service,
            "compose" => RestartKind::Compose,
            "command" => RestartKind::Command,
            _ => RestartKind::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppInput {
    pub name: String,
    pub path: String,
    pub branch: String,
    /// Shell commands, one per line, run in order in the app's folder.
    pub build: String,
    pub restart_kind: RestartKind,
    /// The unit (service), or the command (command).
    pub restart_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct App {
    pub id: String,
    pub profile_id: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub build: String,
    pub restart_kind: RestartKind,
    pub restart_target: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeployRecord {
    pub id: String,
    pub app_id: String,
    /// `deploy` or `rollback`.
    pub kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub from_commit: Option<String>,
    pub to_commit: Option<String>,
    /// `running`, `success` or `failure`.
    pub outcome: String,
    /// The new commits, or why it failed.
    pub summary: Option<String>,
}

static BRANCH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._/\-]{0,99}$").unwrap());
static COMMIT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9a-f]{7,40}$").unwrap());

pub fn validate_branch(b: &str) -> Result<&str> {
    if BRANCH_RE.is_match(b) && !b.contains("..") && !b.ends_with(".lock") {
        Ok(b)
    } else {
        Err(Error::InvalidInput(format!("{b:?} is not a branch name")))
    }
}

pub fn validate_commit(c: &str) -> Result<&str> {
    if COMMIT_RE.is_match(c) {
        Ok(c)
    } else {
        Err(Error::InvalidInput(format!("{c:?} is not a commit id")))
    }
}

pub fn validate(input: &AppInput) -> Result<()> {
    if input.name.trim().is_empty() || input.name.len() > 80 {
        return Err(Error::InvalidInput("give the app a name (up to 80 characters)".into()));
    }
    validate_path(&input.path)?;
    validate_branch(&input.branch)?;
    if input.build.contains('\0') || input.build.len() > 20_000 {
        return Err(Error::InvalidInput("the build commands are too long".into()));
    }
    match (input.restart_kind, input.restart_target.as_deref().map(str::trim)) {
        (RestartKind::Service, Some(u)) => {
            validate_unit(u)?;
        }
        (RestartKind::Service, None) => return Err(Error::InvalidInput("choose the service to restart".into())),
        (RestartKind::Command, Some(c)) if !c.is_empty() && !c.contains('\n') => {}
        (RestartKind::Command, _) => return Err(Error::InvalidInput("enter the restart command, on one line".into())),
        _ => {}
    }
    Ok(())
}

// ---------- storage ----------

fn app_row(r: &Row) -> rusqlite::Result<App> {
    Ok(App {
        id: r.get("id")?,
        profile_id: r.get("profile_id")?,
        name: r.get("name")?,
        path: r.get("path")?,
        branch: r.get("branch")?,
        build: r.get("build")?,
        restart_kind: RestartKind::parse(&r.get::<_, String>("restart_kind")?),
        restart_target: r.get("restart_target")?,
        created_at: r.get("created_at")?,
    })
}

pub fn list_apps(db: &Database, profile_id: &str) -> Result<Vec<App>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM apps WHERE profile_id = ?1 ORDER BY name")?;
    let rows = st.query_map([profile_id], app_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get_app(db: &Database, id: &str) -> Result<App> {
    db.conn()
        .query_row("SELECT * FROM apps WHERE id = ?1", [id], app_row)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("app {id}")))
}

pub fn save_app(db: &Database, profile_id: &str, id: Option<&str>, input: &AppInput) -> Result<App> {
    validate(input)?;
    let target = input.restart_target.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let id = match id {
        Some(id) => {
            let n = db.conn().execute(
                "UPDATE apps SET name=?2, path=?3, branch=?4, build=?5, restart_kind=?6, restart_target=?7 WHERE id=?1 AND profile_id=?8",
                params![
                    id,
                    input.name.trim(),
                    input.path,
                    input.branch,
                    input.build,
                    input.restart_kind.as_str(),
                    target,
                    profile_id
                ],
            )?;
            if n == 0 {
                return Err(Error::NotFound(format!("app {id}")));
            }
            id.to_string()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            db.conn().execute(
                "INSERT INTO apps(id, profile_id, name, path, branch, build, restart_kind, restart_target, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, profile_id, input.name.trim(), input.path, input.branch, input.build, input.restart_kind.as_str(), target, chrono::Utc::now().to_rfc3339()],
            )?;
            id
        }
    };
    get_app(db, &id)
}

pub fn delete_app(db: &Database, id: &str) -> Result<()> {
    db.conn().execute("DELETE FROM apps WHERE id = ?1", [id])?;
    Ok(())
}

pub fn history(db: &Database, app_id: &str, limit: u32) -> Result<Vec<DeployRecord>> {
    let conn = db.conn();
    let mut st = conn.prepare(
        "SELECT id, app_id, kind, started_at, finished_at, from_commit, to_commit, outcome, summary FROM deploys WHERE app_id = ?1 ORDER BY started_at DESC LIMIT ?2",
    )?;
    let rows = st.query_map(params![app_id, limit], |r| {
        Ok(DeployRecord {
            id: r.get(0)?,
            app_id: r.get(1)?,
            kind: r.get(2)?,
            started_at: r.get(3)?,
            finished_at: r.get(4)?,
            from_commit: r.get(5)?,
            to_commit: r.get(6)?,
            outcome: r.get(7)?,
            summary: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn record_start(db: &Database, app_id: &str, kind: &str, from: Option<&str>) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    db.conn().execute(
        "INSERT INTO deploys(id, app_id, kind, started_at, from_commit, outcome) VALUES (?1,?2,?3,?4,?5,'running')",
        params![id, app_id, kind, chrono::Utc::now().to_rfc3339(), from],
    )?;
    Ok(id)
}

pub fn record_finish(db: &Database, id: &str, ok: bool, to: Option<&str>, summary: &str, log: &str) -> Result<()> {
    db.conn().execute(
        "UPDATE deploys SET finished_at=?2, to_commit=?3, outcome=?4, summary=?5, log=?6 WHERE id=?1",
        params![
            id,
            chrono::Utc::now().to_rfc3339(),
            to,
            if ok { "success" } else { "failure" },
            summary,
            log
        ],
    )?;
    Ok(())
}

pub fn deploy_log(db: &Database, id: &str) -> Result<String> {
    Ok(db
        .conn()
        .query_row("SELECT log FROM deploys WHERE id = ?1", [id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten()
        .unwrap_or_default())
}

// ---------- commands ----------

/// Where the checkout is: prints `@@head`, `@@branch`, and `@@dirty` for each
/// changed tracked file. Exit 3: no such folder; 4: not a git checkout.
pub fn inspect_script(path: &str) -> Result<String> {
    Ok(format!(
        "export LC_ALL=C; cd -- {} 2>/dev/null || exit 3; git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 4; \
         echo \"@@head $(git rev-parse HEAD)\"; echo \"@@branch $(git rev-parse --abbrev-ref HEAD)\"; \
         echo \"@@remote $(git remote get-url origin 2>/dev/null)\"; \
         git status --porcelain --untracked-files=no | head -n 20 | sed 's/^/@@dirty /'; \
         git log -n 15 --format='@@commit %H%x09%h%x09%an%x09%cr%x09%s'",
        quote(validate_path(path)?)
    ))
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub id: String,
    pub short: String,
    pub author: String,
    pub when: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Checkout {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub remote: Option<String>,
    /// Changed tracked files: a deploy would refuse to overwrite them.
    pub dirty: Vec<String>,
    pub recent: Vec<Commit>,
}

pub fn parse_checkout(out: &str) -> Checkout {
    let mut c = Checkout::default();
    for l in out.lines() {
        let some = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
        if let Some(v) = l.strip_prefix("@@head ") {
            c.head = some(v);
        } else if let Some(v) = l.strip_prefix("@@branch ") {
            c.branch = some(v);
        } else if let Some(v) = l.strip_prefix("@@remote") {
            c.remote = some(v);
        } else if let Some(v) = l.strip_prefix("@@dirty ") {
            c.dirty.push(v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("@@commit ") {
            let f: Vec<&str> = v.splitn(5, '\t').collect();
            if f.len() == 5 {
                c.recent.push(Commit {
                    id: f[0].into(),
                    short: f[1].into(),
                    author: f[2].into(),
                    when: f[3].into(),
                    subject: f[4].into(),
                });
            }
        }
    }
    c
}

/// Bring the checkout to the newest commit of the branch (deploy) or to
/// `commit` (rollback). Never discards local changes: a fast-forward only.
pub fn update_command(path: &str, branch: &str, commit: Option<&str>) -> Result<String> {
    let p = quote(validate_path(path)?);
    Ok(match commit {
        None => {
            let b = quote(validate_branch(branch)?);
            format!("cd -- {p} && git fetch --prune origin {b} && git checkout -q {b} && git merge --ff-only FETCH_HEAD")
        }
        Some(c) => format!(
            "cd -- {p} && git fetch -q origin 2>/dev/null; git checkout -q --detach {}",
            validate_commit(c)?
        ),
    })
}

pub fn changes_command(path: &str, from: &str, to: &str) -> Result<String> {
    Ok(format!(
        "cd -- {} && git log --oneline --no-decorate {}..{} | head -n 30",
        quote(validate_path(path)?),
        validate_commit(from)?,
        validate_commit(to)?
    ))
}

pub fn head_command(path: &str) -> Result<String> {
    Ok(format!("cd -- {} && git rev-parse HEAD", quote(validate_path(path)?)))
}

/// The build, one command per non-empty, non-comment line, each in the app's folder.
pub fn build_steps(path: &str, build: &str) -> Result<Vec<(String, String)>> {
    let p = quote(validate_path(path)?);
    Ok(build
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| (l.to_string(), format!("cd -- {p} && {l}")))
        .collect())
}

pub fn compose_command(path: &str) -> Result<String> {
    Ok(format!("cd -- {} && docker compose up -d", quote(validate_path(path)?)))
}

pub fn custom_restart_command(path: &str, cmd: &str) -> Result<String> {
    Ok(format!("cd -- {} && {}", quote(validate_path(path)?), cmd.trim()))
}

/// Output that means "not allowed as this user" (worth one more try with sudo).
pub fn refused(out: &str) -> bool {
    let t = out.to_lowercase();
    t.contains("permission denied")
        || t.contains("operation not permitted")
        || t.contains("dubious ownership")
        || t.contains("insufficient permission")
}

/// `cmd` for running with sudo in a checkout owned by someone else: git
/// would refuse it ("dubious ownership"), so the folder is trusted for this
/// one command only (environment, not a config change).
pub fn as_root(cmd: &str) -> String {
    format!("export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0='*'; {cmd}")
}

impl SshSession {
    /// A read in the app's folder: as the login user, again with sudo if refused.
    pub async fn deploy_read(&self, cmd: &str) -> Result<ExecOutput> {
        let o = self.exec(cmd).await?;
        if !o.ok() && refused(&format!("{}{}", o.stdout, o.stderr)) && self.elevation_needed() {
            if let Ok(r) = self.exec_privileged(&as_root(cmd)).await {
                return Ok(r);
            }
        }
        Ok(o)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn refusals_and_root_retry() {
        use super::*;
        assert!(refused("fatal: detected dubious ownership in repository at '/srv/app'"));
        assert!(refused("error: cannot open .git/FETCH_HEAD: Permission denied"));
        assert!(refused("npm ERR! Error: EACCES: permission denied, mkdir '/srv/app/node_modules'"));
        assert!(!refused("fatal: couldn't find remote ref main"));
        assert!(as_root("git -C /srv/app fetch").starts_with("export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory"));
        let o = std::process::Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(as_root("true"))
            .output()
            .unwrap();
        assert!(o.status.success());
    }

    use super::*;

    fn input() -> AppInput {
        AppInput {
            name: "Shop".into(),
            path: "/srv/shop".into(),
            branch: "main".into(),
            build: "# install\nnpm ci\n\nnpm run build\n".into(),
            restart_kind: RestartKind::Service,
            restart_target: Some("shop.service".into()),
        }
    }

    #[test]
    fn stores_apps_and_history() {
        let db = Database::in_memory().unwrap();
        let a = save_app(&db, "p1", None, &input()).unwrap();
        assert_eq!(list_apps(&db, "p1").unwrap().len(), 1);
        assert!(list_apps(&db, "p2").unwrap().is_empty());
        let a2 = save_app(
            &db,
            "p1",
            Some(&a.id),
            &AppInput {
                branch: "release/2.4".into(),
                ..input()
            },
        )
        .unwrap();
        assert_eq!(
            (a2.id.as_str(), a2.branch.as_str(), a2.restart_kind),
            (a.id.as_str(), "release/2.4", RestartKind::Service)
        );
        assert!(
            save_app(&db, "other-profile", Some(&a.id), &input()).is_err(),
            "another server's app can't be changed"
        );

        let run = record_start(&db, &a.id, "deploy", Some("aaaaaaa")).unwrap();
        record_finish(&db, &run, true, Some("bbbbbbb"), "2 new commits", "log text").unwrap();
        let h = history(&db, &a.id, 10).unwrap();
        assert_eq!((h[0].outcome.as_str(), h[0].to_commit.as_deref()), ("success", Some("bbbbbbb")));
        assert_eq!(deploy_log(&db, &run).unwrap(), "log text");
        delete_app(&db, &a.id).unwrap();
        assert!(get_app(&db, &a.id).is_err());
    }

    #[test]
    fn validates_input() {
        assert!(validate(&input()).is_ok());
        for bad in [
            AppInput {
                path: "srv/shop".into(),
                ..input()
            },
            AppInput {
                branch: "-x".into(),
                ..input()
            },
            AppInput {
                branch: "main; reboot".into(),
                ..input()
            },
            AppInput {
                branch: "a..b".into(),
                ..input()
            },
            AppInput {
                restart_target: Some("x; reboot".into()),
                ..input()
            },
            AppInput {
                restart_target: None,
                ..input()
            },
            AppInput {
                restart_kind: RestartKind::Command,
                restart_target: Some("a\nb".into()),
                ..input()
            },
            AppInput {
                name: " ".into(),
                ..input()
            },
        ] {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        assert!(validate_commit("deadbeef").is_ok());
        assert!(validate_commit("HEAD~1").is_err());
    }

    #[test]
    fn builds_commands() {
        assert_eq!(
            update_command("/srv/my shop", "main", None).unwrap(),
            "cd -- '/srv/my shop' && git fetch --prune origin main && git checkout -q main && git merge --ff-only FETCH_HEAD"
        );
        assert!(update_command("/srv/shop", "main", Some("0123abcd"))
            .unwrap()
            .ends_with("git checkout -q --detach 0123abcd"));
        let steps = build_steps("/srv/shop", &input().build).unwrap();
        assert_eq!(steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["npm ci", "npm run build"]);
        assert_eq!(steps[0].1, "cd -- /srv/shop && npm ci");
        assert!(changes_command("/srv/shop", "aaaaaaa", "$(id)").is_err());
    }

    #[test]
    fn reads_the_checkout() {
        let c = parse_checkout("@@head 0123456789abcdef0123456789abcdef01234567\n@@branch main\n@@remote git@github.com:acme/shop.git\n@@dirty  M config/app.php\n@@commit 0123456789abcdef0123456789abcdef01234567\t0123456\tAnna\t2 hours ago\tFix checkout total\n");
        assert_eq!(c.branch.as_deref(), Some("main"));
        assert_eq!(c.dirty, ["M config/app.php"]);
        assert_eq!(c.recent[0].subject, "Fix checkout total");
        assert_eq!(c.remote.as_deref(), Some("git@github.com:acme/shop.git"));
    }
}
