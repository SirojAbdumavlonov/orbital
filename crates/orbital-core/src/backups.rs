//! Scheduled backups that run on the server, whether or not Orbital is open.
//!
//! A job is a fixed script in /usr/local/lib/orbital/backup-<id>.sh plus a
//! line in /etc/cron.d/orbital-backup-<id>. The script's second line holds the
//! job's settings as JSON, so the server itself is the record of its jobs (and
//! Orbital reads them back from there). Backups go to
//! /var/backups/orbital/<id>/ (root only, 0700), named <id>-YYYYmmdd-HHMMSS.ext;
//! only the newest `keep` are kept, and each run adds a line to backup.log.
//!
//! Everything put into a script is validated and quoted; ids are slugs, so
//! paths built from them are safe. Restores are explicit, recorded actions.

use crate::databases::{validate_db_name, DbKind};
use crate::error::{Error, Result};
use crate::shell::{quote, validate_path};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const SCRIPT_DIR: &str = "/usr/local/lib/orbital";
pub const BACKUP_DIR: &str = "/var/backups/orbital";
pub const CRON_DIR: &str = "/etc/cron.d";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum BackupSource {
    /// A folder, as a .tar.gz.
    Folder { path: String },
    /// A database, with its own dump tool: on the server itself, or inside a
    /// running Docker container (`container`), reached with `docker exec`.
    Database {
        engine: DbKind,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Every {
    Day,
    Week,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub every: Every,
    /// 0 = Sunday … 6 = Saturday (weekly only).
    #[serde(default)]
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupJob {
    /// A slug: lowercase letters, digits and dashes.
    pub id: String,
    pub name: String,
    pub source: BackupSource,
    pub schedule: Schedule,
    /// How many backups to keep (1–90).
    pub keep: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub path: String,
    pub bytes: u64,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub job: BackupJob,
    /// Newest first.
    pub files: Vec<BackupFile>,
    /// The last run: when, and its error if it failed.
    pub last_run: Option<i64>,
    pub last_error: Option<String>,
    /// The cron line is in place.
    pub scheduled: bool,
}

static ID: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,39}$").unwrap());
static FOLDER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^/[A-Za-z0-9._@+\-/ ]*$").unwrap());

/// A slug for an id, from a name ("Shop database" → "shop-database").
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').chars().take(40).collect::<String>();
    if s.is_empty() {
        "backup".into()
    } else {
        s
    }
}

/// The job as it will be saved, or why not.
pub fn validate(j: &BackupJob) -> Result<BackupJob> {
    let bad = |m: String| Err(Error::InvalidInput(m));
    if !ID.is_match(&j.id) {
        return bad("the id has lowercase letters, digits and dashes only".into());
    }
    let name = j.name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(|c| c.is_control()) {
        return bad("give the backup a short name".into());
    }
    let source = match &j.source {
        BackupSource::Folder { path } => {
            let p = path.trim().trim_end_matches('/');
            if !FOLDER.is_match(p) || p.is_empty() || p.split('/').any(|x| x == "..") {
                return bad(format!("{path:?} isn't a folder's full path"));
            }
            if ["/", "/proc", "/sys", "/dev", "/run", "/tmp", BACKUP_DIR].contains(&p) || p.starts_with(&format!("{BACKUP_DIR}/")) {
                return bad(format!("{p} can't be backed up this way; choose the folder with your data"));
            }
            validate_path(p)?;
            BackupSource::Folder { path: p.to_string() }
        }
        BackupSource::Database { engine, name, container } => BackupSource::Database {
            engine: *engine,
            name: validate_db_name(name.trim())?.to_string(),
            container: match container.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(c) => Some(crate::shell::validate_container(c)?.to_string()),
                None => None,
            },
        },
    };
    let s = j.schedule;
    if s.hour > 23 || s.minute > 59 || s.weekday > 6 {
        return bad("the time is hh:mm, and the day Sunday–Saturday".into());
    }
    if !(1..=90).contains(&j.keep) {
        return bad("keep between 1 and 90 backups".into());
    }
    Ok(BackupJob {
        id: j.id.clone(),
        name: name.to_string(),
        source,
        schedule: s,
        keep: j.keep,
    })
}

