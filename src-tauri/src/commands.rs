use base64::Engine as _;
use orbital_core::audit::{AuditEvent, AuditFilter, VerifyReport};
use orbital_core::certbot::{CertbotStatus, IssueRequest, RenewMode};
use orbital_core::config_edit::{SafeWriteReport, Validator};
use orbital_core::cron::{CronChange, CronPreview, CronReport};
use orbital_core::databases::{BackupFormat, DbKind, DbServer, RedisAction, RedisInfo};
use orbital_core::deploy::{App, AppInput, Checkout, DeployRecord};
use orbital_core::docker::{ComposeAction, ComposeTarget, Container, ContainerAction, ContainerStats};
use orbital_core::engine::{DeployEvent, Engine, PresenceStatus, SessionInfo};
use orbital_core::fail2ban::Fail2banStatus;
use orbital_core::firewall::{UfwRuleInput, UfwRuleRef, UfwStatus};
use orbital_core::known_hosts::KnownHost;
use orbital_core::logs::{LogFile, LogSource};
use orbital_core::metrics::Metrics;
use orbital_core::nginx::NginxConfig;
use orbital_core::ports::PortsReport;
use orbital_core::profiles::{Profile, ProfileInput};
use orbital_core::secrets::SecretKind;
use orbital_core::services::{ActionReport, Service, ServiceAction, ServiceDetail};
use orbital_core::sftp::{FileContent, FileEntry, TransferReport};
use orbital_core::ssh::{ConnectOptions, ExecOutput};
use orbital_core::sshd::SshdReport;
use orbital_core::streams::StreamEvent;
use orbital_core::sysinfo::SystemInfo;
use orbital_core::terminal::TermEvent;
use orbital_core::tools::DetectedTool;
use orbital_core::tunnels::TunnelInfo;
use orbital_core::uptime::{Check, CheckInput, CheckStatus, UptimeEvent};
use orbital_core::usage::{DiskUsage, ProcessReport};
use orbital_core::users::{NewUser, UserChange, UsersReport};
use orbital_core::webservers::{ApacheStatus, HaServerState, HaproxyStatus};
use orbital_core::Error;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::State;

type Eng<'a> = State<'a, Arc<Engine>>;
type R<T> = Result<T, Error>;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKindArg {
    Password,
    KeyPassphrase,
    SudoPassword,
}

impl From<SecretKindArg> for SecretKind {
    fn from(k: SecretKindArg) -> Self {
        match k {
            SecretKindArg::Password => SecretKind::Password,
            SecretKindArg::KeyPassphrase => SecretKind::KeyPassphrase,
            SecretKindArg::SudoPassword => SecretKind::SudoPassword,
        }
    }
}

// ---------- profiles ----------

#[tauri::command]
pub fn list_profiles(engine: Eng) -> R<Vec<Profile>> {
    engine.list_profiles()
}

#[tauri::command]
pub fn create_profile(engine: Eng, input: ProfileInput) -> R<Profile> {
    engine.create_profile(input)
}

#[tauri::command]
pub fn update_profile(engine: Eng, id: String, input: ProfileInput) -> R<Profile> {
    engine.update_profile(&id, input)
}

#[tauri::command]
pub fn delete_profile(engine: Eng, id: String) -> R<()> {
    engine.delete_profile(&id)
}

#[tauri::command]
pub fn set_secret(engine: Eng, profile_id: String, kind: SecretKindArg, value: Option<String>) -> R<()> {
    engine.set_secret(&profile_id, kind.into(), value.as_deref())
}

#[tauri::command]
pub fn has_secret(engine: Eng, profile_id: String, kind: SecretKindArg) -> R<bool> {
    engine.has_secret(&profile_id, kind.into())
}

#[tauri::command]
pub fn known_hosts(engine: Eng) -> R<Vec<KnownHost>> {
    engine.known_hosts()
}

#[tauri::command]
pub fn forget_host(engine: Eng, host: String, port: u16) -> R<()> {
    engine.forget_host(&host, port)
}

