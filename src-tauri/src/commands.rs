use base64::Engine as _;
use orbital_core::audit::{AuditEvent, AuditFilter, VerifyReport};
use orbital_core::certbot::{CertbotStatus, IssueRequest, RenewMode};
use orbital_core::config_edit::{SafeWriteReport, Validator};
use orbital_core::cron::{CronChange, CronPreview, CronReport};
use orbital_core::databases::{BackupFormat, DbKind, DbServer, RedisAction, RedisInfo};
use orbital_core::deploy::{App, AppInput, Checkout, DeployRecord};
use orbital_core::dns::{DnsAccount, DnsCheck, DnsRecord, DnsZone, ProviderKind, RecordInput, RecordType};
use orbital_core::docker::{ComposeAction, ComposeTarget, Container, ContainerAction, ContainerStats};
use orbital_core::engine::{DeployEvent, Engine, ImportReport, PresenceStatus, SessionInfo};
use orbital_core::fail2ban::Fail2banStatus;
use orbital_core::firewall::{UfwRuleInput, UfwRuleRef, UfwStatus};
use orbital_core::known_hosts::KnownHost;
use orbital_core::logs::{LogFile, LogSource};
use orbital_core::metrics::Metrics;
use orbital_core::nginx::NginxConfig;
use orbital_core::ports::PortsReport;
use orbital_core::profiles::{Profile, ProfileInput};
use orbital_core::progress::{self, ProgressEvent};
use orbital_core::secrets::SecretKind;
use orbital_core::services::{ActionReport, Service, ServiceAction, ServiceDetail};
use orbital_core::sftp::{FileContent, FileEntry, TransferReport};
use orbital_core::snippets::{Snippet, SnippetInput, SnippetRun};
use orbital_core::ssh::{ConnectOptions, ExecOutput};
use orbital_core::ssh_config::Preview;
use orbital_core::sshd::SshdReport;
use orbital_core::sshkeys::{AuthorizedKeys, LocalKey};
use orbital_core::streams::StreamEvent;
use orbital_core::sysinfo::SystemInfo;
use orbital_core::terminal::TermEvent;
use orbital_core::tools::DetectedTool;
use orbital_core::tunnels::TunnelInfo;
use orbital_core::uptime::{Check, CheckInput, CheckStatus, UptimeEvent};
use orbital_core::usage::{DiskUsage, ProcessReport};
use orbital_core::users::{NewUser, UserChange, UsersReport};
use orbital_core::webservers::{ApacheStatus, HaServerState, HaproxyStatus};
use orbital_core::website::{WebsiteInput, WebsitePlan, WebsiteReport};
use orbital_core::Error;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::Channel;

/// A job's progress channel as the engine's sink (see `orbital_core::progress`).
/// The UI passes one for every long operation and shows it when it wants to.
fn sink(ch: Channel<ProgressEvent>) -> Option<progress::Sink> {
    Some(std::sync::Arc::new(move |ev| {
        let _ = ch.send(ev);
    }))
}
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
pub fn ssh_config_preview(engine: Eng, path: Option<String>) -> R<Preview> {
    engine.ssh_config_preview(path.as_deref())
}

#[tauri::command]
pub fn import_ssh_config(engine: Eng, path: Option<String>, keys: Vec<String>) -> R<ImportReport> {
    engine.import_ssh_config(path.as_deref(), &keys)
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
pub async fn read_backup(engine: Eng<'_>, session_id: String, path: String, backup: String, privileged: bool) -> R<String> {
    engine.read_backup(&session_id, &path, &backup, privileged).await
}

#[tauri::command]
pub async fn restore_backup(engine: Eng<'_>, session_id: String, path: String, backup: String, privileged: bool) -> R<SafeWriteReport> {
    engine.restore_backup(&session_id, &path, &backup, privileged).await
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
pub async fn nginx_routes(engine: Eng<'_>, session_id: String, file: String, line: usize) -> R<orbital_core::routes::RoutesView> {
    engine.nginx_routes(&session_id, &file, line).await
}

#[tauri::command]
pub async fn nginx_route_change(
    engine: Eng<'_>,
    session_id: String,
    file: String,
    line: usize,
    change: orbital_core::routes::RouteChange,
    apply: bool,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::routes::RouteChangeReport> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.nginx_route_change(&session_id, &file, line, &change, apply),
    )
    .await
}

#[tauri::command]
pub async fn logins(engine: Eng<'_>, session_id: String) -> R<orbital_core::logins::LoginsReport> {
    engine.logins(&session_id).await
}

#[tauri::command]
pub async fn search_logs(
    engine: Eng<'_>,
    session_id: String,
    term: String,
    hours: u32,
    source: orbital_core::logsearch::LogSource,
) -> R<orbital_core::logsearch::LogSearch> {
    engine.search_logs(&session_id, &term, hours, source).await
}

#[tauri::command]
pub async fn stacks(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::stacks::Stack>> {
    engine.stacks(&session_id).await
}

#[tauri::command]
pub async fn create_stack(
    engine: Eng<'_>,
    session_id: String,
    name: String,
    content: String,
    deploy: bool,
    progress: Channel<ProgressEvent>,
) -> R<Option<ExecOutput>> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.create_stack(&session_id, &name, &content, deploy),
    )
    .await
}

