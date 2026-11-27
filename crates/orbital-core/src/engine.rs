//! `Engine` is the single entry point used by the UI. Every state-changing
//! operation goes through here and is written to the audit log — success or
//! failure — so there is no code path that mutates a server unaudited.

use crate::audit::{self, AuditEvent, AuditFilter, Category, NewEvent, Outcome, VerifyReport};
use crate::certbot::{self, CertbotStatus, IssueRequest, RenewMode};
use crate::config_edit::{detect_validator, SafeWriteReport, Validator};
use crate::cron::{self, CronChange, CronReport};
use crate::databases::{self, BackupFormat, DbKind, DbServer, RedisAction, RedisInfo};
use crate::db::Database;
use crate::deploy::{self, App, AppInput, Checkout, DeployRecord, RestartKind};
use crate::dns::{self, DnsAccount, DnsCheck, DnsRecord, DnsZone, ProviderKind, RecordInput, RecordType};
use crate::docker::{ComposeAction, ComposeTarget, Container, ContainerAction, ContainerStats};
use crate::error::{Error, Result};
use crate::fail2ban::{self, Fail2banStatus};
use crate::firewall::{self, UfwRule, UfwRuleInput, UfwRuleRef, UfwStatus};
use crate::known_hosts::{self, KnownHost};
use crate::logs::{LogFile, LogSource};
use crate::metrics::{self, Metrics};
use crate::nginx::NginxConfig;
use crate::ports::PortsReport;
use crate::presence::{Unavailable, UserPresence};
use crate::profiles::{self, AuthMethod, Profile, ProfileInput, Protocol};
use crate::progress;
use crate::secrets::{secret_key, GatedStore, SecretKind, SecretStore, Unlock};
use crate::services::{ActionReport, Service, ServiceAction, ServiceDetail};
use crate::sftp::{FileContent, FileEntry, TransferReport};
use crate::snippets;
use crate::ssh::{saved_secrets_wanted, ConnectOptions, ExecOutput, SshSession};
use crate::ssh_config;
use crate::sshd::SshdReport;
use crate::sshkeys;
use crate::storage::StorageSession;
use crate::streams::{Stream, StreamEvent};
use crate::sysinfo::SystemInfo;
use crate::terminal::{TermEvent, Terminal};
use crate::tools::DetectedTool;
use crate::tunnels::{self, Tunnel, TunnelInfo};
use crate::uptime::{self, Change, Check, CheckInput, CheckResult, CheckStatus, UptimeEvent};
use crate::usage::{DiskUsage, ProcessReport};
use crate::users::{self, NewUser, UserChange, UsersReport};
use crate::webservers::{self, ApacheStatus, HaServerState, HaproxyStatus};
use crate::website::{SiteKind, WebsiteInput, WebsitePlan, WebsiteReport};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

#[path = "engine_advanced.rs"]
mod advanced;
#[path = "engine_notify.rs"]
mod notify_calls;

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
    /// Jump hosts the session is tunnelled through, outermost first.
    pub via: Vec<String>,
    pub connected_at: String,
    /// SFTP over SSH, or a file-storage protocol (Files tab only).
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceStatus {
    /// This Mac can verify its owner (Touch ID or login password).
    pub available: bool,
    /// Saved credentials are only used after that verification.
    pub enabled: bool,
}

pub struct Engine {
    pub db: Arc<Database>,
    secrets: Arc<GatedStore>,
    presence: Arc<dyn UserPresence>,
    sessions: RwLock<HashMap<String, Arc<SshSession>>>,
    /// FTP, WebDAV and S3 connections: files only.
    storage: RwLock<HashMap<String, Arc<StorageSession>>>,
    terminals: Mutex<HashMap<String, (String, Arc<Terminal>)>>,
    last_metrics: Mutex<HashMap<String, Metrics>>,
    tunnels: Mutex<HashMap<String, Tunnel>>,
    log_streams: Mutex<HashMap<String, (String, Stream)>>,
    deploying: Mutex<HashSet<String>>,
    /// DNS provider clients, kept once their token was read (like a connection).
    dns_clients: Mutex<HashMap<String, Arc<dns::Cloudflare>>>,
    /// Cloudflare's API and the resolvers asked when checking; tests point them at local servers.
    dns_endpoints: std::sync::RwLock<(String, Vec<(String, String)>)>,
    uptime: std::sync::Mutex<uptime::Runtime>,
    uptime_listener: std::sync::RwLock<Option<uptime::Listener>>,
    http: std::sync::OnceLock<reqwest::Client>,
    /// Telegram's Bot API; tests point it at a local stand-in.
    telegram_base: std::sync::RwLock<String>,
    notify_brake: crate::notify::Brake,
}

/// What a deploy reports while it runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum DeployEvent {
    Step { name: String },
    Output { text: String },
    StepEnd { ok: bool, code: Option<i32> },
    Done { ok: bool, record: DeployRecord },
}

fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// What an SSH config import created, and what it couldn't.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub created: Vec<Profile>,
    pub failed: Vec<ImportFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    pub name: String,
    pub error: String,
}

impl Engine {
    /// An engine that can't verify the user: the Touch ID gate can't be turned on.
    pub fn new(db: Arc<Database>, secrets: Arc<dyn SecretStore>) -> Self {
        Self::with_presence(db, secrets, Arc::new(Unavailable))
    }

