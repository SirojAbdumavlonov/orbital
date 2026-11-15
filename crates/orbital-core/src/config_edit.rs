//! "Safe save" for configuration files — the feature that makes editing
//! /etc/nginx/nginx.conf from a GUI not scary:
//!
//! 1. content is uploaded to a private temp file,
//! 2. validators that accept a file path check the temp file *before* install,
//! 3. the original is backed up (`<file>.orbital-bak-<timestamp>`, last 5 kept),
//! 4. the new content is written in place (owner, mode and inode preserved),
//! 5. whole-config validators (`nginx -t`, `sshd -t`, …) run after install and
//!    the original is restored automatically if they fail.

use crate::error::{Error, Result};
use crate::shell::{parent, quote, validate_path};
use crate::ssh::SshSession;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ValidateStage {
    /// Validate the temp file before touching the real one (`{file}` placeholder).
    PreInstall,
    /// Validate the live config after install; roll back on failure.
    PostInstall,
    /// Validated locally before upload.
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Validator {
    pub name: String,
    pub command: String,
    pub stage: ValidateStage,
}

fn v(name: &str, command: &str, stage: ValidateStage) -> Option<Validator> {
    Some(Validator {
        name: name.into(),
        command: command.into(),
        stage,
    })
}

pub fn detect_validator(path: &str) -> Option<Validator> {
    use ValidateStage::*;
    let file = path.rsplit('/').next().unwrap_or(path);
    if path.starts_with("/etc/nginx/") || path.starts_with("/usr/local/openresty/nginx/conf/") {
        return v("nginx", "nginx -t", PostInstall);
    }
    if path == "/etc/ssh/sshd_config" || path.starts_with("/etc/ssh/sshd_config.d/") {
        return v("sshd", "sshd -t", PostInstall);
    }
    if path.starts_with("/etc/apache2/") || path.starts_with("/etc/httpd/") {
        return v("apache", "apachectl configtest", PostInstall);
    }
    if path.starts_with("/etc/haproxy/") && file.ends_with(".cfg") {
        return v("haproxy", "haproxy -c -f {file}", PreInstall);
    }
    if path == "/etc/sudoers" || path.starts_with("/etc/sudoers.d/") {
        return v("sudoers", "visudo -cf {file}", PreInstall);
    }
    if path.starts_with("/etc/caddy/") && file == "Caddyfile" {
        return v("caddy", "caddy validate --adapter caddyfile --config {file}", PreInstall);
    }
    if matches!(file, "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml") {
        return v("docker compose", "docker compose -f {path} config -q", PostInstall);
    }
    if path.starts_with("/etc/systemd/system/") && (file.ends_with(".service") || file.ends_with(".timer")) {
        return v("systemd", "systemd-analyze verify {path}", PostInstall);
    }
    if path.starts_with("/etc/fail2ban/") && (file.ends_with(".conf") || file.ends_with(".local")) {
        return v("fail2ban", "fail2ban-client --test", PostInstall);
    }
    if path == "/etc/fstab" {
        return v("fstab", "findmnt --verify --tab-file {file}", PreInstall);
    }
    if file.ends_with(".json") {
        return v("json", "", Local);
    }
    None
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeWriteReport {
    pub path: String,
    pub bytes: usize,
    pub privileged: bool,
    pub backup_path: Option<String>,
    pub validator: Option<String>,
    pub validator_output: Option<String>,
    pub new_mtime: Option<u32>,
    /// e.g. "nginx" -> UI offers "Reload nginx now".
    pub suggested_reload: Option<String>,
}

fn render(cmd: &str, file: &str, path: &str) -> String {
    cmd.replace("{file}", &quote(file)).replace("{path}", &quote(path))
}

fn suggested_reload(validator: &Option<Validator>) -> Option<String> {
    match validator.as_ref()?.name.as_str() {
        "nginx" => Some("nginx.service".into()),
        "sshd" => Some("ssh.service".into()),
        "apache" => Some("apache2.service".into()),
        "haproxy" => Some("haproxy.service".into()),
        "caddy" => Some("caddy.service".into()),
        "fail2ban" => Some("fail2ban.service".into()),
        _ => None,
    }
}

impl SshSession {
    async fn run_maybe_privileged(&self, cmd: &str, privileged: bool) -> Result<crate::ssh::ExecOutput> {
        if privileged {
            self.exec_privileged(cmd).await
        } else {
            self.exec(cmd).await
        }
    }

    pub async fn safe_write(
        &self,
        path: &str,
        content: &str,
        validator: Option<Validator>,
        privileged: bool,
        expected_mtime: Option<u32>,
    ) -> Result<SafeWriteReport> {
        validate_path(path)?;
        if let Some(val) = &validator {
            if val.stage == ValidateStage::Local && path.ends_with(".json") {
                serde_json::from_str::<serde_json::Value>(content).map_err(|e| Error::ValidationFailed {
                    validator: "json".into(),
                    output: format!("invalid JSON: {e}"),
                })?;
            }
        }
        let sftp = self.sftp().await?;
        if let Some(expected) = expected_mtime {
            if let Ok(a) = sftp.metadata(path).await {
                if a.mtime.is_some() && a.mtime != Some(expected) {
                    return Err(Error::Conflict { remote_mtime: a.mtime });
                }
            }
        }

        // 1. stage to a private temp file (0600) the login user can write.
        let tmp = format!("/tmp/.orbital-{}", uuid::Uuid::new_v4().simple());
        {
            use russh_sftp::protocol::{FileAttributes, OpenFlags};
            use tokio::io::AsyncWriteExt;
            let mut attrs = FileAttributes::empty();
            attrs.permissions = Some(0o600);
            let mut f = sftp
                .open_with_flags_and_attributes(tmp.clone(), OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE, attrs)
                .await?;
            f.write_all(content.as_bytes()).await?;
            f.shutdown().await?;
        }
        let cleanup = || async {
            let _ = sftp.remove_file(tmp.clone()).await;
        };

        // 2. pre-install validation against the temp file.
        let mut validator_output = None;
        if let Some(val) = validator.as_ref().filter(|v| v.stage == ValidateStage::PreInstall) {
            let o = self
                .run_maybe_privileged(&format!("{} 2>&1", render(&val.command, &tmp, path)), privileged)
                .await;
            match o {
                Ok(o) if o.ok() => validator_output = Some(o.combined()),
                Ok(o) => {
                    cleanup().await;
                    return Err(Error::ValidationFailed {
                        validator: val.name.clone(),
                        output: o.combined(),
                    });
                }
                Err(e) => {
                    cleanup().await;
                    return Err(e);
                }
            }
        }

        // 3+4. backup and install in place.
        let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let (f, t) = (quote(path), quote(&tmp));
        let backup = format!("{path}.orbital-bak-{ts}");
        let b = quote(&backup);
        let script = format!(
            "set -e; trap 'rm -f -- {t}' EXIT; \
             if [ -e {f} ]; then cp -p -- {f} {b}; cat -- {t} > {f}; echo BACKUP; \
             ls -1t -- {f}.orbital-bak-* 2>/dev/null | tail -n +6 | while IFS= read -r old; do rm -f -- \"$old\"; done; \
             else mkdir -p -- {d}; cp -- {t} {f}; chmod 644 -- {f}; echo NEW; fi",
            d = quote(&parent(path))
        );
        let out = self.run_maybe_privileged(&script, privileged).await?;
        if !out.ok() {
            cleanup().await;
            let msg = out.combined();
            if msg.to_lowercase().contains("permission denied") {
                return Err(Error::PermissionDenied(msg));
            }
            return Err(Error::Remote {
                code: out.exit_code,
                stderr: msg,
            });
        }
        let backup_path = out.stdout.contains("BACKUP").then(|| backup.clone());

        // 5. post-install validation with automatic rollback.
        if let Some(val) = validator.as_ref().filter(|v| v.stage == ValidateStage::PostInstall) {
            let o = self
                .run_maybe_privileged(&format!("{} 2>&1", render(&val.command, path, path)), privileged)
                .await?;
            if !o.ok() {
                let rollback = match &backup_path {
                    Some(_) => format!("cat -- {b} > {f} && rm -f -- {b}"),
                    None => format!("rm -f -- {f}"),
                };
                let rb = self.run_maybe_privileged(&rollback, privileged).await?;
                let mut output = o.combined();
                if !rb.ok() {
                    output.push_str(&format!("\n\n⚠ ROLLBACK FAILED: {} — backup kept at {backup}", rb.combined()));
                }
                return Err(Error::ValidationFailed {
                    validator: val.name.clone(),
                    output,
                });
            }
            validator_output = Some(o.combined());
        }

        Ok(SafeWriteReport {
            path: path.to_string(),
            bytes: content.len(),
            privileged,
            backup_path,
            suggested_reload: suggested_reload(&validator),
            validator: validator.map(|v| v.name),
            validator_output,
            new_mtime: sftp.metadata(path).await.ok().and_then(|a| a.mtime),
        })
    }

    pub async fn list_backups(&self, path: &str, privileged: bool) -> Result<Vec<String>> {
        validate_path(path)?;
        let o = self
            // Backups made before the app was renamed end in `.outpost-bak-`.
            .run_maybe_privileged(
                &format!("ls -1t -- {p}.orbital-bak-* {p}.outpost-bak-* 2>/dev/null || true", p = quote(path)),
                privileged,
            )
            .await?;
        Ok(o.stdout.lines().map(String::from).filter(|l| !l.is_empty()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detection() {
        assert_eq!(detect_validator("/etc/nginx/sites-enabled/shop").unwrap().command, "nginx -t");
        assert_eq!(detect_validator("/etc/ssh/sshd_config").unwrap().name, "sshd");
        assert_eq!(detect_validator("/etc/sudoers.d/deploy").unwrap().stage, ValidateStage::PreInstall);
        assert_eq!(detect_validator("/srv/app/docker-compose.yml").unwrap().name, "docker compose");
        assert_eq!(detect_validator("/srv/app/package.json").unwrap().stage, ValidateStage::Local);
        assert!(detect_validator("/home/me/notes.txt").is_none());
        assert_eq!(
            render("visudo -cf {file}", "/tmp/.orbital-x", "/etc/sudoers"),
            "visudo -cf /tmp/.orbital-x"
        );
    }
}