#[tauri::command]
pub fn app_options() -> Vec<orbital_core::apps::AppOption> {
    orbital_core::apps::options()
}

#[tauri::command]
pub async fn install_app(
    engine: Eng<'_>,
    session_id: String,
    app: String,
    name: String,
    port: u16,
    force: bool,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::apps::Installed> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.install_app(&session_id, &app, &name, port, force),
    )
    .await
}

#[tauri::command]
pub async fn stack_action(
    engine: Eng<'_>,
    session_id: String,
    name: String,
    file: String,
    action: orbital_core::stacks::StackAction,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.stack_action(&session_id, &name, &file, action),
    )
    .await
}

#[tauri::command]
pub async fn add_swap(engine: Eng<'_>, session_id: String, mb: u32, progress: Channel<ProgressEvent>) -> R<u64> {
    progress::run_job(progress.id(), sink(progress), engine.add_swap(&session_id, mb)).await
}

#[tauri::command]
pub async fn create_service(
    engine: Eng<'_>,
    session_id: String,
    spec: orbital_core::units::ServiceSpec,
    progress: Channel<ProgressEvent>,
) -> R<(bool, String)> {
    progress::run_job(progress.id(), sink(progress), engine.create_service(&session_id, &spec)).await
}

#[tauri::command]
pub async fn remove_service(engine: Eng<'_>, session_id: String, name: String) -> R<()> {
    engine.remove_service(&session_id, &name).await
}

#[tauri::command]
pub async fn backup_jobs(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::backups::BackupStatus>> {
    engine.backup_jobs(&session_id).await
}

#[tauri::command]
pub async fn save_backup_job(
    engine: Eng<'_>,
    session_id: String,
    job: orbital_core::backups::BackupJob,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::backups::BackupJob> {
    progress::run_job(progress.id(), sink(progress), engine.save_backup_job(&session_id, &job)).await
}

#[tauri::command]
pub async fn delete_backup_job(engine: Eng<'_>, session_id: String, id: String, files: bool) -> R<()> {
    engine.delete_backup_job(&session_id, &id, files).await
}

#[tauri::command]
pub async fn run_backup(engine: Eng<'_>, session_id: String, id: String, progress: Channel<ProgressEvent>) -> R<String> {
    progress::run_job(progress.id(), sink(progress), engine.run_backup(&session_id, &id)).await
}

#[tauri::command]
pub async fn restore_scheduled_backup(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    file: String,
    to: orbital_core::backups::RestoreTo,
    progress: Channel<ProgressEvent>,
) -> R<String> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.restore_scheduled_backup(&session_id, &id, &file, to),
    )
    .await
}

#[tauri::command]
pub async fn failed_units(engine: Eng<'_>, session_id: String) -> R<Vec<String>> {
    engine.failed_units(&session_id).await
}

#[tauri::command]
pub async fn copy_within(engine: Eng<'_>, session_id: String, from: String, to: String, progress: Channel<ProgressEvent>) -> R<()> {
    progress::run_job(progress.id(), sink(progress), engine.copy_within(&session_id, &from, &to)).await
}

/// What the system clipboard holds that Files can paste: copied files, text, or an image.
#[tauri::command]
pub fn clipboard_read() -> crate::clipboard::ClipboardContents {
    crate::clipboard::read()
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
pub async fn upload(
    engine: Eng<'_>,
    session_id: String,
    local_path: String,
    remote_path: String,
    progress: Channel<ProgressEvent>,
) -> R<u64> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.upload(&session_id, &PathBuf::from(local_path), &remote_path),
    )
    .await
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
    progress: Channel<ProgressEvent>,
) -> R<TransferReport> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.copy_between(&src_session_id, &src_path, &dst_session_id, &dst_path, overwrite),
    )
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
pub async fn website_plan(engine: Eng<'_>, session_id: String, input: WebsiteInput) -> R<WebsitePlan> {
    engine.website_plan(&session_id, &input).await
}

