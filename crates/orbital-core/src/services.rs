//! systemd services: list, inspect, logs, and guarded lifecycle actions.
//! Restart/reload of services with a known config checker (nginx, sshd,
//! apache, haproxy) runs that checker first and aborts on failure — so a typo
//! can't take down your web server or lock you out of SSH.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_unit};
use crate::ssh::{ExecOutput, SshSession};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
    /// enabled / disabled / static / masked / ...
    pub unit_file_state: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDetail {
    pub unit: String,
    pub properties: HashMap<String, String>,
    pub logs: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Reload,
    Enable,
    Disable,
}

impl ServiceAction {
    pub fn verb(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Reload => "reload",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        }
    }
    pub fn is_destructive(self) -> bool {
        matches!(self, ServiceAction::Stop | ServiceAction::Disable | ServiceAction::Restart)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionReport {
    pub command: String,
    pub preflight: Option<(String, String)>,
    pub output: ExecOutput,
}

pub const LIST_SCRIPT: &str = "LC_ALL=C systemctl list-units --type=service --all --no-legend --no-pager --plain 2>/dev/null; echo '@@files'; LC_ALL=C systemctl list-unit-files --type=service --no-legend --no-pager 2>/dev/null";

pub fn parse_list(out: &str) -> Vec<Service> {
    let (units, files) = out.split_once("@@files").unwrap_or((out, ""));
    let states: HashMap<&str, &str> = files
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?, it.next()?))
        })
        .collect();
    let mut v: Vec<Service> = units
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(['●', '*', ' ']);
            let mut it = l.split_whitespace();
            let unit = it.next()?.to_string();
            if !unit.ends_with(".service") {
                return None;
            }
            let load = it.next()?.to_string();
            let active = it.next()?.to_string();
            let sub = it.next()?.to_string();
            let description = it.collect::<Vec<_>>().join(" ");
            Some(Service {
                unit_file_state: states.get(unit.as_str()).map(|s| s.to_string()),
                unit,
                load,
                active,
                sub,
                description,
            })
        })
        .collect();
    v.sort_by(|a, b| (a.active != "active").cmp(&(b.active != "active")).then(a.unit.cmp(&b.unit)));
    v
}

pub fn parse_show(out: &str) -> HashMap<String, String> {
    out.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Config checker for well-known services (run before restart/reload).
pub fn preflight_for(unit: &str) -> Option<&'static str> {
    let base = unit.trim_end_matches(".service");
    match base {
        "nginx" | "openresty" => Some("nginx -t"),
        "sshd" | "ssh" => Some("sshd -t"),
        "apache2" | "httpd" => Some("apachectl configtest"),
        "haproxy" => Some("haproxy -c -f /etc/haproxy/haproxy.cfg"),
        "caddy" => Some("caddy validate --config /etc/caddy/Caddyfile"),
        "postfix" => Some("postfix check"),
        "named" | "bind9" => Some("named-checkconf"),
        _ => None,
    }
}

pub fn action_command(unit: &str, action: ServiceAction) -> Result<String> {
    validate_unit(unit)?;
    Ok(format!("systemctl {} -- {}", action.verb(), quote(unit)))
}

impl SshSession {
    pub async fn has_systemd(&self) -> bool {
        self.exec("test -d /run/systemd/system && command -v systemctl")
            .await
            .map(|o| o.ok())
            .unwrap_or(false)
    }

    pub async fn list_services(&self) -> Result<Vec<Service>> {
        Ok(parse_list(&self.exec(LIST_SCRIPT).await?.stdout))
    }

    pub async fn service_detail(&self, unit: &str, log_lines: u32) -> Result<ServiceDetail> {
        validate_unit(unit)?;
        let q = quote(unit);
        let props = self
            .exec(&format!(
                "systemctl show {q} --no-pager -p Id,Description,LoadState,ActiveState,SubState,UnitFileState,MainPID,\
                 ExecMainStartTimestamp,ActiveEnterTimestamp,MemoryCurrent,CPUUsageNSec,TasksCurrent,NRestarts,FragmentPath,User,Restart"
            ))
            .await?
            .check()?;
        let jcmd = format!("journalctl -u {q} -n {} --no-pager -o short-iso 2>&1", log_lines.min(5000));
        let mut logs = self.exec(&jcmd).await?.stdout;
        if (logs.contains("No journal files") || logs.contains("insufficient permissions") || logs.trim().is_empty())
            && self.elevation_needed()
        {
            if let Ok(o) = self.exec_privileged(&jcmd).await {
                logs = o.stdout;
            }
        }
        Ok(ServiceDetail {
            unit: unit.to_string(),
            properties: parse_show(&props.stdout),
            logs,
        })
    }

    pub async fn service_action(&self, unit: &str, action: ServiceAction) -> Result<ActionReport> {
        let cmd = action_command(unit, action)?;
        let mut preflight = None;
        if matches!(action, ServiceAction::Restart | ServiceAction::Reload | ServiceAction::Start) {
            if let Some(check) = preflight_for(unit) {
                let o = self.exec_privileged(&format!("{check} 2>&1")).await?;
                if !o.ok() {
                    return Err(Error::PreflightFailed {
                        check: check.into(),
                        output: o.combined(),
                    });
                }
                preflight = Some((check.to_string(), o.combined()));
            }
        }
        let output = self.exec_privileged(&cmd).await?;
        Ok(ActionReport {
            command: self.privileged_command(&cmd),
            preflight,
            output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "nginx.service        loaded active   running A high performance web server\n\
cron.service         loaded active   running Regular background program processing daemon\n\
● mysql.service      loaded failed   failed  MySQL Community Server\n\
apt-daily.timer      loaded active   waiting Daily apt download\n\
@@files\n\
nginx.service enabled enabled\n\
mysql.service enabled enabled\n\
cron.service  disabled enabled\n";

    #[test]
    fn list() {
        let s = parse_list(LIST);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].unit, "cron.service");
        assert_eq!(s[0].unit_file_state.as_deref(), Some("disabled"));
        assert_eq!(s[2].unit, "mysql.service");
        assert_eq!(s[2].active, "failed");
        assert_eq!(s[1].description, "A high performance web server");
    }

    #[test]
    fn commands_are_safe() {
        assert_eq!(
            action_command("nginx.service", ServiceAction::Restart).unwrap(),
            "systemctl restart -- nginx.service"
        );
        assert!(action_command("nginx; reboot", ServiceAction::Stop).is_err());
        assert_eq!(preflight_for("nginx.service"), Some("nginx -t"));
        assert_eq!(preflight_for("ssh"), Some("sshd -t"));
        assert_eq!(preflight_for("redis.service"), None);
    }
}
