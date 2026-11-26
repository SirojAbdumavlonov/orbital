//! Engine calls for kernel tuning, SSH hardening, the security audit, the
//! SQL console and terminal recordings. A child module of `engine`, so it
//! uses the same audit plumbing.

use super::*;
use crate::shell::quote;
use std::collections::BTreeMap;

const APPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

impl Engine {
    // ---------- kernel tuning ----------

    pub async fn kernel_report(&self, sid: &str) -> Result<crate::kernel::KernelReport> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(&crate::kernel::read_script()).await?;
        Ok(crate::kernel::parse(&o.stdout))
    }

    /// Write Orbital's sysctl file with exactly `values` and load it. An empty
    /// map removes the file. A refused value puts the previous file back.
    pub async fn kernel_apply(&self, sid: &str, values: &BTreeMap<String, String>) -> Result<crate::kernel::KernelReport> {
        let s = self.session(sid).await?;
        let content = crate::kernel::render(values)?;
        let keys: Vec<&str> = values.keys().map(String::as_str).collect();
        let script = crate::kernel::apply_script(&keys)?;
        let target = if values.is_empty() {
            format!("{} (removed)", crate::kernel::FILE)
        } else {
            values.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(", ")
        };
        let o = self
            .system_exec_with(&s, "kernel.apply", &target, &script, Some(content.as_bytes()), APPLY_TIMEOUT)
            .await?;
        if !o.ok() {
            if let Some(k) = crate::kernel::unknown_key(&o.stdout) {
                return Err(Error::InvalidInput(format!("{k} doesn't exist on this server's kernel")));
            }
            return Err(Error::Remote {
                code: o.exit_code,
                stderr: format!(
                    "the kernel refused a value, so the previous settings were put back:\n{}",
                    o.combined().trim()
                ),
            });
        }
        self.kernel_report(sid).await
    }
}

impl Engine {
    // ---------- SSH hardening ----------

    pub async fn harden_state(&self, sid: &str) -> Result<crate::harden::HardenState> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::harden::STATE_SCRIPT).await?;
        Ok(crate::harden::parse_state(&o.stdout))
    }

    /// Who this connection is, for the lock-out check.
    async fn ssh_access(&self, sid: &str) -> Result<crate::harden::Access> {
        let s = self.session(sid).await?;
        let keys = self.authorized_keys(sid).await.map(|k| k.keys).unwrap_or_default();
        Ok(crate::harden::Access {
            password_login: s.profile.auth_method == AuthMethod::Password,
            root: s.profile.username == "root",
            key_from_here: keys.iter().any(|k| k.on_this_computer),
        })
    }

    /// Write Orbital's sshd drop-in with exactly `choices` (empty removes it),
    /// test it with `sshd -t` and reload sshd. Refused with `PreflightFailed`
    /// when it would lock this connection out, unless `force`.
    pub async fn harden_ssh(&self, sid: &str, choices: &BTreeMap<String, String>, force: bool) -> Result<crate::harden::HardenResult> {
        let s = self.session(sid).await?;
        let state = self.harden_state(sid).await?;
        if !state.includes_dropins && !choices.is_empty() {
            return Err(Error::InvalidInput(
                "this server's sshd_config doesn't read /etc/ssh/sshd_config.d (no Include line, OpenSSH before 8.2). \
                 Change the settings in sshd_config in the editor instead."
                    .into(),
            ));
        }
        let content = crate::harden::render(choices)?;
        let target = if choices.is_empty() {
            format!("{} (removed)", crate::harden::DROPIN)
        } else {
            choices.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
        };
        if !force {
            if let Some(why) = crate::harden::lockout(choices, self.ssh_access(sid).await?) {
                let e = Error::PreflightFailed {
                    check: "your access".into(),
                    output: why,
                };
                self.record_error(&s, Category::Security, "sshd.harden", &target, None, &e);
                return Err(e);
            }
        }
        let script = crate::harden::apply_script(choices.is_empty());
        let sent = s.privileged_command(&script);
        let o = match s.exec_privileged_with(&script, Some(content.as_bytes()), APPLY_TIMEOUT).await {
            Ok(o) => o,
            Err(e) => {
                self.record_error(&s, Category::Security, "sshd.harden", &target, Some(sent), &e);
                return Err(e);
            }
        };
        // The effective settings sshd prints are long and not part of what was done.
        let shown = ExecOutput {
            stdout: o.stdout.split("@@effective").next().unwrap_or_default().to_string(),
            ..o.clone()
        };
        self.record_exec(&s, Category::Security, "sshd.harden", &target, &sent, &shown);
        if !o.ok() {
            return Err(Error::Remote {
                code: o.exit_code,
                stderr: format!(
                    "sshd wouldn't start with these settings, so nothing changed:\n{}",
                    shown.stdout.trim()
                ),
            });
        }
        Ok(crate::harden::HardenResult {
            not_in_effect: crate::harden::not_in_effect(choices, &o.stdout),
            state: self.harden_state(sid).await?,
        })
    }
}

impl Engine {
    // ---------- security audit ----------

