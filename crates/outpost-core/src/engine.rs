//! `Engine` is the single entry point used by the UI. Every state-changing
//! operation goes through here and is written to the audit log — success or
//! failure — so there is no code path that mutates a server unaudited.

use crate::audit::{self, AuditEvent, AuditFilter, Category, NewEvent, Outcome, VerifyReport};
use crate::config_edit::{detect_validator, SafeWriteReport, Validator};
use crate::db::Database;
use crate::docker::{Container, ContainerAction, ContainerStats};
use crate::error::{Error, Result};
use crate::known_hosts::{self, KnownHost};
use crate::metrics::{self, Metrics};
use crate::profiles::{self, Profile, ProfileInput};
use crate::secrets::{secret_key, SecretKind, SecretStore};
use crate::services::{ActionReport, Service, ServiceAction, ServiceDetail};
use crate::sftp::{FileContent, FileEntry};
use crate::ssh::{ConnectOptions, ExecOutput, SshSession};
use crate::terminal::{TermEvent, Terminal};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub profile_id: String,
    pub label: String,
    pub home: String,
    pub has_systemd: bool,
    pub docker_version: Option<String>,
    pub elevation: bool,
    pub connected_at: String,
}

pub struct Engine {
    pub db: Arc<Database>,
    secrets: Arc<dyn SecretStore>,
    sessions: RwLock<HashMap<String, Arc<SshSession>>>,
    terminals: Mutex<HashMap<String, (String, Arc<Terminal>)>>,
    last_metrics: Mutex<HashMap<String, Metrics>>,
}

fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