/// The file extension of a job's backups.
pub fn ext(source: &BackupSource) -> &'static str {
    match source {
        BackupSource::Folder { .. } => "tar.gz",
        BackupSource::Database {
            engine: DbKind::Postgres, ..
        } => "dump",
        BackupSource::Database { engine: DbKind::Mysql, .. } => "sql.gz",
        BackupSource::Database { engine: DbKind::Mongo, .. } => "archive.gz",
    }
}

fn dump_command(source: &BackupSource) -> String {
    match source {
        BackupSource::Folder { path } => {
            let (parent, base) = path.rsplit_once('/').unwrap_or(("", path));
            let parent = if parent.is_empty() { "/" } else { parent };
            format!("tar czf - -C {} -- {}", quote(parent), quote(base))
        }
        BackupSource::Database {
            engine,
            name,
            container: None,
        } => {
            let db = quote(name);
            match engine {
                DbKind::Postgres => format!("as_pg pg_dump -Fc -d {db}"),
                DbKind::Mysql => format!(
                    "D=$(command -v mysqldump || command -v mariadb-dump); ( \"$D\" --single-transaction --quick --routines --triggers --events --databases {db} || echo x > \"$f.failed\" ) | gzip"
                ),
                DbKind::Mongo => format!("mongodump --quiet --archive --gzip --db={db}"),
            }
        }
        BackupSource::Database {
            engine,
            name,
            container: Some(c),
        } => {
            let db = quote(name);
            let inner = match engine {
                DbKind::Postgres => format!("pg_dump -Fc -d {db}"),
                DbKind::Mysql => format!(
                    "D=$(command -v mysqldump || command -v mariadb-dump); \"$D\" --single-transaction --quick --routines --triggers --events --databases {db}"
                ),
                DbKind::Mongo => format!("mongodump --quiet --archive --gzip --db={db}"),
            };
            let exec = in_container(c, &inner, false);
            match engine {
                // The dump fails inside; the marker file is written out here, then compressed here.
                DbKind::Mysql => format!("( {exec} || echo x > \"$f.failed\" ) | gzip"),
                _ => exec,
            }
        }
    }
}

/// `docker exec` of a fixed script inside a container, with the login details
/// the image's variables give (POSTGRES_USER, MYSQL_ROOT_PASSWORD, …).
/// `stdin`: the script reads what's piped in (restores).
fn in_container(container: &str, script: &str, stdin: bool) -> String {
    format!(
        "docker exec {}-- {} sh -c {}",
        if stdin { "-i " } else { "" },
        quote(container),
        quote(&format!("{}{script}", crate::docker::CONTAINER_PRELUDE))
    )
}

/// A read-only check that the dump tool a job needs is on the server: prints
/// `@@missing <tool>` when it isn't (nothing for folders, which only need tar).
pub fn tool_check(source: &BackupSource) -> Option<String> {
    let BackupSource::Database { engine, container, .. } = source else {
        return None;
    };
    let (test, tool) = match engine {
        DbKind::Postgres => ("command -v pg_dump", "pg_dump (PostgreSQL)"),
        DbKind::Mysql => ("command -v mysqldump || command -v mariadb-dump", "mysqldump (MySQL/MariaDB)"),
        DbKind::Mongo => ("command -v mongodump", "mongodump (MongoDB database tools)"),
    };
    Some(match container {
        None => format!("{{ {test}; }} >/dev/null 2>&1 || echo '@@missing {tool}'"),
        Some(c) => format!(
            "docker exec -- {} sh -c {} >/dev/null 2>&1 || echo {}",
            quote(c),
            quote(test),
            quote(&format!("@@missing {tool} in the container {c}"))
        ),
    })
}

