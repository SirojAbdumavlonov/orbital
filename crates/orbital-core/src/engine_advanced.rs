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

impl Engine {
    // ---------- processes ----------

    pub async fn processes(&self, sid: &str) -> Result<Vec<crate::procs::Proc>> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::procs::LIST_SCRIPT).await?;
        Ok(crate::procs::parse_list(&o.stdout))
    }

    pub async fn process_detail(&self, sid: &str, pid: u32) -> Result<crate::procs::ProcDetail> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(&crate::procs::detail_script(pid)).await?;
        crate::procs::parse_detail(pid, &o.stdout)
    }

    /// The process's name, or NotFound when it has ended.
    async fn process_name(&self, s: &SshSession, pid: u32) -> Result<String> {
        let (o, _) = s.exec_inspect(&format!("ps -o comm= -p {pid}")).await?;
        let name = o.stdout.trim().to_string();
        if name.is_empty() {
            return Err(Error::NotFound(format!("no process {pid} (it may have ended)")));
        }
        Ok(name)
    }

    /// Send a signal (as root when the profile allows). PID 1 and sshd are protected.
    pub async fn signal_process(&self, sid: &str, pid: u32, signal: crate::procs::Signal) -> Result<()> {
        let s = self.session(sid).await?;
        let name = self.process_name(&s, pid).await?;
        let target = format!("{name} (PID {pid}) {}", signal.name());
        crate::procs::check_target(pid, &name, Some(signal))
            .map_err(|e| self.refused_as(&s, Category::Command, "process.signal", &target, e))?;
        self.system_exec(
            &s,
            "process.signal",
            &target,
            &crate::procs::signal_command(pid, signal),
            APPLY_TIMEOUT,
        )
        .await?
        .check()?;
        Ok(())
    }

    /// Change a process's priority (nice -20 … 19).
    pub async fn renice_process(&self, sid: &str, pid: u32, nice: i32) -> Result<()> {
        let s = self.session(sid).await?;
        let name = self.process_name(&s, pid).await?;
        let target = format!("{name} (PID {pid}) nice {nice}");
        crate::procs::check_target(pid, &name, None).map_err(|e| self.refused_as(&s, Category::Command, "process.renice", &target, e))?;
        let cmd = crate::procs::renice_command(pid, nice)?;
        self.system_exec(&s, "process.renice", &target, &cmd, APPLY_TIMEOUT)
            .await?
            .check()?;
        Ok(())
    }

    // ---------- disks ----------

    pub async fn disk_report(&self, sid: &str) -> Result<crate::disks::DiskReport> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::disks::SCRIPT).await?;
        Ok(crate::disks::parse(&o.stdout))
    }

    // ---------- packages ----------

    pub async fn installed_packages(&self, sid: &str) -> Result<Vec<crate::packages::Package>> {
        let s = self.session(sid).await?;
        let pm = s.package_manager().await?;
        let o = s.exec(crate::packages::list_command(pm)).await?;
        Ok(crate::packages::parse_list(pm, &o.stdout))
    }

    pub async fn search_packages(&self, sid: &str, term: &str) -> Result<Vec<crate::packages::Found>> {
        let s = self.session(sid).await?;
        let pm = s.package_manager().await?;
        let cmd = crate::packages::search_command(pm, term)?;
        let o = s.exec(&cmd).await?;
        let installed = crate::packages::parse_list(pm, &s.exec(crate::packages::list_command(pm)).await?.stdout);
        Ok(crate::packages::parse_search(pm, &o.stdout, &installed))
    }

    /// A dry run of installing or removing one package.
    pub async fn package_plan(&self, sid: &str, name: &str, action: crate::packages::PkgAction) -> Result<crate::packages::PkgPlan> {
        let s = self.session(sid).await?;
        let pm = s.package_manager().await?;
        let (o, _) = s.exec_inspect(&crate::packages::plan_command(pm, name, action)?).await?;
        Ok(crate::packages::parse_plan(pm, &o.stdout))
    }

    /// Install or remove one package. A removal that takes along something
    /// the server needs to stay reachable is refused unless `force`.
    pub async fn package_change(&self, sid: &str, name: &str, action: crate::packages::PkgAction, force: bool) -> Result<ExecOutput> {
        use crate::packages::PkgAction;
        let s = self.session(sid).await?;
        let pm = s.package_manager().await?;
        let verb = if action == PkgAction::Install {
            "package.install"
        } else {
            "package.remove"
        };
        if action == PkgAction::Remove && !force {
            progress::step("Checking what removing it would take along …");
            let plan = self.package_plan(sid, name, action).await?;
            if !plan.essential.is_empty() {
                let e = Error::PreflightFailed {
                    check: "your access".into(),
                    output: format!(
                        "Removing {name} would also remove {}, which the server needs to boot or to let you in over SSH.",
                        plan.essential.join(", ")
                    ),
                };
                self.record_error(&s, Category::Command, verb, name, None, &e);
                return Err(e);
            }
        }
        progress::step(if action == PkgAction::Install {
            format!("Installing {name} …")
        } else {
            format!("Removing {name} …")
        });
        let cmd = crate::packages::run_command(pm, name, action)?;
        let o = self.system_exec(&s, verb, name, &cmd, std::time::Duration::from_secs(900)).await?;
        o.check()
    }

    // ---------- TLS inspector ----------

    /// Look at a TLS server's certificate from this computer (nothing on a server changes).
    pub async fn tls_inspect(&self, host: &str, port: u16) -> Result<crate::tlsinspect::TlsReport> {
        crate::tlsinspect::inspect(host.trim(), port).await
    }

    // ---------- timers ----------

    /// What systemd makes of a schedule, and when it would run next.
    pub async fn check_schedule(&self, sid: &str, schedule: &str) -> Result<crate::timers::ScheduleCheck> {
        let s = self.session(sid).await?;
        let o = s.exec(&crate::timers::calendar_command(schedule)?).await?;
        crate::timers::parse_calendar(&o.stdout, o.exit_code)
    }

    /// Create (or replace an Orbital-made) systemd timer and start it.
    pub async fn create_timer(&self, sid: &str, spec: &crate::timers::TimerSpec) -> Result<String> {
        use crate::timers as t;
        let s = self.session(sid).await?;
        let spec = t::validate(spec).map_err(|e| self.refused_as(&s, Category::Service, "timer.create", &spec.name, e))?;
        progress::step("Checking the schedule …");
        self.check_schedule(sid, &spec.schedule)
            .await
            .map_err(|e| self.refused_as(&s, Category::Service, "timer.create", &spec.name, e))?;
        let (o, _) = s.exec_inspect(&t::existing_check(&spec.name)).await?;
        if let Some(path) = o.stdout.lines().find_map(|l| l.strip_prefix("@@theirs ")) {
            let e = Error::InvalidInput(format!("{path} already exists and wasn't created by Orbital; choose another name"));
            return Err(self.refused_as(&s, Category::Service, "timer.create", &spec.name, e));
        }
        let verify = |path: String| crate::config_edit::Validator {
            name: "systemd-analyze".into(),
            command: format!("systemd-analyze verify {}", quote(&path)),
            stage: crate::config_edit::ValidateStage::PostInstall,
        };
        progress::step(format!("Writing {}.service and {}.timer …", spec.name, spec.name));
        self.audited(&s, Category::Service, "timer.create", Some(&spec.name), None, async {
            s.safe_write(&t::service_path(&spec.name), &t::render_service(&spec), None, true, None)
                .await?;
            s.safe_write(
                &t::timer_path(&spec.name),
                &t::render_timer(&spec),
                Some(verify(t::timer_path(&spec.name))),
                true,
                None,
            )
            .await
        })
        .await?;
        progress::step("Starting the timer …");
        let o = self
            .system_exec(&s, "timer.enable", &spec.name, &t::enable_command(&spec.name), APPLY_TIMEOUT)
            .await?
            .check()?;
        Ok(o.stdout.lines().last().unwrap_or("").trim().to_string())
    }

    pub async fn remove_timer(&self, sid: &str, name: &str) -> Result<()> {
        let s = self.session(sid).await?;
        let probe = crate::timers::TimerSpec {
            name: name.into(),
            description: String::new(),
            command: "true".into(),
            user: None,
            schedule: "daily".into(),
            persistent: false,
        };
        let name = crate::timers::validate(&probe)?.name;
        self.system_exec(&s, "timer.remove", &name, &crate::timers::remove_command(&name), APPLY_TIMEOUT)
            .await?
            .check()?;
        Ok(())
    }
}