#[tauri::command]
pub fn presence_status(engine: Eng) -> R<PresenceStatus> {
    Ok(engine.presence_status())
}

#[tauri::command]
pub async fn set_presence_gate(engine: Eng<'_>, enabled: bool) -> R<PresenceStatus> {
    engine.set_presence_gate(enabled).await
}

// ---------- sessions ----------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectArgs {
    accept_fingerprint: Option<String>,
    /// The endpoint the trust dialog showed that fingerprint for.
    accept_host: Option<String>,
    accept_port: Option<u16>,
    secret: Option<String>,
    /// Secrets typed for jump hosts, by profile id.
    hop_secrets: Option<HashMap<String, String>>,
    remember: Option<bool>,
}

#[tauri::command]
pub async fn connect(engine: Eng<'_>, profile_id: String, opts: Option<ConnectArgs>) -> R<SessionInfo> {
    let o = opts.unwrap_or_default();
    engine
        .connect(
            &profile_id,
            ConnectOptions {
                accept_fingerprint: o.accept_fingerprint,
                accept_host: o.accept_host.zip(o.accept_port),
                secret: o.secret,
                hop_secrets: o.hop_secrets.unwrap_or_default(),
                remember: o.remember.unwrap_or(false),
                timeout: None,
            },
        )
        .await
}

#[tauri::command]
pub async fn disconnect(engine: Eng<'_>, session_id: String) -> R<()> {
    engine.disconnect(&session_id).await
}

// ---------- files ----------

#[tauri::command]
pub async fn list_dir(engine: Eng<'_>, session_id: String, path: String) -> R<Vec<FileEntry>> {
    engine.list_dir(&session_id, &path).await
}

#[tauri::command]
pub async fn read_file(engine: Eng<'_>, session_id: String, path: String) -> R<FileContent> {
    engine.read_file(&session_id, &path).await
}

#[tauri::command]
pub async fn write_file(engine: Eng<'_>, session_id: String, path: String, content: String, expected_mtime: Option<u32>) -> R<Option<u32>> {
    engine.write_file(&session_id, &path, &content, expected_mtime).await
}

#[tauri::command]
pub async fn safe_write(
    engine: Eng<'_>,
    session_id: String,
    path: String,
    content: String,
    validate: bool,
    privileged: bool,
    expected_mtime: Option<u32>,
) -> R<SafeWriteReport> {
    engine
        .safe_write(&session_id, &path, &content, validate, privileged, expected_mtime)
        .await
}

#[tauri::command]
pub fn validator_for(engine: Eng, path: String) -> Option<Validator> {
    engine.validator_for(&path)
}

#[tauri::command]
pub async fn list_backups(engine: Eng<'_>, session_id: String, path: String, privileged: bool) -> R<Vec<String>> {
    engine.list_backups(&session_id, &path, privileged).await
}

#[tauri::command]
pub async fn mkdir(engine: Eng<'_>, session_id: String, path: String) -> R<()> {
    engine.mkdir(&session_id, &path).await
}

#[tauri::command]
pub async fn create_file(engine: Eng<'_>, session_id: String, path: String) -> R<()> {
    engine.create_file(&session_id, &path).await
}

#[tauri::command]
pub async fn rename(engine: Eng<'_>, session_id: String, from: String, to: String) -> R<()> {
    engine.rename(&session_id, &from, &to).await
}

#[tauri::command]
pub async fn remove(engine: Eng<'_>, session_id: String, path: String, recursive: bool) -> R<()> {
    engine.remove(&session_id, &path, recursive).await
}

#[tauri::command]
pub async fn chmod(engine: Eng<'_>, session_id: String, path: String, mode: u32) -> R<()> {
    engine.chmod(&session_id, &path, mode).await
}

#[tauri::command]
pub async fn upload(engine: Eng<'_>, session_id: String, local_path: String, remote_path: String) -> R<u64> {
    engine.upload(&session_id, &PathBuf::from(local_path), &remote_path).await
}

#[tauri::command]
pub async fn download(engine: Eng<'_>, session_id: String, remote_path: String, local_path: String) -> R<u64> {
    engine.download(&session_id, &remote_path, &PathBuf::from(local_path)).await
}

