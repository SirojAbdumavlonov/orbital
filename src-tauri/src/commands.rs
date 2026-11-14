use base64::Engine as _;
use outpost_core::audit::{AuditEvent, AuditFilter, VerifyReport};
use outpost_core::config_edit::{SafeWriteReport, Validator};
use outpost_core::docker::{Container, ContainerAction, ContainerStats};
use outpost_core::engine::{Engine, SessionInfo};
use outpost_core::known_hosts::KnownHost;
use outpost_core::metrics::Metrics;
use outpost_core::profiles::{Profile, ProfileInput};
use outpost_core::secrets::SecretKind;
use outpost_core::services::{ActionReport, Service, ServiceAction, ServiceDetail};
use outpost_core::sftp::{FileContent, FileEntry};
use outpost_core::ssh::{ConnectOptions, ExecOutput};
use outpost_core::terminal::TermEvent;
use outpost_core::Error;
use serde::{Deserialize, Serialize};
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

// ---------- sessions ----------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectArgs {
    accept_fingerprint: Option<String>,
    secret: Option<String>,
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
                secret: o.secret,
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

// ---------- monitoring / services / docker ----------

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
    on_event: Channel<TermMessage>,
) -> R<String> {
    engine
        .open_terminal(&session_id, cols, rows, cwd, move |ev| {
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
