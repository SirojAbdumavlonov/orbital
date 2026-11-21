//! Docker containers via the docker CLI on the server (no exposed daemon
//! socket needed). Falls back to sudo when the user isn't in the docker group.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_container, validate_path};
use crate::ssh::{ExecOutput, SshSession};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub id: String,
    pub names: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub created_at: String,
    pub compose_project: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStats {
    pub id: String,
    pub name: String,
    pub cpu_percent: String,
    pub mem_usage: String,
    pub mem_percent: String,
    pub net_io: String,
    pub block_io: String,
    pub pids: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Remove,
    ForceRemove,
}

impl ContainerAction {
    pub fn args(self) -> &'static str {
        match self {
            ContainerAction::Start => "start",
            ContainerAction::Stop => "stop",
            ContainerAction::Restart => "restart",
            ContainerAction::Pause => "pause",
            ContainerAction::Unpause => "unpause",
            ContainerAction::Remove => "rm",
            ContainerAction::ForceRemove => "rm -f",
        }
    }
}

/// What to do with a Compose project (or one service of it).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ComposeAction {
    /// Apply the compose file: create or recreate what changed (`up -d`).
    Up,
    /// Restart the containers without recreating them.
    Restart,
    /// Download newer images (takes effect on the next `up`).
    Pull,
}

impl ComposeAction {
    pub fn verb(self) -> &'static str {
        match self {
            ComposeAction::Up => "up",
            ComposeAction::Restart => "restart",
            ComposeAction::Pull => "pull",
        }
    }
}

/// Where a running Compose project came from, as Docker labels record it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposeTarget {
    pub project: String,
    /// The compose files it was started with (`-f`), in order.
    pub files: Vec<String>,
    /// The folder it was started from; relative paths in the files resolve against it.
    pub working_dir: Option<String>,
    /// Only this service, or the whole project.
    pub service: Option<String>,
}