#[tauri::command]
pub async fn copy_between(
    engine: Eng<'_>,
    src_session_id: String,
    src_path: String,
    dst_session_id: String,
    dst_path: String,
    overwrite: bool,
) -> R<TransferReport> {
    engine
        .copy_between(&src_session_id, &src_path, &dst_session_id, &dst_path, overwrite)
        .await
}

// ---------- monitoring / services / docker ----------

#[tauri::command]
pub async fn detect_tools(engine: Eng<'_>, session_id: String) -> R<Vec<DetectedTool>> {
    engine.detect_tools(&session_id).await
}

#[tauri::command]
pub async fn nginx_config(engine: Eng<'_>, session_id: String) -> R<NginxConfig> {
    engine.nginx_config(&session_id).await
}

#[tauri::command]
pub async fn compose_action(engine: Eng<'_>, session_id: String, target: ComposeTarget, action: ComposeAction) -> R<ExecOutput> {
    engine.compose_action(&session_id, target, action).await
}

#[tauri::command]
pub async fn disk_usage(engine: Eng<'_>, session_id: String, path: String) -> R<DiskUsage> {
    engine.disk_usage(&session_id, &path).await
}

#[tauri::command]
pub async fn process_usage(engine: Eng<'_>, session_id: String) -> R<ProcessReport> {
    engine.process_usage(&session_id).await
}

#[tauri::command]
pub async fn system_info(engine: Eng<'_>, session_id: String) -> R<SystemInfo> {
    engine.system_info(&session_id).await
}

#[tauri::command]
pub async fn cron_report(engine: Eng<'_>, session_id: String) -> R<CronReport> {
    engine.cron_report(&session_id).await
}

#[tauri::command]
pub async fn cron_change(engine: Eng<'_>, session_id: String, user: String, change: CronChange) -> R<CronReport> {
    engine.cron_change(&session_id, &user, change).await
}

#[tauri::command]
pub async fn cron_run(engine: Eng<'_>, session_id: String, user: String, command: String) -> R<ExecOutput> {
    engine.cron_run(&session_id, &user, &command).await
}

/// Describe a schedule and list its next runs, for the job form's live preview.
#[tauri::command]
pub fn cron_preview(schedule: String, server_now: String) -> R<CronPreview> {
    orbital_core::cron::preview(&schedule, &server_now)
}

#[tauri::command]
pub async fn list_users(engine: Eng<'_>, session_id: String) -> R<UsersReport> {
    engine.list_users(&session_id).await
}

#[tauri::command]
pub async fn create_user(engine: Eng<'_>, session_id: String, user: NewUser, password: Option<String>) -> R<UsersReport> {
    engine.create_user(&session_id, user, password).await
}

#[tauri::command]
pub async fn change_user(
    engine: Eng<'_>,
    session_id: String,
    name: String,
    change: UserChange,
    password: Option<String>,
    force: bool,
) -> R<UsersReport> {
    engine.change_user(&session_id, &name, change, password, force).await
}

#[tauri::command]
pub async fn apache_status(engine: Eng<'_>, session_id: String) -> R<ApacheStatus> {
    engine.apache_status(&session_id).await
}

#[tauri::command]
pub async fn apache_site(engine: Eng<'_>, session_id: String, name: String, enable: bool) -> R<ApacheStatus> {
    engine.apache_site(&session_id, &name, enable).await
}

#[tauri::command]
pub async fn haproxy_status(engine: Eng<'_>, session_id: String) -> R<HaproxyStatus> {
    engine.haproxy_status(&session_id).await
}

#[tauri::command]
pub async fn haproxy_server_state(
    engine: Eng<'_>,
    session_id: String,
    backend: String,
    server: String,
    state: HaServerState,
) -> R<HaproxyStatus> {
    engine.haproxy_server_state(&session_id, &backend, &server, state).await
}

#[tauri::command]
pub async fn list_databases(engine: Eng<'_>, session_id: String, kind: DbKind) -> R<DbServer> {
    engine.list_databases(&session_id, kind).await
}