impl Engine {
    pub fn new(db: Arc<Database>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            db,
            secrets,
            sessions: RwLock::new(HashMap::new()),
            terminals: Mutex::new(HashMap::new()),
            last_metrics: Mutex::new(HashMap::new()),
        }
    }

    // ---------- audit plumbing ----------

    fn record(&self, s: Option<&SshSession>, ev: NewEvent) {
        let ev = NewEvent {
            profile_id: ev.profile_id.or_else(|| s.map(|s| s.profile.id.clone())),
            server: ev.server.or_else(|| s.map(|s| s.label())),
            ..ev
        };
        if let Err(e) = audit::record(&self.db, ev) {
            tracing::error!("failed to write audit event: {e}");
        }
    }

    /// Run `fut` and record its outcome. `command` is what was sent to the server.
    async fn audited<T, F>(
        &self,
        s: &SshSession,
        category: Category,
        action: &str,
        target: Option<&str>,
        command: Option<String>,
        fut: F,
    ) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        let t = Instant::now();
        let r = fut.await;
        let (outcome, details) = match &r {
            Ok(_) => (Outcome::Success, None),
            Err(Error::PermissionDenied(m)) => (Outcome::Denied, Some(serde_json::json!({ "error": m }))),
            Err(e) => (
                Outcome::Failure,
                Some(serde_json::json!({ "error": e.to_string(), "kind": e.kind() })),
            ),
        };
        self.record(
            Some(s),
            NewEvent {
                category: Some(category),
                action: action.into(),
                target: target.map(String::from),
                command,
                outcome: Some(outcome),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details,
                ..Default::default()
            },
        );
        r
    }

    fn record_exec(&self, s: &SshSession, category: Category, action: &str, target: &str, command: &str, o: &ExecOutput) {
        self.record(
            Some(s),
            NewEvent {
                category: Some(category),
                action: action.into(),
                target: Some(target.into()),
                command: Some(command.into()),
                outcome: Some(if o.ok() { Outcome::Success } else { Outcome::Failure }),
                exit_code: Some(o.exit_code),
                duration_ms: Some(o.duration_ms),
                details: (!o.ok()).then(|| serde_json::json!({ "stderr": o.stderr.chars().take(4000).collect::<String>() })),
                ..Default::default()
            },
        );
    }

    fn record_error(&self, s: &SshSession, category: Category, action: &str, target: &str, command: Option<String>, e: &Error) {
        let outcome = match e {
            Error::PermissionDenied(_) => Outcome::Denied,
            Error::PreflightFailed { .. } => Outcome::Denied,
            _ => Outcome::Failure,
        };
        self.record(
            Some(s),
            NewEvent {
                category: Some(category),
                action: action.into(),
                target: Some(target.into()),
                command,
                outcome: Some(outcome),
                details: Some(serde_json::json!({ "error": e.to_string(), "kind": e.kind() })),
                ..Default::default()
            },
        );
    }

    pub async fn session(&self, id: &str) -> Result<Arc<SshSession>> {
        let s = self
            .sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotConnected(id.into()))?;
        if s.is_closed() {
            self.sessions.write().await.remove(id);
            return Err(Error::NotConnected(id.into()));
        }
        Ok(s)
    }

    // ---------- profiles & credentials ----------

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        profiles::list(&self.db)
    }

    pub fn create_profile(&self, input: ProfileInput) -> Result<Profile> {
        let p = profiles::create(&self.db, input)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: "profile.create".into(),
                profile_id: Some(p.id.clone()),
                target: Some(format!("{}@{}:{}", p.username, p.host, p.port)),
                ..Default::default()
            },
        );
        Ok(p)
    }

    pub fn update_profile(&self, id: &str, input: ProfileInput) -> Result<Profile> {
        let p = profiles::update(&self.db, id, input)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: "profile.update".into(),
                profile_id: Some(p.id.clone()),
                target: Some(format!("{}@{}:{}", p.username, p.host, p.port)),
                ..Default::default()
            },
        );
        Ok(p)
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        for kind in [SecretKind::Password, SecretKind::KeyPassphrase, SecretKind::SudoPassword] {
            self.secrets.delete(&secret_key(id, kind))?;
        }
        profiles::delete(&self.db, id)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: "profile.delete".into(),
                profile_id: Some(id.into()),
                ..Default::default()
            },
        );
        Ok(())
    }

    pub fn set_secret(&self, profile_id: &str, kind: SecretKind, value: Option<&str>) -> Result<()> {
        let key = secret_key(profile_id, kind);
        match value {
            Some(v) if !v.is_empty() => self.secrets.set(&key, v)?,
            _ => self.secrets.delete(&key)?,
        }
        // The secret itself is never logged — only that it changed.
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: format!("credential.{}", if value.is_some() { "save" } else { "remove" }),
                profile_id: Some(profile_id.into()),
                target: Some(format!("{kind:?}")),
                ..Default::default()
            },
        );
        Ok(())
    }

    pub fn has_secret(&self, profile_id: &str, kind: SecretKind) -> Result<bool> {
        Ok(self.secrets.get(&secret_key(profile_id, kind))?.is_some())
    }

    pub fn known_hosts(&self) -> Result<Vec<KnownHost>> {
        known_hosts::list(&self.db)
    }

    pub fn forget_host(&self, host: &str, port: u16) -> Result<()> {
        known_hosts::forget(&self.db, host, port)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: "hostkey.forget".into(),
                target: Some(format!("{host}:{port}")),
                ..Default::default()
            },
        );
        Ok(())
    }

    // ---------- sessions ----------

    pub async fn connect(&self, profile_id: &str, opts: ConnectOptions) -> Result<SessionInfo> {
        let profile = profiles::get(&self.db, profile_id)?;
        let target = format!("{}@{}:{}", profile.username, profile.host, profile.port);
        let accepted = opts.accept_fingerprint.clone();
        let t = Instant::now();
        let res = SshSession::connect(self.db.clone(), self.secrets.as_ref(), profile.clone(), opts).await;
        // A host key may get pinned even if authentication then fails — audit it either way.
        if let Some(fp) = &accepted {
            let pinned = known_hosts::list(&self.db)?
                .into_iter()
                .any(|k| k.host == profile.host && k.port == profile.port && &k.fingerprint == fp);
            if pinned {
                self.record(
                    None,
                    NewEvent {
                        category: Some(Category::Security),
                        action: "hostkey.trust".into(),
                        profile_id: Some(profile.id.clone()),
                        server: Some(target.clone()),
                        target: Some(target.clone()),
                        details: Some(serde_json::json!({ "fingerprint": fp })),
                        ..Default::default()
                    },
                );
            }
        }
        let session = match res {
            Ok(s) => s,
            Err(e) => {
                let (cat, outcome) = match &e {
                    Error::HostKeyMismatch { .. } => (Category::Security, Outcome::Denied),
                    Error::UnknownHostKey { .. } | Error::CredentialsRequired { .. } => (Category::Connection, Outcome::Denied),
                    Error::AuthFailed { .. } => (Category::Security, Outcome::Failure),
                    _ => (Category::Connection, Outcome::Failure),
                };
                self.record(
                    None,
                    NewEvent {
                        category: Some(cat),
                        action: "session.open".into(),
                        profile_id: Some(profile.id.clone()),
                        server: Some(target.clone()),
                        target: Some(target),
                        outcome: Some(outcome),
                        duration_ms: Some(t.elapsed().as_millis() as i64),
                        details: Some(serde_json::json!({ "error": e.to_string(), "kind": e.kind() })),
                        ..Default::default()
                    },
                );
                return Err(e);
            }
        };
        profiles::touch(&self.db, &profile.id)?;
        let home = match profile.initial_path.clone().filter(|p| p.starts_with('/')) {
            Some(p) => p,
            None => session.home_dir().await.unwrap_or_else(|_| "/".into()),
        };
        let has_systemd = session.has_systemd().await;
        let docker_version = session.docker_available().await.ok().flatten();
        let info = SessionInfo {
            id: session.id.clone(),
            profile_id: profile.id.clone(),
            label: session.label(),
            home,
            has_systemd,
            docker_version,
            elevation: session.elevation_needed(),
            connected_at: session.connected_at.to_rfc3339(),
        };
        self.record(
            Some(&session),
            NewEvent {
                category: Some(Category::Connection),
                action: "session.open".into(),
                target: Some(target),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                ..Default::default()
            },
        );
        self.sessions.write().await.insert(session.id.clone(), Arc::new(session));
        Ok(info)
    }

    pub async fn disconnect(&self, session_id: &str) -> Result<()> {
        let s = self.sessions.write().await.remove(session_id);
        let mut terms = self.terminals.lock().await;
        let ids: Vec<String> = terms
            .iter()
            .filter(|(_, (sid, _))| sid == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some((_, t)) = terms.remove(&id) {
                t.close().await;
            }
        }
        drop(terms);
        self.last_metrics.lock().await.remove(session_id);
        if let Some(s) = s {
            s.close().await;
            self.record(
                Some(&s),
                NewEvent {
                    category: Some(Category::Connection),
                    action: "session.close".into(),
                    target: Some(s.label()),
                    ..Default::default()
                },
            );
        }
        Ok(())
    }

    // ---------- files ----------

    pub async fn list_dir(&self, sid: &str, path: &str) -> Result<Vec<FileEntry>> {
        self.session(sid).await?.list_dir(path).await
    }

    pub async fn read_file(&self, sid: &str, path: &str) -> Result<FileContent> {
        self.session(sid).await?.read_text(path).await
    }

    pub fn validator_for(&self, path: &str) -> Option<Validator> {
        detect_validator(path)
    }

    /// Plain save (SFTP, in place) with conflict detection.
    pub async fn write_file(&self, sid: &str, path: &str, content: &str, expected_mtime: Option<u32>) -> Result<Option<u32>> {
        let s = self.session(sid).await?;
        let details_hash = sha256(content.as_bytes());
        let r = self
            .audited(
                &s,
                Category::File,
                "file.write",
                Some(path),
                None,
                s.write_file(path, content.as_bytes(), expected_mtime),
            )
            .await;
        if r.is_ok() {
            tracing::debug!("wrote {path} sha256={details_hash}");
        }
        r
    }

    /// Safe save: backup + validate + rollback, optionally via sudo.
    pub async fn safe_write(
        &self,
        sid: &str,
        path: &str,
        content: &str,
        use_validator: bool,
        privileged: bool,
        expected_mtime: Option<u32>,
    ) -> Result<SafeWriteReport> {
        let s = self.session(sid).await?;
        let validator = if use_validator { detect_validator(path) } else { None };
        let t = Instant::now();
        let r = s.safe_write(path, content, validator.clone(), privileged, expected_mtime).await;
        let (outcome, details) = match &r {
            Ok(rep) => (
                Outcome::Success,
                serde_json::json!({ "sha256": sha256(content.as_bytes()), "bytes": rep.bytes, "backup": rep.backup_path, "validator": rep.validator, "privileged": privileged }),
            ),
            Err(e) => (
                if matches!(e, Error::PermissionDenied(_)) {
                    Outcome::Denied
                } else {
                    Outcome::Failure
                },
                serde_json::json!({ "error": e.to_string(), "kind": e.kind(), "validator": validator.map(|v| v.name), "privileged": privileged }),
            ),
        };
        self.record(
            Some(&s),
            NewEvent {
                category: Some(Category::Config),
                action: "config.write".into(),
                target: Some(path.into()),
                outcome: Some(outcome),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details: Some(details),
                ..Default::default()
            },
        );
        r
    }

    pub async fn list_backups(&self, sid: &str, path: &str, privileged: bool) -> Result<Vec<String>> {
        self.session(sid).await?.list_backups(path, privileged).await
    }

    pub async fn mkdir(&self, sid: &str, path: &str) -> Result<()> {
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.mkdir", Some(path), None, s.mkdir(path))
            .await
    }

    pub async fn create_file(&self, sid: &str, path: &str) -> Result<()> {
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.create", Some(path), None, s.create_file(path))
            .await
    }

    pub async fn rename(&self, sid: &str, from: &str, to: &str) -> Result<()> {
        let s = self.session(sid).await?;
        self.audited(
            &s,
            Category::File,
            "file.rename",
            Some(&format!("{from} → {to}")),
            None,
            s.rename(from, to),
        )
        .await
    }

    pub async fn remove(&self, sid: &str, path: &str, recursive: bool) -> Result<()> {
        let s = self.session(sid).await?;
        let action = if recursive { "file.delete_recursive" } else { "file.delete" };
        self.audited(&s, Category::File, action, Some(path), None, s.remove(path, recursive))
            .await
    }

    pub async fn chmod(&self, sid: &str, path: &str, mode: u32) -> Result<()> {
        let s = self.session(sid).await?;
        self.audited(
            &s,
            Category::File,
            "file.chmod",
            Some(&format!("{path} ({mode:o})")),
            None,
            s.chmod(path, mode),
        )
        .await
    }

    pub async fn upload(&self, sid: &str, local: &std::path::Path, remote: &str) -> Result<u64> {
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.upload", Some(remote), None, s.upload(local, remote))
            .await
    }

    pub async fn download(&self, sid: &str, remote: &str, local: &std::path::Path) -> Result<u64> {
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.download", Some(remote), None, s.download(remote, local))
            .await
    }

    // ---------- monitoring ----------

    pub async fn metrics(&self, sid: &str) -> Result<Metrics> {
        let s = self.session(sid).await?;
        let snap = s.snapshot().await?;
        let mut last = self.last_metrics.lock().await;
        let m = metrics::derive(last.get(sid), snap, chrono::Utc::now().timestamp_millis());
        last.insert(sid.to_string(), m.clone());
        Ok(m)
    }

    // ---------- services ----------

    pub async fn list_services(&self, sid: &str) -> Result<Vec<Service>> {
        self.session(sid).await?.list_services().await
    }

    pub async fn service_detail(&self, sid: &str, unit: &str) -> Result<ServiceDetail> {
        self.session(sid).await?.service_detail(unit, 200).await
    }

    pub async fn service_action(&self, sid: &str, unit: &str, action: ServiceAction) -> Result<ActionReport> {
        let s = self.session(sid).await?;
        let name = format!("service.{}", action.verb());
        match s.service_action(unit, action).await {
            Ok(rep) => {
                self.record_exec(&s, Category::Service, &name, unit, &rep.command, &rep.output);
                Ok(rep)
            }
            Err(e) => {
                let cmd = crate::services::action_command(unit, action).ok().map(|c| s.privileged_command(&c));
                self.record_error(&s, Category::Service, &name, unit, cmd, &e);
                Err(e)
            }
        }
    }

    // ---------- docker ----------

    pub async fn list_containers(&self, sid: &str) -> Result<Vec<Container>> {
        self.session(sid).await?.list_containers().await
    }

    pub async fn container_stats(&self, sid: &str) -> Result<Vec<ContainerStats>> {
        self.session(sid).await?.container_stats().await
    }

    pub async fn inspect_container(&self, sid: &str, id: &str) -> Result<serde_json::Value> {
        self.session(sid).await?.inspect_container(id).await
    }

    pub async fn container_logs(&self, sid: &str, id: &str, tail: u32) -> Result<String> {
        self.session(sid).await?.container_logs(id, tail).await
    }

    pub async fn container_action(&self, sid: &str, id: &str, action: ContainerAction) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let name = format!("docker.{}", action.args().replace(' ', ""));
        let (cmd, out) = match s.container_action(id, action).await {
            Ok(v) => v,
            Err(e) => {
                self.record_error(&s, Category::Docker, &name, id, None, &e);
                return Err(e);
            }
        };
        self.record_exec(&s, Category::Docker, &name, id, &cmd, &out);
        out.check()
    }

    // ---------- commands & terminals ----------

    pub async fn run_command(&self, sid: &str, command: &str, privileged: bool) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let sent = if privileged {
            s.privileged_command(command)
        } else {
            command.to_string()
        };
        let r = if privileged {
            s.exec_privileged(command).await
        } else {
            s.exec(command).await
        };
        match &r {
            Ok(o) => self.record_exec(&s, Category::Command, "command.run", command, &sent, o),
            Err(e) => self.record_error(&s, Category::Command, "command.run", command, Some(sent), e),
        }
        r
    }

    pub async fn open_terminal<F>(&self, sid: &str, cols: u32, rows: u32, cwd: Option<String>, sink: F) -> Result<String>
    where
        F: Fn(TermEvent) + Send + Sync + 'static,
    {
        let s = self.session(sid).await?;
        let t = self
            .audited(
                &s,
                Category::Terminal,
                "terminal.open",
                cwd.as_deref().or(Some("~")),
                None,
                s.open_terminal(cols, rows, cwd.clone(), sink),
            )
            .await?;
        let id = t.id.clone();
        self.terminals.lock().await.insert(id.clone(), (sid.to_string(), Arc::new(t)));
        Ok(id)
    }

    async fn terminal(&self, tid: &str) -> Result<Arc<Terminal>> {
        self.terminals
            .lock()
            .await
            .get(tid)
            .map(|(_, t)| t.clone())
            .ok_or_else(|| Error::NotFound(format!("terminal {tid}")))
    }

    pub async fn terminal_write(&self, tid: &str, data: &[u8]) -> Result<()> {
        self.terminal(tid).await?.write(data).await
    }

    pub async fn terminal_resize(&self, tid: &str, cols: u32, rows: u32) -> Result<()> {
        self.terminal(tid).await?.resize(cols, rows).await
    }

    pub async fn terminal_close(&self, tid: &str) -> Result<()> {
        if let Some((sid, t)) = self.terminals.lock().await.remove(tid) {
            t.close().await;
            if let Ok(s) = self.session(&sid).await {
                self.record(
                    Some(&s),
                    NewEvent {
                        category: Some(Category::Terminal),
                        action: "terminal.close".into(),
                        target: t.cwd.clone(),
                        ..Default::default()
                    },
                );
            }
        }
        Ok(())
    }

    // ---------- audit ----------

    pub fn audit_query(&self, f: &AuditFilter) -> Result<Vec<AuditEvent>> {
        audit::query(&self.db, f)
    }

    pub fn audit_verify(&self) -> Result<VerifyReport> {
        audit::verify(&self.db)
    }

    pub fn audit_export(&self, format: &str) -> Result<String> {
        match format {
            "csv" => audit::export_csv(&self.db),
            _ => audit::export_json(&self.db),
        }
    }
}