    /// The read-only security scan (as root when sudo works).
    pub async fn security_audit(&self, sid: &str) -> Result<crate::secaudit::AuditReport> {
        let s = self.session(sid).await?;
        progress::step("Scanning accounts, permissions and open ports …");
        let (o, root) = s.exec_inspect(crate::secaudit::SCRIPT).await?;
        Ok(crate::secaudit::parse(&o.stdout, root))
    }
}

impl Engine {
    // ---------- SQL console ----------

    /// Run `query` against `db` on the server or in `container`. The query
    /// travels on stdin; the activity log gets its kind, size and hash only.
    pub async fn sql_query(
        &self,
        sid: &str,
        kind: crate::databases::DbKind,
        container: Option<&str>,
        db: &str,
        query: &str,
        read_only: bool,
    ) -> Result<crate::sqlconsole::SqlResult> {
        use crate::sqlconsole as sq;
        let s = self.session(sid).await?;
        sq::validate_query(query)?;
        let script = sq::script(kind, db, read_only)?;
        let timeout = std::time::Duration::from_secs(300);
        let started = Instant::now();
        let r = match container {
            Some(c) => {
                crate::shell::validate_container(c)?;
                let cmd = format!(
                    "docker exec -i -- {} sh -c {}",
                    quote(c),
                    quote(&format!("{}{script}", crate::docker::CONTAINER_PRELUDE))
                );
                match s.exec_with_input(&cmd, Some(query.as_bytes()), timeout).await {
                    Ok(o) if crate::docker::is_socket_denied(&o) && s.elevation_needed() => {
                        s.exec_privileged_with(&cmd, Some(query.as_bytes()), timeout).await
                    }
                    other => other,
                }
            }
            None if s.elevation_needed() => s.exec_privileged_with(&script, Some(query.as_bytes()), timeout).await,
            None => s.exec_with_input(&script, Some(query.as_bytes()), timeout).await,
        };
        let ms = started.elapsed().as_millis() as i64;
        let parsed = r.and_then(|o| sq::parse(kind, &o.stdout, ms));
        let (outcome, error) = match &parsed {
            Ok(res) if res.error.is_none() => (Outcome::Success, None),
            Ok(res) => (Outcome::Failure, res.error.clone()),
            Err(e) => (Outcome::Failure, Some(e.to_string())),
        };
        self.record(
            Some(&s),
            NewEvent {
                category: Some(Category::Command),
                action: "sql.query".into(),
                target: Some(format!(
                    "{} {db}{}{}",
                    kind.label(),
                    container.map(|c| format!(" in {c}")).unwrap_or_default(),
                    if read_only { " (read-only)" } else { "" }
                )),
                outcome: Some(outcome),
                duration_ms: Some(ms),
                details: Some(serde_json::json!({
                    "statement": sq::statement(query),
                    "bytes": query.len(),
                    "sha256": sq::fingerprint(query),
                    "readOnly": read_only,
                    "error": error.map(|e| e.chars().take(300).collect::<String>()),
                })),
                ..Default::default()
            },
        );
        parsed
    }
}

impl Engine {
    // ---------- terminal recordings ----------

    /// Open a terminal like [`Engine::open_terminal`], recording its output to
    /// a `.cast` file in the data folder.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_recorded_terminal<F>(
        &self,
        sid: &str,
        cols: u32,
        rows: u32,
        cwd: Option<String>,
        as_root: bool,
        launch: crate::terminal::Launch,
        sink: F,
    ) -> Result<String>
    where
        F: Fn(TermEvent) + Send + Sync + 'static,
    {
        let s = self.session(sid).await?;
        let title = format!("{}: {}", s.label(), cwd.as_deref().unwrap_or("~"));
        let rec = crate::recording::Recorder::create(crate::recording::dir()?, &title, cols, rows)?;
        let r2 = rec.clone();
        let tid = self
            .open_terminal(sid, cols, rows, cwd, as_root, launch, move |ev| {
                if let TermEvent::Data(d) = &ev {
                    r2.output(d);
                }
                sink(ev)
            })
            .await?;
        crate::recording::register(&tid, rec.clone());
        self.record(
            Some(&s),
            NewEvent {
                category: Some(Category::Terminal),
                action: "terminal.record".into(),
                target: Some(rec.name.clone()),
                outcome: Some(Outcome::Success),
                ..Default::default()
            },
        );
        Ok(tid)
    }

    pub fn recordings(&self) -> Result<Vec<crate::recording::RecordingInfo>> {
        Ok(crate::recording::list(crate::recording::dir()?))
    }

    /// The whole `.cast` file, to play back.
    pub fn read_recording(&self, name: &str) -> Result<String> {
        let path = crate::recording::dir()?.join(crate::recording::check_name(name)?);
        let meta = std::fs::metadata(&path)?;
        if meta.len() > 64 * 1024 * 1024 {
            return Err(Error::InvalidInput("this recording is too large to play here".into()));
        }
        Ok(std::fs::read_to_string(path)?)
    }

    pub fn delete_recording(&self, name: &str) -> Result<()> {
        let path = crate::recording::dir()?.join(crate::recording::check_name(name)?);
        std::fs::remove_file(path)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Terminal),
                action: "terminal.recording_delete".into(),
                target: Some(name.into()),
                outcome: Some(Outcome::Success),
                ..Default::default()
            },
        );
        Ok(())
    }
}