#[tauri::command]
pub async fn backup_database(
    engine: Eng<'_>,
    session_id: String,
    kind: DbKind,
    name: String,
    format: BackupFormat,
    local_path: String,
) -> R<u64> {
    engine
        .backup_database(&session_id, kind, &name, format, &PathBuf::from(local_path))
        .await
}

#[tauri::command]
pub async fn redis_info(engine: Eng<'_>, session_id: String) -> R<RedisInfo> {
    engine.redis_info(&session_id).await
}

#[tauri::command]
pub async fn redis_action(engine: Eng<'_>, session_id: String, action: RedisAction) -> R<RedisInfo> {
    engine.redis_action(&session_id, action).await
}

#[tauri::command]
pub async fn listening_ports(engine: Eng<'_>, session_id: String) -> R<PortsReport> {
    engine.listening_ports(&session_id).await
}

#[tauri::command]
pub async fn sshd_report(engine: Eng<'_>, session_id: String) -> R<SshdReport> {
    engine.sshd_report(&session_id).await
}

// ---------- firewall, fail2ban, certificates ----------

#[tauri::command]
pub async fn ufw_status(engine: Eng<'_>, session_id: String) -> R<UfwStatus> {
    engine.ufw_status(&session_id).await
}

#[tauri::command]
pub async fn ufw_add(engine: Eng<'_>, session_id: String, rule: UfwRuleInput, force: bool) -> R<UfwStatus> {
    engine.ufw_add(&session_id, rule, force).await
}

#[tauri::command]
pub async fn ufw_update(engine: Eng<'_>, session_id: String, old: Vec<UfwRuleRef>, rule: UfwRuleInput, force: bool) -> R<UfwStatus> {
    engine.ufw_update(&session_id, old, rule, force).await
}

#[tauri::command]
pub async fn ufw_delete(engine: Eng<'_>, session_id: String, rules: Vec<UfwRuleRef>, force: bool) -> R<UfwStatus> {
    engine.ufw_delete(&session_id, rules, force).await
}

#[tauri::command]
pub async fn ufw_set_enabled(engine: Eng<'_>, session_id: String, enabled: bool, force: bool) -> R<UfwStatus> {
    engine.ufw_set_enabled(&session_id, enabled, force).await
}

#[tauri::command]
pub async fn fail2ban_status(engine: Eng<'_>, session_id: String) -> R<Fail2banStatus> {
    engine.fail2ban_status(&session_id).await
}

#[tauri::command]
pub async fn fail2ban_unban(engine: Eng<'_>, session_id: String, jail: Option<String>, ip: String) -> R<Fail2banStatus> {
    engine.fail2ban_unban(&session_id, jail.as_deref(), &ip).await
}

#[tauri::command]
pub async fn fail2ban_ban(engine: Eng<'_>, session_id: String, jail: String, ip: String) -> R<Fail2banStatus> {
    engine.fail2ban_ban(&session_id, &jail, &ip).await
}

#[tauri::command]
pub async fn certbot_status(engine: Eng<'_>, session_id: String) -> R<CertbotStatus> {
    engine.certbot_status(&session_id).await
}

#[tauri::command]
pub async fn certbot_issue(engine: Eng<'_>, session_id: String, request: IssueRequest) -> R<ExecOutput> {
    engine.certbot_issue(&session_id, request).await
}

#[tauri::command]
pub async fn certbot_renew(engine: Eng<'_>, session_id: String, name: Option<String>, mode: RenewMode) -> R<ExecOutput> {
    engine.certbot_renew(&session_id, name.as_deref(), mode).await
}

#[tauri::command]
pub async fn certbot_delete(engine: Eng<'_>, session_id: String, name: String, force: bool) -> R<ExecOutput> {
    engine.certbot_delete(&session_id, &name, force).await
}

#[tauri::command]
pub async fn metrics(engine: Eng<'_>, session_id: String) -> R<Metrics> {
    engine.metrics(&session_id).await
}

