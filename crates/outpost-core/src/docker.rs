//! Docker containers via the docker CLI on the server (no exposed daemon
//! socket needed). Falls back to sudo when the user isn't in the docker group.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_container};
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

fn is_socket_denied(o: &ExecOutput) -> bool {
    !o.ok() && o.stderr.to_lowercase().contains("permission denied") && o.stderr.contains("docker")
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

    pub async fn container_logs(&self, id: &str, tail: u32) -> Result<String> {
        validate_container(id)?;
        let (_, o) = self
            .docker(&format!("logs --timestamps --tail {} -- {} 2>&1", tail.min(10_000), quote(id)))
            .await?;
        Ok(o.stdout)
    }

    pub async fn container_action(&self, id: &str, action: ContainerAction) -> Result<(String, ExecOutput)> {
        validate_container(id)?;
        let (cmd, o) = self.docker(&format!("{} -- {}", action.args(), quote(id))).await?;
        Ok((cmd, o))
    }
}

#[cfg(test)]
mod tests {
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
    fn stats() {
        let s = parse_stats(
            r#"{"BlockIO":"1MB / 0B","CPUPerc":"0.15%","Container":"3f2a","ID":"3f2a","MemPerc":"1.2%","MemUsage":"20MiB / 7.7GiB","Name":"shop-web-1","NetIO":"1kB / 2kB","PIDs":"5"}"#,
        );
        assert_eq!(s[0].cpu_percent, "0.15%");
        assert_eq!(s[0].pids, "5");
    }
}
