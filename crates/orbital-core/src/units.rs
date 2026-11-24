//! Turn a command into a systemd service (Services → New service).
//!
//! Orbital writes /etc/systemd/system/<name>.service (marked as its own on the
//! first line), checks it with `systemd-analyze verify` (rolled back if that
//! fails), then `daemon-reload` and `enable --now`. The command runs through
//! `/bin/sh -c`, escaped for systemd, so pipes and `&&` work. Environment
//! variables go into /etc/orbital/env/<name>.env (root only, 0600), never into
//! the world-readable unit file or the activity log. A unit Orbital didn't
//! create is never overwritten or shadowed.

use crate::error::{Error, Result};
use crate::shell::quote;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const MARK: &str = "# Created by Orbital";
pub const UNIT_DIR: &str = "/etc/systemd/system";
pub const ENV_DIR: &str = "/etc/orbital/env";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Restart {
    Always,
    OnFailure,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSpec {
    /// Without `.service`: lowercase letters, digits and `-_.@`.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A shell command line (run with /bin/sh -c).
    pub command: String,
    #[serde(default)]
    pub workdir: Option<String>,
    /// Run as this user (root when empty).
    #[serde(default)]
    pub user: Option<String>,
    pub restart: Restart,
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

static NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9_.@-]{0,62}$").unwrap());
static KEY: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]{0,127}$").unwrap());
static DIR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^/[A-Za-z0-9._@+\-/ ]*$").unwrap());

/// The spec as it will be written, or why not.
pub fn validate(s: &ServiceSpec) -> Result<ServiceSpec> {
    let bad = |m: String| Err(Error::InvalidInput(m));
    let name = s.name.trim().trim_end_matches(".service").to_string();
    if !NAME.is_match(&name) {
        return bad("the service name has lowercase letters, digits and - _ . @ (e.g. shop-api)".into());
    }
    let command = s.command.trim();
    if command.is_empty() {
        return bad("give the command to run".into());
    }
    if command.chars().any(|c| c.is_control()) || command.len() > 4000 {
        return bad("the command must be one line".into());
    }
    let description = s.description.trim();
    if description.chars().any(|c| c.is_control()) || description.chars().count() > 120 {
        return bad("the description is one short line".into());
    }
    let workdir = match s.workdir.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) if DIR.is_match(w) && !w.split('/').any(|p| p == "..") => Some(w.trim_end_matches('/').to_string())
            .filter(|w| !w.is_empty())
            .or(Some("/".into())),
        Some(w) => return bad(format!("{w:?} isn't a folder's full path")),
        None => None,
    };
    let user = match s.user.as_deref().map(str::trim).filter(|u| !u.is_empty() && *u != "root") {
        Some(u) => Some(crate::users::validate_user(u)?.to_string()),
        None => None,
    };
    let mut env = Vec::new();
    for (k, v) in &s.env {
        let k = k.trim();
        if k.is_empty() && v.is_empty() {
            continue;
        }
        if !KEY.is_match(k) {
            return bad(format!("{k:?} isn't a variable name (letters, digits and _)"));
        }
        if v.contains('\n') || v.contains('\0') {
            return bad(format!("the value of {k} must be one line"));
        }
        if env.iter().any(|(x, _): &(String, String)| x == k) {
            return bad(format!("{k} is set twice"));
        }
        env.push((k.to_string(), v.clone()));
    }
    Ok(ServiceSpec {
        name,
        description: description.to_string(),
        command: command.to_string(),
        workdir,
        user,
        restart: s.restart,
        env,
    })
}

/// A string as one systemd word in double quotes: `\` and `"` escaped, `%` and
/// `$` doubled so systemd doesn't expand them (the shell still does `$VAR`).
pub fn systemd_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn unit_path(name: &str) -> String {
    format!("{UNIT_DIR}/{name}.service")
}

pub fn env_path(name: &str) -> String {
    format!("{ENV_DIR}/{name}.env")
}

/// The unit file for a validated spec.
pub fn render_unit(s: &ServiceSpec) -> String {
    let mut u = format!("{MARK}; change it in Orbital (Services) or here.\n[Unit]\nDescription={}\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\n",
        if s.description.is_empty() { s.name.clone() } else { s.description.replace('%', "%%") });
    if let Some(w) = &s.workdir {
        // Taken literally (no quotes; spaces are fine), with systemd's % specifiers escaped.
        u.push_str(&format!("WorkingDirectory={}\n", w.replace('%', "%%")));
    }
    if let Some(user) = &s.user {
        u.push_str(&format!("User={user}\n"));
    }
    if !s.env.is_empty() {
        u.push_str(&format!("EnvironmentFile={}\n", env_path(&s.name)));
    }
    u.push_str(&format!("ExecStart=/bin/sh -c {}\n", systemd_quote(&s.command)));
    match s.restart {
        Restart::Always => u.push_str("Restart=always\nRestartSec=3\n"),
        Restart::OnFailure => u.push_str("Restart=on-failure\nRestartSec=3\n"),
        Restart::No => u.push_str("Restart=no\n"),
    }
    u.push_str("\n[Install]\nWantedBy=multi-user.target\n");
    u
}

/// The environment file (root only): `KEY="value"` lines.
pub fn render_env(s: &ServiceSpec) -> String {
    s.env
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"\n", v.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect()
}