#[tauri::command]
pub async fn list_services(engine: Eng<'_>, session_id: String) -> R<Vec<Service>> {
    engine.list_services(&session_id).await
}

#[tauri::command]
pub async fn service_detail(engine: Eng<'_>, session_id: String, unit: String) -> R<ServiceDetail> {
    engine.service_detail(&session_id, &unit).await
}

#[tauri::command]
pub async fn service_action(engine: Eng<'_>, session_id: String, unit: String, action: ServiceAction) -> R<ActionReport> {
    engine.service_action(&session_id, &unit, action).await
}

#[tauri::command]
pub async fn list_containers(engine: Eng<'_>, session_id: String) -> R<Vec<Container>> {
    engine.list_containers(&session_id).await
}

#[tauri::command]
pub async fn container_stats(engine: Eng<'_>, session_id: String) -> R<Vec<ContainerStats>> {
    engine.container_stats(&session_id).await
}

#[tauri::command]
pub async fn inspect_container(engine: Eng<'_>, session_id: String, id: String) -> R<serde_json::Value> {
    engine.inspect_container(&session_id, &id).await
}

#[tauri::command]
pub async fn container_logs(engine: Eng<'_>, session_id: String, id: String, tail: Option<u32>) -> R<String> {
    engine.container_logs(&session_id, &id, tail.unwrap_or(500)).await
}

#[tauri::command]
pub async fn container_action(engine: Eng<'_>, session_id: String, id: String, action: ContainerAction) -> R<ExecOutput> {
    engine.container_action(&session_id, &id, action).await
}

#[tauri::command]
pub async fn run_command(engine: Eng<'_>, session_id: String, command: String, privileged: bool) -> R<ExecOutput> {
    engine.run_command(&session_id, &command, privileged).await
}

// ---------- terminal ----------

// ---------- port forwarding ----------

#[tauri::command]
pub async fn open_tunnel(
    engine: Eng<'_>,
    session_id: String,
    remote_host: String,
    remote_port: u16,
    local_port: Option<u16>,
    label: Option<String>,
) -> R<TunnelInfo> {
    engine.open_tunnel(&session_id, &remote_host, remote_port, local_port, label).await
}

#[tauri::command]
pub async fn close_tunnel(engine: Eng<'_>, id: String) -> R<()> {
    engine.close_tunnel(&id).await
}

#[tauri::command]
pub async fn list_tunnels(engine: Eng<'_>) -> R<Vec<TunnelInfo>> {
    Ok(engine.list_tunnels().await)
}

// ---------- live logs ----------

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LogMessage {
    Data { text: String },
    End { code: Option<i32> },
}

#[tauri::command]
pub async fn log_files(engine: Eng<'_>, session_id: String) -> R<Vec<LogFile>> {
    engine.log_files(&session_id).await
}

#[tauri::command]
pub async fn follow_logs(engine: Eng<'_>, session_id: String, source: LogSource, lines: u32, on_event: Channel<LogMessage>) -> R<String> {
    engine
        .follow_logs(&session_id, source, lines, move |ev| {
            let _ = on_event.send(match ev {
                StreamEvent::Data(d) => LogMessage::Data {
                    text: String::from_utf8_lossy(&d).into_owned(),
                },
                StreamEvent::Exit(code) => LogMessage::End { code },
            });
        })
        .await
}

#[tauri::command]
pub async fn stop_logs(engine: Eng<'_>, id: String) -> R<()> {
    engine.stop_logs(&id).await
}

// ---------- deploys ----------

#[tauri::command]
pub fn list_apps(engine: Eng<'_>, profile_id: String) -> R<Vec<App>> {
    engine.list_apps(&profile_id)
}

#[tauri::command]
pub fn save_app(engine: Eng<'_>, profile_id: String, id: Option<String>, input: AppInput) -> R<App> {
    engine.save_app(&profile_id, id.as_deref(), input)
}

#[tauri::command]
pub fn delete_app(engine: Eng<'_>, id: String) -> R<()> {
    engine.delete_app(&id)
}

#[tauri::command]
pub fn deploy_history(engine: Eng<'_>, app_id: String) -> R<Vec<DeployRecord>> {
    engine.deploy_history(&app_id)
}