#[tauri::command]
pub async fn create_website(
    engine: Eng<'_>,
    session_id: String,
    input: WebsiteInput,
    progress: Channel<ProgressEvent>,
) -> R<WebsiteReport> {
    progress::run_job(progress.id(), sink(progress), engine.create_website(&session_id, &input)).await
}

#[tauri::command]
pub async fn compose_action(
    engine: Eng<'_>,
    session_id: String,
    target: ComposeTarget,
    action: ComposeAction,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(progress.id(), sink(progress), engine.compose_action(&session_id, target, action)).await
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
pub async fn apache_status(engine: Eng<'_>, session_id: String, container: Option<String>) -> R<ApacheStatus> {
    engine.apache_status(&session_id, container.as_deref()).await
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
pub async fn list_databases(engine: Eng<'_>, session_id: String, kind: DbKind, container: Option<String>) -> R<DbServer> {
    engine.list_databases(&session_id, kind, container.as_deref()).await
}

#[tauri::command]
pub async fn backup_database(
    engine: Eng<'_>,
    session_id: String,
    kind: DbKind,
    name: String,
    format: BackupFormat,
    local_path: String,
    progress: Channel<ProgressEvent>,
) -> R<u64> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.backup_database(&session_id, kind, &name, format, &PathBuf::from(local_path)),
    )
    .await
}

#[tauri::command]
pub async fn redis_info(engine: Eng<'_>, session_id: String, container: Option<String>) -> R<RedisInfo> {
    engine.redis_info(&session_id, container.as_deref()).await
}

#[tauri::command]
pub async fn redis_action(engine: Eng<'_>, session_id: String, action: RedisAction, container: Option<String>) -> R<RedisInfo> {
    engine.redis_action(&session_id, action, container.as_deref()).await
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
pub async fn fail2ban_protection(engine: Eng<'_>, session_id: String) -> R<Option<orbital_core::fail2ban::SshProtection>> {
    engine.fail2ban_protection(&session_id).await
}

#[tauri::command]
pub async fn fail2ban_protect(
    engine: Eng<'_>,
    session_id: String,
    protection: orbital_core::fail2ban::SshProtection,
    force: bool,
    progress: Channel<ProgressEvent>,
) -> R<Fail2banStatus> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.fail2ban_protect(&session_id, &protection, force),
    )
    .await
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
pub async fn certbot_issue(engine: Eng<'_>, session_id: String, request: IssueRequest, progress: Channel<ProgressEvent>) -> R<ExecOutput> {
    progress::run_job(progress.id(), sink(progress), engine.certbot_issue(&session_id, request)).await
}

#[tauri::command]
pub async fn certbot_renew(
    engine: Eng<'_>,
    session_id: String,
    name: Option<String>,
    mode: RenewMode,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.certbot_renew(&session_id, name.as_deref(), mode),
    )
    .await
}