/// Read-only: is there a unit by this name, and is it Orbital's? Prints
/// `@@none`, `@@ours` or `@@theirs <path>`.
pub fn existing_check(name: &str) -> String {
    let unit = quote(&format!("{name}.service"));
    let ours = quote(&unit_path(name));
    format!(
        "f=$(systemctl show -p FragmentPath --value -- {unit} 2>/dev/null); \
         if [ -f {ours} ] && head -1 {ours} | grep -q '^{MARK}'; then echo @@ours; \
         elif [ -n \"$f\" ] || [ -e {ours} ]; then echo \"@@theirs ${{f:-{plain}}}\"; else echo @@none; fi",
        plain = unit_path(name)
    )
}

/// Writes stdin to a root-only file (the env file) through a temporary file.
pub fn write_env_command(name: &str) -> String {
    let p = quote(&env_path(name));
    let t = quote(&format!("{}.orbital-tmp", env_path(name)));
    format!("umask 077 && mkdir -p -- {ENV_DIR} && chmod 700 -- {ENV_DIR} && cat > {t} && chmod 600 -- {t} && mv -f -- {t} {p}")
}

pub fn start_command(name: &str) -> String {
    let u = quote(&format!("{name}.service"));
    format!("systemctl daemon-reload && systemctl enable --now -- {u} 2>&1 && sleep 2 && systemctl is-active -- {u}")
}

/// Stops and removes an Orbital-made service (refused for any other unit).
pub fn remove_command(name: &str) -> String {
    let u = quote(&format!("{name}.service"));
    let f = quote(&unit_path(name));
    format!(
        "if ! head -1 {f} 2>/dev/null | grep -q '^{MARK}'; then echo 'only services created by Orbital can be removed here' >&2; exit 3; fi; \
         systemctl disable --now -- {u} 2>&1; rm -f -- {f} {e}; systemctl daemon-reload",
        e = quote(&env_path(name))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            name: "shop-api.service".into(),
            description: "Shop API".into(),
            command: "node server.js --port 3000 | tee -a /var/log/shop.log".into(),
            workdir: Some("/srv/shop/".into()),
            user: Some("deploy".into()),
            restart: Restart::Always,
            env: vec![
                ("NODE_ENV".into(), "production".into()),
                ("DB_PASSWORD".into(), "p\"a$s%".into()),
                ("".into(), "".into()),
            ],
        }
    }

    #[test]
    fn validates() {
        let v = validate(&spec()).unwrap();
        assert_eq!(v.name, "shop-api");
        assert_eq!(v.workdir.as_deref(), Some("/srv/shop"));
        assert_eq!(v.env.len(), 2, "empty rows are dropped");
        for bad in [
            ServiceSpec {
                name: "Shop API".into(),
                ..spec()
            },
            ServiceSpec {
                command: " ".into(),
                ..spec()
            },
            ServiceSpec {
                command: "a\nExecStartPre=/bin/evil".into(),
                ..spec()
            },
            ServiceSpec {
                workdir: Some("srv".into()),
                ..spec()
            },
            ServiceSpec {
                user: Some("a;b".into()),
                ..spec()
            },
            ServiceSpec {
                env: vec![("BAD KEY".into(), "x".into())],
                ..spec()
            },
            ServiceSpec {
                env: vec![("K".into(), "a\nb".into())],
                ..spec()
            },
            ServiceSpec {
                env: vec![("K".into(), "1".into()), ("K".into(), "2".into())],
                ..spec()
            },
        ] {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            validate(&ServiceSpec {
                user: Some("root".into()),
                ..spec()
            })
            .unwrap()
            .user,
            None
        );
    }

    #[test]
    fn renders_a_unit_without_secrets() {
        let v = validate(&spec()).unwrap();
        let u = render_unit(&v);
        assert!(u.starts_with(MARK));
        assert!(
            u.contains("ExecStart=/bin/sh -c \"node server.js --port 3000 | tee -a /var/log/shop.log\"\n"),
            "{u}"
        );
        assert!(u.contains("User=deploy\n") && u.contains("WorkingDirectory=/srv/shop\n") && u.contains("Restart=always\n"));
        assert!(u.contains("EnvironmentFile=/etc/orbital/env/shop-api.env\n"));
        assert!(!u.contains("DB_PASSWORD") && !u.contains("p\\\"a"), "secrets stay out of the unit");
        let e = render_env(&v);
        assert_eq!(e, "NODE_ENV=\"production\"\nDB_PASSWORD=\"p\\\"a$s%\"\n");
        assert_eq!(systemd_quote("a \"b\" $HOME 100% \\"), "\"a \\\"b\\\" $$HOME 100%% \\\\\"");
        let none = validate(&ServiceSpec {
            env: vec![],
            workdir: None,
            user: None,
            restart: Restart::OnFailure,
            ..spec()
        })
        .unwrap();
        let u = render_unit(&none);
        assert!(!u.contains("EnvironmentFile") && !u.contains("User=") && u.contains("Restart=on-failure"));
    }

    #[test]
    fn commands_are_valid_sh() {
        for c in [
            existing_check("shop-api"),
            write_env_command("shop-api"),
            start_command("shop-api"),
            remove_command("shop-api"),
        ] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
            assert!(o.status.success(), "{c}");
        }
        assert!(remove_command("x").contains("only services created by Orbital"));
    }
}