/// `docker compose …` for a target. Paths must be absolute, names must be
/// plain identifiers; everything is quoted.
pub fn compose_args(t: &ComposeTarget, action: ComposeAction) -> Result<String> {
    if t.files.is_empty() || t.files.len() > 8 {
        return Err(Error::InvalidInput("give between 1 and 8 compose files".into()));
    }
    validate_container(&t.project)?;
    let mut args = format!("compose -p {}", quote(&t.project));
    for f in &t.files {
        args.push_str(&format!(" -f {}", quote(validate_path(f)?)));
    }
    if let Some(d) = &t.working_dir {
        args.push_str(&format!(" --project-directory {}", quote(validate_path(d)?)));
    }
    args.push(' ');
    args.push_str(match action {
        ComposeAction::Up => "up -d",
        ComposeAction::Restart => "restart",
        ComposeAction::Pull => "pull",
    });
    if let Some(svc) = &t.service {
        args.push_str(&format!(" -- {}", quote(validate_container(svc)?)));
    }
    Ok(args)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PsLine {
    #[serde(rename = "ID")]
    id: String,
    names: String,
    image: String,
    state: Option<String>,
    status: String,
    ports: String,
    created_at: String,
    labels: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StatsLine {
    #[serde(rename = "ID")]
    id: String,
    name: String,
    #[serde(rename = "CPUPerc")]
    cpu_perc: String,
    mem_usage: String,
    mem_perc: String,
    #[serde(rename = "NetIO")]
    net_io: String,
    #[serde(rename = "BlockIO")]
    block_io: String,
    #[serde(rename = "PIDs")]
    pids: String,
}

fn infer_state(status: &str) -> String {
    let s = status.to_lowercase();
    if s.starts_with("up") {
        if s.contains("paused") { "paused" } else { "running" }.into()
    } else if s.starts_with("exited") {
        "exited".into()
    } else if s.starts_with("created") {
        "created".into()
    } else if s.contains("restarting") {
        "restarting".into()
    } else {
        "unknown".into()
    }
}

pub fn parse_ps(out: &str) -> Vec<Container> {
    out.lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str::<PsLine>(l).ok())
        .map(|p| {
            let compose_project = p.labels.as_deref().and_then(|ls| {
                ls.split(',')
                    .find_map(|kv| kv.strip_prefix("com.docker.compose.project=").map(String::from))
            });
            Container {
                state: p.state.filter(|s| !s.is_empty()).unwrap_or_else(|| infer_state(&p.status)),
                id: p.id,
                names: p.names,
                image: p.image,
                status: p.status,
                ports: p.ports,
                created_at: p.created_at,
                compose_project,
            }
        })
        .collect()
}

pub fn parse_stats(out: &str) -> Vec<ContainerStats> {
    out.lines()
        .filter_map(|l| serde_json::from_str::<StatsLine>(l).ok())
        .map(|s| ContainerStats {
            id: s.id,
            name: s.name,
            cpu_percent: s.cpu_perc,
            mem_usage: s.mem_usage,
            mem_percent: s.mem_perc,
            net_io: s.net_io,
            block_io: s.block_io,
            pids: s.pids,
        })
        .collect()
}

/// The login user can't reach the Docker daemon. Looks at both outputs:
/// several commands fold errors into stdout (`2>&1`, e.g. logs, compose).
pub(crate) fn is_socket_denied(o: &ExecOutput) -> bool {
    let e = format!("{}\n{}", o.stderr, o.stdout).to_lowercase();
    !o.ok() && e.contains("permission denied") && e.contains("docker")
}

impl SshSession {
    /// Run a docker command, retrying with sudo if the daemon socket is not accessible.
    pub async fn docker(&self, args: &str) -> Result<(String, ExecOutput)> {
        let cmd = format!("docker {args}");
        let o = self.exec(&cmd).await?;
        if is_socket_denied(&o) && self.elevation_needed() {
            let o2 = self.exec_privileged(&cmd).await?;
            return Ok((self.privileged_command(&cmd), o2));
        }
        Ok((cmd, o))
    }

    /// Run one of Orbital's fixed read scripts on the server itself, or with
    /// `container` inside that container (`docker exec … sh -c`), where
    /// [`CONTAINER_PRELUDE`] first picks up the image's own login settings.
    pub async fn inspect_in(&self, container: Option<&str>, script: &str) -> Result<ExecOutput> {
        match container {
            None => Ok(self.exec_inspect(script).await?.0),
            Some(c) => Ok(self.docker(&in_container_args(c, script)?).await?.1),
        }
    }

    pub async fn docker_available(&self) -> Result<Option<String>> {
        let o = self.exec("command -v docker >/dev/null 2>&1 && echo yes").await?;
        if !o.stdout.contains("yes") {
            return Ok(None);
        }
        let (_, v) = self.docker("version --format '{{.Server.Version}}'").await?;
        if v.ok() {
            Ok(Some(v.stdout.trim().to_string()))
        } else {
            Err(Error::Remote {
                code: v.exit_code,
                stderr: v.stderr.trim().to_string(),
            })
        }
    }

    pub async fn list_containers(&self) -> Result<Vec<Container>> {
        let (_, o) = self.docker("ps -a --no-trunc --format '{{json .}}'").await?;
        Ok(parse_ps(&o.check()?.stdout))
    }

    pub async fn container_stats(&self) -> Result<Vec<ContainerStats>> {
        let (_, o) = self.docker("stats --no-stream --format '{{json .}}'").await?;
        Ok(parse_stats(&o.check()?.stdout))
    }

    pub async fn inspect_container(&self, id: &str) -> Result<serde_json::Value> {
        validate_container(id)?;
        let (_, o) = self.docker(&format!("inspect -- {}", quote(id))).await?;
        let v: serde_json::Value = serde_json::from_str(&o.check()?.stdout)?;
        Ok(v.get(0).cloned().unwrap_or(v))
    }

    /// The last `tail` lines, or with `since`/`until` (RFC 3339 times) the
    /// last `tail` lines written in that window.
    pub async fn container_logs(&self, id: &str, tail: u32, since: Option<&str>, until: Option<&str>) -> Result<String> {
        let (_, o) = self.docker(&logs_args(id, tail, since, until)?).await?;
        Ok(o.stdout)
    }

    /// Run a Compose action. Compose writes its progress to stderr, so both
    /// streams are returned as they are.
    pub async fn compose(&self, t: &ComposeTarget, action: ComposeAction) -> Result<(String, ExecOutput)> {
        let args = compose_args(t, action)?;
        self.docker(&format!("{args} 2>&1")).await
    }

    pub async fn container_action(&self, id: &str, action: ContainerAction) -> Result<(String, ExecOutput)> {
        validate_container(id)?;
        let (cmd, o) = self.docker(&format!("{} -- {}", action.args(), quote(id))).await?;
        Ok((cmd, o))
    }
}

/// Run first inside a container: the official images keep their database
/// login in environment variables (POSTGRES_USER, MYSQL_ROOT_PASSWORD, …),
/// which the clients read from PG*/MYSQL_PWD/REDISCLI_AUTH. Nothing leaves
/// the container; Orbital never sees these values.
pub const CONTAINER_PRELUDE: &str = r#"ORBITAL_CONTAINER=1
u=${POSTGRES_USER:-${POSTGRESQL_USERNAME:-postgres}}; export PGUSER="${PGUSER:-$u}"
export PGDATABASE="${PGDATABASE:-${POSTGRES_DB:-${POSTGRESQL_DATABASE:-postgres}}}"
p=${POSTGRES_PASSWORD:-${POSTGRESQL_PASSWORD:-}}; [ -n "$p" ] && export PGPASSWORD="$p"
p=${MYSQL_ROOT_PASSWORD:-${MARIADB_ROOT_PASSWORD:-}}; [ -n "$p" ] && export MYSQL_PWD="$p"
p=${REDIS_PASSWORD:-}; [ -n "$p" ] && export REDISCLI_AUTH="$p"
unset p u
"#;

/// `exec -- NAME sh -c SCRIPT` (for `docker`): a fixed script inside a container.
pub fn in_container_args(name: &str, script: &str) -> Result<String> {
    validate_container(name)?;
    Ok(format!(
        "exec -- {} sh -c {}",
        quote(name),
        quote(&format!("{CONTAINER_PRELUDE}{script}"))
    ))
}

/// `logs …` for [`SshSession::container_logs`]. Times must be RFC 3339
/// (`2026-11-20T09:00:00Z`, `…+05:00`), checked before they reach the server.
pub fn logs_args(id: &str, tail: u32, since: Option<&str>, until: Option<&str>) -> Result<String> {
    validate_container(id)?;
    let mut a = format!("logs --timestamps --tail {}", tail.min(10_000));
    for (flag, t) in [("--since", since), ("--until", until)] {
        if let Some(t) = t {
            let t = validate_time(t)?;
            a.push_str(&format!(" {flag} {}", quote(t)));
        }
    }
    a.push_str(&format!(" -- {} 2>&1", quote(id)));
    Ok(a)
}

fn validate_time(t: &str) -> Result<&str> {
    let b = t.as_bytes();
    let date_ok = b.len() >= 20
        && b.len() <= 35
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b.iter().all(|c| c.is_ascii_digit() || b"-:T.Z+".contains(c));
    if date_ok {
        Ok(t)
    } else {
        Err(Error::InvalidInput(format!("not a date and time: {t}")))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn permission_errors_are_seen_in_either_output() {
        let out = |stdout: &str, stderr: &str, code| crate::ssh::ExecOutput {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code: code,
            duration_ms: 0,
        };
        let msg = "permission denied while trying to connect to the docker API at unix:///var/run/docker.sock";
        assert!(super::is_socket_denied(&out("", msg, 1)));
        // `docker logs … 2>&1`: the error arrives on stdout.
        assert!(super::is_socket_denied(&out(msg, "", 1)));
        // Older Docker's wording.
        assert!(super::is_socket_denied(&out(
            "",
            "Got permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock",
            1
        )));
        // A log line that merely mentions it isn't a failure.
        assert!(!super::is_socket_denied(&out(msg, "", 0)));
        assert!(!super::is_socket_denied(&out("", "Error: No such container: web", 1)));
    }

    #[test]
    fn scripts_run_inside_a_container() {
        let a = super::in_container_args("shop-db-1", "psql -c 'select 1'").unwrap();
        assert!(a.starts_with("exec -- shop-db-1 sh -c 'ORBITAL_CONTAINER=1"), "{a}");
        assert!(a.contains("select 1"), "{a}");
        assert!(super::in_container_args("db; reboot", "true").is_err());
        // The prelude maps the image's settings to the clients' variables.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{}echo \"$PGUSER $PGDATABASE $PGPASSWORD $MYSQL_PWD $ORBITAL_CONTAINER\"",
                super::CONTAINER_PRELUDE
            ))
            .env_clear()
            .env("POSTGRES_USER", "app")
            .env("POSTGRES_PASSWORD", "pw")
            .env("MARIADB_ROOT_PASSWORD", "root")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "app postgres pw root 1");
    }

    #[test]
    fn logs_between_two_times() {
        use super::logs_args;
        assert_eq!(
            logs_args("web", 400, None, None).unwrap(),
            "logs --timestamps --tail 400 -- web 2>&1"
        );
        assert_eq!(
            logs_args("web", 99_999, Some("2026-11-20T09:00:00.000Z"), Some("2026-11-20T10:30:00+05:00")).unwrap(),
            "logs --timestamps --tail 10000 --since 2026-11-20T09:00:00.000Z --until 2026-11-20T10:30:00+05:00 -- web 2>&1"
        );
        for bad in [
            "yesterday",
            "2026-11-20",
            "2026-11-20T09:00:00Z; rm -rf /",
            "2026-11-20T09:00:00 Z",
            "$(id)2026-11-20T09:00:00Z",
        ] {
            assert!(logs_args("web", 10, Some(bad), None).is_err(), "{bad}");
        }
        assert!(logs_args("bad name", 10, None, None).is_err());
    }

    use super::*;

    const PS: &str = r#"{"Command":"\"/docker-entrypoint.…\"","CreatedAt":"2026-11-01 10:00:00 +0000 UTC","ID":"3f2a9c11aa","Image":"nginx:1.27","Labels":"com.docker.compose.project=shop,com.docker.compose.service=web","LocalVolumes":"0","Mounts":"","Names":"shop-web-1","Networks":"shop_default","Ports":"0.0.0.0:80->80/tcp","RunningFor":"2 weeks ago","Size":"0B","State":"running","Status":"Up 2 weeks"}
{"Command":"\"redis\"","CreatedAt":"2026-11-01 10:00:00 +0000 UTC","ID":"9b1c","Image":"redis:7","Labels":"","Names":"cache","Ports":"","State":"","Status":"Exited (0) 3 days ago"}
WARNING: something not json"#;

    #[test]
    fn ps() {
        let c = parse_ps(PS);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].compose_project.as_deref(), Some("shop"));
        assert_eq!(c[0].state, "running");
        assert_eq!(c[1].state, "exited");
        assert_eq!(c[1].compose_project, None);
    }

    #[test]
    fn compose_commands_are_built_from_checked_parts() {
        let t = ComposeTarget {
            project: "shop".into(),
            files: vec!["/srv/shop/docker-compose.yml".into(), "/srv/shop/compose.prod.yml".into()],
            working_dir: Some("/srv/shop".into()),
            service: Some("web".into()),
        };
        assert_eq!(
            compose_args(&t, ComposeAction::Up).unwrap(),
            "compose -p shop -f /srv/shop/docker-compose.yml -f /srv/shop/compose.prod.yml --project-directory /srv/shop up -d -- web"
        );
        let all = ComposeTarget {
            service: None,
            working_dir: None,
            ..t.clone()
        };
        assert!(compose_args(&all, ComposeAction::Pull).unwrap().ends_with("compose.prod.yml pull"));
        let spaced = ComposeTarget {
            files: vec!["/srv/my app/compose.yml".into()],
            ..all.clone()
        };
        assert!(compose_args(&spaced, ComposeAction::Restart)
            .unwrap()
            .contains("-f '/srv/my app/compose.yml'"));

        for bad in [
            ComposeTarget {
                project: "shop; reboot".into(),
                ..all.clone()
            },
            ComposeTarget {
                files: vec!["compose.yml".into()],
                ..all.clone()
            },
            ComposeTarget {
                files: vec![],
                ..all.clone()
            },
            ComposeTarget {
                service: Some("--privileged".into()),
                ..all.clone()
            },
            ComposeTarget {
                working_dir: Some("relative".into()),
                ..all.clone()
            },
        ] {
            assert!(compose_args(&bad, ComposeAction::Up).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn stats() {
        let s = parse_stats(
            r#"{"BlockIO":"1MB / 0B","CPUPerc":"0.15%","Container":"3f2a","ID":"3f2a","MemPerc":"1.2%","MemUsage":"20MiB / 7.7GiB","Name":"shop-web-1","NetIO":"1kB / 2kB","PIDs":"5"}"#,
        );
        assert_eq!(s[0].cpu_percent, "0.15%");
        assert_eq!(s[0].pids, "5");
    }
}