/// The job's script, writing into `base`/<id> (BACKUP_DIR on servers; a temp folder in tests).
pub fn script_in(j: &BackupJob, base: &str) -> Result<String> {
    let j = validate(j)?;
    let meta = serde_json::to_string(&j).map_err(|e| Error::Other(e.to_string()))?;
    let id = &j.id;
    let ext = ext(&j.source);
    let dir = quote(&format!("{base}/{id}"));
    Ok(format!(
        r#"#!/bin/sh
# orbital-backup {meta}
# Written by Orbital; runs from {CRON_DIR}/orbital-backup-{id}. Change it in Orbital (Backups).
set -u
umask 077
export LC_ALL=C
d={dir}
mkdir -p "$d" && chmod 700 "$d"
f="$d/{id}-$(date +%Y%m%d-%H%M%S).{ext}"
log() {{ echo "$(date +%s) $*" >> "$d/backup.log"; }}
as_pg() {{ if command -v runuser >/dev/null 2>&1; then runuser -u postgres -- "$@"; else sudo -n -u postgres -- "$@"; fi; }}
rm -f -- "$f.failed"
if ! {{ {dump}; }} > "$f.part" 2> "$d/last-error.txt" || [ -e "$f.failed" ]; then
  log "failed $(head -c 300 "$d/last-error.txt" | tr '\n' ' ')"
  rm -f -- "$f.part" "$f.failed"
  echo "the backup failed: $(head -c 300 "$d/last-error.txt")" >&2
  exit 1
fi
mv -- "$f.part" "$f"
ls -1t "$d"/{id}-*.{ext} 2>/dev/null | tail -n +{next} | while IFS= read -r old; do rm -f -- "$old"; done
log "ok $f $(wc -c < "$f" | tr -d ' ')"
echo "@@file $f"
"#,
        dump = dump_command(&j.source),
        next = j.keep + 1,
    ))
}

pub fn script(j: &BackupJob) -> Result<String> {
    script_in(j, BACKUP_DIR)
}

/// The /etc/cron.d line (runs as root).
pub fn cron_file(j: &BackupJob) -> Result<String> {
    let j = validate(j)?;
    let s = j.schedule;
    let dow = match s.every {
        Every::Day => "*".to_string(),
        Every::Week => s.weekday.to_string(),
    };
    Ok(format!(
        "# Orbital backup \"{}\"; change it in Orbital (Backups).\nSHELL=/bin/sh\nPATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n{} {} * * {dow} root {SCRIPT_DIR}/backup-{}.sh >/dev/null 2>&1\n",
        j.name.replace('\n', " "),
        s.minute,
        s.hour,
        j.id
    ))
}

/// Paths for a job: (script, cron file, backup folder).
pub fn paths(id: &str) -> Result<(String, String, String)> {
    if !ID.is_match(id) {
        return Err(Error::InvalidInput(format!("{id:?} isn't a backup id")));
    }
    Ok((
        format!("{SCRIPT_DIR}/backup-{id}.sh"),
        format!("{CRON_DIR}/orbital-backup-{id}"),
        format!("{BACKUP_DIR}/{id}"),
    ))
}

/// Writes stdin to `path` (mode, root-owned) through a temporary file, so a half-written file never runs.
pub fn write_command(path: &str, mode: &str) -> String {
    let p = quote(path);
    let t = quote(&format!("{path}.orbital-tmp"));
    format!("mkdir -p -- {SCRIPT_DIR} && cat > {t} && chmod {mode} -- {t} && chown root:root -- {t} && mv -f -- {t} {p}")
}

/// Removes a job's script and cron line; with `files`, its backups too.
pub fn delete_command(id: &str, files: bool) -> Result<String> {
    let (s, c, d) = paths(id)?;
    let mut cmd = format!("rm -f -- {} {}", quote(&c), quote(&s));
    if files {
        cmd.push_str(&format!(" && rm -rf -- {}", quote(&d)));
    }
    Ok(cmd)
}

/// Lists every job with its backups and last run (read-only; nothing interpolated).
pub const LIST_SCRIPT: &str = r#"for s in /usr/local/lib/orbital/backup-*.sh; do
  [ -f "$s" ] || continue
  id=$(basename "$s" .sh); id=${id#backup-}
  echo "@@job"
  sed -n 2p "$s"
  [ -f "/etc/cron.d/orbital-backup-$id" ] && echo "@@cron"
  d="/var/backups/orbital/$id"
  if [ -d "$d" ]; then
    for f in "$d/$id"-*; do
      case "$f" in *.part|*.failed) continue;; esac
      [ -f "$f" ] && echo "@@file $(stat -c '%s %Y' "$f") $f"
    done
    [ -f "$d/backup.log" ] && tail -n 1 "$d/backup.log" | sed 's/^/@@log /'
  fi
