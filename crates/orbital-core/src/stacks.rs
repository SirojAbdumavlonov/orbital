//! Docker Compose stacks (Stacks tab): the projects `docker compose ls` knows
//! on the server, plus Orbital's own in /opt/stacks/<name>/compose.yaml that
//! aren't running. A new stack's file is written through safe save, checked by
//! `docker compose config` before it's kept. Actions run `docker compose -p
//! NAME -f FILE …` (sudo when Docker refuses the login user) and stream their
//! output. Names and paths are validated and quoted.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_path};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const STACK_DIR: &str = "/opt/stacks";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stack {
    pub name: String,
    /// "running(3)", "exited(1)", or "not deployed".
    pub status: String,
    /// The compose file(s), as docker compose reports them.
    pub files: Vec<String>,
    /// Running now (some container up).
    pub running: bool,
    /// Lives in /opt/stacks (made in Orbital).
    pub ours: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StackAction {
    /// `up -d --remove-orphans`
    Up,
    /// `pull`, then `up -d --remove-orphans`
    PullUp,
    Restart,
    /// `down` (containers and networks; volumes are kept)
    Down,
    /// The last 200 lines of every service's log.
    Logs,
}

static NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]{0,62}$").unwrap());

pub fn validate_name(name: &str) -> Result<&str> {
    if NAME.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!(
            "{name:?} isn't a stack name: lowercase letters, digits, - and _ (Compose's own rule)"
        )))
    }
}

pub fn stack_file(name: &str) -> String {
    format!("{STACK_DIR}/{name}/compose.yaml")
}

/// The `docker compose` arguments (after `docker `) for an action.
pub fn args(name: &str, file: &str, action: StackAction) -> Result<String> {
    validate_name(name)?;
    validate_path(file)?;
    let base = format!("compose -p {} -f {}", quote(name), quote(file));
    Ok(match action {
        StackAction::Up => format!("{base} up -d --remove-orphans 2>&1"),
        StackAction::PullUp => format!("{base} pull 2>&1 && docker {base} up -d --remove-orphans 2>&1"),
        StackAction::Restart => format!("{base} restart 2>&1"),
        StackAction::Down => format!("{base} down 2>&1"),
        StackAction::Logs => format!("{base} logs --tail 200 --no-color --timestamps 2>&1"),
    })
}

/// Lists Compose projects (all states) and Orbital's stack folders. Read-only.
pub const LIST_SCRIPT: &str = r#"echo "@@ls"
docker compose ls -a --format json 2>/dev/null || echo '[]'
echo "@@ours"
for f in /opt/stacks/*/compose.yaml; do [ -f "$f" ] && echo "$f"; done
true"#;

#[derive(Deserialize)]
struct LsEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "ConfigFiles", default)]
    config_files: String,
}

pub fn parse_list(out: &str) -> Vec<Stack> {
    let ls = out.split("@@ours").next().unwrap_or("").replace("@@ls", "");
    let ours: Vec<&str> = out
        .split("@@ours")
        .nth(1)
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/'))
        .collect();
    let mut stacks: Vec<Stack> = serde_json::from_str::<Vec<LsEntry>>(ls.trim())
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let files: Vec<String> = e
                .config_files
                .split(',')
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(str::to_string)
                .collect();
            Stack {
                running: e.status.contains("running"),
                ours: files.iter().any(|f| f.starts_with(&format!("{STACK_DIR}/"))),
                status: e.status,
                files,
                name: e.name,
            }
        })
        .collect();
    for f in ours {
        let name = f.trim_start_matches(&format!("{STACK_DIR}/")).trim_end_matches("/compose.yaml");
        if validate_name(name).is_ok() && !stacks.iter().any(|s| s.name == name) {
            stacks.push(Stack {
                name: name.to_string(),
                status: "not deployed".into(),
                files: vec![f.to_string()],
                running: false,
                ours: true,
            });
        }
    }
    stacks.sort_by(|a, b| b.running.cmp(&a.running).then(a.name.cmp(&b.name)));
    stacks
}

/// A starting point for a new stack's file.
pub fn template(name: &str) -> String {
    format!(
        "# {name}: a Docker Compose stack made in Orbital.\n# Deploy runs `docker compose up -d` in this folder ({STACK_DIR}/{name}).\nservices:\n  web:\n    image: nginx:alpine\n    restart: unless-stopped\n    ports:\n      - \"127.0.0.1:8080:80\"\n"
    )
}

/// Checks a stack file with Compose itself (`{file}` is the staged copy).
pub const CONFIG_CHECK: &str = "docker compose -f {file} config -q";

pub fn mkdir_command(name: &str) -> Result<String> {
    validate_name(name)?;
    Ok(format!("mkdir -p -- {}", quote(&format!("{STACK_DIR}/{name}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_commands() {
        assert!(validate_name("shop").is_ok() && validate_name("shop_2-x").is_ok());
        for bad in ["Shop", "-x", "a b", "a;b", "", "a.b"] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        let f = stack_file("shop");
        assert_eq!(f, "/opt/stacks/shop/compose.yaml");
        assert_eq!(
            args("shop", &f, StackAction::Up).unwrap(),
            "compose -p shop -f /opt/stacks/shop/compose.yaml up -d --remove-orphans 2>&1"
        );
        assert!(args("shop", &f, StackAction::PullUp)
            .unwrap()
            .contains(" pull 2>&1 && docker compose -p shop -f "));
        assert!(args("shop", "/srv/my app/docker-compose.yml", StackAction::Down)
            .unwrap()
            .contains("-f '/srv/my app/docker-compose.yml' down"));
        assert!(args("shop", "relative.yml", StackAction::Up).is_err());
        for a in [
            StackAction::Up,
            StackAction::PullUp,
            StackAction::Restart,
            StackAction::Down,
            StackAction::Logs,
        ] {
            let c = format!("docker {}", args("shop", &f, a).unwrap());
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
            assert!(o.status.success(), "{c}");
        }
    }

    #[test]
    fn lists_projects_and_our_undeployed_stacks() {
        let out = "@@ls\n[{\"Name\":\"grafana\",\"Status\":\"running(2)\",\"ConfigFiles\":\"/srv/obs/docker-compose.yml\"},{\"Name\":\"shop\",\"Status\":\"exited(1)\",\"ConfigFiles\":\"/opt/stacks/shop/compose.yaml\"}]\n@@ours\n/opt/stacks/shop/compose.yaml\n/opt/stacks/blog/compose.yaml\n/opt/stacks/Bad Name/compose.yaml\n";
        let l = parse_list(out);
        assert_eq!(
            l.iter().map(|s| (s.name.as_str(), s.running, s.ours)).collect::<Vec<_>>(),
            [("grafana", true, false), ("blog", false, true), ("shop", false, true)]
        );
        assert_eq!(l[1].status, "not deployed");
        assert!(parse_list("@@ls\nnot json\n@@ours\n").is_empty());
        let o = std::process::Command::new("sh")
            .arg("-n")
            .arg("-c")
            .arg(LIST_SCRIPT)
            .output()
            .unwrap();
        assert!(o.status.success());
        assert!(template("shop").contains("services:"));
    }
}