#[tauri::command]
pub fn deploy_log(engine: Eng<'_>, run_id: String) -> R<String> {
    engine.deploy_log(&run_id)
}

#[tauri::command]
pub async fn app_checkout(engine: Eng<'_>, session_id: String, app_id: String) -> R<Checkout> {
    engine.app_checkout(&session_id, &app_id).await
}

#[tauri::command]
pub async fn run_deploy(
    engine: Eng<'_>,
    session_id: String,
    app_id: String,
    rollback_to: Option<String>,
    on_event: Channel<DeployEvent>,
) -> R<DeployRecord> {
    engine
        .run_deploy(&session_id, &app_id, rollback_to, move |ev| {
            let _ = on_event.send(ev);
        })
        .await
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TermMessage {
    /// base64-encoded bytes
    Data {
        data: String,
    },
    Exit {
        code: Option<u32>,
    },
}

#[tauri::command]
pub async fn open_terminal(
    engine: Eng<'_>,
    session_id: String,
    cols: u32,
    rows: u32,
    cwd: Option<String>,
    as_root: bool,
    on_event: Channel<TermMessage>,
) -> R<String> {
    engine
        .open_terminal(&session_id, cols, rows, cwd, as_root, move |ev| {
            let msg = match ev {
                TermEvent::Data(d) => TermMessage::Data {
                    data: base64::engine::general_purpose::STANDARD.encode(d),
                },
                TermEvent::Exit(code) => TermMessage::Exit { code },
            };
            let _ = on_event.send(msg);
        })
        .await
}

#[tauri::command]
pub async fn terminal_write(engine: Eng<'_>, terminal_id: String, data: String) -> R<()> {
    engine.terminal_write(&terminal_id, data.as_bytes()).await
}

#[tauri::command]
pub async fn terminal_resize(engine: Eng<'_>, terminal_id: String, cols: u32, rows: u32) -> R<()> {
    engine.terminal_resize(&terminal_id, cols, rows).await
}

#[tauri::command]
pub async fn terminal_close(engine: Eng<'_>, terminal_id: String) -> R<()> {
    engine.terminal_close(&terminal_id).await
}

// ---------- audit ----------

#[tauri::command]
pub fn audit_query(engine: Eng, filter: Option<AuditFilter>) -> R<Vec<AuditEvent>> {
    engine.audit_query(&filter.unwrap_or_default())
}

#[tauri::command]
pub fn audit_verify(engine: Eng) -> R<VerifyReport> {
    engine.audit_verify()
}

#[tauri::command]
pub fn audit_export(engine: Eng, format: String, path: String) -> R<()> {
    std::fs::write(path, engine.audit_export(&format)?)?;
    Ok(())
}

// ---------- uptime checks ----------

/// The page's channel for uptime events (results and alerts); replaced when the page reloads.
#[derive(Default)]
pub struct UptimeWatch(pub std::sync::Mutex<Option<Channel<UptimeEvent>>>);

#[tauri::command]
pub fn uptime_watch(watch: State<'_, UptimeWatch>, on_event: Channel<UptimeEvent>) {
    *watch.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(on_event);
}

#[tauri::command]
pub fn list_uptime(engine: Eng<'_>) -> R<Vec<CheckStatus>> {
    engine.list_uptime()
}

#[tauri::command]
pub fn save_uptime_check(engine: Eng<'_>, id: Option<String>, input: CheckInput) -> R<Check> {
    engine.save_uptime_check(id.as_deref(), input)
}

#[tauri::command]
pub fn set_uptime_paused(engine: Eng<'_>, id: String, paused: bool) -> R<Check> {
    engine.set_uptime_paused(&id, paused)
}

#[tauri::command]
pub fn delete_uptime_check(engine: Eng<'_>, id: String) -> R<()> {
    engine.delete_uptime_check(&id)
}

#[tauri::command]
pub async fn run_uptime_check(engine: Eng<'_>, id: String) -> R<CheckStatus> {
    engine.run_uptime_check(&id).await
}