done
true"#;

pub fn parse_list(out: &str) -> Vec<BackupStatus> {
    let mut jobs: Vec<BackupStatus> = Vec::new();
    let mut cur: Option<BackupStatus> = None;
    let mut want_meta = false;
    for line in out.lines() {
        if line == "@@job" {
            if let Some(c) = cur.take() {
                jobs.push(c);
            }
            want_meta = true;
            continue;
        }
        if want_meta {
            want_meta = false;
            cur = line
                .strip_prefix("# orbital-backup ")
                .and_then(|j| serde_json::from_str::<BackupJob>(j).ok())
                .and_then(|j| validate(&j).ok())
                .map(|job| BackupStatus {
                    job,
                    files: vec![],
                    last_run: None,
                    last_error: None,
                    scheduled: false,
                });
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        if line == "@@cron" {
            c.scheduled = true;
        } else if let Some(rest) = line.strip_prefix("@@file ") {
            let mut it = rest.splitn(3, ' ');
            if let (Some(b), Some(t), Some(p)) = (it.next(), it.next(), it.next()) {
                if let (Ok(bytes), Ok(at)) = (b.parse(), t.parse()) {
                    c.files.push(BackupFile {
                        path: p.to_string(),
                        bytes,
                        at,
                    });
                }
            }
        } else if let Some(rest) = line.strip_prefix("@@log ") {
            let mut it = rest.splitn(3, ' ');
            let at = it.next().and_then(|t| t.parse().ok());
            let what = it.next().unwrap_or("");
            c.last_run = at;
            c.last_error = (what == "failed").then(|| it.next().unwrap_or("").trim().to_string());
        }
    }
    if let Some(c) = cur {
        jobs.push(c);
    }
    for j in &mut jobs {
        j.files.sort_by(|a, b| b.at.cmp(&a.at).then(b.path.cmp(&a.path)));
    }
    jobs
}

/// A backup file of this job (in its folder, named like its backups), or why not.
pub fn check_file<'a>(j: &BackupJob, file: &'a str, base: &str) -> Result<&'a str> {
    let re = Regex::new(&format!(
        r"^{}/{id}/{id}-\d{{8}}-\d{{6}}\.{}$",
        regex::escape(base),
        regex::escape(ext(&j.source)),
        id = regex::escape(&j.id)
    ))
    .unwrap();
    if re.is_match(file) {
        Ok(file)
    } else {
        Err(Error::InvalidInput(format!("{file} isn't one of this job's backups")))
    }
}

/// Where a restore puts things.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreTo {
    /// A folder: into a new folder next to the original (`<path>.restored-<stamp>`).
    Copy,
    /// The original place: the folder's files are overwritten; a database's contents are replaced.
    Original,
}

/// The restore command; prints `@@restored <where>`.
pub fn restore_command_in(j: &BackupJob, file: &str, to: RestoreTo, base: &str) -> Result<String> {
    let j = validate(j)?;
    let f = quote(check_file(&j, file, base)?);
    Ok(match (&j.source, to) {
        (BackupSource::Folder { path }, RestoreTo::Copy) => {
            let stamp = file
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_start_matches(&format!("{}-", j.id))
                .trim_end_matches(".tar.gz");
            let dest = quote(&format!("{path}.restored-{stamp}"));
            format!("set -e; mkdir -p -- {dest}; tar xzf {f} -C {dest}; echo \"@@restored \"{dest}")
        }
        (BackupSource::Folder { path }, RestoreTo::Original) => {
            let (parent, _) = path.rsplit_once('/').unwrap_or(("", path));
            let parent = if parent.is_empty() { "/" } else { parent };
            format!("set -e; tar xzf {f} -C {}; echo \"@@restored \"{}", quote(parent), quote(path))
        }
        (
            BackupSource::Database {
                engine,
                name,
                container: Some(c),
            },
            _,
        ) => {
            let db = quote(name);
            let run = match engine {
                DbKind::Postgres => format!(
                    "{} < {f}",
                    in_container(c, &format!("pg_restore --clean --if-exists --no-owner -d {db}"), true)
                ),
                DbKind::Mysql => format!(
                    "gunzip -c {f} | {}",
                    in_container(c, "M=$(command -v mysql || command -v mariadb); \"$M\"", true)
                ),
                DbKind::Mongo => format!(
                    "{} < {f}",
                    in_container(
                        c,
                        &format!(
                            "mongorestore --quiet --drop --archive --gzip --nsInclude={}",
                            quote(&format!("{name}.*"))
                        ),
                        true
                    )
                ),
            };
            format!("set -e\n{run}\necho \"@@restored \"{db}")
        }
        (
            BackupSource::Database {
                engine,
                name,
                container: None,
            },
            _,
        ) => {
            let db = quote(name);
            let run = match engine {
                DbKind::Postgres => format!("as_pg pg_restore --clean --if-exists --no-owner -d {db} < {f}"),
                DbKind::Mysql => format!("M=$(command -v mysql || command -v mariadb); gunzip -c {f} | \"$M\""),
                DbKind::Mongo => format!(
                    "mongorestore --quiet --drop --archive --gzip --nsInclude={} < {f}",
                    quote(&format!("{name}.*"))
                ),
            };
            format!(
                "set -e\nas_pg() {{ if command -v runuser >/dev/null 2>&1; then runuser -u postgres -- \"$@\"; else sudo -n -u postgres -- \"$@\"; fi; }}\n{run}\necho \"@@restored \"{db}"
            )
        }
    })
}