    pub fn with_presence(db: Arc<Database>, secrets: Arc<dyn SecretStore>, presence: Arc<dyn UserPresence>) -> Self {
        Self {
            db,
            secrets: Arc::new(GatedStore::new(secrets)),
            presence,
            sessions: RwLock::new(HashMap::new()),
            storage: RwLock::new(HashMap::new()),
            terminals: Mutex::new(HashMap::new()),
            last_metrics: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            log_streams: Mutex::new(HashMap::new()),
            deploying: Mutex::new(HashSet::new()),
            dns_clients: Mutex::new(HashMap::new()),
            dns_endpoints: std::sync::RwLock::new((
                dns::CLOUDFLARE_API.into(),
                dns::RESOLVERS.iter().map(|(l, u)| (l.to_string(), u.to_string())).collect(),
            )),
            uptime: Default::default(),
            uptime_listener: Default::default(),
            http: Default::default(),
            telegram_base: std::sync::RwLock::new("https://api.telegram.org".into()),
            notify_brake: Default::default(),
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
            .ok_or_else(|| Error::NotConnected(id.into()));
        let s = match s {
            Ok(s) => s,
            Err(_) if self.storage.read().await.contains_key(id) => {
                return Err(Error::InvalidInput(
                    "this is a file-storage connection (FTP, WebDAV or S3); this needs an SSH server".into(),
                ));
            }
            Err(e) => return Err(e),
        };
        if s.is_closed() {
            self.sessions.write().await.remove(id);
            return Err(Error::NotConnected(id.into()));
        }
        Ok(s)
    }

    async fn storage_session(&self, id: &str) -> Option<Arc<StorageSession>> {
        self.storage.read().await.get(id).cloned()
    }

    /// Like `audited`, for a file-storage connection.
    async fn storage_audited<T, F>(&self, s: &StorageSession, action: &str, target: &str, fut: F) -> Result<T>
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
            None,
            NewEvent {
                category: Some(Category::File),
                action: action.into(),
                profile_id: Some(s.profile.id.clone()),
                server: Some(s.label()),
                target: Some(target.into()),
                outcome: Some(outcome),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details,
                ..Default::default()
            },
        );
        r
    }

    /// Connect over FTP, FTPS, WebDAV or S3. The password (or S3 secret key)
    /// is the one typed now, or the saved one, read only through the gate.
    async fn connect_storage(&self, profile: Profile, opts: ConnectOptions) -> Result<SessionInfo> {
        let target = format!("{}@{}:{}", profile.username, profile.host, profile.port);
        let t = Instant::now();
        let res = async {
            let route = vec![profile.clone()];
            let _unlock = self.unlock_for(&route, &opts).await?;
            let typed = opts.typed_for(&profile, true);
            let key = secret_key(&profile.id, SecretKind::Password);
            let secret = match &typed {
                Some(s) => Some(s.clone()),
                None => self.secrets.get(&key)?,
            };
            let anonymous = profile.protocol != Protocol::S3
                && (profile.username.is_empty() || profile.username == "anonymous" || profile.username == "ftp");
            if secret.is_none() && !anonymous {
                return Err(Error::CredentialsRequired {
                    what: if profile.protocol == Protocol::S3 {
                        "Secret access key".into()
                    } else {
                        "Password".into()
                    },
                    user: profile.username.clone(),
                    host: profile.host.clone(),
                    profile_id: profile.id.clone(),
                });
            }
            let s = StorageSession::connect(profile.clone(), secret).await?;
            if opts.remember {
                if let Some(t) = &typed {
                    self.secrets.set(&key, t)?;
                }
            }
            Ok(s)
        }
        .await;
        let s = match res {
            Ok(s) => s,
            Err(e) => {
                let (cat, outcome) = match &e {
                    Error::PresenceRequired(_) => (Category::Security, Outcome::Denied),
                    Error::CredentialsRequired { .. } => (Category::Connection, Outcome::Denied),
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
                        details: Some(serde_json::json!({ "error": e.to_string(), "kind": e.kind(), "protocol": profile.protocol })),
                        ..Default::default()
                    },
                );
                return Err(e);
            }
        };
        profiles::touch(&self.db, &profile.id)?;
        let info = SessionInfo {
            id: s.id.clone(),
            profile_id: profile.id.clone(),
            label: s.label(),
            home: s.home.clone(),
            has_systemd: false,
            docker_version: None,
            elevation: false,
            via: vec![],
            connected_at: s.connected_at.to_rfc3339(),
            protocol: profile.protocol,
        };
        self.record(
            None,
            NewEvent {
                category: Some(Category::Connection),
                action: "session.open".into(),
                profile_id: Some(profile.id.clone()),
                server: Some(s.label()),
                target: Some(target),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details: Some(serde_json::json!({ "protocol": profile.protocol })),
                ..Default::default()
            },
        );
        self.storage.write().await.insert(s.id.clone(), Arc::new(s));
        Ok(info)
    }

    // ---------- profiles & credentials ----------

    // ---------- anonymous usage statistics ----------

    pub fn stats_state(&self) -> Result<crate::stats::StatsState> {
        crate::stats::state(&self.db)
    }

    /// The user's yes or no to anonymous usage statistics.
    pub fn set_stats(&self, enabled: bool) -> Result<crate::stats::StatsState> {
        crate::stats::set_enabled(&self.db, enabled)?;
        self.stats_state()
    }

    /// Send "opened" or "active" with the two counts, if the user said yes (see `stats`).
    pub async fn stats_ping(&self, event: crate::stats::StatsEvent) {
        let counts = crate::stats::Counts {
            servers_connected: self.sessions.read().await.len() + self.storage.read().await.len(),
            servers_saved: self.list_profiles().map(|p| p.len()).unwrap_or(0),
        };
        crate::stats::send(&self.db, event, counts).await;
    }

    /// Feedback from the Feedback page, sent to the project's server (see `stats::send_feedback`).
    /// Not in the activity log: it changes nothing on a server.
    pub async fn send_feedback(&self, feedback: &crate::stats::Feedback) -> Result<()> {
        crate::stats::send_feedback(feedback).await
    }

    pub fn feedback_available(&self) -> bool {
        crate::stats::feedback_available()
    }

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
        // Check before touching the Keychain: a refused delete must change nothing.
        let used_by = profiles::dependents(&self.db, id)?;
        if !used_by.is_empty() {
            return Err(Error::InvalidInput(format!(
                "this server is the jump host for {}; change those servers first",
                used_by.join(", ")
            )));
        }
        for kind in [SecretKind::Password, SecretKind::KeyPassphrase, SecretKind::SudoPassword] {
            self.secrets.delete(&secret_key(id, kind))?;
        }
        profiles::delete(&self.db, id)?;
        let _ = crate::metric_store::forget(&self.db, id);
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

    // ---------- ~/.ssh/config import ----------

    /// Describe the hosts in `~/.ssh/config` and which of them are already saved.
    /// Preview the hosts in `path` (`~/…` allowed), or in `~/.ssh/config`.
    pub fn ssh_config_preview(&self, path: Option<&str>) -> Result<ssh_config::Preview> {
        let ctx = ssh_config::Context::from_env();
        self.ssh_config_preview_at(&ctx.config_path(path), &ctx)
    }

    pub fn ssh_config_preview_at(&self, path: &Path, ctx: &ssh_config::Context) -> Result<ssh_config::Preview> {
        let mut preview = ssh_config::preview(path, ctx);
        let saved = profiles::list(&self.db)?;
        for c in &mut preview.candidates {
            if let Some(p) = saved
                .iter()
                .find(|p| p.protocol.is_ssh() && p.host.eq_ignore_ascii_case(&c.host) && p.port == c.port && p.username == c.username)
            {
                c.existing_id = Some(p.id.clone());
                c.existing_name = Some(p.name.clone());
            }
        }
        Ok(preview)
    }

    /// Create profiles for the chosen candidates (by key), jump hosts first.
    /// The config is read again here: the UI only says which keys it wants.
    /// A chosen host's jump hosts are created too unless already saved.
    pub fn import_ssh_config(&self, path: Option<&str>, keys: &[String]) -> Result<ImportReport> {
        let ctx = ssh_config::Context::from_env();
        self.import_ssh_config_at(&ctx.config_path(path), &ctx, keys)
    }

    pub fn import_ssh_config_at(&self, path: &Path, ctx: &ssh_config::Context, keys: &[String]) -> Result<ImportReport> {
        let preview = self.ssh_config_preview_at(path, ctx)?;
        let by_key: HashMap<&str, &ssh_config::Candidate> = preview.candidates.iter().map(|c| (c.key.as_str(), c)).collect();
        let mut done: HashMap<String, std::result::Result<String, String>> = HashMap::new();
        let mut report = ImportReport::default();
        for key in keys {
            let Some(c) = by_key.get(key.as_str()) else {
                report.failed.push(ImportFailure {
                    name: key.clone(),
                    error: "no longer in the SSH config".into(),
                });
                continue;
            };
            if c.existing_id.is_some() {
                continue;
            }
            let _ = self.import_one(c, &by_key, &preview.path, &mut done, &mut report);
        }
        Ok(report)
    }

    fn import_one(
        &self,
        c: &ssh_config::Candidate,
        by_key: &HashMap<&str, &ssh_config::Candidate>,
        file: &str,
        done: &mut HashMap<String, std::result::Result<String, String>>,
        report: &mut ImportReport,
    ) -> std::result::Result<String, String> {
        if let Some(r) = done.get(&c.key) {
            return r.clone();
        }
        let r = (|| {
            if let Some(id) = &c.existing_id {
                return Ok(id.clone());
            }
            if !c.importable {
                let why = c
                    .notes
                    .iter()
                    .find(|n| n.level == ssh_config::NoteLevel::Warn)
                    .map(|n| n.text.clone());
                return Err(why.unwrap_or_else(|| "can't be imported".into()));
            }
            let jump = match &c.jump_key {
                None => None,
                Some(k) => {
                    let j = by_key.get(k.as_str()).ok_or("its jump host is missing")?;
                    Some(
                        self.import_one(j, by_key, file, done, report)
                            .map_err(|e| format!("jump host {}: {e}", j.name))?,
                    )
                }
            };
            let p = profiles::create(
                &self.db,
                ProfileInput {
                    name: c.name.clone(),
                    host: c.host.clone(),
                    port: Some(c.port),
                    username: c.username.clone(),
                    auth_method: c.auth_method,
                    key_path: c.key_path.clone(),
                    sudo_mode: None,
                    group_name: None,
                    color: None,
                    initial_path: None,
                    jump_profile_id: jump.clone(),
                    protocol: Protocol::Sftp,
                    region: None,
                    path_style: false,
                },
            )
            .map_err(|e| e.to_string())?;
            self.record(
                None,
                NewEvent {
                    category: Some(Category::Security),
                    action: "profile.import".into(),
                    profile_id: Some(p.id.clone()),
                    target: Some(format!("{}@{}:{}", p.username, p.host, p.port)),
                    details: Some(serde_json::json!({
                        "from": c.source.clone().unwrap_or_else(|| file.to_string()),
                        "alias": c.name,
                        "jumpProfileId": jump,
                    })),
                    ..Default::default()
                },
            );
            let id = p.id.clone();
            report.created.push(p);
            Ok(id)
        })();
        if let Err(e) = &r {
            report.failed.push(ImportFailure {
                name: c.name.clone(),
                error: e.clone(),
            });
        }
        done.insert(c.key.clone(), r.clone());
        r
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
        self.secrets.exists(&secret_key(profile_id, kind))
    }

    // ---------- Touch ID gate ----------

    pub fn presence_status(&self) -> PresenceStatus {
        PresenceStatus {
            available: self.presence.available(),
            enabled: self.secrets.gate_enabled(),
        }
    }

    async fn verify_presence(&self, reason: String) -> Result<()> {
        let presence = self.presence.clone();
        // The system dialog blocks until the user answers; keep it off the async workers.
        tokio::task::spawn_blocking(move || presence.verify(&reason))
            .await
            .map_err(|e| Error::PresenceRequired(format!("verification did not complete: {e}")))?
    }

    /// Turn the gate on or off. Both directions require the user to
    /// authenticate first: on, to prove it works before it can lock them out;
    /// off, because that is exactly what the gate is there to prevent.
    pub async fn set_presence_gate(&self, enabled: bool) -> Result<PresenceStatus> {
        let action = if enabled { "presence_gate.enable" } else { "presence_gate.disable" };
        let reason = if enabled {
            "require authentication before saved server credentials are used"
        } else {
            "stop requiring authentication for saved server credentials"
        };
        let r = match self.verify_presence(reason.into()).await {
            Ok(()) => self.secrets.set_gate(enabled),
            Err(e) => Err(e),
        };
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: action.into(),
                outcome: Some(match &r {
                    Ok(()) => Outcome::Success,
                    Err(Error::PresenceRequired(_)) => Outcome::Denied,
                    Err(_) => Outcome::Failure,
                }),
                details: r
                    .as_ref()
                    .err()
                    .map(|e| serde_json::json!({ "error": e.to_string(), "kind": e.kind() })),
                ..Default::default()
            },
        );
        r.map(|()| self.presence_status())
    }

    /// If connecting along `route` would read saved credentials while the gate
    /// is on, verify the user and return the unlock that makes them readable.
    /// The store itself refuses locked reads, so a mistake here fails closed.
    async fn unlock_for(&self, route: &[Profile], opts: &ConnectOptions) -> Result<Option<Unlock>> {
        if !self.secrets.gate_enabled() {
            return Ok(None);
        }
        let last = route.len().saturating_sub(1);
        let mut needed = false;
        for (i, p) in route.iter().enumerate() {
            let typed = opts.typed_for(p, i == last).is_some();
            for kind in saved_secrets_wanted(p, typed, i == last) {
                needed |= self.secrets.exists(&secret_key(&p.id, kind))?;
            }
        }
        if !needed {
            return Ok(None);
        }
        let target = &route[last];
        self.verify_presence(format!("connect to {}@{} with saved credentials", target.username, target.host))
            .await?;
        Ok(Some(self.secrets.unlock()))
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
        if !profile.protocol.is_ssh() {
            return self.connect_storage(profile, opts).await;
        }
        let target = format!("{}@{}:{}", profile.username, profile.host, profile.port);
        let accepted = opts.accept_fingerprint.clone();
        // The endpoint the fingerprint was confirmed for: a jump host, or the target.
        let (pin_host, pin_port) = opts.accept_host.clone().unwrap_or((profile.host.clone(), profile.port));
        let t = Instant::now();
        let mut verified_user = false;
        let res = async {
            let route = profiles::chain(&self.db, profile_id)?;
            // Held until authentication is over, then saved credentials lock again.
            let unlock = self.unlock_for(&route, &opts).await?;
            verified_user = unlock.is_some();
            let secrets: &dyn SecretStore = self.secrets.as_ref();
            SshSession::connect_route(self.db.clone(), secrets, route, opts).await
        }
        .await;
        // A host key may get pinned even if authentication then fails — audit it either way.
        if let Some(fp) = &accepted {
            let pinned = known_hosts::list(&self.db)?
                .into_iter()
                .any(|k| k.host == pin_host && k.port == pin_port && &k.fingerprint == fp);
            if pinned {
                self.record(
                    None,
                    NewEvent {
                        category: Some(Category::Security),
                        action: "hostkey.trust".into(),
                        profile_id: Some(profile.id.clone()),
                        server: Some(target.clone()),
                        target: Some(format!("{pin_host}:{pin_port}")),
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
                    Error::HostKeyMismatch { .. } | Error::PresenceRequired(_) => (Category::Security, Outcome::Denied),
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
            via: session.via(),
            connected_at: session.connected_at.to_rfc3339(),
            protocol: Protocol::Sftp,
        };
        self.record(
            Some(&session),
            NewEvent {
                category: Some(Category::Connection),
                action: "session.open".into(),
                target: Some(target),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details: (verified_user || !info.via.is_empty())
                    .then(|| serde_json::json!({ "via": info.via, "userVerified": verified_user })),
                ..Default::default()
            },
        );
        self.sessions.write().await.insert(session.id.clone(), Arc::new(session));
        Ok(info)
    }

    pub async fn disconnect(&self, session_id: &str) -> Result<()> {
        if let Some(st) = self.storage.write().await.remove(session_id) {
            st.close().await;
            self.record(
                None,
                NewEvent {
                    category: Some(Category::Connection),
                    action: "session.close".into(),
                    profile_id: Some(st.profile.id.clone()),
                    server: Some(st.label()),
                    target: Some(st.label()),
                    ..Default::default()
                },
            );
            return Ok(());
        }
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
        // Tunnels and log streams ride on this connection: they end with it.
        self.tunnels.lock().await.retain(|_, t| {
            let keep = t.info.session_id != session_id;
            if !keep {
                t.close();
            }
            keep
        });
        let streams: Vec<String> = self
            .log_streams
            .lock()
            .await
            .iter()
            .filter(|(_, (sid, _))| sid == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for id in streams {
            let _ = self.stop_logs(&id).await;
        }
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
        if let Some(st) = self.storage_session(sid).await {
            return st.list_dir(path).await;
        }
        self.session(sid).await?.list_dir(path).await
    }

    pub async fn read_file(&self, sid: &str, path: &str) -> Result<FileContent> {
        if let Some(st) = self.storage_session(sid).await {
            return st.read_text(path).await;
        }
        self.session(sid).await?.read_text(path).await
    }

    pub fn validator_for(&self, path: &str) -> Option<Validator> {
        detect_validator(path)
    }

    /// Plain save (SFTP, in place) with conflict detection.
    pub async fn write_file(&self, sid: &str, path: &str, content: &str, expected_mtime: Option<u32>) -> Result<Option<u32>> {
        if let Some(st) = self.storage_session(sid).await {
            // These protocols can't tell reliably whether the file changed meanwhile.
            return self
                .storage_audited(&st, "file.write", path, st.write(path, content.as_bytes().to_vec()))
                .await
                .map(|_| None);
        }
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

    /// Read one backup of a config file (to compare it with the current one).
    pub async fn read_backup(&self, sid: &str, path: &str, backup: &str, privileged: bool) -> Result<String> {
        self.session(sid).await?.read_backup(path, backup, privileged).await
    }

    /// Put a backup back, through safe save: validated, and the current
    /// version is itself kept as a backup, so a restore can be undone.
    pub async fn restore_backup(&self, sid: &str, path: &str, backup: &str, privileged: bool) -> Result<SafeWriteReport> {
        let content = self.read_backup(sid, path, backup, privileged).await?;
        let r = self.safe_write(sid, path, &content, true, privileged, None).await;
        let s = self.session(sid).await?;
        self.record(
            Some(&s),
            NewEvent {
                category: Some(Category::Config),
                action: "config.restore".into(),
                target: Some(path.into()),
                outcome: Some(if r.is_ok() { Outcome::Success } else { Outcome::Failure }),
                details: Some(serde_json::json!({ "from": backup, "error": r.as_ref().err().map(|e| e.to_string()) })),
                ..Default::default()
            },
        );
        r
    }

    pub async fn list_backups(&self, sid: &str, path: &str, privileged: bool) -> Result<Vec<String>> {
        self.session(sid).await?.list_backups(path, privileged).await
    }

    pub async fn mkdir(&self, sid: &str, path: &str) -> Result<()> {
        if let Some(st) = self.storage_session(sid).await {
            return self.storage_audited(&st, "file.mkdir", path, st.mkdir(path)).await;
        }
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.mkdir", Some(path), None, s.mkdir(path))
            .await
    }

    pub async fn create_file(&self, sid: &str, path: &str) -> Result<()> {
        if let Some(st) = self.storage_session(sid).await {
            return self.storage_audited(&st, "file.create", path, st.write(path, Vec::new())).await;
        }
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.create", Some(path), None, s.create_file(path))
            .await
    }

    pub async fn rename(&self, sid: &str, from: &str, to: &str) -> Result<()> {
        if let Some(st) = self.storage_session(sid).await {
            return self
                .storage_audited(&st, "file.rename", &format!("{from} → {to}"), st.rename(from, to))
                .await;
        }
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

    /// Paste in Files: copy a file or folder to another place on the same server.
    pub async fn copy_within(&self, sid: &str, from: &str, to: &str) -> Result<()> {
        if self.storage_session(sid).await.is_some() {
            return Err(Error::InvalidInput(
                "copying within this storage isn't supported; download and upload instead".into(),
            ));
        }
        let s = self.session(sid).await?;
        progress::step(format!("Copying {from} to {to} …"));
        self.audited(
            &s,
            Category::File,
            "file.copy",
            Some(&format!("{from} → {to}")),
            None,
            s.copy_within(from, to),
        )
        .await
    }

    pub async fn remove(&self, sid: &str, path: &str, recursive: bool) -> Result<()> {
        if let Some(st) = self.storage_session(sid).await {
            let action = if recursive { "file.delete_recursive" } else { "file.delete" };
            return self.storage_audited(&st, action, path, st.remove(path, recursive)).await;
        }
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

    /// Upload a file or a whole folder. Returns the bytes sent.
    pub async fn upload(&self, sid: &str, local: &std::path::Path, remote: &str) -> Result<u64> {
        if let Some(st) = self.storage_session(sid).await {
            return self.storage_audited(&st, "file.upload", remote, st.upload(local, remote)).await;
        }
        let s = self.session(sid).await?;
        let up = async {
            match s.upload_tree(local, remote).await {
                // A folder only root can write: through /tmp, then sudo.
                Err(e) if crate::sftp::denied(&e) && s.elevation_needed() => s.upload_as_root(local, remote).await,
                r => r,
            }
        };
        self.audited(&s, Category::File, "file.upload", Some(remote), None, up)
            .await
            .map(|r| r.bytes)
    }

    /// Copy a file or folder from one connected server to another. Recorded
    /// in the destination server's log, since that is the server that changes.
    pub async fn copy_between(
        &self,
        src_sid: &str,
        src_path: &str,
        dst_sid: &str,
        dst_path: &str,
        overwrite: bool,
    ) -> Result<TransferReport> {
        let src = self.session(src_sid).await?;
        let dst = self.session(dst_sid).await?;
        let t = Instant::now();
        let r = if src_sid == dst_sid {
            Err(Error::InvalidInput("choose a different server to copy to".into()))
        } else {
            progress::step(format!("Copying {src_path} from {} to {} …", src.label(), dst.label()));
            match src.copy_to(src_path, &dst, dst_path, overwrite).await {
                // The destination folder needs root there: stage it in /tmp, then sudo.
                Err(e) if crate::sftp::denied(&e) && dst.elevation_needed() => {
                    src.copy_to_as_root(src_path, &dst, dst_path, overwrite).await
                }
                r => r,
            }
        };
        let from = format!("{}:{}", src.label(), src_path);
        let (outcome, details) = match &r {
            Ok(rep) => (
                Outcome::Success,
                serde_json::json!({ "from": from, "files": rep.files, "folders": rep.dirs, "bytes": rep.bytes, "skipped": rep.skipped }),
            ),
            Err(e) => (
                if matches!(e, Error::PermissionDenied(_)) {
                    Outcome::Denied
                } else {
                    Outcome::Failure
                },
                serde_json::json!({ "from": from, "error": e.to_string(), "kind": e.kind() }),
            ),
        };
        self.record(
            Some(&dst),
            NewEvent {
                category: Some(Category::File),
                action: "file.copy_from_server".into(),
                target: Some(dst_path.into()),
                outcome: Some(outcome),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details: Some(details),
                ..Default::default()
            },
        );
        r
    }

    pub async fn download(&self, sid: &str, remote: &str, local: &std::path::Path) -> Result<u64> {
        if let Some(st) = self.storage_session(sid).await {
            return self.storage_audited(&st, "file.download", remote, st.download(remote, local)).await;
        }
        let s = self.session(sid).await?;
        self.audited(&s, Category::File, "file.download", Some(remote), None, s.download(remote, local))
            .await
    }

    /// Files and folders whose names contain `query`, below `dir` (read-only).
    pub async fn find_files(&self, sid: &str, dir: &str, query: &str) -> Result<crate::filetools::FindReport> {
        self.session(sid).await?.find_files(dir, query).await
    }

    /// Download a folder as one .tar.gz, streamed into `local` (a job: progress, stoppable).
    pub async fn download_folder(&self, sid: &str, remote: &str, local: &std::path::Path) -> Result<u64> {
        let s = self.session(sid).await?;
        self.audited(
            &s,
            Category::File,
            "file.download_folder",
            Some(remote),
            None,
            s.download_folder(remote, local),
        )
        .await
    }

    // ---------- installed software ----------

    /// Which well-known server software is on this host (read-only).
    pub async fn detect_tools(&self, sid: &str) -> Result<Vec<DetectedTool>> {
        self.session(sid).await?.detect_tools().await
    }

    /// Test and read the live nginx configuration (read-only).
    pub async fn nginx_config(&self, sid: &str) -> Result<NginxConfig> {
        self.session(sid).await?.nginx_config().await
    }

    /// What "New website" would write and where (read-only).
    pub async fn website_plan(&self, sid: &str, input: &WebsiteInput) -> Result<WebsitePlan> {
        self.session(sid).await?.website_plan(input).await
    }

    /// Create an nginx site for one domain: the folder (static sites), the
    /// server block through safe save with `nginx -t` and rollback, the
    /// sites-enabled link where that's the layout, then a reload.
    /// Test a website: nginx -t, a request on the server itself, and one from this computer. Read-only.
    pub async fn website_test(&self, sid: &str, domain: &str, https: bool) -> Result<crate::website::SiteTest> {
        use crate::website::{self as w, SiteProbe};
        let s = self.session(sid).await?;
        let d = w::validate_domain(domain)?;
        let (cfg, _) = s.exec_inspect("nginx -t 2>&1").await?;
        let local = s.exec(&w::local_test_command(&d, https)?).await?;
        let server = w::parse_local_test(&d, https, &local.stdout);
        let url = format!("{}://{d}/", if https { "https" } else { "http" });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("Orbital/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        let internet = match client.get(&url).send().await {
            Ok(r) => SiteProbe {
                url,
                status: Some(r.status().as_u16()),
                error: None,
            },
            Err(e) => {
                let why = if e.is_timeout() {
                    "no answer within 10 s: a firewall (the server's or the cloud provider's) may block the port".to_string()
                } else if e.is_connect() {
                    format!("couldn't connect: the name may not point here yet, or the port is closed ({e})")
                } else {
                    e.to_string()
                };
                SiteProbe {
                    url,
                    status: None,
                    error: Some(why),
                }
            }
        };
        Ok(crate::website::SiteTest {
            config_ok: cfg.ok(),
            config_output: cfg.combined(),
            server,
            internet,
        })
    }

    /// Remove a website: its nginx file and enabled links (kept in /var/backups/orbital-nginx;
    /// put back if nginx -t fails), then optionally its certificate and its folder under /var/www.
    pub async fn remove_website(&self, sid: &str, file: &str, certificate: Option<&str>, root: Option<&str>) -> Result<String> {
        use crate::website as w;
        let s = self.session(sid).await?;
        const ACTION: &str = "website.remove";
        let script = w::remove_script(file).map_err(|e| self.refused_as(&s, Category::Config, ACTION, file, e))?;
        if let Some(r) = root {
            if !w::removable_root(r) {
                return Err(self.refused_as(
                    &s,
                    Category::Config,
                    ACTION,
                    file,
                    Error::InvalidInput(format!("{r} isn't a site folder under /var/www, so it isn't deleted")),
                ));
            }
        }
        progress::step(format!("Removing {file} and testing nginx …"));
        let sent = s.privileged_command(&script);
        let o = match s.exec_privileged(&script).await {
            Ok(o) => o,
            Err(e) => {
                self.record_error(&s, Category::Config, ACTION, file, Some(sent), &e);
                return Err(e);
            }
        };
        self.record_exec(&s, Category::Config, ACTION, file, &sent, &o);
        if !o.ok() {
            return Err(Error::ValidationFailed {
                validator: "nginx -t".into(),
                output: format!("nginx -t failed without this site, so it was put back:\n{}", o.combined().trim()),
            });
        }
        let backup = o.stdout.lines().find_map(|l| l.strip_prefix("@@backup ")).unwrap_or("").to_string();
        progress::step("Reloading nginx …");
        let (_reloaded, _) = s.reload_nginx().await?;
        if let Some(name) = certificate {
            progress::step(format!("Deleting the certificate {name} …"));
            self.certbot_delete(sid, name, false).await?;
        }
        if let Some(r) = root {
            progress::step(format!("Deleting {r} …"));
            let cmd = format!("rm -rf -- {}", crate::shell::quote(r.trim_end_matches('/')));
            let o = s.exec_privileged(&cmd).await?;
            self.record_exec(&s, Category::File, "file.delete_recursive", r, &s.privileged_command(&cmd), &o);
            o.check()?;
        }
        Ok(backup)
    }

    /// The real file behind a site path (sites-enabled links resolved), checked for route edits.
    async fn route_file(&self, s: &SshSession, file: &str) -> Result<String> {
        crate::shell::validate_path(file)?;
        let (o, _) = s.exec_inspect(&format!("readlink -f -- {}", crate::shell::quote(file))).await?;
        let real = o.stdout.trim().to_string();
        let real = if real.is_empty() { file.to_string() } else { real };
        crate::routes::check_site_file(file, &real)?;
        Ok(real)
    }

    /// The routes (`location` blocks) of the server block on `line` of `file`.
    pub async fn nginx_routes(&self, sid: &str, file: &str, line: usize) -> Result<crate::routes::RoutesView> {
        let s = self.session(sid).await?;
        let real = self.route_file(&s, file).await?;
        let content = s.read_text(&real).await?;
        Ok(crate::routes::RoutesView {
            routes: crate::routes::list(&content.content, line)?,
            file: real,
            line,
        })
    }

    /// Preview (`apply` false) or make a route change in a site's own file:
    /// safe save with `nginx -t` (rolled back when it fails), then reload.
    pub async fn nginx_route_change(
        &self,
        sid: &str,
        file: &str,
        line: usize,
        change: &crate::routes::RouteChange,
        apply: bool,
    ) -> Result<crate::routes::RouteChangeReport> {
        use crate::routes::RouteChange as C;
        let s = self.session(sid).await?;
        let real = self.route_file(&s, file).await?;
        let content = s.read_text(&real).await?;
        let after = crate::routes::apply(&content.content, line, change)?;
        let mut report = crate::routes::RouteChangeReport {
            file: real.clone(),
            before: content.content.clone(),
            after: after.clone(),
            applied: false,
            backup_path: None,
            reloaded: false,
            reload_output: String::new(),
        };
        if !apply {
            return Ok(report);
        }
        let what = match change {
            C::Add { route } => format!("add {}", route.path),
            C::Replace { spec, route } => format!("change {spec} → {}", route.path),
            C::Remove { spec } => format!("remove {spec}"),
        };
        progress::step(format!("Writing {real} and testing it with nginx -t …"));
        let target = format!("{real}: {what}");
        let rep = self
            .audited(
                &s,
                Category::Config,
                "nginx.route",
                Some(&target),
                None,
                s.safe_write(&real, &after, Some(crate::website::nginx_test()), true, content.mtime),
            )
            .await?;
        progress::step("Reloading nginx …");
        let (reloaded, out) = s.reload_nginx().await?;
        report.applied = true;
        report.backup_path = rep.backup_path;
        report.reloaded = reloaded;
        report.reload_output = out;
        Ok(report)
    }

    /// Turn nginx's stock default site off by removing its sites-enabled link;
    /// the file itself is never changed. Put back (and refused) if nginx still
    /// fails without it. Returns the file the link pointed at.
    pub async fn disable_default_site(&self, sid: &str) -> Result<String> {
        self.disable_site(sid, crate::website::DEFAULT_LINK).await
    }

    /// Enabled nginx files that break the configuration (a deleted
    /// certificate, a dangling link). Read-only.
    pub async fn nginx_broken_sites(&self, sid: &str) -> Result<Vec<crate::website::BrokenSite>> {
        self.session(sid).await?.nginx_broken_sites().await
    }

    /// Turn a site off by removing its link in sites-enabled (its file is kept);
    /// put back, and refused, when nginx still fails without it.
    pub async fn disable_site(&self, sid: &str, link: &str) -> Result<String> {
        let s = self.session(sid).await?;
        const ACTION: &str = "nginx.site_off";
        let script = crate::website::site_off_script(link).map_err(|e| self.refused_as(&s, Category::Config, ACTION, link, e))?;
        progress::step(format!(
            "Turning {} off (its link only) and testing nginx …",
            link.rsplit('/').next().unwrap_or(link)
        ));
        let sent = s.privileged_command(&script);
        let o = match s.exec_privileged(&script).await {
            Ok(o) => o,
            Err(e) => {
                self.record_error(&s, Category::Config, ACTION, link, Some(sent), &e);
                return Err(e);
            }
        };
        self.record_exec(&s, Category::Config, ACTION, link, &sent, &o);
        if !o.ok() {
            return Err(Error::ValidationFailed {
                validator: "nginx -t".into(),
                output: o.combined().trim().to_string(),
            });
        }
        let target = o
            .stdout
            .lines()
            .find_map(|l| l.strip_prefix("@@site_off "))
            .unwrap_or("")
            .to_string();
        if !target.is_empty() {
            progress::step("Reloading nginx …");
            s.reload_nginx().await?;
        }
        Ok(target)
    }

    pub async fn create_website(&self, sid: &str, input: &WebsiteInput) -> Result<WebsiteReport> {
        let s = self.session(sid).await?;
        const ACTION: &str = "website.create";
        let target = input.domain.trim().to_ascii_lowercase();
        let t = Instant::now();
        let r = async {
            progress::step(format!("Checking nginx's sites for {target} …"));
            let plan = s.website_plan(input).await?;
            if !plan.blockers.is_empty() {
                return Err(Error::InvalidInput(plan.blockers.join("\n")));
            }
            let mut root_created = false;
            if let (Some(root), false) = (&plan.root, plan.root_exists) {
                progress::step(format!("Creating {root} …"));
                s.create_site_root(root, &plan.domain).await?;
                root_created = true;
            }
            progress::step(format!("Writing {} and testing it with nginx -t …", plan.file));
            let write = s.install_site(&plan.file, plan.enabled_link.as_deref(), &plan.content).await?;
            progress::step("Reloading nginx …");
            let (reloaded, reload_output) = progress::with_output(s.reload_nginx()).await?;
            Ok(WebsiteReport {
                plan,
                write,
                root_created,
                reloaded,
                reload_output,
            })
        }
        .await;
        let kind = match &input.site {
            SiteKind::Static { .. } => "static",
            SiteKind::Proxy { .. } => "proxy",
        };
        match &r {
            Ok(rep) => self.record(
                Some(&s),
                NewEvent {
                    category: Some(Category::Config),
                    action: ACTION.into(),
                    target: Some(rep.plan.domain.clone()),
                    outcome: Some(Outcome::Success),
                    duration_ms: Some(t.elapsed().as_millis() as i64),
                    details: Some(serde_json::json!({
                        "kind": kind,
                        "file": rep.plan.file,
                        "link": rep.plan.enabled_link,
                        "root": rep.plan.root,
                        "rootCreated": rep.root_created,
                        "sha256": sha256(rep.plan.content.as_bytes()),
                        "reloaded": rep.reloaded,
                    })),
                    ..Default::default()
                },
            ),
            Err(e) => self.record_error(&s, Category::Config, ACTION, &target, None, e),
        }
        r
    }

    /// Which ports are open and which program listens on each (read-only).
    pub async fn listening_ports(&self, sid: &str) -> Result<PortsReport> {
        self.session(sid).await?.listening_ports().await
    }

    /// The SSH server's effective settings, checked against hardening advice (read-only).
    pub async fn sshd_report(&self, sid: &str) -> Result<SshdReport> {
        self.session(sid).await?.sshd_report().await
    }

    // ---------- security tools: ufw, fail2ban, certbot ----------

    /// Run a root command for a security tool and record it, success or failure.
    async fn security_exec(&self, s: &SshSession, action: &str, target: &str, cmd: &str) -> Result<ExecOutput> {
        let sent = s.privileged_command(cmd);
        // In a job, the tool's own output (certbot, ufw, …) is what the user sees happen.
        match progress::with_output(s.exec_privileged(cmd)).await {
            Ok(o) => {
                self.record_exec(s, Category::Security, action, target, &sent, &o);
                Ok(o)
            }
            Err(e) => {
                self.record_error(s, Category::Security, action, target, Some(sent), &e);
                Err(e)
            }
        }
    }

    /// Record a change that was refused before anything was sent, and pass the error on.
    fn refused(&self, s: &SshSession, action: &str, target: &str, e: Error) -> Error {
        self.refused_as(s, Category::Security, action, target, e)
    }

    fn refused_as(&self, s: &SshSession, category: Category, action: &str, target: &str, e: Error) -> Error {
        self.record_error(s, category, action, target, None, &e);
        e
    }

    /// Fail unless the SSH connection in use would still get through `rules`.
    async fn ssh_lockout_check(&self, s: &SshSession, rules: &[UfwRule], default_incoming: Option<&str>) -> Result<()> {
        let (client, server_port) = s.ssh_connection().await;
        let port = server_port.unwrap_or(s.profile.port);
        if firewall::ssh_reachable(rules, default_incoming, port, client) {
            return Ok(());
        }
        let from = client.map(|c| format!(" from {c}")).unwrap_or_default();
        Err(Error::PreflightFailed {
            check: "SSH access".into(),
            output: format!(
                "After this change the firewall would block SSH on port {port}{from}, which would cut off this connection and lock you out. Allow port {port} first."
            ),
        })
    }

    /// Check that each picked rule is still at its number with the same text.
    fn verify_refs(st: &UfwStatus, refs: &[UfwRuleRef]) -> Result<()> {
        if !st.active {
            return Err(Error::InvalidInput(
                "rules can be changed or deleted by number only while the firewall is on".into(),
            ));
        }
        if refs.is_empty() {
            return Err(Error::InvalidInput("no rule chosen".into()));
        }
        for r in refs {
            if !st.rules.iter().any(|x| x.number == r.number && x.raw == r.raw) {
                return Err(Error::InvalidInput(
                    "the firewall rules changed since they were loaded; refresh and try again".into(),
                ));
            }
        }
        Ok(())
    }

    pub async fn ufw_status(&self, sid: &str) -> Result<UfwStatus> {
        self.session(sid).await?.ufw_status().await
    }

    /// Add a rule at the end. Refused if it would block the SSH connection in
    /// use, unless `force`.
    pub async fn ufw_add(&self, sid: &str, rule: UfwRuleInput, force: bool) -> Result<UfwStatus> {
        let s = self.session(sid).await?;
        const ACTION: &str = "firewall.rule_add";
        let args = firewall::rule_args(&rule).map_err(|e| self.refused(&s, ACTION, "ufw", e))?;
        let st = s.ufw_status().await?;
        if st.active && !force {
            let mut rules = st.effective_rules();
            rules.push(firewall::preview_rule(&rule)?);
            if let Err(e) = self.ssh_lockout_check(&s, &rules, st.default_incoming.as_deref()).await {
                return Err(self.refused(&s, ACTION, &args, e));
            }
        }
        self.security_exec(&s, ACTION, &args, &format!("ufw {args}")).await?.check()?;
        s.ufw_status().await
    }

    /// Replace rules with a new one at the position of the first. The new rule
    /// goes in before the old ones are removed, so a failure leaves the old rules.
    pub async fn ufw_update(&self, sid: &str, old: Vec<UfwRuleRef>, rule: UfwRuleInput, force: bool) -> Result<UfwStatus> {
        let s = self.session(sid).await?;
        const ACTION: &str = "firewall.rule_update";
        let args = firewall::rule_args(&rule).map_err(|e| self.refused(&s, ACTION, "ufw", e))?;
        let st = s.ufw_status().await?;
        Self::verify_refs(&st, &old).map_err(|e| self.refused(&s, ACTION, &args, e))?;
        let pos = old.iter().map(|r| r.number).min().unwrap_or(1);
        if !force {
            let preview = firewall::preview_rule(&rule)?;
            let mut rules = Vec::new();
            for r in &st.rules {
                if r.number == pos {
                    rules.push(preview.clone());
                }
                if !old.iter().any(|o| o.number == r.number) {
                    rules.push(r.clone());
                }
            }
            if let Err(e) = self.ssh_lockout_check(&s, &rules, st.default_incoming.as_deref()).await {
                return Err(self.refused(&s, ACTION, &args, e));
            }
        }
        let mut o = self.security_exec(&s, ACTION, &args, &format!("ufw insert {pos} {args}")).await?;
        if !o.ok() && format!("{}{}", o.stdout, o.stderr).contains("position") {
            // An IPv4 rule can't go into the IPv6 part of the list: append instead.
            o = self.security_exec(&s, ACTION, &args, &format!("ufw {args}")).await?;
        }
        let o = o.check()?;
        if o.stdout.contains("Skipping") {
            return Err(Error::InvalidInput(
                "an identical rule already exists, so nothing was changed".into(),
            ));
        }
        let after = s.ufw_status().await?;
        let mut numbers: Vec<u32> = old
            .iter()
            .filter_map(|r| after.rules.iter().find(|x| x.raw == r.raw).map(|x| x.number))
            .collect();
        numbers.sort_unstable_by(|a, b| b.cmp(a));
        numbers.dedup();
        for n in numbers {
            self.security_exec(&s, ACTION, &args, &format!("ufw --force delete {n}"))
                .await?
                .check()?;
        }
        s.ufw_status().await
    }

    /// Delete rules by number, after checking each number still means the rule
    /// the user saw. Refused if SSH would be blocked afterwards, unless `force`.
    pub async fn ufw_delete(&self, sid: &str, refs: Vec<UfwRuleRef>, force: bool) -> Result<UfwStatus> {
        let s = self.session(sid).await?;
        const ACTION: &str = "firewall.rule_delete";
        let target = refs.iter().map(|r| r.raw.as_str()).collect::<Vec<_>>().join("; ");
        let st = s.ufw_status().await?;
        Self::verify_refs(&st, &refs).map_err(|e| self.refused(&s, ACTION, &target, e))?;
        if !force {
            let rest: Vec<UfwRule> = st
                .rules
                .iter()
                .filter(|r| !refs.iter().any(|x| x.number == r.number))
                .cloned()
                .collect();
            if let Err(e) = self.ssh_lockout_check(&s, &rest, st.default_incoming.as_deref()).await {
                return Err(self.refused(&s, ACTION, &target, e));
            }
        }
        let mut numbers: Vec<u32> = refs.iter().map(|r| r.number).collect();
        numbers.sort_unstable_by(|a, b| b.cmp(a));
        numbers.dedup();
        for n in numbers {
            let raw = &st.rules.iter().find(|r| r.number == n).expect("verified").raw;
            self.security_exec(&s, ACTION, raw, &format!("ufw --force delete {n}"))
                .await?
                .check()?;
        }
        s.ufw_status().await
    }

    /// Turn the firewall on or off. Turning it on is refused if its rules
    /// would block the SSH connection in use, unless `force`.
    pub async fn ufw_set_enabled(&self, sid: &str, enabled: bool, force: bool) -> Result<UfwStatus> {
        let s = self.session(sid).await?;
        let action = if enabled { "firewall.enable" } else { "firewall.disable" };
        if enabled && !force {
            let st = s.ufw_status().await?;
            if let Err(e) = self
                .ssh_lockout_check(&s, &st.effective_rules(), st.default_incoming.as_deref())
                .await
            {
                return Err(self.refused(&s, action, "ufw", e));
            }
        }
        let cmd = if enabled { "ufw --force enable" } else { "ufw disable" };
        self.security_exec(&s, action, "ufw", cmd).await?.check()?;
        s.ufw_status().await
    }

    pub async fn fail2ban_status(&self, sid: &str) -> Result<Fail2banStatus> {
        self.session(sid).await?.fail2ban_status().await
    }

    /// Unban an address from one jail, or from every jail when `jail` is `None`.
    pub async fn fail2ban_unban(&self, sid: &str, jail: Option<&str>, ip: &str) -> Result<Fail2banStatus> {
        let s = self.session(sid).await?;
        const ACTION: &str = "fail2ban.unban";
        let target = format!("{ip} ({})", jail.unwrap_or("all jails"));
        let cmd = fail2ban::unban_command(jail, ip).map_err(|e| self.refused(&s, ACTION, &target, e))?;
        self.security_exec(&s, ACTION, &target, &cmd).await?.check()?;
        s.fail2ban_status().await
    }

    /// Ban an address in a jail. The address this connection comes from is refused.
    pub async fn fail2ban_ban(&self, sid: &str, jail: &str, ip: &str) -> Result<Fail2banStatus> {
        let s = self.session(sid).await?;
        const ACTION: &str = "fail2ban.ban";
        let target = format!("{ip} ({jail})");
        let cmd = fail2ban::ban_command(jail, ip).map_err(|e| self.refused(&s, ACTION, &target, e))?;
        if let (Some(client), Ok(asked)) = (s.ssh_connection().await.0, fail2ban::validate_ip(ip)) {
            if client == asked {
                let e = Error::PreflightFailed {
                    check: "SSH access".into(),
                    output: format!("{ip} is the address this connection comes from; banning it would lock you out."),
                };
                return Err(self.refused(&s, ACTION, &target, e));
            }
        }
        self.security_exec(&s, ACTION, &target, &cmd).await?.check()?;
        s.fail2ban_status().await
    }

    pub async fn certbot_status(&self, sid: &str) -> Result<CertbotStatus> {
        self.session(sid).await?.certbot_status().await
    }

    /// Request a new certificate. Returns certbot's output whether it worked or
    /// not, so the user can see why Let's Encrypt refused.
    pub async fn certbot_issue(&self, sid: &str, req: IssueRequest) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        const ACTION: &str = "certificate.issue";
        let target = req.domains.join(", ");
        certbot::issue_command(&req).map_err(|e| self.refused(&s, ACTION, &target, e))?;
        // certbot's nginx installer writes into whichever server block it picks,
        // and into the default site when no block serves the name. Let it change
        // only the name's own site file; otherwise get the certificate alone.
        let mut not_installed = None;
        if req.method == certbot::CertMethod::Nginx {
            progress::step("Checking which nginx site serves these names …");
            let cfg = s.nginx_config().await?;
            match certbot::nginx_install_targets(&cfg, &req.domains) {
                Ok(files) => progress::step(format!("HTTPS goes into {} …", files.join(", "))),
                Err(why) => not_installed = Some(why),
            }
        }
        let cmd = certbot::issue_command_with(&req, not_installed.is_none()).map_err(|e| self.refused(&s, ACTION, &target, e))?;
        progress::step(format!("Asking Let's Encrypt for a certificate for {target} …"));
        let mut out = self.security_exec(&s, ACTION, &target, &cmd).await?;
        if let (Some(why), true) = (not_installed, out.ok()) {
            out.stdout.push_str(&format!(
                "\nOrbital got the certificate only and changed no nginx file: {why}. It's in /etc/letsencrypt/live/{}/; use Get certificate again once the site exists to turn HTTPS on.\n",
                req.domains.first().map(|d| d.trim().to_ascii_lowercase()).unwrap_or_default()
            ));
        }
        Ok(out)
    }

    pub async fn certbot_renew(&self, sid: &str, name: Option<&str>, mode: RenewMode) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let action = if mode == RenewMode::DryRun {
            "certificate.renew_test"
        } else {
            "certificate.renew"
        };
        let target = name.unwrap_or("all certificates");
        let cmd = certbot::renew_command(name, mode).map_err(|e| self.refused(&s, action, target, e))?;
        progress::step(if mode == RenewMode::DryRun {
            format!("Testing renewal of {target} against Let's Encrypt's staging servers …")
        } else {
            format!("Renewing {target} with Let's Encrypt …")
        });
        self.security_exec(&s, action, target, &cmd).await
    }

    /// Delete a certificate. Refused while a web-server config still points at
    /// it (that server would fail to start), unless `force`.
    pub async fn certbot_delete(&self, sid: &str, name: &str, force: bool) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        const ACTION: &str = "certificate.delete";
        let check = certbot::usage_check_command(name).map_err(|e| self.refused(&s, ACTION, name, e))?;
        if !force {
            progress::step("Checking that no web-server config still uses it …");
            let (o, _) = s.exec_inspect(&check).await?;
            let users: Vec<&str> = o.stdout.lines().filter(|l| l.starts_with('/')).collect();
            if !users.is_empty() {
                let e = Error::PreflightFailed {
                    check: "web server config".into(),
                    output: format!(
                        "These files still use this certificate, so the web server would fail to start without it:\n{}",
                        users.join("\n")
                    ),
                };
                return Err(self.refused(&s, ACTION, name, e));
            }
        }
        let cmd = certbot::delete_command(name)?;
        self.security_exec(&s, ACTION, name, &cmd).await
    }

    // ---------- port forwarding ----------

    /// Forward a port on this Mac (127.0.0.1) to `remote_host:remote_port` as the server sees it.
    pub async fn open_tunnel(
        &self,
        sid: &str,
        remote_host: &str,
        remote_port: u16,
        local_port: Option<u16>,
        label: Option<String>,
    ) -> Result<TunnelInfo> {
        let s = self.session(sid).await?;
        let r = async {
            let host = tunnels::validate_target(remote_host)?;
            if remote_port == 0 {
                return Err(Error::InvalidInput("choose the port on the server".into()));
            }
            // Check the route first, so a tunnel that can't work says why now.
            tunnels::probe(&s, &host, remote_port).await?;
            let listener = tunnels::bind_local(local_port, remote_port).await?;
            let mut t = tunnels::start(s.clone(), listener, host, remote_port, label)?;
            t.info.session_id = sid.to_string();
            Ok(t)
        }
        .await;
        let target = format!(
            "localhost:{} → {remote_host}:{remote_port}",
            r.as_ref().map(|t| t.info.local_port).unwrap_or(local_port.unwrap_or(0))
        );
        match r {
            Ok(t) => {
                self.record(
                    Some(&s),
                    NewEvent {
                        category: Some(Category::Connection),
                        action: "tunnel.open".into(),
                        target: Some(target),
                        ..Default::default()
                    },
                );
                let info = t.info();
                self.tunnels.lock().await.insert(info.id.clone(), t);
                Ok(info)
            }
            Err(e) => {
                self.record_error(&s, Category::Connection, "tunnel.open", &target, None, &e);
                Err(e)
            }
        }
    }

    pub async fn close_tunnel(&self, id: &str) -> Result<()> {
        if let Some(t) = self.tunnels.lock().await.remove(id) {
            t.close();
            let i = t.info();
            if let Ok(s) = self.session(&i.session_id).await {
                self.record(
                    Some(&s),
                    NewEvent {
                        category: Some(Category::Connection),
                        action: "tunnel.close".into(),
                        target: Some(format!("localhost:{} → {}:{}", i.local_port, i.remote_host, i.remote_port)),
                        details: Some(serde_json::json!({ "connections": i.total_connections, "bytes": i.bytes })),
                        ..Default::default()
                    },
                );
            }
        }
        Ok(())
    }

    pub async fn list_tunnels(&self) -> Vec<TunnelInfo> {
        let mut v: Vec<TunnelInfo> = self.tunnels.lock().await.values().map(|t| t.info()).collect();
        v.sort_by_key(|t| t.local_port);
        v
    }

    // ---------- live logs ----------

    pub async fn log_files(&self, sid: &str) -> Result<Vec<LogFile>> {
        self.session(sid).await?.log_files().await
    }

    /// Follow a log; output goes to `sink` until `stop_logs`. Read-only, so not audited.
    pub async fn follow_logs<F>(&self, sid: &str, source: LogSource, lines: u32, sink: F) -> Result<String>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        let s = self.session(sid).await?;
        let cmd = crate::logs::follow_command(&source, lines)?;
        // Most logs and the journal need root, so use sudo when it works; otherwise read as the login user.
        let root = s.sudo_works().await;
        let stream = s.open_stream(&cmd, root, sink).await?;
        let id = stream.id.clone();
        self.log_streams.lock().await.insert(id.clone(), (sid.to_string(), stream));
        Ok(id)
    }

    pub async fn stop_logs(&self, id: &str) -> Result<()> {
        if let Some((_, st)) = self.log_streams.lock().await.remove(id) {
            st.stop().await;
        }
        Ok(())
    }

    // ---------- deploys ----------

    pub fn list_apps(&self, profile_id: &str) -> Result<Vec<App>> {
        deploy::list_apps(&self.db, profile_id)
    }

    pub fn save_app(&self, profile_id: &str, id: Option<&str>, input: AppInput) -> Result<App> {
        let app = deploy::save_app(&self.db, profile_id, id, &input)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Config),
                action: if id.is_some() { "deploy.app_update" } else { "deploy.app_create" }.into(),
                profile_id: Some(profile_id.into()),
                target: Some(format!("{} ({})", app.name, app.path)),
                ..Default::default()
            },
        );
        Ok(app)
    }

    pub fn delete_app(&self, id: &str) -> Result<()> {
        let app = deploy::get_app(&self.db, id)?;
        deploy::delete_app(&self.db, id)?;
        self.record(
            None,
            NewEvent {
                category: Some(Category::Config),
                action: "deploy.app_delete".into(),
                profile_id: Some(app.profile_id),
                target: Some(app.name),
                ..Default::default()
            },
        );
        Ok(())
    }

    pub fn deploy_history(&self, app_id: &str) -> Result<Vec<DeployRecord>> {
        deploy::history(&self.db, app_id, 50)
    }

    pub fn deploy_log(&self, run_id: &str) -> Result<String> {
        deploy::deploy_log(&self.db, run_id)
    }

    /// The app's checkout on the server: current commit, branch, local changes, recent commits.
    pub async fn app_checkout(&self, sid: &str, app_id: &str) -> Result<Checkout> {
        let s = self.session(sid).await?;
        let app = deploy::get_app(&self.db, app_id)?;
        let o = s.deploy_read(&deploy::inspect_script(&app.path)?).await?;
        match o.exit_code {
            3 => Err(Error::NotFound(format!("{} doesn't exist on this server", app.path))),
            4 => Err(Error::InvalidInput(format!("{} is not a git checkout", app.path))),
            _ => Ok(deploy::parse_checkout(&o.stdout)),
        }
    }

    /// Deploy the newest commit of the app's branch, or roll back to `rollback_to`:
    /// update the code, run the build, restart. Stops at the first failing step.
    pub async fn run_deploy<F>(&self, sid: &str, app_id: &str, rollback_to: Option<String>, sink: F) -> Result<DeployRecord>
    where
        F: Fn(DeployEvent) + Send + Sync + 'static,
    {
        let s = self.session(sid).await?;
        let app = deploy::get_app(&self.db, app_id)?;
        if app.profile_id != s.profile.id {
            return Err(Error::InvalidInput("that app belongs to another server".into()));
        }
        if !self.deploying.lock().await.insert(app.id.clone()) {
            return Err(Error::InvalidInput(format!("{} is already being deployed", app.name)));
        }
        let r = self.deploy_inner(&s, &app, rollback_to, Arc::new(sink)).await;
        self.deploying.lock().await.remove(&app.id);
        r
    }

    async fn deploy_inner<F>(&self, s: &Arc<SshSession>, app: &App, rollback_to: Option<String>, sink: Arc<F>) -> Result<DeployRecord>
    where
        F: Fn(DeployEvent) + Send + Sync + 'static,
    {
        let kind = if rollback_to.is_some() { "rollback" } else { "deploy" };
        let action = format!("deploy.{}", if rollback_to.is_some() { "rollback" } else { "run" });
        if let Some(c) = &rollback_to {
            deploy::validate_commit(c)?;
        }
        // Pre-flight: the checkout exists and has no uncommitted changes to tracked files.
        let checkout = self
            .app_checkout(&s.id, &app.id)
            .await
            .map_err(|e| self.refused_as(s, Category::Config, &action, &app.name, e))?;
        if !checkout.dirty.is_empty() {
            let e = Error::PreflightFailed {
                check: "uncommitted changes".into(),
                output: format!(
                    "These files were changed on the server and aren't committed, so the deploy stopped instead of overwriting them:\n{}",
                    checkout.dirty.join("\n")
                ),
            };
            return Err(self.refused_as(s, Category::Config, &action, &app.name, e));
        }
        let from = checkout.head.clone();
        let run_id = deploy::record_start(&self.db, &app.id, kind, from.as_deref())?;
        let log = Arc::new(std::sync::Mutex::new(String::new()));
        let started = Instant::now();

        let mut ok = true;
        let mut failed_step = String::new();
        let mut steps: Vec<(String, String, bool)> = vec![(
            if rollback_to.is_some() {
                "Check out the earlier version".into()
            } else {
                format!("Get the newest {}", app.branch)
            },
            deploy::update_command(&app.path, &app.branch, rollback_to.as_deref())?,
            false,
        )];
        for (name, cmd) in deploy::build_steps(&app.path, &app.build)? {
            steps.push((name, cmd, false));
        }
        match app.restart_kind {
            RestartKind::Compose => steps.push(("Restart: docker compose up -d".into(), deploy::compose_command(&app.path)?, true)),
            RestartKind::Command => {
                let c = app.restart_target.clone().unwrap_or_default();
                steps.push((format!("Restart: {c}"), deploy::custom_restart_command(&app.path, &c)?, false))
            }
            _ => {}
        }
        for (name, cmd, privileged) in steps {
            if !self.deploy_step(s, &name, &cmd, privileged, &sink, &log).await {
                ok = false;
                failed_step = name;
                break;
            }
        }
        if ok && app.restart_kind == RestartKind::Service {
            let unit = app.restart_target.clone().unwrap_or_default();
            sink(DeployEvent::Step {
                name: format!("Restart {unit}"),
            });
            let res = self.service_action(&s.id, &unit, crate::services::ServiceAction::Restart).await;
            let (good, text) = match &res {
                Ok(rep) => (rep.output.ok(), format!("{}{}", rep.output.stdout, rep.output.stderr)),
                Err(e) => (false, e.to_string()),
            };
            if !text.trim().is_empty() {
                log.lock().unwrap().push_str(&text);
                sink(DeployEvent::Output { text: text.clone() });
            }
            sink(DeployEvent::StepEnd {
                ok: good,
                code: res.ok().map(|r| r.output.exit_code),
            });
            if !good {
                ok = false;
                failed_step = format!("Restart {unit}");
            }
        }

        let to = s
            .deploy_read(&deploy::head_command(&app.path)?)
            .await
            .ok()
            .map(|o| o.stdout.trim().to_string())
            .filter(|h| h.len() >= 7);
        let summary = if !ok {
            format!("Stopped at: {failed_step}")
        } else if let Some(c) = &rollback_to {
            format!("Rolled back to {}", &c[..c.len().min(7)])
        } else {
            match (&from, &to) {
                (Some(f), Some(t)) if f == t => "Already up to date".to_string(),
                (Some(f), Some(t)) => {
                    let n = s
                        .deploy_read(&deploy::changes_command(&app.path, f, t)?)
                        .await
                        .map(|o| o.stdout.lines().count())
                        .unwrap_or(0);
                    format!("{n} new commit{}", if n == 1 { "" } else { "s" })
                }
                _ => "Deployed".to_string(),
            }
        };
        let text = { log.lock().unwrap().clone() };
        let tail: String = text.chars().rev().take(64_000).collect::<Vec<_>>().into_iter().rev().collect();
        deploy::record_finish(&self.db, &run_id, ok, to.as_deref(), &summary, &tail)?;
        self.record(
            Some(s),
            NewEvent {
                category: Some(Category::Config),
                action,
                target: Some(format!("{} ({})", app.name, app.path)),
                outcome: Some(if ok { Outcome::Success } else { Outcome::Failure }),
                duration_ms: Some(started.elapsed().as_millis() as i64),
                details: Some(serde_json::json!({ "from": from, "to": to, "summary": summary, "branch": app.branch })),
                ..Default::default()
            },
        );
        let record = deploy::history(&self.db, &app.id, 1)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("deploy record missing".into()))?;
        sink(DeployEvent::Done {
            ok,
            record: record.clone(),
        });
        Ok(record)
    }

    async fn deploy_step<F>(
        &self,
        s: &SshSession,
        name: &str,
        cmd: &str,
        privileged: bool,
        sink: &Arc<F>,
        log: &Arc<std::sync::Mutex<String>>,
    ) -> bool
    where
        F: Fn(DeployEvent) + Send + Sync + 'static,
    {
        sink(DeployEvent::Step { name: name.to_string() });
        log.lock().unwrap().push_str(&format!("\n$ {name}\n"));
        let mark = log.lock().unwrap().len();
        let mut code = self.deploy_run(s, cmd, privileged, sink, log).await;
        // Refused as the login user (a folder owned by root, git's ownership
        // check): the same step once more with sudo, saying so.
        let said = log.lock().unwrap()[mark..].to_string();
        if code != Some(0) && !privileged && s.elevation_needed() && deploy::refused(&said) {
            let text = format!("\nNot allowed as {}; running this step again with sudo …\n", s.profile.username);
            log.lock().unwrap().push_str(&text);
            sink(DeployEvent::Output { text });
            code = self.deploy_run(s, &deploy::as_root(cmd), true, sink, log).await;
        }
        let ok = code == Some(0);
        sink(DeployEvent::StepEnd { ok, code });
        ok
    }

    async fn deploy_run<F>(
        &self,
        s: &SshSession,
        cmd: &str,
        privileged: bool,
        sink: &Arc<F>,
        log: &Arc<std::sync::Mutex<String>>,
    ) -> Option<i32>
    where
        F: Fn(DeployEvent) + Send + Sync + 'static,
    {
        let (out_sink, out_log) = (sink.clone(), log.clone());
        let stream = s
            .open_stream(cmd, privileged, move |ev| {
                if let StreamEvent::Data(d) = ev {
                    let text = String::from_utf8_lossy(&d).into_owned();
                    out_log.lock().unwrap().push_str(&text);
                    out_sink(DeployEvent::Output { text });
                }
            })
            .await;
        match stream {
            Ok(st) => st.wait().await,
            Err(e) => {
                let text = format!("{e}\n");
                log.lock().unwrap().push_str(&text);
                sink(DeployEvent::Output { text });
                None
            }
        }
    }

    // ---------- scheduled jobs ----------

    pub async fn cron_report(&self, sid: &str) -> Result<CronReport> {
        self.session(sid).await?.cron_report().await
    }

    /// Add, change, pause, resume or delete a job in `user`'s crontab. The new
    /// crontab goes to `crontab -u USER -` over stdin, which refuses invalid
    /// syntax and then installs nothing. A change names the line it replaces;
    /// if that line is gone, nothing is written.
    pub async fn cron_change(&self, sid: &str, user: &str, change: CronChange) -> Result<CronReport> {
        let s = self.session(sid).await?;
        let action = format!("cron.{}", change.verb());
        let read = cron::read_command(user).map_err(|e| self.refused_as(&s, Category::Config, &action, user, e))?;
        let write = cron::write_command(user)?;
        // Your own crontab needs no sudo; anyone else's does.
        let own = user == s.profile.username;
        let current = if own {
            s.exec("crontab -l 2>/dev/null || true").await?
        } else {
            s.exec_privileged(&read).await?
        };
        let next = cron::apply(&current.stdout, &change).map_err(|e| self.refused_as(&s, Category::Config, &action, user, e))?;
        let (sent, out) = if own {
            (
                "crontab -".to_string(),
                s.exec_with_input("crontab -", Some(next.as_bytes()), std::time::Duration::from_secs(60))
                    .await,
            )
        } else {
            (
                s.privileged_command(&write),
                s.exec_privileged_with(&write, Some(next.as_bytes()), std::time::Duration::from_secs(60))
                    .await,
            )
        };
        let target = format!("crontab of {user}");
        match out {
            Ok(o) => {
                self.record_exec(&s, Category::Config, &action, &target, &sent, &o);
                o.check()?;
            }
            Err(e) => {
                self.record_error(&s, Category::Config, &action, &target, Some(sent), &e);
                return Err(e);
            }
        }
        s.cron_report().await
    }

    /// Run a listed job's command now, as its user, and return what it printed.
    /// Only commands that are actually in a crontab can be run this way.
    pub async fn cron_run(&self, sid: &str, user: &str, command: &str) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let action = "cron.run";
        let report = s.cron_report().await?;
        if !report.jobs.iter().any(|j| j.user == user && j.command == command) {
            let e = Error::NotFound("that job is no longer scheduled; refresh and try again".into());
            return Err(self.refused_as(&s, Category::Command, action, user, e));
        }
        let script = cron::run_now_command(user, command).map_err(|e| self.refused_as(&s, Category::Command, action, user, e))?;
        let own = user == s.profile.username;
        let sent = if own { script.clone() } else { s.privileged_command(&script) };
        let r = if own {
            s.exec_with_input(&script, None, std::time::Duration::from_secs(600)).await
        } else {
            s.exec_privileged_with(&script, None, std::time::Duration::from_secs(600)).await
        };
        match &r {
            Ok(o) => self.record_exec(&s, Category::Command, action, command, &sent, o),
            Err(e) => self.record_error(&s, Category::Command, action, command, Some(sent.clone()), e),
        }
        r
    }

    // ---------- user accounts ----------

    pub async fn list_users(&self, sid: &str) -> Result<UsersReport> {
        self.session(sid).await?.list_users().await
    }

    /// Run a root script for a user account, with optional stdin (passwords).
    /// The script never contains a secret, so it is what gets recorded.
    async fn user_exec(&self, s: &SshSession, action: &str, target: &str, script: &str, input: Option<&[u8]>) -> Result<()> {
        let sent = s.privileged_command(script);
        match s.exec_privileged_with(script, input, std::time::Duration::from_secs(120)).await {
            Ok(o) => {
                self.record_exec(s, Category::Security, action, target, &sent, &o);
                o.check().map(|_| ())
            }
            Err(e) => {
                self.record_error(s, Category::Security, action, target, Some(sent), &e);
                Err(e)
            }
        }
    }

    /// Create an account with every option in `input`. A password, when given,
    /// goes to chpasswd over stdin; without one the password is locked and the
    /// user signs in with SSH keys.
    pub async fn create_user(&self, sid: &str, input: NewUser, password: Option<String>) -> Result<UsersReport> {
        let s = self.session(sid).await?;
        let password = password.filter(|p| !p.is_empty()).map(zeroize::Zeroizing::new);
        let target = input.name.clone();
        let prepared = users::create_script(&input, password.is_some()).and_then(|script| {
            let stdin = match &password {
                Some(p) => Some(zeroize::Zeroizing::new(users::password_input(&input.name, p)?)),
                None => None,
            };
            Ok((script, stdin))
        });
        let (script, stdin) = prepared.map_err(|e| self.refused(&s, "user.create", &target, e))?;
        self.user_exec(&s, "user.create", &target, &script, stdin.as_ref().map(|v| v.as_slice()))
            .await?;
        s.list_users().await
    }

    /// Change, lock or delete an account. Changes that would lock the current
    /// connection's account out (or root) are refused; dropping your own sudo
    /// group or login shell needs `force`, deleting or locking yourself is never allowed.
    pub async fn change_user(
        &self,
        sid: &str,
        name: &str,
        change: UserChange,
        password: Option<String>,
        force: bool,
    ) -> Result<UsersReport> {
        let s = self.session(sid).await?;
        let action = format!("user.{}", change.verb());
        let report = s.list_users().await?;
        if !report.users.iter().any(|u| u.name == name) {
            return Err(self.refused(&s, &action, name, Error::NotFound(format!("no user {name:?} on this server"))));
        }
        if let Some(reason) = users::lockout_reason(name, &s.profile.username, &change, report.admin_group.as_deref()) {
            let hard = matches!(change, UserChange::Delete { .. } | UserChange::Lock);
            if hard || !force {
                let e = Error::PreflightFailed {
                    check: "your access".into(),
                    output: reason,
                };
                return Err(self.refused(&s, &action, name, e));
            }
        }
        let script = users::change_script(name, &change).map_err(|e| self.refused(&s, &action, name, e))?;
        let stdin = match (&change, password) {
            (UserChange::Password { .. }, Some(p)) => Some(zeroize::Zeroizing::new(
                users::password_input(name, &p).map_err(|e| self.refused(&s, &action, name, e))?,
            )),
            (UserChange::Password { .. }, None) => {
                return Err(self.refused(&s, &action, name, Error::InvalidInput("enter the new password".into())))
            }
            _ => None,
        };
        self.user_exec(&s, &action, name, &script, stdin.as_ref().map(|v| v.as_slice()))
            .await?;
        s.list_users().await
    }

    // ---------- Apache, HAProxy ----------

    pub async fn apache_status(&self, sid: &str, container: Option<&str>) -> Result<ApacheStatus> {
        self.session(sid).await?.apache_status(container).await
    }

    /// Enable or disable a Debian-style Apache site. Undone automatically if
    /// `configtest` fails afterwards; reload Apache to apply it.
    pub async fn apache_site(&self, sid: &str, name: &str, enable: bool) -> Result<ApacheStatus> {
        let s = self.session(sid).await?;
        let action = if enable { "apache.site_enable" } else { "apache.site_disable" };
        let script = webservers::apache_site_script(name, enable).map_err(|e| self.refused_as(&s, Category::Config, action, name, e))?;
        let sent = s.privileged_command(&script);
        let o = match s.exec_privileged(&script).await {
            Ok(o) => o,
            Err(e) => {
                self.record_error(&s, Category::Config, action, name, Some(sent), &e);
                return Err(e);
            }
        };
        if o.stdout.contains("@@rolledback") {
            let e = Error::PreflightFailed {
                check: "apachectl configtest".into(),
                output: o.stdout.replace("@@rolledback", "").trim().to_string(),
            };
            self.record_error(&s, Category::Config, action, name, Some(sent), &e);
            return Err(e);
        }
        self.record_exec(&s, Category::Config, action, name, &sent, &o);
        o.check()?;
        s.apache_status(None).await
    }

    pub async fn haproxy_status(&self, sid: &str) -> Result<HaproxyStatus> {
        self.session(sid).await?.haproxy_status().await
    }

    /// Put one HAProxy server back in rotation, drain it, or take it out for
    /// maintenance, over the stats socket. Takes effect at once, until changed
    /// back or HAProxy restarts.
    pub async fn haproxy_server_state(&self, sid: &str, backend: &str, server: &str, state: HaServerState) -> Result<HaproxyStatus> {
        let s = self.session(sid).await?;
        let action = format!("haproxy.server_{}", format!("{state:?}").to_lowercase());
        let target = format!("{backend}/{server}");
        let st = s.haproxy_status().await?;
        let known = st
            .sections
            .iter()
            .any(|sec| sec.name == backend && sec.servers.iter().any(|x| x.name == server));
        let socket = match (&st.socket, known) {
            (Some(sock), true) => sock.clone(),
            (None, _) => {
                let e = Error::InvalidInput(
                    "HAProxy has no stats socket Orbital can use; add `stats socket /run/haproxy/admin.sock mode 660 level admin` to the global section".into(),
                );
                return Err(self.refused_as(&s, Category::Service, &action, &target, e));
            }
            (_, false) => {
                let e = Error::NotFound(format!("{target} is not in the HAProxy configuration"));
                return Err(self.refused_as(&s, Category::Service, &action, &target, e));
            }
        };
        let script = webservers::haproxy_state_script(&socket, backend, server, state)
            .map_err(|e| self.refused_as(&s, Category::Service, &action, &target, e))?;
        let sent = s.privileged_command(&script);
        let o = match s.exec_privileged(&script).await {
            Ok(o) => o,
            Err(e) => {
                self.record_error(&s, Category::Service, &action, &target, Some(sent), &e);
                return Err(e);
            }
        };
        // The socket answers with an empty line on success, or a message on error.
        let answer = o.stdout.trim().to_string();
        let o = crate::ssh::ExecOutput {
            exit_code: if o.ok() && answer.is_empty() { 0 } else { o.exit_code.max(1) },
            stderr: if answer.is_empty() { o.stderr } else { answer },
            ..o
        };
        self.record_exec(&s, Category::Service, &action, &target, &sent, &o);
        o.check()?;
        s.haproxy_status().await
    }

    // ---------- databases ----------

    pub async fn list_databases(&self, sid: &str, kind: DbKind, container: Option<&str>) -> Result<DbServer> {
        self.session(sid).await?.list_databases(kind, container).await
    }

    /// Dump one database on the server into a private temp file, download it
    /// to `local` on this Mac, and remove the server copy whatever happens.
    /// Returns the bytes downloaded.
    pub async fn backup_database(&self, sid: &str, kind: DbKind, name: &str, format: BackupFormat, local: &std::path::Path) -> Result<u64> {
        let s = self.session(sid).await?;
        let action = "database.backup";
        let target = format!("{} {name}", kind.label());
        let script = databases::backup_script(kind, name, format, &s.profile.username)
            .map_err(|e| self.refused_as(&s, Category::Command, action, &target, e))?;
        let t = Instant::now();
        let r = async {
            progress::step(format!("Dumping {target} on the server (large databases take a while) …"));
            let out = s
                .exec_privileged_with(&script, None, std::time::Duration::from_secs(3600))
                .await?
                .check()?;
            let remote = databases::parse_backup_path(&out.stdout)
                .ok_or_else(|| Error::Other("the dump finished but its file wasn't reported".into()))?;
            progress::step("Downloading the dump to this computer …");
            let got = s.download(&remote, local).await;
            // The dump is sensitive: never leave it on the server, even if the download failed.
            let _ = s.exec_privileged(&format!("rm -f -- {}", crate::shell::quote(&remote))).await;
            got
        }
        .await;
        let file = local.file_name().map(|f| f.to_string_lossy().into_owned());
        self.record(
            Some(&s),
            NewEvent {
                category: Some(Category::Command),
                action: action.into(),
                target: Some(target),
                outcome: Some(match &r {
                    Ok(_) => Outcome::Success,
                    Err(Error::PermissionDenied(_)) => Outcome::Denied,
                    Err(_) => Outcome::Failure,
                }),
                duration_ms: Some(t.elapsed().as_millis() as i64),
                details: Some(match &r {
                    Ok(bytes) => serde_json::json!({ "bytes": bytes, "savedAs": file, "format": format }),
                    Err(e) => serde_json::json!({ "error": e.to_string(), "kind": e.kind() }),
                }),
                ..Default::default()
            },
        );
        r
    }

    pub async fn redis_info(&self, sid: &str, container: Option<&str>) -> Result<RedisInfo> {
        self.session(sid).await?.redis_info(container).await
    }

    /// Snapshot or flush Redis. Flushing deletes data; the UI asks the user to type a confirmation.
    /// On the server, or inside `container`.
    pub async fn redis_action(&self, sid: &str, action: RedisAction, container: Option<&str>) -> Result<RedisInfo> {
        let s = self.session(sid).await?;
        let (name, target) = match action {
            RedisAction::Snapshot => ("redis.snapshot", "all".to_string()),
            RedisAction::FlushDb(n) => ("redis.flushdb", format!("db{n}")),
            RedisAction::FlushAll => ("redis.flushall", "all databases".to_string()),
        };
        let script = databases::redis_action_script(action).map_err(|e| self.refused_as(&s, Category::Command, name, &target, e))?;
        let o = s.inspect_in(container, &script).await?;
        let ok = o.ok() && !o.stdout.contains("ERR") && !o.stdout.contains("NOAUTH");
        let o = crate::ssh::ExecOutput {
            exit_code: if ok { 0 } else { o.exit_code.max(1) },
            stderr: if ok { o.stderr } else { o.stdout.clone() },
            ..o
        };
        // The script reads redis.conf for a password; record what was asked, not the script.
        let target = match container {
            Some(c) => format!("{target} in container {c}"),
            None => target,
        };
        self.record_exec(&s, Category::Command, name, &target, &format!("redis-cli {name}"), &o);
        o.check()?;
        s.redis_info(container).await
    }

    // ---------- monitoring ----------

    /// Folder sizes one level below `path`, largest first (read-only).
    pub async fn disk_usage(&self, sid: &str, path: &str) -> Result<DiskUsage> {
        self.session(sid).await?.disk_usage(path).await
    }

    /// CPU (measured over one second) and memory by process, program, service and user (read-only).
    pub async fn process_usage(&self, sid: &str) -> Result<ProcessReport> {
        self.session(sid).await?.process_usage().await
    }

    /// Details that change rarely: identity, network, pending updates (read-only).
    pub async fn system_info(&self, sid: &str) -> Result<SystemInfo> {
        self.session(sid).await?.system_info().await
    }

    pub async fn metrics(&self, sid: &str) -> Result<Metrics> {
        let s = self.session(sid).await?;
        let snap = s.snapshot().await?;
        let mut last = self.last_metrics.lock().await;
        let m = metrics::derive(last.get(sid), snap, chrono::Utc::now().timestamp_millis());
        last.insert(sid.to_string(), m.clone());
        drop(last);
        // Monitoring history (a point a minute, kept 30 days); a failure here never fails the read.
        let saved = crate::metric_store::record(&self.db, &s.profile.id, &crate::metric_store::point(&m))
            .and_then(|_| crate::metric_store::record_moment(&self.db, &s.profile.id, &crate::metric_store::moment(&m)));
        if let Err(e) = saved {
            tracing::warn!("couldn't record monitoring history: {e}");
        }
        Ok(m)
    }

    /// A server's monitoring history between two times (Unix seconds), in at most `max` points.
    /// Returns the bucket size in seconds too.
    pub fn metric_history(&self, profile_id: &str, from: i64, to: i64, max: u32) -> Result<(i64, Vec<crate::metric_store::Point>)> {
        crate::metric_store::range(&self.db, profile_id, from, to, max.clamp(10, 2000))
    }

    /// The recorded minute nearest `t` (busiest programs, every disk), within `within` seconds.
    pub fn metric_moment(&self, profile_id: &str, t: i64, within: i64) -> Result<Option<crate::metric_store::Moment>> {
        crate::metric_store::moment_at(&self.db, profile_id, t, within.clamp(60, 86_400))
    }

    /// Every disk's use over a range, with how fast it grows and when it fills.
    pub fn disk_history(&self, profile_id: &str, from: i64, to: i64, max: u32) -> Result<Vec<crate::metric_store::DiskSeries>> {
        crate::metric_store::disk_series(&self.db, profile_id, from, to, max.clamp(10, 2000))
    }

    /// What changed on the server in a range (from the audit log) and its restarts, to mark on charts.
    pub fn metric_events(&self, profile_id: &str, from: i64, to: i64) -> Result<Vec<crate::metric_store::Event>> {
        crate::metric_store::events(&self.db, profile_id, from, to)
    }

    /// History kept by older versions in the UI's local storage, brought in once.
    pub fn import_metric_history(&self, profile_id: &str, points: &[crate::metric_store::Point]) -> Result<usize> {
        crate::metric_store::import(&self.db, profile_id, points)
    }

    /// Each uptime check over a date range (the Uptime page's shared range).
    pub fn uptime_range(&self, from: i64, to: i64, max: u32) -> Result<Vec<uptime::RangeSummary>> {
        uptime::list_checks(&self.db)?
            .iter()
            .map(|c| uptime::range_summary(&self.db, &c.id, from, to, max.clamp(10, 2000)))
            .collect()
    }

    // ---------- services ----------

    /// Failed services, polled for the health dot and overview (read-only, not audited).
    pub async fn failed_units(&self, sid: &str) -> Result<Vec<String>> {
        self.session(sid).await?.failed_units().await
    }

    pub async fn list_services(&self, sid: &str) -> Result<Vec<Service>> {
        self.session(sid).await?.list_services().await
    }

    pub async fn service_detail(&self, sid: &str, unit: &str) -> Result<ServiceDetail> {
        self.session(sid).await?.service_detail(unit, 200).await
    }

    /// Ping, DNS, a TCP port, HTTP(S) or traceroute from the server (read-only).
    pub async fn net_check(
        &self,
        sid: &str,
        check: crate::netcheck::NetCheck,
        target: &str,
        port: Option<u16>,
    ) -> Result<crate::netcheck::NetResult> {
        self.session(sid).await?.net_check(check, target, port).await
    }

    pub async fn unit_logs(&self, sid: &str, unit: &str, tail: u32, since: Option<i64>, until: Option<i64>) -> Result<String> {
        self.session(sid).await?.unit_logs(unit, tail, since, until).await
    }

    pub async fn service_action(&self, sid: &str, unit: &str, action: ServiceAction) -> Result<ActionReport> {
        let s = self.session(sid).await?;
        let name = format!("service.{}", action.verb());
        progress::step(format!("Running systemctl {} {unit} …", action.verb()));
        match progress::with_output(s.service_action(unit, action)).await {
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

    pub async fn container_logs(&self, sid: &str, id: &str, tail: u32, since: Option<&str>, until: Option<&str>) -> Result<String> {
        self.session(sid).await?.container_logs(id, tail, since, until).await
    }

    pub async fn container_action(&self, sid: &str, id: &str, action: ContainerAction) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let name = format!("docker.{}", action.args().replace(' ', ""));
        progress::step(format!("docker {} …", action.args()));
        let (cmd, out) = match progress::with_output(s.container_action(id, action)).await {
            Ok(v) => v,
            Err(e) => {
                self.record_error(&s, Category::Docker, &name, id, None, &e);
                return Err(e);
            }
        };
        self.record_exec(&s, Category::Docker, &name, id, &cmd, &out);
        out.check()
    }

    /// What moving a container to `to` would do, and what on `to` is in the way. Read-only.
    pub async fn move_preview(
        &self,
        sid: &str,
        id: &str,
        to: &str,
        options: &crate::migrate::MoveOptions,
    ) -> Result<crate::migrate::MovePreview> {
        use crate::migrate;
        if sid == to {
            return Err(Error::InvalidInput("pick another server".into()));
        }
        let (src, dst) = (self.session(sid).await?, self.session(to).await?);
        let mut plan = migrate::plan(&migrate::inspect_for_move(&src, id).await?)?;
        // The original ports, before the dialog's changes, are what nginx here points at.
        let original_ports = plan.host_ports.clone();
        plan.apply(options)?;
        if !original_ports.is_empty() {
            if let Ok(cfg) = src.nginx_config().await {
                plan.hints
                    .extend(migrate::nginx_hints(&cfg, &original_ports, &src.label(), &dst.label()));
            }
        }
        let (o, _) = dst.exec_inspect(&migrate::preflight_script(&plan)).await?;
        Ok(migrate::MovePreview {
            clashes: migrate::parse_preflight(&o.stdout),
            plan,
            target: dst.label(),
        })
    }

    /// Recreate a container on another server with its image, volumes, bind-mounted
    /// files, network and options (see `migrate`). In `Move` mode the original is
    /// stopped first and stays stopped (started again if anything fails). Audited
    /// on both servers. Existing files at bind paths on the target need `force`.
    pub async fn move_container(
        &self,
        sid: &str,
        id: &str,
        to: &str,
        mode: crate::migrate::MoveMode,
        force: bool,
        options: &crate::migrate::MoveOptions,
    ) -> Result<crate::migrate::MovePlan> {
        use crate::migrate::{self, MoveMode};
        use crate::shell::quote;
        let t = Instant::now();
        let preview = self.move_preview(sid, id, to, options).await?;
        let (src, dst) = (self.session(sid).await?, self.session(to).await?);
        let p = preview.plan;
        let c = preview.clashes;
        let action = if mode == MoveMode::Move { "docker.move" } else { "docker.copy" };
        let target = format!("{} → {}", p.name, dst.label());
        let refuse = |e: Error| {
            self.record_error(&src, Category::Docker, action, &target, None, &e);
            e
        };
        if c.no_docker {
            return Err(refuse(Error::InvalidInput(format!("Docker isn't installed on {}", dst.label()))));
        }
        if c.name {
            return Err(refuse(Error::InvalidInput(format!(
                "{} already has a container named {}",
                dst.label(),
                p.name
            ))));
        }
        if !c.ports.is_empty() {
            let ports: Vec<String> = c.ports.iter().map(u16::to_string).collect();
            return Err(refuse(Error::InvalidInput(format!(
                "port {} is already in use on {}",
                ports.join(", "),
                dst.label()
            ))));
        }
        if !c.paths.is_empty() && !force {
            return Err(refuse(Error::PreflightFailed {
                check: "files already on the target".into(),
                output: format!(
                    "These already exist on {} and would be overwritten:\n{}",
                    dst.label(),
                    c.paths.join("\n")
                ),
            }));
        }
        let (src_docker, dst_docker) = (migrate::docker_root(&src).await?, migrate::docker_root(&dst).await?);
        for s in [&src, &dst] {
            if p.copies.iter().any(|c| c.volume.is_some()) && !s.elevation_needed() && s.profile.username != "root" {
                return Err(refuse(Error::PermissionDenied(format!(
                    "copying Docker volumes needs root on {}: turn on sudo for it",
                    s.label()
                ))));
            }
        }

        let mut stopped = false;
        if mode == MoveMode::Move && p.running {
            progress::step(format!("Stopping {} on {} …", p.name, src.label()));
            if let Err(e) = src
                .container_action(id, crate::docker::ContainerAction::Stop)
                .await
                .and_then(|(_, o)| o.check())
            {
                return Err(refuse(e));
            }
            stopped = true;
        }
        let r: Result<ExecOutput> = async {
            let dx = |cmd: String| {
                let dst = Arc::clone(&dst);
                async move {
                    if dst_docker {
                        dst.exec_privileged(&cmd).await
                    } else {
                        dst.exec(&cmd).await
                    }
                }
            };
            if let Some(n) = &p.network {
                progress::step(format!("Creating network {n} on {} …", dst.label()));
                dx(format!(
                    "docker network inspect -- {q} >/dev/null 2>&1 || docker network create -- {q}",
                    q = quote(n)
                ))
                .await?
                .check()?;
            }
            let have = dx(format!("docker image inspect -- {} >/dev/null 2>&1", quote(&p.image_id)))
                .await?
                .ok();
            if !have {
                let pulled = match &p.repo_digest {
                    Some(d) => {
                        progress::step(format!("Pulling {d} on {} …", dst.label()));
                        dx(format!("docker pull -- {} 2>&1", quote(d))).await?.ok()
                    }
                    None => false,
                };
                if !pulled {
                    progress::step(format!("Copying image {} …", p.image));
                    migrate::pipe(
                        &src,
                        &format!("docker save -- {}", quote(&p.image_id)),
                        src_docker,
                        &dst,
                        "docker load",
                        dst_docker,
                        &p.image,
                    )
                    .await?;
                }
            }
            if !p.image.starts_with("sha256:") {
                dx(format!("docker tag {} {}", quote(&p.image_id), quote(&p.image)))
                    .await?
                    .check()?;
            }
            for m in &p.copies {
                progress::step(format!(
                    "Copying {} …",
                    m.volume
                        .as_deref()
                        .map(|v| format!("volume {v}"))
                        .unwrap_or_else(|| m.source.clone())
                ));
                let root_src = src.elevation_needed();
                let root_dst = dst.elevation_needed();
                migrate::pipe(&src, &migrate::tar_out(m), root_src, &dst, &migrate::tar_in(m), root_dst, &m.dest).await?;
            }
            progress::step(format!("Starting {} on {} …", p.name, dst.label()));
            let cmd = migrate::run_command(&p);
            let env = zeroize::Zeroizing::new(migrate::env_file(&p).into_bytes());
            let o = if dst_docker {
                dst.exec_privileged_with(&cmd, Some(&env), std::time::Duration::from_secs(600))
                    .await?
            } else {
                dst.exec_with_input(&cmd, Some(&env), std::time::Duration::from_secs(600)).await?
            };
            Ok(o)
        }
        .await;
        // Docker refused the login user and sudo isn't allowed: say what to change.
        let r = r.map_err(|e| {
            let t = e.to_string().to_lowercase();
            if t.contains("permission denied") && t.contains("docker") {
                let who = if t.contains("on the source") { &src } else { &dst };
                Error::PermissionDenied(format!(
                    "{} can't use Docker as {}: add that user to the docker group (sudo usermod -aG docker {}, then reconnect) or allow sudo in its server settings",
                    who.label(),
                    who.profile.username,
                    who.profile.username
                ))
            } else {
                e
            }
        });
        let ok = matches!(&r, Ok(o) if o.ok());
        if !ok && stopped {
            progress::step(format!("Starting {} again on {} …", p.name, src.label()));
            let _ = src.container_action(id, crate::docker::ContainerAction::Start).await;
        }
        let details = serde_json::json!({
            "to": dst.label(), "image": p.image, "copies": p.copies.len(), "mode": action,
            "error": r.as_ref().err().map(|e| e.to_string()),
        });
        let outcome = if ok { Outcome::Success } else { Outcome::Failure };
        let duration = Some(t.elapsed().as_millis() as i64);
        self.record(
            Some(&src),
            NewEvent {
                category: Some(Category::Docker),
                action: format!("{action}_out"),
                target: Some(target.clone()),
                outcome: Some(outcome),
                duration_ms: duration,
                details: Some(details.clone()),
                ..Default::default()
            },
        );
        self.record(
            Some(&dst),
            NewEvent {
                category: Some(Category::Docker),
                action: format!("{action}_in"),
                target: Some(p.name.clone()),
                command: Some(migrate::run_command(&p)),
                outcome: Some(outcome),
                exit_code: r.as_ref().ok().map(|o| o.exit_code),
                duration_ms: duration,
                details: Some(details),
                ..Default::default()
            },
        );
        r?.check()?;
        Ok(p)
    }

    /// Apply, restart or pull a Compose project (or one of its services).
    /// Returns Compose's output whether it worked or not.
    pub async fn compose_action(&self, sid: &str, target: ComposeTarget, action: ComposeAction) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let name = format!("compose.{}", action.verb());
        let label = match &target.service {
            Some(svc) => format!("{}/{svc}", target.project),
            None => target.project.clone(),
        };
        progress::step(format!("docker compose {} for {label} …", action.verb()));
        match progress::with_output(s.compose(&target, action)).await {
            Ok((cmd, out)) => {
                self.record_exec(&s, Category::Docker, &name, &label, &cmd, &out);
                Ok(out)
            }
            Err(e) => {
                self.record_error(&s, Category::Docker, &name, &label, None, &e);
                Err(e)
            }
        }
    }

    // ---------- sign-ins and log search (read-only) ----------

    /// Who is signed in, recent sign-ins and failed SSH attempts (24 h).
    pub async fn logins(&self, sid: &str) -> Result<crate::logins::LoginsReport> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::logins::SCRIPT).await?;
        Ok(crate::logins::parse(&o.stdout))
    }

    /// A fixed-string, case-insensitive search through the journal or /var/log.
    pub async fn search_logs(
        &self,
        sid: &str,
        term: &str,
        hours: u32,
        source: crate::logsearch::LogSource,
    ) -> Result<crate::logsearch::LogSearch> {
        let s = self.session(sid).await?;
        let cmd = crate::logsearch::command(term, hours, source)?;
        let (o, _) = s.exec_inspect(&cmd).await?;
        Ok(crate::logsearch::parse(&o.stdout, source))
    }

    // ---------- Docker Compose stacks ----------

    /// Compose projects on the server and Orbital's undeployed stacks. Read-only.
    pub async fn stacks(&self, sid: &str) -> Result<Vec<crate::stacks::Stack>> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::stacks::LIST_SCRIPT).await?;
        Ok(crate::stacks::parse_list(&o.stdout))
    }

    /// A new stack in /opt/stacks/<name>/compose.yaml, checked by `docker
    /// compose config` before it's kept; deployed at once with `deploy`.
    pub async fn create_stack(&self, sid: &str, name: &str, content: &str, deploy: bool) -> Result<Option<ExecOutput>> {
        use crate::stacks as st;
        let s = self.session(sid).await?;
        st::validate_name(name).map_err(|e| self.refused_as(&s, Category::Docker, "stack.create", name, e))?;
        if self.stacks(sid).await?.iter().any(|x| x.name == name) {
            let e = Error::InvalidInput(format!("a stack named {name} already exists; open it instead"));
            return Err(self.refused_as(&s, Category::Docker, "stack.create", name, e));
        }
        progress::step(format!("Creating {} …", st::stack_file(name)));
        self.system_exec(
            &s,
            "stack.mkdir",
            name,
            &st::mkdir_command(name)?,
            std::time::Duration::from_secs(30),
        )
        .await?
        .check()?;
        let file = st::stack_file(name);
        let validator = crate::config_edit::Validator {
            name: "docker compose config".into(),
            command: st::CONFIG_CHECK.into(),
            stage: crate::config_edit::ValidateStage::PreInstall,
        };
        progress::step("Checking it with docker compose config …");
        if let Err(e) = self
            .audited(
                &s,
                Category::Docker,
                "stack.create",
                Some(name),
                None,
                s.safe_write(&file, content, Some(validator), true, None),
            )
            .await
        {
            // Nothing was kept: take the empty folder back too.
            let dir = crate::shell::quote(&format!("{}/{name}", st::STACK_DIR));
            let _ = s.exec_privileged(&format!("rmdir -- {dir} 2>/dev/null; true")).await;
            return Err(e);
        }
        if deploy {
            return Ok(Some(self.stack_action(sid, name, &file, crate::stacks::StackAction::Up).await?));
        }
        Ok(None)
    }

    /// Deploy, pull and redeploy, restart, stop or read the logs of a stack.
    /// Returns Compose's output whether it worked or not.
    pub async fn stack_action(&self, sid: &str, name: &str, file: &str, action: crate::stacks::StackAction) -> Result<ExecOutput> {
        use crate::stacks::StackAction as A;
        let s = self.session(sid).await?;
        let args = crate::stacks::args(name, file, action)?;
        let verb = match action {
            A::Up => "up",
            A::PullUp => "pull_up",
            A::Restart => "restart",
            A::Down => "down",
            A::Logs => "logs",
        };
        if action == A::Logs {
            return Ok(s.docker(&args).await?.1);
        }
        progress::step(format!("docker compose {} for {name} …", verb.replace('_', " + ")));
        let action_name = format!("stack.{verb}");
        // Pulling images can take minutes.
        let limit = std::time::Duration::from_secs(if matches!(action, A::Up | A::PullUp) { 1800 } else { 300 });
        match progress::with_output(s.docker_for(&args, limit)).await {
            Ok((cmd, out)) => {
                self.record_exec(&s, Category::Docker, &action_name, name, &cmd, &out);
                Ok(out)
            }
            Err(e) => {
                self.record_error(&s, Category::Docker, &action_name, name, None, &e);
                Err(e)
            }
        }
    }

    // ---------- Apps (ready-made stacks) ----------

    /// Install an app from [`crate::apps::APPS`] as stack `name` on `port`
    /// (127.0.0.1): checks first, writes the compose file through safe save,
    /// generated secrets to a 0600 `.env` over stdin, then pulls and starts it.
    /// Returns what the user needs to sign in; Orbital keeps none of it.
    pub async fn install_app(&self, sid: &str, app: &str, name: &str, port: u16, force: bool) -> Result<crate::apps::Installed> {
        use crate::apps as ap;
        let s = self.session(sid).await?;
        let spec = ap::find(app).map_err(|e| self.refused_as(&s, Category::Docker, "app.install", app, e))?;
        let script = ap::preflight_script(name, port).map_err(|e| self.refused_as(&s, Category::Docker, "app.install", name, e))?;
        progress::step(format!("Checking the server for {} …", spec.name));
        let (o, _) = s.exec_inspect(&script).await?;
        let pre = ap::parse_preflight(&o.stdout);
        if let Some((why, overridable)) = ap::blocker(spec, name, port, &pre) {
            if !(overridable && force) {
                let e = if overridable {
                    Error::PreflightFailed {
                        check: "memory".into(),
                        output: why,
                    }
                } else {
                    Error::InvalidInput(why)
                };
                return Err(self.refused_as(&s, Category::Docker, "app.install", name, e));
            }
        }
        let content = ap::compose(spec, name, port)?;
        self.create_stack(sid, name, &content, false).await?;
        let (env, shown) = ap::env_file(spec);
        progress::step("Writing generated passwords to .env (0600) …");
        self.system_exec_with(
            &s,
            "app.env",
            name,
            &ap::write_env_command(name)?,
            Some(env.as_bytes()),
            std::time::Duration::from_secs(30),
        )
        .await?
        .check()?;
        let file = crate::stacks::stack_file(name);
        let out = self.stack_action(sid, name, &file, crate::stacks::StackAction::PullUp).await?;
        Ok(ap::Installed {
            name: name.into(),
            app: spec.id.into(),
            port,
            started: out.ok(),
            output: out.stdout,
            shown: shown.into_iter().map(|(label, value)| ap::Shown { label, value }).collect(),
            first_steps: spec.first_steps.into(),
        })
    }

    // ---------- swap ----------

    /// Add a swap file of `mb` MB (checks first; refused when swap is on already
    /// or the disk hasn't room). Returns the new swap size in bytes.
    pub async fn add_swap(&self, sid: &str, mb: u32) -> Result<u64> {
        let s = self.session(sid).await?;
        let script = crate::swap::add_script(mb).map_err(|e| self.refused_as(&s, Category::Command, "swap.add", &format!("{mb} MB"), e))?;
        progress::step(format!("Adding {mb} MB of swap …"));
        let o = self
            .system_exec(&s, "swap.add", &format!("{mb} MB"), &script, std::time::Duration::from_secs(600))
            .await?;
        if !o.ok() {
            return Err(Error::InvalidInput(o.stderr.trim().to_string()));
        }
        Ok(o.stdout
            .lines()
            .find_map(|l| l.strip_prefix("@@swap "))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0))
    }

    // ---------- services made by Orbital ----------

    /// Turn a command into a systemd service: unit (+ root-only env file),
    /// verified with systemd-analyze (rolled back if it fails), then enabled
    /// and started. Refused when a unit by that name exists and isn't Orbital's.
    /// Returns systemctl's output and whether it's running.
    pub async fn create_service(&self, sid: &str, spec: &crate::units::ServiceSpec) -> Result<(bool, String)> {
        use crate::units as u;
        let s = self.session(sid).await?;
        let spec = u::validate(spec).map_err(|e| self.refused_as(&s, Category::Service, "service.create", &spec.name, e))?;
        progress::step(format!("Checking that {}.service is free …", spec.name));
        let (o, _) = s.exec_inspect(&u::existing_check(&spec.name)).await?;
        if let Some(path) = o.stdout.lines().find_map(|l| l.strip_prefix("@@theirs ")) {
            let e = Error::InvalidInput(format!(
                "a service named {}.service already exists ({path}) and wasn't created by Orbital; choose another name",
                spec.name
            ));
            return Err(self.refused_as(&s, Category::Service, "service.create", &spec.name, e));
        }
        let t = std::time::Duration::from_secs(60);
        if !spec.env.is_empty() {
            progress::step("Writing its environment (root only) …");
            self.system_exec_with(
                &s,
                "service.env",
                &spec.name,
                &u::write_env_command(&spec.name),
                Some(u::render_env(&spec).as_bytes()),
                t,
            )
            .await?
            .check()?;
        }
        progress::step(format!(
            "Writing {} and checking it with systemd-analyze …",
            u::unit_path(&spec.name)
        ));
        let path = u::unit_path(&spec.name);
        let validator = crate::config_edit::Validator {
            name: "systemd-analyze".into(),
            command: format!("systemd-analyze verify {}", crate::shell::quote(&path)),
            stage: crate::config_edit::ValidateStage::PostInstall,
        };
        self.audited(
            &s,
            Category::Service,
            "service.create",
            Some(&spec.name),
            None,
            s.safe_write(&path, &u::render_unit(&spec), Some(validator), true, None),
        )
        .await?;
        progress::step(format!("Starting {} …", spec.name));
        let o = self
            .system_exec(&s, "service.start", &spec.name, &u::start_command(&spec.name), t)
            .await?;
        Ok((o.stdout.lines().last().map(str::trim) == Some("active"), o.combined()))
    }

    /// Stop and remove a service Orbital created (its unit and env file).
    pub async fn remove_service(&self, sid: &str, name: &str) -> Result<()> {
        let s = self.session(sid).await?;
        crate::shell::validate_unit(name)?;
        let name = name.trim_end_matches(".service");
        self.system_exec(
            &s,
            "service.remove",
            name,
            &crate::units::remove_command(name),
            std::time::Duration::from_secs(60),
        )
        .await?
        .check()?;
        Ok(())
    }

    // ---------- scheduled backups ----------

    /// Backup jobs on this server, with their backups and last run (read-only).
    pub async fn backup_jobs(&self, sid: &str) -> Result<Vec<crate::backups::BackupStatus>> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::backups::LIST_SCRIPT).await?;
        Ok(crate::backups::parse_list(&o.stdout))
    }

    /// Create or change a job: its script and cron line, written as root (contents on stdin).
    pub async fn save_backup_job(&self, sid: &str, job: &crate::backups::BackupJob) -> Result<crate::backups::BackupJob> {
        use crate::backups as b;
        let s = self.session(sid).await?;
        let job = b::validate(job).map_err(|e| self.refused_as(&s, Category::Command, "backup.save", &job.id, e))?;
        let (script, cron, _) = b::paths(&job.id)?;
        if let Some(check) = b::tool_check(&job.source) {
            progress::step("Checking that the database's backup tool is installed …");
            let (o, _) = s.exec_inspect(&check).await?;
            if let Some(tool) = o.stdout.lines().find_map(|l| l.strip_prefix("@@missing ")) {
                let e = Error::InvalidInput(format!(
                    "{tool} isn't installed on this server, so this database can't be backed up here"
                ));
                return Err(self.refused_as(&s, Category::Command, "backup.save", &job.id, e));
            }
        }
        progress::step(format!("Writing the backup job {} …", job.name));
        let t = std::time::Duration::from_secs(60);
        self.system_exec_with(
            &s,
            "backup.save",
            &job.id,
            &b::write_command(&script, "755"),
            Some(b::script(&job)?.as_bytes()),
            t,
        )
        .await?
        .check()?;
        self.system_exec_with(
            &s,
            "backup.save",
            &job.id,
            &b::write_command(&cron, "644"),
            Some(b::cron_file(&job)?.as_bytes()),
            t,
        )
        .await?
        .check()?;
        Ok(job)
    }

    /// Remove a job (its schedule and script); with `files`, its backups too.
    pub async fn delete_backup_job(&self, sid: &str, id: &str, files: bool) -> Result<()> {
        let s = self.session(sid).await?;
        let cmd = crate::backups::delete_command(id, files)?;
        self.system_exec(
            &s,
            if files { "backup.delete_all" } else { "backup.delete" },
            id,
            &cmd,
            std::time::Duration::from_secs(120),
        )
        .await?
        .check()?;
        Ok(())
    }

    /// Run a job now (as cron would). Returns the new backup's path.
    pub async fn run_backup(&self, sid: &str, id: &str) -> Result<String> {
        use crate::backups as b;
        let s = self.session(sid).await?;
        let (script, _, _) = b::paths(id)?;
        progress::step("Backing up …");
        let o = self
            .system_exec(
                &s,
                "backup.run",
                id,
                &crate::shell::quote(&script),
                std::time::Duration::from_secs(3 * 3600),
            )
            .await?
            .check()?;
        Ok(o.stdout.lines().find_map(|l| l.strip_prefix("@@file ")).unwrap_or("").to_string())
    }

    /// Restore one of a job's backups. A folder goes to a new copy beside the
    /// original unless `to` is Original; a database's contents are replaced.
    pub async fn restore_scheduled_backup(&self, sid: &str, id: &str, file: &str, to: crate::backups::RestoreTo) -> Result<String> {
        use crate::backups as b;
        let job = self
            .backup_jobs(sid)
            .await?
            .into_iter()
            .find(|x| x.job.id == id)
            .ok_or_else(|| Error::NotFound(format!("no backup job {id} on this server")))?
            .job;
        let s = self.session(sid).await?;
        let cmd = b::restore_command(&job, file, to)?;
        progress::step(format!("Restoring {} …", file.rsplit('/').next().unwrap_or(file)));
        let o = self
            .system_exec(
                &s,
                "backup.restore",
                &format!("{id}: {file}"),
                &cmd,
                std::time::Duration::from_secs(3 * 3600),
            )
            .await?
            .check()?;
        Ok(o.stdout
            .lines()
            .find_map(|l| l.strip_prefix("@@restored "))
            .unwrap_or("")
            .to_string())
    }

    // ---------- system updates ----------

    /// Upgradable packages (security first) and whether a reboot is needed. Read-only.
    pub async fn updates(&self, sid: &str) -> Result<crate::updates::UpdatesReport> {
        self.session(sid).await?.updates().await
    }

    /// Run a root command for system maintenance, streaming its output to the
    /// job, with its own timeout, and record it.
    async fn system_exec(&self, s: &SshSession, action: &str, target: &str, cmd: &str, timeout: std::time::Duration) -> Result<ExecOutput> {
        self.system_exec_with(s, action, target, cmd, None, timeout).await
    }

    /// [`system_exec`] without streaming the command's output to the job: the
    /// user sees only the step, not installer noise. Recorded the same way.
    async fn system_exec_quiet(
        &self,
        s: &SshSession,
        action: &str,
        target: &str,
        cmd: &str,
        timeout: std::time::Duration,
    ) -> Result<ExecOutput> {
        let sent = s.privileged_command(cmd);
        match s.exec_privileged_with(cmd, None, timeout).await {
            Ok(o) => {
                self.record_exec(s, Category::Command, action, target, &sent, &o);
                Ok(o)
            }
            Err(e) => {
                self.record_error(s, Category::Command, action, target, Some(sent), &e);
                Err(e)
            }
        }
    }

    /// [`system_exec`] with data on stdin (file contents, secrets): recorded
    /// with the command only, never the data.
    async fn system_exec_with(
        &self,
        s: &SshSession,
        action: &str,
        target: &str,
        cmd: &str,
        input: Option<&[u8]>,
        timeout: std::time::Duration,
    ) -> Result<ExecOutput> {
        let sent = s.privileged_command(cmd);
        match progress::with_output(s.exec_privileged_with(cmd, input, timeout)).await {
            Ok(o) => {
                self.record_exec(s, Category::Command, action, target, &sent, &o);
                Ok(o)
            }
            Err(e) => {
                self.record_error(s, Category::Command, action, target, Some(sent), &e);
                Err(e)
            }
        }
    }

    /// Refresh the package list (`apt-get update`, `dnf makecache`), then list again.
    pub async fn refresh_updates(&self, sid: &str) -> Result<crate::updates::UpdatesReport> {
        let s = self.session(sid).await?;
        let pm = s.package_manager().await?;
        progress::step("Asking the package mirrors for new versions …");
        self.system_exec(
            &s,
            "updates.refresh",
            "package list",
            pm.refresh_command(),
            std::time::Duration::from_secs(600),
        )
        .await?
        .check()?;
        progress::step("Listing what can be upgraded …");
        s.updates().await
    }

    /// Install every available update. Returns the package manager's output
    /// whether it worked or not.
    pub async fn install_updates(&self, sid: &str) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let pm = s.package_manager().await?;
        progress::step("Installing updates (this can take several minutes) …");
        self.system_exec(
            &s,
            "updates.install",
            "all packages",
            pm.upgrade_command(),
            std::time::Duration::from_secs(3600),
        )
        .await
    }

    /// The Docker version now (None if it isn't installed or doesn't answer):
    /// read again after installing, so the Docker tab works without reconnecting.
    pub async fn docker_version(&self, sid: &str) -> Result<Option<String>> {
        Ok(self.session(sid).await?.docker_available().await.ok().flatten())
    }

    /// Software Orbital can install (the "+" menu's "You may try").
    pub fn install_options(&self) -> Vec<crate::install::InstallOption> {
        crate::install::options()
    }

    /// Install one of `install::INSTALLABLE` from the server's own repositories
    /// and start it. A web server when port 80 is taken needs `force`.
    pub async fn install_software(&self, sid: &str, id: &str, force: bool) -> Result<ExecOutput> {
        let s = self.session(sid).await?;
        let item = crate::install::find(id).map_err(|e| self.refused_as(&s, Category::Command, "software.install", id, e))?;
        if crate::install::needs_port80(item) && !force {
            let (o, _) = s.exec_inspect(crate::install::PORT80_CHECK).await?;
            if !o.stdout.trim().is_empty() {
                let e = Error::PreflightFailed {
                    check: "port 80 is already in use".into(),
                    output: format!("Another program listens on port 80, so this web server would fail to start (or the other one would, after a reboot):\n{}", o.stdout.trim()),
                };
                self.record_error(&s, Category::Command, "software.install", id, None, &e);
                return Err(e);
            }
        }
        progress::step(format!("Installing {id} from the server's package repositories …"));
        self.system_exec(
            &s,
            "software.install",
            id,
            &crate::install::script(item),
            std::time::Duration::from_secs(1800),
        )
        .await?
        .check()
    }

    /// Reboot the server. The connection drops a few seconds later.
    pub async fn reboot(&self, sid: &str) -> Result<()> {
        let s = self.session(sid).await?;
        self.system_exec(
            &s,
            "server.reboot",
            &s.profile.host,
            crate::updates::REBOOT_COMMAND,
            std::time::Duration::from_secs(30),
        )
        .await?
        .check()?;
        Ok(())
    }

    // ---------- AI agent (Telegram) ----------

    /// The agent's state on a server (read-only).
    pub async fn agent_status(&self, sid: &str) -> Result<crate::agent::AgentStatus> {
        self.session(sid).await?.agent_status().await
    }

    /// Check the bot token and the AI key from this computer: the bot's @name and the models the key can use.
    pub async fn agent_check(&self, setup: &crate::agent::AgentSetup) -> Result<crate::agent::AgentCheck> {
        crate::agent::validate_setup(setup)?;
        let bot = crate::agent::telegram_bot_name(&setup.telegram_token).await?;
        let models = crate::agent::check_ai_key(setup.provider, &setup.api_key).await?;
        if let Some(m) = setup.model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            if !models.iter().any(|x| x.id == m) {
                return Err(Error::InvalidInput(format!(
                    "this key can't use the model {m}; pick one from the list"
                )));
            }
        }
        Ok(crate::agent::AgentCheck { bot, models })
    }

    /// Install (or reconfigure) the agent: Python and the AI libraries, the
    /// agent, its root-only config with the keys (through stdin), the
    /// service, and a pairing code for Telegram.
    pub async fn agent_install(&self, sid: &str, setup: &crate::agent::AgentSetup) -> Result<crate::agent::PairCode> {
        use crate::agent as a;
        let s = self.session(sid).await?;
        progress::step("Checking the bot token and the API key …");
        let bot = self
            .agent_check(setup)
            .await
            .map_err(|e| self.refused_as(&s, Category::Command, "agent.install", "AI agent", e))?
            .bot;
        let target = format!("@{bot}");
        let long = std::time::Duration::from_secs(900);
        let short = std::time::Duration::from_secs(60);
        progress::step("Setting up the agent on the server (the first time takes a minute or two) …");
        self.system_exec_quiet(&s, "agent.prepare", &target, a::PREPARE_SCRIPT, long)
            .await?
            .check()?;
        progress::step("Installing the agent …");
        self.system_exec_with(&s, "agent.upload", &target, a::WRITE_AGENT, Some(a::AGENT_PY.as_bytes()), short)
            .await?
            .check()?;
        progress::step("Saving your settings …");
        let config = a::config_json(setup, &bot, &s.profile.name)?;
        self.system_exec_with(&s, "agent.configure", &target, a::WRITE_CONFIG, Some(config.as_bytes()), short)
            .await?
            .check()?;
        progress::step("Starting the agent …");
        self.system_exec_with(&s, "agent.start", &target, a::WRITE_UNIT, Some(a::UNIT.as_bytes()), short)
            .await?
            .check()?;
        progress::step("Making a pairing code …");
        let mut code = self.agent_pair_code_inner(&s).await?;
        code.bot = Some(bot);
        Ok(code)
    }

    async fn agent_pair_code_inner(&self, s: &SshSession) -> Result<crate::agent::PairCode> {
        let code = crate::agent::new_pair_code();
        let input = format!("{code}\n");
        self.system_exec_with(
            s,
            "agent.pair",
            "new pairing code",
            crate::agent::SET_PAIR_CODE,
            Some(input.as_bytes()),
            std::time::Duration::from_secs(30),
        )
        .await?
        .check()?;
        Ok(crate::agent::PairCode {
            code,
            expires: chrono::Utc::now().timestamp() + 15 * 60,
            bot: None,
        })
    }

    /// A new pairing code (valid 15 minutes); the Telegram account paired now is unpaired.
    pub async fn agent_pair_code(&self, sid: &str) -> Result<crate::agent::PairCode> {
        let s = self.session(sid).await?;
        let mut code = self.agent_pair_code_inner(&s).await?;
        code.bot = s.agent_status().await.ok().and_then(|st| st.bot);
        Ok(code)
    }

    /// Install this Orbital's version of the agent (and newer AI libraries), keeping its config.
    pub async fn agent_update(&self, sid: &str) -> Result<()> {
        use crate::agent as a;
        let s = self.session(sid).await?;
        progress::step("Updating the agent …");
        self.system_exec_quiet(
            &s,
            "agent.prepare",
            "AI agent",
            a::PREPARE_SCRIPT,
            std::time::Duration::from_secs(900),
        )
        .await?
        .check()?;

        self.system_exec_with(
            &s,
            "agent.upload",
            "AI agent",
            a::WRITE_AGENT,
            Some(a::AGENT_PY.as_bytes()),
            std::time::Duration::from_secs(60),
        )
        .await?
        .check()?;
        progress::step("Restarting it …");
        self.system_exec(&s, "agent.start", "AI agent", a::RESTART, std::time::Duration::from_secs(60))
            .await?
            .check()?;
        Ok(())
    }

    /// Start or stop the agent service.
    pub async fn agent_set_running(&self, sid: &str, on: bool) -> Result<()> {
        let s = self.session(sid).await?;
        let (action, cmd) = if on {
            ("agent.start", crate::agent::RESTART)
        } else {
            ("agent.stop", crate::agent::STOP)
        };
        self.system_exec(&s, action, "AI agent", cmd, std::time::Duration::from_secs(60))
            .await?
            .check()?;
        Ok(())
    }

    /// Turn Telegram alerts (failed services, full disks, restarting containers) on or off.
    pub async fn agent_set_alerts(&self, sid: &str, on: bool) -> Result<()> {
        let s = self.session(sid).await?;
        let cmd = if on { crate::agent::ALERTS_ON } else { crate::agent::ALERTS_OFF };
        self.system_exec(
            &s,
            "agent.alerts",
            if on { "on" } else { "off" },
            cmd,
            std::time::Duration::from_secs(30),
        )
        .await?
        .check()?;
        Ok(())
    }

    /// The models the agent's key can use (the agent lists them; Orbital doesn't keep the key).
    pub async fn agent_models(&self, sid: &str) -> Result<Vec<crate::agent::AiModel>> {
        let s = self.session(sid).await?;
        let (o, _) = s.exec_inspect(crate::agent::LIST_MODELS).await?;
        crate::agent::parse_model_list(&o.stdout, &o.stderr)
    }

    /// Switch the agent to another model the key can use; it applies from the next message.
    pub async fn agent_set_model(&self, sid: &str, model: &str) -> Result<()> {
        let s = self.session(sid).await?;
        let cmd = crate::agent::set_model_command(model).map_err(|e| self.refused_as(&s, Category::Command, "agent.model", model, e))?;
        let models = self.agent_models(sid).await?;
        if !models.iter().any(|m| m.id == model.trim()) {
            return Err(self.refused_as(
                &s,
                Category::Command,
                "agent.model",
                model,
                Error::InvalidInput(format!("the agent's key can't use {model}")),
            ));
        }
        self.system_exec(&s, "agent.model", model.trim(), &cmd, std::time::Duration::from_secs(30))
            .await?
            .check()?;
        Ok(())
    }

    /// Have the agent message the paired Telegram account; returns who it reached.
    pub async fn agent_test(&self, sid: &str) -> Result<Option<String>> {
        let s = self.session(sid).await?;
        let o = self
            .system_exec(
                &s,
                "agent.test",
                "Telegram",
                crate::agent::TEST_COMMAND,
                std::time::Duration::from_secs(40),
            )
            .await?;
        crate::agent::parse_test(&o.stdout, &o.stderr)
    }

    /// Daily summary at a time of day (server time), or off.
    pub async fn agent_set_daily(&self, sid: &str, time: Option<&str>) -> Result<()> {
        let s = self.session(sid).await?;
        let cmd = crate::agent::set_daily_command(time)
            .map_err(|e| self.refused_as(&s, Category::Command, "agent.daily", time.unwrap_or("off"), e))?;
        self.system_exec(&s, "agent.daily", time.unwrap_or("off"), &cmd, std::time::Duration::from_secs(30))
            .await?
            .check()?;
        Ok(())
    }

    /// Remove the agent, its config (with the keys) and its log from the server.
    pub async fn agent_remove(&self, sid: &str) -> Result<()> {
        let s = self.session(sid).await?;
        self.system_exec(
            &s,
            "agent.remove",
            "AI agent",
            crate::agent::REMOVE,
            std::time::Duration::from_secs(60),
        )
        .await?
        .check()?;
        Ok(())
    }

    // ---------- disk cleanup and processes ----------

    /// What could be freed on the disk (read-only).
    pub async fn cleanup_scan(&self, sid: &str) -> Result<Vec<crate::cleanup::CleanupItem>> {
        self.session(sid).await?.cleanup_scan().await
    }

    /// Free one kind of space (`cleanup::command`), then scan again.
    pub async fn cleanup_run(&self, sid: &str, id: &str) -> Result<Vec<crate::cleanup::CleanupItem>> {
        let s = self.session(sid).await?;
        let action = format!("cleanup.{id}");
        let cmd = crate::cleanup::command(id).map_err(|e| self.refused_as(&s, Category::Command, &action, id, e))?;
        progress::step("Freeing space …");
        self.system_exec(&s, &action, id, cmd, std::time::Duration::from_secs(1800))
            .await?
            .check()?;
        progress::step("Measuring again …");
        s.cleanup_scan().await
    }

    /// Stop a process: SIGTERM, or SIGKILL when `force`. PID 1 and sshd are
    /// refused (stopping sshd would cut this connection; restart it from
    /// Services, which checks its config first).
    pub async fn stop_process(&self, sid: &str, pid: u32, force: bool) -> Result<()> {
        let s = self.session(sid).await?;
        const ACTION: &str = "process.stop";
        let target = format!("PID {pid}");
        if pid <= 1 {
            return Err(self.refused_as(
                &s,
                Category::Command,
                ACTION,
                &target,
                Error::InvalidInput("PID 1 is the system itself".into()),
            ));
        }
        let (o, _) = s.exec_inspect(&format!("ps -o comm= -p {pid}")).await?;
        let name = o.stdout.trim().to_string();
        if name.is_empty() {
            return Err(Error::NotFound(format!("no process {pid} (it may have ended)")));
        }
        if name == "sshd" || name.starts_with("sshd:") {
            return Err(self.refused_as(
                &s,
                Category::Command,
                ACTION,
                &target,
                Error::InvalidInput("stopping sshd would cut this connection; restart ssh from Services instead".into()),
            ));
        }
        let cmd = format!("kill -{} {pid}", if force { "KILL" } else { "TERM" });
        self.system_exec(&s, ACTION, &format!("{name} ({target})"), &cmd, std::time::Duration::from_secs(30))
            .await?
            .check()?;
        Ok(())
    }

    // ---------- commands & terminals ----------

    /// The server's shell history (newest first, likely secrets left out), for
    /// the terminal's suggestions. Root's history for root terminals. Read-only.
    pub async fn shell_history(&self, sid: &str, root: bool) -> Result<Vec<String>> {
        self.session(sid).await?.shell_history(root).await
    }

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

    /// Open a PTY shell. With `as_root`, and when the profile can use sudo,
    /// the shell runs as root (`sudo -i`, or `sudo -s` in `cwd`).
    #[allow(clippy::too_many_arguments)]
    pub async fn open_terminal<F>(
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
        let root = as_root && s.terminal_root_available();
        let what = match &launch {
            crate::terminal::Launch::Shell => cwd.as_deref().unwrap_or("~").to_string(),
            crate::terminal::Launch::Tmux { session } => format!("tmux {session}"),
            crate::terminal::Launch::Container { id } => format!("container {id}"),
        };
        let target = format!("{what}{}", if root { " (as root)" } else { "" });
        let t = self
            .audited(
                &s,
                Category::Terminal,
                "terminal.open",
                Some(&target),
                None,
                s.open_terminal(cols, rows, cwd.clone(), as_root, &launch, sink),
            )
            .await?;
        let id = t.id.clone();
        self.terminals.lock().await.insert(id.clone(), (sid.to_string(), Arc::new(t)));
        Ok(id)
    }

    /// tmux sessions on the server, to reattach (root's for root terminals). Read-only.
    pub async fn tmux_sessions(&self, sid: &str, root: bool) -> Result<Vec<crate::terminal::TmuxSession>> {
        self.session(sid).await?.tmux_sessions(root).await
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
        crate::recording::resized(tid, cols, rows);
        self.terminal(tid).await?.resize(cols, rows).await
    }

    pub async fn terminal_close(&self, tid: &str) -> Result<()> {
        crate::recording::finish(tid);
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

    // ---------- uptime checks (run from this Mac) ----------

    fn uptime_rt(&self) -> std::sync::MutexGuard<'_, uptime::Runtime> {
        self.uptime.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn uptime_emit(&self, ev: UptimeEvent) {
        let l = self.uptime_listener.read().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(l) = l {
            l(ev);
        }
    }

    /// Where uptime results and alerts go (notifications, toasts).
    pub fn set_uptime_listener(&self, f: impl Fn(UptimeEvent) + Send + Sync + 'static) {
        *self.uptime_listener.write().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(f));
    }

    fn record_uptime(&self, action: &str, c: &Check, outcome: Outcome, details: Option<serde_json::Value>) {
        self.record(
            None,
            NewEvent {
                category: Some(Category::Uptime),
                action: action.into(),
                target: Some(if c.name == c.target {
                    c.target.clone()
                } else {
                    format!("{} ({})", c.name, c.target)
                }),
                outcome: Some(outcome),
                details,
                ..Default::default()
            },
        );
    }

    pub fn list_uptime(&self) -> Result<Vec<CheckStatus>> {
        let now = chrono::Utc::now();
        uptime::list_checks(&self.db)?
            .into_iter()
            .map(|c| uptime::status(&self.db, c, now))
            .collect()
    }

    pub fn save_uptime_check(&self, id: Option<&str>, input: CheckInput) -> Result<Check> {
        let c = uptime::save_check(&self.db, id, &input)?;
        // An edited check starts over: it runs on the next tick and alerts afresh.
        self.uptime_rt().forget(&c.id);
        self.record_uptime(
            if id.is_some() {
                "uptime.check_update"
            } else {
                "uptime.check_create"
            },
            &c,
            Outcome::Success,
            None,
        );
        Ok(c)
    }

    pub fn set_uptime_paused(&self, id: &str, paused: bool) -> Result<Check> {
        let c = uptime::set_paused(&self.db, id, paused)?;
        self.uptime_rt().forget(id);
        self.record_uptime(if paused { "uptime.pause" } else { "uptime.resume" }, &c, Outcome::Success, None);
        Ok(c)
    }

    pub fn delete_uptime_check(&self, id: &str) -> Result<()> {
        let c = uptime::get_check(&self.db, id)?;
        uptime::delete_check(&self.db, id)?;
        self.uptime_rt().forget(id);
        self.record_uptime("uptime.check_delete", &c, Outcome::Success, None);
        Ok(())
    }

    /// Run a check now (the "Check now" button). Alerts like a scheduled run.
    pub async fn run_uptime_check(&self, id: &str) -> Result<CheckStatus> {
        let c = uptime::get_check(&self.db, id)?;
        self.uptime_run(c, false).await?;
        uptime::status(&self.db, uptime::get_check(&self.db, id)?, chrono::Utc::now())
    }

    /// Run one check, store the result and alert on a change. With `retry`, a
    /// failure is tried once more a few seconds later before it counts, so a
    /// single dropped request doesn't page anyone.
    async fn uptime_run(&self, c: Check, retry: bool) -> Result<Option<CheckResult>> {
        if !self.uptime_rt().running.insert(c.id.clone()) {
            return Ok(None);
        }
        struct Running<'a>(&'a Engine, String);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.uptime_rt().running.remove(&self.1);
            }
        }
        let _guard = Running(self, c.id.clone());
        let client = self.http.get_or_init(uptime::http_client);
        let mut r = uptime::probe(client, &c).await;
        if !r.up && retry {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            r = uptime::probe(client, &c).await;
        }
        // Deleted or changed while it ran: drop the result.
        match uptime::get_check(&self.db, &c.id) {
            Ok(now) if now.target == c.target && now.kind == c.kind => {}
            Ok(_) | Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        }
        let prev = self.uptime_rt().states.get(&c.id).copied();
        // Without one, pick up where the last launch left off, so a restart doesn't re-alert.
        let prev = match prev {
            Some(p) => p,
            None => uptime::last_state(&self.db, &c.id)?,
        };
        uptime::store_result(&self.db, &c.id, &r)?;
        let (state, change) = uptime::transition(prev, r.up);
        let warn = {
            let mut rt = self.uptime_rt();
            rt.states.insert(c.id.clone(), state);
            rt.last_run.insert(c.id.clone(), Instant::now());
            match &r.warning {
                Some(w) if rt.cert_warned.insert(c.id.clone()) => Some(w.clone()),
                Some(_) => None,
                None => {
                    rt.cert_warned.remove(&c.id);
                    None
                }
            }
        };
        if let Some(change) = change {
            uptime::store_change(&self.db, &c.id, &r.at, r.up, r.error.as_deref())?;
            let (action, outcome) = match change {
                Change::WentDown => ("uptime.down", Outcome::Failure),
                Change::Recovered => ("uptime.up", Outcome::Success),
            };
            self.record_uptime(
                action,
                &c,
                outcome,
                Some(serde_json::json!({ "error": r.error, "statusCode": r.status_code, "latencyMs": r.latency_ms })),
            );
            self.uptime_emit(UptimeEvent::Changed {
                check_id: c.id.clone(),
                name: c.name.clone(),
                target: c.target.clone(),
                change,
                error: r.error.clone(),
            });
            // And to Telegram, email or the agent, when set up.
            let alert = match change {
                Change::WentDown => crate::notify::Alert {
                    kind: crate::notify::AlertKind::Uptime,
                    level: crate::notify::Level::Bad,
                    title: format!("{} is down", c.name),
                    body: format!("{}\n{}", c.target, r.error.clone().unwrap_or_else(|| "The check failed.".into())),
                },
                Change::Recovered => crate::notify::Alert {
                    kind: crate::notify::AlertKind::Uptime,
                    level: crate::notify::Level::Ok,
                    title: format!("{} is back up", c.name),
                    body: c.target.clone(),
                },
            };
            self.send_alert(&alert).await;
        }
        if let Some(message) = warn {
            self.send_alert(&crate::notify::Alert {
                kind: crate::notify::AlertKind::Uptime,
                level: crate::notify::Level::Warn,
                title: format!("{}: certificate expiring", c.name),
                body: message.clone(),
            })
            .await;
            self.uptime_emit(UptimeEvent::CertWarning {
                check_id: c.id.clone(),
                name: c.name.clone(),
                message,
            });
        }
        self.uptime_emit(UptimeEvent::Result {
            check_id: c.id.clone(),
            result: r.clone(),
        });
        Ok(Some(r))
    }

    /// The scheduler: every few seconds, start the checks that are due. Runs
    /// until the app quits; spawn it once.
    pub async fn run_uptime(self: Arc<Self>) {
        loop {
            match uptime::list_checks(&self.db) {
                Ok(checks) => {
                    let now = Instant::now();
                    let due: Vec<Check> = {
                        let rt = self.uptime_rt();
                        checks
                            .into_iter()
                            .filter(|c| !rt.running.contains(&c.id) && uptime::is_due(c, rt.last_run.get(&c.id).copied(), now))
                            .collect()
                    };
                    for c in due {
                        // Mark it now so the next tick doesn't start it again while it runs.
                        self.uptime_rt().last_run.insert(c.id.clone(), now);
                        let e = self.clone();
                        tokio::spawn(async move {
                            if let Err(err) = e.uptime_run(c, true).await {
                                tracing::error!("uptime check failed to run: {err}");
                            }
                        });
                    }
                }
                Err(e) => tracing::error!("couldn't read uptime checks: {e}"),
            }
            let prune = {
                let mut rt = self.uptime_rt();
                let due = rt.last_prune.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(3600));
                if due {
                    rt.last_prune = Some(Instant::now());
                }
                due
            };
            if prune {
                if let Err(e) = uptime::prune(&self.db, chrono::Utc::now()) {
                    tracing::error!("couldn't prune uptime results: {e}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

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

// ---------- snippets ----------

impl Engine {
    pub fn snippets(&self) -> Result<Vec<snippets::Snippet>> {
        snippets::list(&self.db)
    }

    fn record_snippet(&self, action: &str, name: &str) {
        self.record(
            None,
            NewEvent {
                category: Some(Category::Command),
                action: action.into(),
                target: Some(name.into()),
                outcome: Some(Outcome::Success),
                ..Default::default()
            },
        );
    }

    pub fn save_snippet(&self, id: Option<&str>, input: &snippets::SnippetInput) -> Result<snippets::Snippet> {
        let s = snippets::save(&self.db, id, input)?;
        self.record_snippet(if id.is_some() { "snippet.update" } else { "snippet.create" }, &s.name);
        Ok(s)
    }

    pub fn delete_snippet(&self, id: &str) -> Result<()> {
        let s = snippets::get(&self.db, id)?;
        snippets::delete(&self.db, id)?;
        self.record_snippet("snippet.delete", &s.name);
        Ok(())
    }

    /// Add the example snippets (only when there are none).
    pub fn add_example_snippets(&self) -> Result<Vec<snippets::Snippet>> {
        if snippets::list(&self.db)?.is_empty() {
            for e in snippets::examples() {
                snippets::save(&self.db, None, &e)?;
            }
            self.record_snippet("snippet.create", "examples");
        }
        snippets::list(&self.db)
    }

    /// Run a snippet on every server in `session_ids` at once. Each server's
    /// run is audited with the exact command sent; one failing doesn't stop the others.
    pub async fn run_snippet(
        &self,
        id: &str,
        session_ids: &[String],
        values: &HashMap<String, String>,
        with_sudo: bool,
    ) -> Result<Vec<snippets::SnippetRun>> {
        let snip = snippets::get(&self.db, id)?;
        let command = snippets::render(&snip.command, values)?;
        if session_ids.is_empty() {
            return Err(Error::InvalidInput("choose at least one server".into()));
        }
        if session_ids.len() > 50 {
            return Err(Error::InvalidInput("run on at most 50 servers at once".into()));
        }
        let runs = session_ids.iter().map(|sid| {
            let command = command.clone();
            let name = snip.name.clone();
            async move {
                let s = match self.session(sid).await {
                    Ok(s) => s,
                    Err(e) => {
                        return snippets::SnippetRun {
                            session_id: sid.clone(),
                            server: sid.clone(),
                            command,
                            output: None,
                            error: Some(e.to_string()),
                            needs_sudo: false,
                        }
                    }
                };
                let sudo = snip.sudo || with_sudo;
                let sent = if sudo { s.privileged_command(&command) } else { command.clone() };
                let r = if sudo {
                    s.exec_privileged(&command).await
                } else {
                    s.exec(&command).await
                };
                match &r {
                    Ok(o) => self.record_exec(&s, Category::Command, "snippet.run", &name, &sent, o),
                    Err(e) => self.record_error(&s, Category::Command, "snippet.run", &name, Some(sent.clone()), e),
                }
                let needs_sudo = !sudo
                    && s.elevation_needed()
                    && matches!(&r, Ok(o) if !o.ok() && deploy::refused(&format!("{}{}", o.stdout, o.stderr)));
                snippets::SnippetRun {
                    session_id: sid.clone(),
                    server: s.label(),
                    command,
                    error: r.as_ref().err().map(|e| e.to_string()),
                    output: r.ok(),
                    needs_sudo,
                }
            }
        });
        Ok(futures_util::future::join_all(runs).await)
    }
}

// ---------- SSH keys ----------

/// Reads the login user's authorized_keys (nothing is interpolated).
const READ_AUTHORIZED: &str = "cd && if [ -f .ssh/authorized_keys ]; then echo @@file; cat .ssh/authorized_keys; else echo @@missing; fi";
/// Replaces it with stdin: written beside it, then moved over it, so a broken
/// connection never leaves half a file. sshd wants 700 and 600.
const WRITE_AUTHORIZED: &str = "set -e; umask 077; cd; mkdir -p .ssh; chmod 700 .ssh; \
t=.ssh/.authorized_keys.orbital-$$; cat > \"$t\"; chmod 600 \"$t\"; mv -f \"$t\" .ssh/authorized_keys";

impl Engine {
    /// Key pairs in this computer's ~/.ssh.
    pub fn local_ssh_keys(&self) -> Vec<sshkeys::LocalKey> {
        sshkeys::ssh_dir().map(|d| sshkeys::local_keys(&d)).unwrap_or_default()
    }

    /// Make a new ed25519 key pair in ~/.ssh. The passphrase only encrypts the file.
    pub fn generate_ssh_key(&self, name: &str, comment: &str, passphrase: Option<&str>) -> Result<sshkeys::LocalKey> {
        let dir = sshkeys::ssh_dir().ok_or_else(|| Error::Other("couldn't find your home folder".into()))?;
        let r = sshkeys::generate(&dir, name, comment, passphrase);
        let (outcome, details) = match &r {
            Ok(_) => (Outcome::Success, None),
            Err(e) => (Outcome::Failure, Some(serde_json::json!({ "error": e.to_string() }))),
        };
        self.record(
            None,
            NewEvent {
                category: Some(Category::Security),
                action: "sshkey.generate".into(),
                target: Some(format!("~/.ssh/{name}")),
                outcome: Some(outcome),
                details: details.or_else(|| {
                    r.as_ref()
                        .ok()
                        .map(|k| serde_json::json!({ "fingerprint": k.fingerprint, "passphrase": k.encrypted }))
                }),
                ..Default::default()
            },
        );
        r
    }

    /// Fingerprints of the keys this computer could sign in with: ~/.ssh,
    /// the profile's own key file, and the SSH agent.
    async fn my_key_fingerprints(&self, s: &SshSession) -> Vec<String> {
        let mut out: Vec<String> = self.local_ssh_keys().into_iter().map(|k| k.fingerprint).collect();
        if let Some(path) = s.profile.key_path.as_deref().filter(|p| !p.is_empty()) {
            let path = crate::ssh::expand_tilde(path);
            if let Ok(k) = russh::keys::load_public_key(format!("{path}.pub")) {
                out.push(k.fingerprint(russh::keys::HashAlg::Sha256).to_string());
            }
        }
        out.extend(crate::ssh::agent_fingerprints().await);
        out.sort();
        out.dedup();
        out
    }

    async fn read_authorized(&self, s: &SshSession) -> Result<(String, bool)> {
        let o = s.exec(READ_AUTHORIZED).await?;
        if o.exit_code != 0 {
            return Err(Error::PermissionDenied(format!(
                "couldn't read ~/.ssh/authorized_keys: {}",
                o.stderr.trim()
            )));
        }
        match o.stdout.split_once('\n') {
            Some(("@@file", rest)) => Ok((rest.to_string(), false)),
            _ if o.stdout.starts_with("@@missing") => Ok((String::new(), true)),
            _ => Ok((String::new(), true)),
        }
    }

    /// The keys that can sign in as the connected user.
    pub async fn authorized_keys(&self, sid: &str) -> Result<sshkeys::AuthorizedKeys> {
        let s = self.session(sid).await?;
        let (text, missing) = self.read_authorized(&s).await?;
        let mine = self.my_key_fingerprints(&s).await;
        Ok(sshkeys::AuthorizedKeys {
            user: s.profile.username.clone(),
            path: "~/.ssh/authorized_keys".into(),
            keys: sshkeys::parse_authorized(&text, &mine),
            missing,
        })
    }

    async fn write_authorized(&self, s: &SshSession, action: &str, target: &str, text: &str) -> Result<()> {
        let o = s
            .exec_with_input(WRITE_AUTHORIZED, Some(text.as_bytes()), std::time::Duration::from_secs(30))
            .await;
        match o {
            Ok(o) => {
                self.record_exec(s, Category::Security, action, target, WRITE_AUTHORIZED, &o);
                o.check().map(|_| ())
            }
            Err(e) => {
                self.record_error(s, Category::Security, action, target, Some(WRITE_AUTHORIZED.into()), &e);
                Err(e)
            }
        }
    }

    /// Let a public key sign in as the connected user (like ssh-copy-id).
    pub async fn add_authorized_key(&self, sid: &str, line: &str) -> Result<sshkeys::AuthorizedKeys> {
        let s = self.session(sid).await?;
        let line = sshkeys::validate_public_line(line)?;
        let (text, _) = self.read_authorized(&s).await?;
        let new = sshkeys::with_key(&text, &line)?;
        let (_, a, b, c) = sshkeys::split_line(&line).expect("validated");
        let target = format!("{} {}", sshkeys::fingerprint(&a, &b).unwrap_or_default(), c)
            .trim()
            .to_string();
        self.write_authorized(&s, "sshkey.authorize", &target, &new).await?;
        self.authorized_keys(sid).await
    }

    /// Stop a key from signing in. Removing the last key from this computer
    /// while this connection uses a key is refused unless `force`.
    pub async fn remove_authorized_key(&self, sid: &str, line: &str, force: bool) -> Result<sshkeys::AuthorizedKeys> {
        let s = self.session(sid).await?;
        let (text, _) = self.read_authorized(&s).await?;
        let new = sshkeys::without_line(&text, line)?;
        let mine = self.my_key_fingerprints(&s).await;
        let keys = sshkeys::parse_authorized(&text, &mine);
        let target = keys
            .iter()
            .find(|k| k.line == line)
            .map(|k| {
                format!("{} {}", k.fingerprint.as_deref().unwrap_or("?"), k.comment)
                    .trim()
                    .to_string()
            })
            .unwrap_or_else(|| "?".into());
        let key_login = matches!(s.profile.auth_method, AuthMethod::Key | AuthMethod::Agent);
        if !force {
            if let Some(reason) = sshkeys::lockout_reason(&keys, line, key_login) {
                let e = Error::PreflightFailed {
                    check: "your access".into(),
                    output: reason,
                };
                self.record_error(&s, Category::Security, "sshkey.revoke", &target, None, &e);
                return Err(e);
            }
        }
        self.write_authorized(&s, "sshkey.revoke", &target, &new).await?;
        self.authorized_keys(sid).await
    }
}

// ---------- DNS ----------

impl Engine {
    /// Point DNS calls somewhere else (tests use local mock servers).
    #[doc(hidden)]
    pub fn set_dns_endpoints(&self, cloudflare_api: &str, resolvers: Vec<(String, String)>) {
        *self.dns_endpoints.write().unwrap_or_else(|p| p.into_inner()) = (cloudflare_api.into(), resolvers);
    }

    fn cloudflare_base(&self) -> String {
        self.dns_endpoints.read().unwrap_or_else(|p| p.into_inner()).0.clone()
    }

    fn record_dns(&self, account: &DnsAccount, action: &str, target: &str, command: Option<String>, r: &Result<impl Sized>) {
        let (outcome, details) = match r {
            Ok(_) => (Outcome::Success, None),
            Err(Error::PermissionDenied(m)) => (Outcome::Denied, Some(serde_json::json!({ "error": m }))),
            Err(e) => (
                Outcome::Failure,
                Some(serde_json::json!({ "error": e.to_string(), "kind": e.kind() })),
            ),
        };
        self.record(
            None,
            NewEvent {
                category: Some(Category::Dns),
                action: action.into(),
                server: Some(format!("{}: {}", account.kind.label(), account.name)),
                target: Some(target.into()),
                command,
                outcome: Some(outcome),
                details,
                ..Default::default()
            },
        );
    }

    pub fn dns_accounts(&self) -> Result<Vec<DnsAccount>> {
        dns::list_accounts(&self.db)
    }

    /// Save a provider account after checking that the token works (it must
    /// be able to list zones). Nothing is saved when it doesn't.
    pub async fn add_dns_account(&self, name: &str, kind: ProviderKind, token: &str) -> Result<DnsAccount> {
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::InvalidInput("paste the API token".into()));
        }
        let client = dns::Cloudflare::new(&self.cloudflare_base(), zeroize::Zeroizing::new(token.to_string()))?;
        let probe = DnsAccount {
            id: String::new(),
            name: name.trim().to_string(),
            kind,
            created_at: String::new(),
        };
        if let Err(e) = client.zones().await {
            self.record_dns(
                &probe,
                "dns.account_add",
                &probe.name,
                None,
                &Err::<(), _>(Error::Other(e.to_string())),
            );
            return Err(e);
        }
        let account = dns::insert_account(&self.db, name, kind)?;
        if let Err(e) = self.secrets.set(&dns::token_key(&account.id), token) {
            let _ = dns::delete_account(&self.db, &account.id);
            return Err(e);
        }
        self.dns_clients.lock().await.insert(account.id.clone(), Arc::new(client));
        self.record_dns(&account, "dns.account_add", &account.name, None, &Ok(()));
        Ok(account)
    }

    pub async fn remove_dns_account(&self, id: &str) -> Result<()> {
        let account = dns::get_account(&self.db, id)?;
        self.dns_clients.lock().await.remove(id);
        self.secrets.delete(&dns::token_key(id))?;
        dns::delete_account(&self.db, id)?;
        self.record_dns(&account, "dns.account_remove", &account.name, None, &Ok(()));
        Ok(())
    }

    /// The account's client, reading its token (through the Touch ID gate) the first time.
    async fn dns_client(&self, id: &str) -> Result<(DnsAccount, Arc<dns::Cloudflare>)> {
        let account = dns::get_account(&self.db, id)?;
        if let Some(c) = self.dns_clients.lock().await.get(id) {
            return Ok((account, c.clone()));
        }
        let key = dns::token_key(id);
        let _unlock = if self.secrets.gate_enabled() && self.secrets.exists(&key)? {
            self.verify_presence(format!("use the {} DNS token \"{}\"", account.kind.label(), account.name))
                .await?;
            Some(self.secrets.unlock())
        } else {
            None
        };
        let token = self
            .secrets
            .get(&key)?
            .ok_or_else(|| Error::InvalidInput("this account's token is missing; remove the account and add it again".into()))?;
        let client = Arc::new(dns::Cloudflare::new(&self.cloudflare_base(), token)?);
        self.dns_clients.lock().await.insert(id.to_string(), client.clone());
        Ok((account, client))
    }

    pub async fn dns_zones(&self, account_id: &str) -> Result<Vec<DnsZone>> {
        self.dns_client(account_id).await?.1.zones().await
    }

    pub async fn dns_records(&self, account_id: &str, zone_id: &str) -> Result<Vec<DnsRecord>> {
        self.dns_client(account_id).await?.1.records(zone_id).await
    }

    fn describe(r: &RecordInput) -> String {
        let ttl = if r.ttl == 1 { "auto".to_string() } else { format!("{}s", r.ttl) };
        let prio = r.priority.map(|p| format!("{p} ")).unwrap_or_default();
        let proxy = if r.proxied { ", proxied" } else { "" };
        format!("{} {} {prio}{} (ttl {ttl}{proxy})", r.rtype.as_str(), r.name, r.content)
    }

    pub async fn create_dns_record(&self, account_id: &str, zone_id: &str, zone: &str, input: &RecordInput) -> Result<DnsRecord> {
        let (account, c) = self.dns_client(account_id).await?;
        let r = dns::validate(input, zone)?;
        let out = c.create(zone_id, &r).await;
        self.record_dns(&account, "dns.record_create", &r.name, Some(Self::describe(&r)), &out);
        out
    }

    pub async fn update_dns_record(
        &self,
        account_id: &str,
        zone_id: &str,
        zone: &str,
        record_id: &str,
        input: &RecordInput,
    ) -> Result<DnsRecord> {
        let (account, c) = self.dns_client(account_id).await?;
        let r = dns::validate(input, zone)?;
        let out = c.update(zone_id, record_id, &r).await;
        self.record_dns(&account, "dns.record_update", &r.name, Some(Self::describe(&r)), &out);
        out
    }

    /// `label` (type and name) is only for the activity log.
    pub async fn delete_dns_record(&self, account_id: &str, zone_id: &str, record_id: &str, label: &str) -> Result<()> {
        let (account, c) = self.dns_client(account_id).await?;
        let out = c.delete(zone_id, record_id).await;
        self.record_dns(&account, "dns.record_delete", label, None, &out);
        out
    }

    /// What the public resolvers answer for `name`, compared with `expected`
    /// (all of it must be there; empty means any answer).
    pub async fn dns_check(&self, name: &str, rtype: RecordType, expected: &[String]) -> Result<DnsCheck> {
        let resolvers = self.dns_endpoints.read().unwrap_or_else(|p| p.into_inner()).1.clone();
        dns::check(&dns::http_client()?, &resolvers, name, rtype, expected, true).await
    }

    /// The server's addresses that the internet can reach: public addresses on
    /// its interfaces, and the address Orbital connects to when that is public.
    pub async fn server_public_addresses(&self, sid: &str) -> Result<Vec<String>> {
        let s = self.session(sid).await?;
        let mut out: Vec<std::net::IpAddr> = Vec::new();
        if let Ok(ip) = s.profile.host.parse::<std::net::IpAddr>() {
            out.push(ip);
        }
        if let Ok(info) = s.system_info().await {
            for a in info.addresses {
                if let Ok(ip) = a.address.split('/').next().unwrap_or_default().parse() {
                    out.push(ip);
                }
            }
        }
        let mut seen = HashSet::new();
        Ok(out
            .into_iter()
            .filter(dns::is_public)
            .filter(|ip| seen.insert(*ip))
            .map(|ip| ip.to_string())
            .collect())
    }

    /// For each name: does it resolve to this server? (Before asking Let's Encrypt.)
    /// A names are checked when the server has a public IPv4 address, else AAAA.
    pub async fn dns_points_here(&self, sid: &str, names: &[String]) -> Result<(Vec<String>, Vec<DnsCheck>)> {
        let addrs = self.server_public_addresses(sid).await?;
        let v4: Vec<String> = addrs.iter().filter(|a| !a.contains(':')).cloned().collect();
        let (rtype, want) = if v4.is_empty() {
            (RecordType::AAAA, addrs.clone())
        } else {
            (RecordType::A, v4)
        };
        let resolvers = self.dns_endpoints.read().unwrap_or_else(|p| p.into_inner()).1.clone();
        let http = dns::http_client()?;
        let mut checks = Vec::new();
        if want.is_empty() {
            return Ok((addrs, checks));
        }
        for n in names.iter().take(20) {
            checks.push(dns::check(&http, &resolvers, n, rtype, &want, false).await?);
        }
        Ok((addrs, checks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditFilter;
    use crate::profiles::{AuthMethod, SudoMode};
    use crate::secrets::MemoryStore;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct FakePresence {
        approve: AtomicBool,
        asked: AtomicUsize,
    }

    impl UserPresence for FakePresence {
        fn available(&self) -> bool {
            true
        }
        fn verify(&self, _reason: &str) -> Result<()> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            if self.approve.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(Error::PresenceRequired("cancelled".into()))
            }
        }
    }

    fn input(auth: AuthMethod) -> ProfileInput {
        ProfileInput {
            name: "t".into(),
            host: "127.0.0.1".into(),
            port: Some(1), // nothing listens here: connects fail fast, offline
            username: "deploy".into(),
            auth_method: auth,
            key_path: None,
            sudo_mode: Some(SudoMode::Never),
            group_name: None,
            color: None,
            initial_path: None,
            jump_profile_id: None,
            protocol: Default::default(),
            region: None,
            path_style: false,
        }
    }

    fn quick() -> ConnectOptions {
        ConnectOptions {
            timeout: Some(Duration::from_secs(3)),
            ..Default::default()
        }
    }

    fn outcomes(e: &Engine, action: &str) -> Vec<Outcome> {
        let mut evs = e.audit_query(&AuditFilter::default()).unwrap();
        evs.reverse();
        evs.into_iter().filter(|ev| ev.action == action).map(|ev| ev.outcome).collect()
    }

    #[test]
    fn ssh_config_import_builds_routes_and_skips_saved_hosts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        let cfg = dir.path().join(".ssh/config");
        std::fs::write(
            &cfg,
            "Host bastion\n  HostName bastion.example.com\n  User ops\n  IdentityFile ~/.ssh/id_ed25519\n\
             Host db\n  HostName 10.0.4.21\n  ProxyJump bastion,admin@10.0.0.5:2200\n\
             Host old\n  HostName 203.0.113.10\n  User deploy\n\
             Host loop\n  ProxyJump loop\n",
        )
        .unwrap();
        let ctx = ssh_config::Context {
            home: dir.path().into(),
            user: "me".into(),
        };
        let engine = Engine::new(Arc::new(Database::in_memory().unwrap()), Arc::new(MemoryStore::default()));
        let saved = engine
            .create_profile(ProfileInput {
                host: "203.0.113.10".into(),
                port: Some(22),
                ..input(AuthMethod::Agent)
            })
            .unwrap();

        let preview = engine.ssh_config_preview_at(&cfg, &ctx).unwrap();
        let old = preview.candidates.iter().find(|c| c.name == "old").unwrap();
        assert_eq!(old.existing_id.as_deref(), Some(saved.id.as_str()));

        // Choosing only `db` brings its two jump hosts along, outermost first.
        let key = |n: &str| preview.candidates.iter().find(|c| c.name == n).unwrap().key.clone();
        let report = engine
            .import_ssh_config_at(&cfg, &ctx, &[key("db"), key("old"), key("loop"), "gone".into()])
            .unwrap();
        let names: Vec<&str> = report.created.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["bastion", "admin@10.0.0.5:2200", "db"]);
        let db = report.created.iter().find(|p| p.name == "db").unwrap();
        let route = profiles::chain(&engine.db, &db.id).unwrap();
        let hosts: Vec<(&str, u16, &str)> = route.iter().map(|p| (p.host.as_str(), p.port, p.username.as_str())).collect();
        assert_eq!(
            hosts,
            [
                ("bastion.example.com", 22, "ops"),
                ("10.0.0.5", 2200, "admin"),
                ("10.0.4.21", 22, "me")
            ]
        );
        assert_eq!(route[0].auth_method, AuthMethod::Key);
        assert_eq!(route[0].key_path.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(route[2].auth_method, AuthMethod::Agent);
        // The saved host is left alone; the loop and the unknown key fail with a reason.
        let failed: Vec<&str> = report.failed.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(failed, ["loop", "gone"]);
        assert!(report.failed[0].error.contains("loop"));
        assert_eq!(engine.list_profiles().unwrap().len(), 4);
        assert_eq!(outcomes(&engine, "profile.import").len(), 3);

        // Importing again finds everything saved and creates nothing.
        let again = engine.ssh_config_preview_at(&cfg, &ctx).unwrap();
        assert!(again.candidates.iter().filter(|c| c.importable).all(|c| c.existing_id.is_some()));
        let report = engine.import_ssh_config_at(&cfg, &ctx, &[key("db")]).unwrap();
        assert!(report.created.is_empty() && report.failed.is_empty());
    }

    #[tokio::test]
    async fn gate_requires_the_user_before_saved_credentials_are_used() {
        let presence = Arc::new(FakePresence::default());
        let engine = Engine::with_presence(
            Arc::new(Database::in_memory().unwrap()),
            Arc::new(MemoryStore::default()),
            presence.clone(),
        );
        let p = engine.create_profile(input(AuthMethod::Password)).unwrap();
        engine.set_secret(&p.id, SecretKind::Password, Some("hunter2")).unwrap();
        let key = secret_key(&p.id, SecretKind::Password);

        // Turning the gate on needs the user; a cancelled prompt changes nothing.
        assert!(matches!(engine.set_presence_gate(true).await, Err(Error::PresenceRequired(_))));
        assert!(!engine.presence_status().enabled);
        presence.approve.store(true, Ordering::SeqCst);
        assert!(engine.set_presence_gate(true).await.unwrap().enabled);
        assert_eq!(outcomes(&engine, "presence_gate.enable"), [Outcome::Denied, Outcome::Success]);

        // User cancels: refused before any network traffic, secret stays locked.
        presence.approve.store(false, Ordering::SeqCst);
        let asked = presence.asked.load(Ordering::SeqCst);
        let r = engine.connect(&p.id, quick()).await;
        assert!(matches!(r, Err(Error::PresenceRequired(_))), "{:?}", r.map(|s| s.label));
        assert_eq!(presence.asked.load(Ordering::SeqCst), asked + 1);
        assert!(matches!(engine.secrets.get(&key), Err(Error::PresenceRequired(_))));
        let log = engine.audit_query(&AuditFilter::default()).unwrap();
        assert!(log
            .iter()
            .any(|ev| ev.action == "session.open" && ev.outcome == Outcome::Denied && ev.category == Category::Security));

        // User approves: the connect proceeds (and fails on the network here),
        // and the credentials are locked again afterwards.
        presence.approve.store(true, Ordering::SeqCst);
        let r = engine.connect(&p.id, quick()).await;
        assert!(matches!(r, Err(Error::Ssh(_)) | Err(Error::Io(_))), "{:?}", r.map(|s| s.label));
        assert_eq!(presence.asked.load(Ordering::SeqCst), asked + 2);
        assert!(matches!(engine.secrets.get(&key), Err(Error::PresenceRequired(_))));

        // A typed password doesn't touch saved credentials: no prompt.
        let typed = ConnectOptions {
            secret: Some("typed".into()),
            ..quick()
        };
        let _ = engine.connect(&p.id, typed).await;
        assert_eq!(presence.asked.load(Ordering::SeqCst), asked + 2);
        // Neither does agent auth.
        let agent = engine.create_profile(input(AuthMethod::Agent)).unwrap();
        let _ = engine.connect(&agent.id, quick()).await;
        assert_eq!(presence.asked.load(Ordering::SeqCst), asked + 2);

        // A saved credential on a jump host is gated just like the target's.
        let via = engine
            .create_profile(ProfileInput {
                jump_profile_id: Some(p.id.clone()),
                ..input(AuthMethod::Agent)
            })
            .unwrap();
        presence.approve.store(false, Ordering::SeqCst);
        assert!(matches!(engine.connect(&via.id, quick()).await, Err(Error::PresenceRequired(_))));

        // Turning the gate off needs the user as well.
        assert!(matches!(engine.set_presence_gate(false).await, Err(Error::PresenceRequired(_))));
        assert!(engine.presence_status().enabled);
        presence.approve.store(true, Ordering::SeqCst);
        assert!(!engine.set_presence_gate(false).await.unwrap().enabled);
        assert_eq!(engine.secrets.get(&key).unwrap().unwrap().as_str(), "hunter2");

        // Nothing secret reached the audit log.
        let dump = engine.audit_export("json").unwrap();
        assert!(!dump.contains("hunter2") && !dump.contains("typed"));
    }

    #[tokio::test]
    async fn gate_cannot_be_enabled_without_a_way_to_verify() {
        let engine = Engine::new(Arc::new(Database::in_memory().unwrap()), Arc::new(MemoryStore::default()));
        assert!(!engine.presence_status().available);
        assert!(engine.set_presence_gate(true).await.is_err());
        assert!(!engine.presence_status().enabled);
    }

    #[tokio::test]
    async fn jump_host_in_use_is_not_deleted_and_keeps_its_credentials() {
        let engine = Engine::new(Arc::new(Database::in_memory().unwrap()), Arc::new(MemoryStore::default()));
        let bastion = engine.create_profile(input(AuthMethod::Password)).unwrap();
        engine.set_secret(&bastion.id, SecretKind::Password, Some("pw")).unwrap();
        let web = engine
            .create_profile(ProfileInput {
                jump_profile_id: Some(bastion.id.clone()),
                ..input(AuthMethod::Agent)
            })
            .unwrap();
        assert!(matches!(engine.delete_profile(&bastion.id), Err(Error::InvalidInput(_))));
        assert!(engine.has_secret(&bastion.id, SecretKind::Password).unwrap());
        engine.delete_profile(&web.id).unwrap();
        engine.delete_profile(&bastion.id).unwrap();
        assert!(!engine.has_secret(&bastion.id, SecretKind::Password).unwrap());
    }
}