#[tauri::command]
pub async fn certbot_delete(
    engine: Eng<'_>,
    session_id: String,
    name: String,
    force: bool,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(progress.id(), sink(progress), engine.certbot_delete(&session_id, &name, force)).await
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
pub async fn net_check(
    engine: Eng<'_>,
    session_id: String,
    check: orbital_core::netcheck::NetCheck,
    target: String,
    port: Option<u16>,
) -> R<orbital_core::netcheck::NetResult> {
    engine.net_check(&session_id, check, &target, port).await
}

#[tauri::command]
pub async fn unit_logs(
    engine: Eng<'_>,
    session_id: String,
    unit: String,
    tail: Option<u32>,
    since: Option<i64>,
    until: Option<i64>,
) -> R<String> {
    engine.unit_logs(&session_id, &unit, tail.unwrap_or(400), since, until).await
}

#[tauri::command]
pub async fn service_detail(engine: Eng<'_>, session_id: String, unit: String) -> R<ServiceDetail> {
    engine.service_detail(&session_id, &unit).await
}

#[tauri::command]
pub async fn service_action(
    engine: Eng<'_>,
    session_id: String,
    unit: String,
    action: ServiceAction,
    progress: Channel<ProgressEvent>,
) -> R<ActionReport> {
    progress::run_job(progress.id(), sink(progress), engine.service_action(&session_id, &unit, action)).await
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
pub async fn container_logs(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    tail: Option<u32>,
    since: Option<String>,
    until: Option<String>,
) -> R<String> {
    engine
        .container_logs(&session_id, &id, tail.unwrap_or(500), since.as_deref(), until.as_deref())
        .await
}

#[tauri::command]
pub async fn container_action(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    action: ContainerAction,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(progress.id(), sink(progress), engine.container_action(&session_id, &id, action)).await
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
#[allow(clippy::too_many_arguments)]
pub async fn open_terminal(
    engine: Eng<'_>,
    session_id: String,
    cols: u32,
    rows: u32,
    cwd: Option<String>,
    as_root: bool,
    launch: Option<orbital_core::terminal::Launch>,
    record: Option<bool>,
    on_event: Channel<TermMessage>,
) -> R<String> {
    let sink = move |ev| {
        let msg = match ev {
            TermEvent::Data(d) => TermMessage::Data {
                data: base64::engine::general_purpose::STANDARD.encode(d),
            },
            TermEvent::Exit(code) => TermMessage::Exit { code },
        };
        let _ = on_event.send(msg);
    };
    let launch = launch.unwrap_or_default();
    if record.unwrap_or(false) {
        engine
            .open_recorded_terminal(&session_id, cols, rows, cwd, as_root, launch, sink)
            .await
    } else {
        engine.open_terminal(&session_id, cols, rows, cwd, as_root, launch, sink).await
    }
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

#[tauri::command]
pub fn stats_state(engine: Eng) -> R<orbital_core::stats::StatsState> {
    engine.stats_state()
}

#[tauri::command]
pub fn set_stats(engine: Eng, enabled: bool) -> R<orbital_core::stats::StatsState> {
    engine.set_stats(enabled)
}

#[tauri::command]
pub async fn send_feedback(engine: Eng<'_>, feedback: orbital_core::stats::Feedback) -> R<()> {
    engine.send_feedback(&feedback).await
}

#[tauri::command]
pub fn feedback_available(engine: Eng) -> bool {
    engine.feedback_available()
}

#[tauri::command]
pub async fn stats_ping(engine: Eng<'_>, event: orbital_core::stats::StatsEvent) -> R<()> {
    engine.stats_ping(event).await;
    Ok(())
}

/// Save text the UI already shows (a log pane) to a file the user picked in the save dialog.
#[tauri::command]
pub fn save_text(path: String, text: String) -> R<()> {
    std::fs::write(path, text)?;
    Ok(())
}

// ---------- DNS ----------

// ---------- monitoring history over a date range ----------

#[tauri::command]
pub fn metric_history(engine: Eng, profile_id: String, from: i64, to: i64, max: u32) -> R<(i64, Vec<orbital_core::metric_store::Point>)> {
    engine.metric_history(&profile_id, from, to, max)
}

#[tauri::command]
pub fn metric_moment(engine: Eng, profile_id: String, t: i64, within: i64) -> R<Option<orbital_core::metric_store::Moment>> {
    engine.metric_moment(&profile_id, t, within)
}

#[tauri::command]
pub fn disk_history(engine: Eng, profile_id: String, from: i64, to: i64, max: u32) -> R<Vec<orbital_core::metric_store::DiskSeries>> {
    engine.disk_history(&profile_id, from, to, max)
}

#[tauri::command]
pub fn metric_events(engine: Eng, profile_id: String, from: i64, to: i64) -> R<Vec<orbital_core::metric_store::Event>> {
    engine.metric_events(&profile_id, from, to)
}

#[tauri::command]
pub fn import_metric_history(engine: Eng, profile_id: String, points: Vec<orbital_core::metric_store::Point>) -> R<usize> {
    engine.import_metric_history(&profile_id, &points)
}

#[tauri::command]
pub fn uptime_range(engine: Eng, from: i64, to: i64, max: u32) -> R<Vec<orbital_core::uptime::RangeSummary>> {
    engine.uptime_range(from, to, max)
}

// ---------- notifications elsewhere (Telegram, email, the AI agent) ----------

#[tauri::command]
pub fn notify_channels(engine: Eng) -> R<Vec<orbital_core::notify::Channel>> {
    engine.notify_channels()
}

#[tauri::command]
pub fn save_notify_channel(engine: Eng, id: Option<String>, input: orbital_core::notify::ChannelInput) -> R<orbital_core::notify::Channel> {
    engine.save_notify_channel(id.as_deref(), &input)
}

#[tauri::command]
pub fn delete_notify_channel(engine: Eng, id: String) -> R<()> {
    engine.delete_notify_channel(&id)
}

#[tauri::command]
pub async fn test_notify_channel(engine: Eng<'_>, id: String) -> R<()> {
    engine.test_notify_channel(&id).await
}

#[tauri::command]
pub async fn telegram_lookup(engine: Eng<'_>, token: String) -> R<orbital_core::notify::TelegramBot> {
    engine.telegram_lookup(&token).await
}

/// A health alert found by the UI, sent on to the channels that take it.
#[tauri::command]
pub async fn send_alert(engine: Eng<'_>, alert: orbital_core::notify::Alert) -> R<Vec<orbital_core::notify::SendResult>> {
    Ok(engine.send_alert(&alert).await)
}

#[tauri::command]
pub fn dns_accounts(engine: Eng) -> R<Vec<DnsAccount>> {
    engine.dns_accounts()
}

#[tauri::command]
pub async fn add_dns_account(engine: Eng<'_>, name: String, kind: ProviderKind, token: String) -> R<DnsAccount> {
    engine.add_dns_account(&name, kind, &token).await
}

#[tauri::command]
pub async fn remove_dns_account(engine: Eng<'_>, id: String) -> R<()> {
    engine.remove_dns_account(&id).await
}

#[tauri::command]
pub async fn dns_zones(engine: Eng<'_>, account_id: String) -> R<Vec<DnsZone>> {
    engine.dns_zones(&account_id).await
}

#[tauri::command]
pub async fn dns_records(engine: Eng<'_>, account_id: String, zone_id: String) -> R<Vec<DnsRecord>> {
    engine.dns_records(&account_id, &zone_id).await
}

#[tauri::command]
pub async fn create_dns_record(engine: Eng<'_>, account_id: String, zone_id: String, zone: String, input: RecordInput) -> R<DnsRecord> {
    engine.create_dns_record(&account_id, &zone_id, &zone, &input).await
}

#[tauri::command]
pub async fn update_dns_record(
    engine: Eng<'_>,
    account_id: String,
    zone_id: String,
    zone: String,
    record_id: String,
    input: RecordInput,
) -> R<DnsRecord> {
    engine.update_dns_record(&account_id, &zone_id, &zone, &record_id, &input).await
}

#[tauri::command]
pub async fn delete_dns_record(engine: Eng<'_>, account_id: String, zone_id: String, record_id: String, label: String) -> R<()> {
    engine.delete_dns_record(&account_id, &zone_id, &record_id, &label).await
}

#[tauri::command]
pub async fn dns_check(engine: Eng<'_>, name: String, rtype: RecordType, expected: Vec<String>) -> R<DnsCheck> {
    engine.dns_check(&name, rtype, &expected).await
}

#[tauri::command]
pub async fn server_public_addresses(engine: Eng<'_>, session_id: String) -> R<Vec<String>> {
    engine.server_public_addresses(&session_id).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointsHere {
    addresses: Vec<String>,
    checks: Vec<DnsCheck>,
}

#[tauri::command]
pub async fn dns_points_here(engine: Eng<'_>, session_id: String, names: Vec<String>) -> R<PointsHere> {
    let (addresses, checks) = engine.dns_points_here(&session_id, &names).await?;
    Ok(PointsHere { addresses, checks })
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

// ---------- SSH keys ----------

#[tauri::command]
pub fn local_ssh_keys(engine: Eng) -> Vec<LocalKey> {
    engine.local_ssh_keys()
}

#[tauri::command]
pub fn generate_ssh_key(engine: Eng, name: String, comment: String, passphrase: Option<String>) -> R<LocalKey> {
    engine.generate_ssh_key(&name, &comment, passphrase.as_deref())
}

#[tauri::command]
pub async fn authorized_keys(engine: Eng<'_>, session_id: String) -> R<AuthorizedKeys> {
    engine.authorized_keys(&session_id).await
}

#[tauri::command]
pub async fn add_authorized_key(engine: Eng<'_>, session_id: String, line: String) -> R<AuthorizedKeys> {
    engine.add_authorized_key(&session_id, &line).await
}

#[tauri::command]
pub async fn remove_authorized_key(engine: Eng<'_>, session_id: String, line: String, force: bool) -> R<AuthorizedKeys> {
    engine.remove_authorized_key(&session_id, &line, force).await
}

// ---------- snippets ----------

#[tauri::command]
pub fn snippets(engine: Eng) -> R<Vec<Snippet>> {
    engine.snippets()
}

#[tauri::command]
pub fn save_snippet(engine: Eng, id: Option<String>, input: SnippetInput) -> R<Snippet> {
    engine.save_snippet(id.as_deref(), &input)
}

#[tauri::command]
pub fn delete_snippet(engine: Eng, id: String) -> R<()> {
    engine.delete_snippet(&id)
}

#[tauri::command]
pub fn add_example_snippets(engine: Eng) -> R<Vec<Snippet>> {
    engine.add_example_snippets()
}

#[tauri::command]
pub async fn run_snippet(
    engine: Eng<'_>,
    id: String,
    session_ids: Vec<String>,
    values: HashMap<String, String>,
    with_sudo: Option<bool>,
) -> R<Vec<SnippetRun>> {
    engine.run_snippet(&id, &session_ids, &values, with_sudo.unwrap_or(false)).await
}

/// A system notification for a job that finished while Orbital was in the
/// background. Text only: the UI never puts command output or secrets in it.
#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("couldn't show a notification: {e}");
    }
}

#[tauri::command]
pub async fn shell_history(engine: Eng<'_>, session_id: String, root: bool) -> R<Vec<String>> {
    engine.shell_history(&session_id, root).await
}

/// Stop a running job (its id is its progress channel's). False if it already finished.
#[tauri::command]
pub fn cancel_job(id: u32) -> bool {
    progress::cancel(id)
}

#[tauri::command]
pub async fn updates(engine: Eng<'_>, session_id: String) -> R<orbital_core::updates::UpdatesReport> {
    engine.updates(&session_id).await
}

#[tauri::command]
pub async fn refresh_updates(
    engine: Eng<'_>,
    session_id: String,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::updates::UpdatesReport> {
    progress::run_job(progress.id(), sink(progress), engine.refresh_updates(&session_id)).await
}

#[tauri::command]
pub async fn install_updates(engine: Eng<'_>, session_id: String, progress: Channel<ProgressEvent>) -> R<ExecOutput> {
    progress::run_job(progress.id(), sink(progress), engine.install_updates(&session_id)).await
}

#[tauri::command]
pub async fn reboot_server(engine: Eng<'_>, session_id: String) -> R<()> {
    engine.reboot(&session_id).await
}

#[tauri::command]
pub async fn tmux_sessions(engine: Eng<'_>, session_id: String, root: bool) -> R<Vec<orbital_core::terminal::TmuxSession>> {
    engine.tmux_sessions(&session_id, root).await
}

#[tauri::command]
pub async fn cleanup_scan(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::cleanup::CleanupItem>> {
    engine.cleanup_scan(&session_id).await
}

#[tauri::command]
pub async fn cleanup_run(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    progress: Channel<ProgressEvent>,
) -> R<Vec<orbital_core::cleanup::CleanupItem>> {
    progress::run_job(progress.id(), sink(progress), engine.cleanup_run(&session_id, &id)).await
}

#[tauri::command]
pub async fn stop_process(engine: Eng<'_>, session_id: String, pid: u32, force: bool) -> R<()> {
    engine.stop_process(&session_id, pid, force).await
}

#[tauri::command]
pub async fn agent_status(engine: Eng<'_>, session_id: String) -> R<orbital_core::agent::AgentStatus> {
    engine.agent_status(&session_id).await
}

#[tauri::command]
pub async fn agent_check(engine: Eng<'_>, setup: orbital_core::agent::AgentSetup) -> R<orbital_core::agent::AgentCheck> {
    engine.agent_check(&setup).await
}

#[tauri::command]
pub async fn agent_install(
    engine: Eng<'_>,
    session_id: String,
    setup: orbital_core::agent::AgentSetup,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::agent::PairCode> {
    progress::run_job(progress.id(), sink(progress), engine.agent_install(&session_id, &setup)).await
}

#[tauri::command]
pub async fn agent_pair_code(engine: Eng<'_>, session_id: String) -> R<orbital_core::agent::PairCode> {
    engine.agent_pair_code(&session_id).await
}

#[tauri::command]
pub async fn agent_update(engine: Eng<'_>, session_id: String, progress: Channel<ProgressEvent>) -> R<()> {
    progress::run_job(progress.id(), sink(progress), engine.agent_update(&session_id)).await
}

#[tauri::command]
pub async fn agent_set_running(engine: Eng<'_>, session_id: String, on: bool) -> R<()> {
    engine.agent_set_running(&session_id, on).await
}

#[tauri::command]
pub async fn agent_set_alerts(engine: Eng<'_>, session_id: String, on: bool) -> R<()> {
    engine.agent_set_alerts(&session_id, on).await
}

#[tauri::command]
pub async fn agent_remove(engine: Eng<'_>, session_id: String) -> R<()> {
    engine.agent_remove(&session_id).await
}

#[tauri::command]
pub async fn agent_models(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::agent::AiModel>> {
    engine.agent_models(&session_id).await
}

#[tauri::command]
pub async fn agent_set_model(engine: Eng<'_>, session_id: String, model: String) -> R<()> {
    engine.agent_set_model(&session_id, &model).await
}

#[tauri::command]
pub async fn agent_test(engine: Eng<'_>, session_id: String) -> R<Option<String>> {
    engine.agent_test(&session_id).await
}

#[tauri::command]
pub async fn agent_set_daily(engine: Eng<'_>, session_id: String, time: Option<String>) -> R<()> {
    engine.agent_set_daily(&session_id, time.as_deref()).await
}

#[tauri::command]
pub async fn find_files(engine: Eng<'_>, session_id: String, dir: String, query: String) -> R<orbital_core::filetools::FindReport> {
    engine.find_files(&session_id, &dir, &query).await
}

#[tauri::command]
pub async fn docker_version(engine: Eng<'_>, session_id: String) -> R<Option<String>> {
    engine.docker_version(&session_id).await
}

#[tauri::command]
pub async fn website_test(engine: Eng<'_>, session_id: String, domain: String, https: bool) -> R<orbital_core::website::SiteTest> {
    engine.website_test(&session_id, &domain, https).await
}

#[tauri::command]
pub async fn nginx_broken_sites(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::website::BrokenSite>> {
    engine.nginx_broken_sites(&session_id).await
}

#[tauri::command]
pub async fn disable_site(engine: Eng<'_>, session_id: String, link: String, progress: Channel<ProgressEvent>) -> R<String> {
    progress::run_job(progress.id(), sink(progress), engine.disable_site(&session_id, &link)).await
}

#[tauri::command]
pub async fn disable_default_site(engine: Eng<'_>, session_id: String, progress: Channel<ProgressEvent>) -> R<String> {
    progress::run_job(progress.id(), sink(progress), engine.disable_default_site(&session_id)).await
}

#[tauri::command]
pub async fn remove_website(
    engine: Eng<'_>,
    session_id: String,
    file: String,
    certificate: Option<String>,
    root: Option<String>,
    progress: Channel<ProgressEvent>,
) -> R<String> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.remove_website(&session_id, &file, certificate.as_deref(), root.as_deref()),
    )
    .await
}

#[tauri::command]
pub fn install_options(engine: Eng) -> Vec<orbital_core::install::InstallOption> {
    engine.install_options()
}

#[tauri::command]
pub async fn install_software(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    force: bool,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(progress.id(), sink(progress), engine.install_software(&session_id, &id, force)).await
}

#[tauri::command]
pub async fn move_preview(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    to: String,
    options: Option<orbital_core::migrate::MoveOptions>,
) -> R<orbital_core::migrate::MovePreview> {
    engine.move_preview(&session_id, &id, &to, &options.unwrap_or_default()).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn move_container(
    engine: Eng<'_>,
    session_id: String,
    id: String,
    to: String,
    mode: orbital_core::migrate::MoveMode,
    force: bool,
    options: Option<orbital_core::migrate::MoveOptions>,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::migrate::MovePlan> {
    let options = options.unwrap_or_default();
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.move_container(&session_id, &id, &to, mode, force, &options),
    )
    .await
}

#[tauri::command]
pub async fn download_folder(
    engine: Eng<'_>,
    session_id: String,
    remote_path: String,
    local_path: String,
    progress: Channel<ProgressEvent>,
) -> R<u64> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.download_folder(&session_id, &remote_path, &PathBuf::from(local_path)),
    )
    .await
}

// ---------- kernel tuning, SSH hardening, security audit, SQL console, recordings ----------

#[tauri::command]
pub async fn kernel_report(engine: Eng<'_>, session_id: String) -> R<orbital_core::kernel::KernelReport> {
    engine.kernel_report(&session_id).await
}

#[tauri::command]
pub async fn kernel_apply(
    engine: Eng<'_>,
    session_id: String,
    values: std::collections::BTreeMap<String, String>,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::kernel::KernelReport> {
    progress::run_job(progress.id(), sink(progress), engine.kernel_apply(&session_id, &values)).await
}

#[tauri::command]
pub async fn harden_state(engine: Eng<'_>, session_id: String) -> R<orbital_core::harden::HardenState> {
    engine.harden_state(&session_id).await
}

#[tauri::command]
pub async fn harden_ssh(
    engine: Eng<'_>,
    session_id: String,
    choices: std::collections::BTreeMap<String, String>,
    force: bool,
) -> R<orbital_core::harden::HardenResult> {
    engine.harden_ssh(&session_id, &choices, force).await
}

#[tauri::command]
pub async fn security_audit(
    engine: Eng<'_>,
    session_id: String,
    progress: Channel<ProgressEvent>,
) -> R<orbital_core::secaudit::AuditReport> {
    progress::run_job(progress.id(), sink(progress), engine.security_audit(&session_id)).await
}

#[tauri::command]
pub async fn sql_query(
    engine: Eng<'_>,
    session_id: String,
    kind: DbKind,
    container: Option<String>,
    db: String,
    query: String,
    read_only: bool,
) -> R<orbital_core::sqlconsole::SqlResult> {
    engine
        .sql_query(&session_id, kind, container.as_deref(), &db, &query, read_only)
        .await
}

#[tauri::command]
pub fn recordings(engine: Eng) -> R<Vec<orbital_core::recording::RecordingInfo>> {
    engine.recordings()
}

#[tauri::command]
pub fn read_recording(engine: Eng, name: String) -> R<String> {
    engine.read_recording(&name)
}

#[tauri::command]
pub fn delete_recording(engine: Eng, name: String) -> R<()> {
    engine.delete_recording(&name)
}

// ---------- processes, disks, packages, TLS inspector, timers ----------

#[tauri::command]
pub async fn processes(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::procs::Proc>> {
    engine.processes(&session_id).await
}

#[tauri::command]
pub async fn process_detail(engine: Eng<'_>, session_id: String, pid: u32) -> R<orbital_core::procs::ProcDetail> {
    engine.process_detail(&session_id, pid).await
}

#[tauri::command]
pub async fn signal_process(engine: Eng<'_>, session_id: String, pid: u32, signal: orbital_core::procs::Signal) -> R<()> {
    engine.signal_process(&session_id, pid, signal).await
}

#[tauri::command]
pub async fn renice_process(engine: Eng<'_>, session_id: String, pid: u32, nice: i32) -> R<()> {
    engine.renice_process(&session_id, pid, nice).await
}

#[tauri::command]
pub async fn disk_report(engine: Eng<'_>, session_id: String) -> R<orbital_core::disks::DiskReport> {
    engine.disk_report(&session_id).await
}

#[tauri::command]
pub async fn installed_packages(engine: Eng<'_>, session_id: String) -> R<Vec<orbital_core::packages::Package>> {
    engine.installed_packages(&session_id).await
}

#[tauri::command]
pub async fn search_packages(engine: Eng<'_>, session_id: String, term: String) -> R<Vec<orbital_core::packages::Found>> {
    engine.search_packages(&session_id, &term).await
}

#[tauri::command]
pub async fn package_plan(
    engine: Eng<'_>,
    session_id: String,
    name: String,
    action: orbital_core::packages::PkgAction,
) -> R<orbital_core::packages::PkgPlan> {
    engine.package_plan(&session_id, &name, action).await
}

#[tauri::command]
pub async fn package_change(
    engine: Eng<'_>,
    session_id: String,
    name: String,
    action: orbital_core::packages::PkgAction,
    force: bool,
    progress: Channel<ProgressEvent>,
) -> R<ExecOutput> {
    progress::run_job(
        progress.id(),
        sink(progress),
        engine.package_change(&session_id, &name, action, force),
    )
    .await
}

#[tauri::command]
pub async fn tls_inspect(engine: Eng<'_>, host: String, port: u16) -> R<orbital_core::tlsinspect::TlsReport> {
    engine.tls_inspect(&host, port).await
}

#[tauri::command]
pub async fn check_schedule(engine: Eng<'_>, session_id: String, schedule: String) -> R<orbital_core::timers::ScheduleCheck> {
    engine.check_schedule(&session_id, &schedule).await
}

#[tauri::command]
pub async fn create_timer(
    engine: Eng<'_>,
    session_id: String,
    spec: orbital_core::timers::TimerSpec,
    progress: Channel<ProgressEvent>,
) -> R<String> {
    progress::run_job(progress.id(), sink(progress), engine.create_timer(&session_id, &spec)).await
}

#[tauri::command]
pub async fn remove_timer(engine: Eng<'_>, session_id: String, name: String) -> R<()> {
    engine.remove_timer(&session_id, &name).await
}