pub fn restore_command(j: &BackupJob, file: &str, to: RestoreTo) -> Result<String> {
    restore_command_in(j, file, to, BACKUP_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_job(path: &str, keep: u32) -> BackupJob {
        BackupJob {
            id: "site-files".into(),
            name: "Site files".into(),
            source: BackupSource::Folder { path: path.into() },
            schedule: Schedule {
                every: Every::Day,
                weekday: 0,
                hour: 3,
                minute: 15,
            },
            keep,
        }
    }

    #[test]
    fn validates_jobs() {
        assert!(validate(&folder_job("/var/www/shop", 7)).is_ok());
        assert_eq!(
            validate(&folder_job("/var/www/shop/", 7)).unwrap().source,
            BackupSource::Folder {
                path: "/var/www/shop".into()
            }
        );
        for bad in ["var/www", "/", "/tmp", "/var/backups/orbital/x", "/a/../etc", "/a;rm", "/a$(x)"] {
            assert!(validate(&folder_job(bad, 7)).is_err(), "{bad}");
        }
        assert!(validate(&BackupJob {
            keep: 0,
            ..folder_job("/srv", 1)
        })
        .is_err());
        assert!(validate(&BackupJob {
            keep: 91,
            ..folder_job("/srv", 1)
        })
        .is_err());
        assert!(validate(&BackupJob {
            id: "Bad Id".into(),
            ..folder_job("/srv", 1)
        })
        .is_err());
        let mut late = folder_job("/srv", 1);
        late.schedule.hour = 24;
        assert!(validate(&late).is_err());
        let db = BackupJob {
            source: BackupSource::Database {
                engine: DbKind::Postgres,
                name: "shop; drop".into(),
                container: None,
            },
            ..folder_job("/srv", 1)
        };
        assert!(validate(&db).is_err());
        assert_eq!(slug("  Shop database (main)! "), "shop-database-main");
        assert_eq!(slug("!!!"), "backup");
    }

    #[test]
    fn databases_in_containers_are_dumped_and_restored_through_docker_exec() {
        let j = BackupJob {
            source: BackupSource::Database {
                engine: DbKind::Mysql,
                name: "shop".into(),
                container: Some("shop-db".into()),
            },
            ..folder_job("/srv", 3)
        };
        let s = script(&j).unwrap();
        assert!(s.contains("( docker exec -- shop-db sh -c "), "{s}");
        assert!(
            s.contains("MYSQL_ROOT_PASSWORD") && s.contains("--databases shop"),
            "the image's login and the database"
        );
        assert!(
            s.contains(" || echo x > \"$f.failed\" ) | gzip"),
            "failure seen on the host, compressed on the host"
        );
        let pg = BackupJob {
            source: BackupSource::Database {
                engine: DbKind::Postgres,
                name: "shop".into(),
                container: Some("pg".into()),
            },
            ..folder_job("/srv", 3)
        };
        let r = restore_command(
            &pg,
            "/var/backups/orbital/site-files/site-files-20261122-031500.dump",
            RestoreTo::Original,
        )
        .unwrap();
        assert!(
            r.contains("docker exec -i -- pg sh -c ") && r.contains("pg_restore --clean") && r.ends_with("echo \"@@restored \"shop"),
            "{r}"
        );
        for c in [s, r, tool_check(&pg.source).unwrap()] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
            assert!(o.status.success(), "{c}");
        }
        assert!(tool_check(&pg.source).unwrap().contains("in the container pg"));
        // A container name that isn't one is refused; the job's JSON keeps it.
        let bad = BackupJob {
            source: BackupSource::Database {
                engine: DbKind::Postgres,
                name: "shop".into(),
                container: Some("a;b".into()),
            },
            ..folder_job("/srv", 3)
        };
        assert!(validate(&bad).is_err());
        let back: BackupJob = serde_json::from_str(&serde_json::to_string(&validate(&pg).unwrap()).unwrap()).unwrap();
        assert_eq!(back, validate(&pg).unwrap());
    }

    // Runs the check under /bin/sh with an empty PATH; Windows has no /bin/sh (the app runs these on Linux servers).
    #[cfg(unix)]
    #[test]
    fn database_jobs_check_their_dump_tool() {
        assert!(tool_check(&BackupSource::Folder { path: "/srv".into() }).is_none());
        for engine in [DbKind::Postgres, DbKind::Mysql, DbKind::Mongo] {
            let c = tool_check(&BackupSource::Database {
                engine,
                name: "x".into(),
                container: None,
            })
            .unwrap();
            let c = c.as_str();
            let o = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(c)
                .env("PATH", "/nonexistent")
                .output()
                .unwrap();
            assert!(String::from_utf8_lossy(&o.stdout).starts_with("@@missing "), "{c}");
        }
    }

    #[test]
    fn cron_lines_and_paths() {
        let mut j = folder_job("/srv/app", 7);
        assert!(cron_file(&j)
            .unwrap()
            .contains("\n15 3 * * * root /usr/local/lib/orbital/backup-site-files.sh >/dev/null 2>&1\n"));
        j.schedule = Schedule {
            every: Every::Week,
            weekday: 0,
            hour: 2,
            minute: 0,
        };
        assert!(cron_file(&j).unwrap().contains("\n0 2 * * 0 root "));
        assert_eq!(
            paths("site-files").unwrap(),
            (
                "/usr/local/lib/orbital/backup-site-files.sh".into(),
                "/etc/cron.d/orbital-backup-site-files".into(),
                "/var/backups/orbital/site-files".into()
            )
        );
        assert!(paths("../x").is_err());
        assert_eq!(
            delete_command("site-files", false).unwrap(),
            "rm -f -- /etc/cron.d/orbital-backup-site-files /usr/local/lib/orbital/backup-site-files.sh"
        );
        assert!(delete_command("site-files", true)
            .unwrap()
            .ends_with("rm -rf -- /var/backups/orbital/site-files"));
    }

    #[test]
    fn every_script_is_valid_sh() {
        let db = |engine| BackupJob {
            source: BackupSource::Database {
                engine,
                name: "shop".into(),
                container: None,
            },
            ..folder_job("/srv", 3)
        };
        for j in [
            folder_job("/srv/my app", 3),
            db(DbKind::Postgres),
            db(DbKind::Mysql),
            db(DbKind::Mongo),
        ] {
            let s = script(&j).unwrap();
            assert!(s.starts_with("#!/bin/sh\n# orbital-backup {"), "{s}");
            for c in [
                s.clone(),
                restore_command(
                    &j,
                    &format!("/var/backups/orbital/site-files/site-files-20261122-031500.{}", ext(&j.source)),
                    RestoreTo::Original,
                )
                .unwrap(),
            ] {
                let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
                assert!(o.status.success(), "{c}\n{}", String::from_utf8_lossy(&o.stderr));
            }
        }
        assert!(write_command("/usr/local/lib/orbital/backup-x.sh", "755")
            .contains("mv -f -- /usr/local/lib/orbital/backup-x.sh.orbital-tmp /usr/local/lib/orbital/backup-x.sh"));
    }

    #[test]
    fn only_the_jobs_own_backups_can_be_restored() {
        let j = folder_job("/srv/app", 3);
        assert!(check_file(&j, "/var/backups/orbital/site-files/site-files-20261122-031500.tar.gz", BACKUP_DIR).is_ok());
        for bad in [
            "/var/backups/orbital/site-files/../../etc/shadow",
            "/var/backups/orbital/other/other-20261122-031500.tar.gz",
            "/var/backups/orbital/site-files/site-files-20261122-031500.tar.gz.part",
            "/etc/passwd",
        ] {
            assert!(check_file(&j, bad, BACKUP_DIR).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_the_list_back() {
        let j = folder_job("/srv/app", 3);
        let meta = serde_json::to_string(&validate(&j).unwrap()).unwrap();
        let out = format!(
            "@@job\n# orbital-backup {meta}\n@@cron\n@@file 100 1795300000 /var/backups/orbital/site-files/site-files-20261121-031500.tar.gz\n@@file 120 1795386400 /var/backups/orbital/site-files/site-files-20261122-031500.tar.gz\n@@log 1795386401 ok /var/backups/orbital/site-files/site-files-20261122-031500.tar.gz 120\n@@job\n# not ours\n@@job\n# orbital-backup {{\"id\":\"x\"}}\n"
        );
        let l = parse_list(&out);
        assert_eq!(l.len(), 1, "broken or foreign entries are skipped");
        assert!(l[0].scheduled && l[0].last_error.is_none() && l[0].last_run == Some(1795386401));
        assert_eq!(l[0].files[0].bytes, 120, "newest first");
        let failed = parse_list(&format!(
            "@@job\n# orbital-backup {meta}\n@@log 5 failed tar: /srv/app: No such file\n"
        ));
        assert_eq!(failed[0].last_error.as_deref(), Some("tar: /srv/app: No such file"));
        assert!(!failed[0].scheduled);
    }

    /// The real thing, under sh in a temp folder: a folder is backed up three
    /// times, only the newest two are kept, the log says ok, and a restore to a
    /// copy brings the files back.
    #[cfg(unix)]
    #[test]
    fn a_folder_backup_runs_keeps_the_newest_and_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_str().unwrap();
        let src = format!("{root}/data/my app");
        std::fs::create_dir_all(format!("{src}/sub")).unwrap();
        std::fs::write(format!("{src}/sub/a.txt"), "hello").unwrap();
        let j = folder_job(&src, 2);
        let base = format!("{root}/backups");
        let s = script_in(&j, &base).unwrap();
        for _ in 0..3 {
            let o = std::process::Command::new("sh").arg("-c").arg(&s).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let dir = format!("{base}/site-files");
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".tar.gz"))
            .collect();
        files.sort();
        assert_eq!(files.len(), 2, "{files:?}");
        let log = std::fs::read_to_string(format!("{dir}/backup.log")).unwrap();
        assert_eq!(log.lines().filter(|l| l.contains(" ok ")).count(), 3, "{log}");
        let newest = format!("{dir}/{}", files[1]);
        let cmd = restore_command_in(&j, &newest, RestoreTo::Copy, &base).unwrap();
        let o = std::process::Command::new("sh").arg("-c").arg(&cmd).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout);
        let restored = out.lines().find_map(|l| l.strip_prefix("@@restored ")).unwrap().to_string();
        assert!(restored.starts_with(&format!("{src}.restored-")), "{restored}");
        assert_eq!(std::fs::read_to_string(format!("{restored}/my app/sub/a.txt")).unwrap(), "hello");
        // A failing source is logged as failed and leaves no partial file.
        let gone = folder_job(&format!("{root}/nope"), 2);
        let s = script_in(&gone, &format!("{root}/b2")).unwrap();
        let o = std::process::Command::new("sh").arg("-c").arg(&s).output().unwrap();
        assert!(!o.status.success());
        let log = std::fs::read_to_string(format!("{root}/b2/site-files/backup.log")).unwrap();
        assert!(log.contains(" failed "), "{log}");
        assert!(std::fs::read_dir(format!("{root}/b2/site-files")).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
    }
}
