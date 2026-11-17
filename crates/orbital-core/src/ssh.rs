//! SSH session: connect, verify host key, authenticate, run commands.

use crate::db::Database;
use crate::error::{Error, Result};
use crate::known_hosts::{self, HostKeyStatus};
use crate::profiles::{AuthMethod, Profile, SudoMode};
use crate::secrets::{secret_key, Secret, SecretKind, SecretStore};
use crate::shell::quote;
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;

/// Hard cap on captured command output, so a runaway command can't exhaust memory.
const MAX_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct ConnectOptions {
    /// Fingerprint the user explicitly confirmed in the trust dialog.
    pub accept_fingerprint: Option<String>,
    /// The host and port that dialog was shown for. The fingerprint is only
    /// ever pinned for exactly this endpoint; when absent it applies to the
    /// target server and never to a jump host.
    pub accept_host: Option<(String, u16)>,
    /// Password / passphrase typed into the login prompt for the target server.
    pub secret: Option<String>,
    /// Secrets typed for other profiles on the route (jump hosts), by profile id.
    pub hop_secrets: HashMap<String, String>,
    /// Save typed secrets to the Keychain on successful login.
    pub remember: bool,
    pub timeout: Option<Duration>,
}

// Hand-written so typed secrets can never end up in a log line.
impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectOptions")
            .field("accept_fingerprint", &self.accept_fingerprint)
            .field("accept_host", &self.accept_host)
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .field("hop_secrets", &self.hop_secrets.keys().collect::<Vec<_>>())
            .field("remember", &self.remember)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl ConnectOptions {
    /// The confirmed fingerprint, if it was confirmed for this very endpoint.
    fn accepted_fingerprint(&self, profile: &Profile, is_target: bool) -> Option<String> {
        let applies = match &self.accept_host {
            Some((host, port)) => *host == profile.host && *port == profile.port,
            None => is_target,
        };
        self.accept_fingerprint.clone().filter(|_| applies)
    }

    /// The secret the user typed for this profile, if any.
    pub(crate) fn typed_for(&self, profile: &Profile, is_target: bool) -> Option<Secret> {
        self.hop_secrets
            .get(&profile.id)
            .or(if is_target { self.secret.as_ref() } else { None })
            .map(|s| Zeroizing::new(s.clone()))
    }
}

/// Which saved secrets connecting to `profile` would read from the store.
/// Must mirror [`SshSession::authenticate`]; the engine uses it to decide
/// whether the user has to be verified first.
pub(crate) fn saved_secrets_wanted(profile: &Profile, has_typed: bool, is_target: bool) -> Vec<SecretKind> {
    let mut kinds = Vec::new();
    // File-storage connections (FTP, WebDAV, S3) read one secret: the password
    // or S3 secret key, in `Engine::connect_storage`.
    if !profile.protocol.is_ssh() {
        return if has_typed { kinds } else { vec![SecretKind::Password] };
    }
    match profile.auth_method {
        AuthMethod::Password if !has_typed => kinds.push(SecretKind::Password),
        AuthMethod::Key if !has_typed => kinds.push(SecretKind::KeyPassphrase),
        _ => {}
    }
    if is_target && profile.sudo_mode == SudoMode::Password {
        kinds.push(SecretKind::SudoPassword);
        if profile.auth_method == AuthMethod::Password && !has_typed && !kinds.contains(&SecretKind::Password) {
            kinds.push(SecretKind::Password);
        }
    }
    kinds
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: i64,
}

impl ExecOutput {
    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }
    pub fn check(self) -> Result<Self> {
        if self.ok() {
            Ok(self)
        } else {
            let msg = if self.stderr.trim().is_empty() {
                self.stdout.clone()
            } else {
                self.stderr.clone()
            };
            Err(Error::Remote {
                code: self.exit_code,
                stderr: msg.trim().to_string(),
            })
        }
    }
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr).trim().to_string()
    }
}

pub struct ClientHandler {
    db: Arc<Database>,
    host: String,
    port: u16,
    accept_fingerprint: Option<String>,
    rejection: Arc<Mutex<Option<Error>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let key: PublicKey = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => PublicKey::new(cert.public_key().clone(), ""),
        };
        let fp = known_hosts::fingerprint(&key);
        let status = match known_hosts::check(&self.db, &self.host, self.port, &key) {
            Ok(s) => s,
            Err(e) => {
                *self.rejection.lock().unwrap() = Some(e);
                return Ok(false);
            }
        };
        match status {
            HostKeyStatus::Trusted => Ok(true),
            HostKeyStatus::Unknown if self.accept_fingerprint.as_deref() == Some(fp.as_str()) => {
                // User confirmed exactly this fingerprint -> pin it.
                if let Err(e) = known_hosts::trust(&self.db, &self.host, self.port, &key) {
                    *self.rejection.lock().unwrap() = Some(e);
                    return Ok(false);
                }
                Ok(true)
            }
            HostKeyStatus::Unknown => {
                *self.rejection.lock().unwrap() = Some(Error::UnknownHostKey {
                    host: self.host.clone(),
                    port: self.port,
                    algorithm: known_hosts::algorithm(&key),
                    fingerprint: fp,
                });
                Ok(false)
            }
            // A changed key is NEVER silently accepted: the user must remove the
            // old pin explicitly in Settings → Known Hosts.
            HostKeyStatus::Mismatch { expected } => {
                *self.rejection.lock().unwrap() = Some(Error::HostKeyMismatch {
                    host: self.host.clone(),
                    port: self.port,
                    expected,
                    actual: fp,
                });
                Ok(false)
            }
        }
    }
}

pub struct SshSession {
    pub id: String,
    pub profile: Profile,
    handle: Handle<ClientHandler>,
    pub(crate) sudo_password: Option<Secret>,
    /// The session this one is tunnelled through (ProxyJump). Kept alive for
    /// as long as this session exists and closed together with it.
    jump: Option<Box<SshSession>>,
    pub(crate) sftp: AsyncMutex<Option<Arc<russh_sftp::client::SftpSession>>>,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

/// The user's home folder: `HOME` on macOS and Linux, `USERPROFILE` on Windows.
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
}

pub(crate) fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(home) = home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

fn endpoint(p: &Profile) -> String {
    format!("{}@{}:{}", p.username, p.host, p.port)
}

impl SshSession {
    /// Connect straight to `profile`.
    pub async fn connect(db: Arc<Database>, secrets: &dyn SecretStore, profile: Profile, opts: ConnectOptions) -> Result<Self> {
        Self::connect_route(db, secrets, vec![profile], opts).await
    }

    /// Connect along `route` (see [`crate::profiles::chain`]): jump hosts
    /// first, the target last. Each hop is a full SSH session — host key
    /// checked against its own pin, authenticated with its own credentials —
    /// carried inside a direct-tcpip channel of the previous hop, so a jump
    /// host only ever sees ciphertext for the hops behind it.
    pub async fn connect_route(db: Arc<Database>, secrets: &dyn SecretStore, route: Vec<Profile>, opts: ConnectOptions) -> Result<Self> {
        let last = route.len().saturating_sub(1);
        let mut session: Option<SshSession> = None;
        for (i, profile) in route.into_iter().enumerate() {
            let via = session.take();
            session = Some(Self::connect_hop(db.clone(), secrets, profile, via, &opts, i == last).await?);
        }
        session.ok_or_else(|| Error::InvalidInput("empty route".into()))
    }

    async fn connect_hop(
        db: Arc<Database>,
        secrets: &dyn SecretStore,
        profile: Profile,
        via: Option<SshSession>,
        opts: &ConnectOptions,
        is_target: bool,
    ) -> Result<Self> {
        match Self::open(db, secrets, &profile, via.as_ref(), opts, is_target).await {
            Ok((handle, sudo_password)) => Ok(Self {
                id: uuid::Uuid::new_v4().to_string(),
                profile,
                handle,
                sudo_password,
                jump: via.map(Box::new),
                sftp: AsyncMutex::new(None),
                connected_at: chrono::Utc::now(),
            }),
            Err(e) => {
                // Don't leave half a route connected.
                if let Some(j) = via {
                    j.close().await;
                }
                Err(e)
            }
        }
    }

    /// Transport + host-key check for one hop, then authentication.
    async fn open(
        db: Arc<Database>,
        secrets: &dyn SecretStore,
        profile: &Profile,
        via: Option<&SshSession>,
        opts: &ConnectOptions,
        is_target: bool,
    ) -> Result<(Handle<ClientHandler>, Option<Secret>)> {
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(20)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let rejection = Arc::new(Mutex::new(None));
        // The handler verifies the key against the pin for *this* hop's
        // host:port, whether the bytes arrive over TCP or through a tunnel.
        let handler = ClientHandler {
            db,
            host: profile.host.clone(),
            port: profile.port,
            accept_fingerprint: opts.accepted_fingerprint(profile, is_target),
            rejection: rejection.clone(),
        };
        let timeout = opts.timeout.unwrap_or(Duration::from_secs(15));
        let transport = async {
            match via {
                None => client::connect(config, (profile.host.as_str(), profile.port), handler).await,
                Some(jump) => {
                    let channel = jump
                        .handle
                        .channel_open_direct_tcpip(profile.host.clone(), profile.port as u32, "127.0.0.1", 0)
                        .await
                        .map_err(|e| {
                            Error::Ssh(format!(
                                "jump host {} couldn't open a connection to {}:{}: {e}",
                                jump.label(),
                                profile.host,
                                profile.port
                            ))
                        });
                    match channel {
                        Ok(ch) => client::connect_stream(config, ch.into_stream(), handler).await,
                        Err(e) => {
                            *rejection.lock().unwrap() = Some(e);
                            Err(russh::Error::Disconnect)
                        }
                    }
                }
            }
        };
        let mut handle = match tokio::time::timeout(timeout, transport).await {
            Err(_) => return Err(Error::Ssh(format!("connection to {}:{} timed out", profile.host, profile.port))),
            Ok(Err(e)) => {
                if let Some(rej) = rejection.lock().unwrap().take() {
                    return Err(rej);
                }
                return Err(e.into());
            }
            Ok(Ok(h)) => h,
        };
        let sudo_password = Self::authenticate(&mut handle, secrets, profile, opts, is_target).await?;
        Ok((handle, sudo_password))
    }

    /// Authenticate one hop. Returns the sudo password for the target (jump
    /// hosts never get one). Keep [`saved_secrets_wanted`] in sync with the
    /// store reads here.
    async fn authenticate(
        handle: &mut Handle<ClientHandler>,
        secrets: &dyn SecretStore,
        profile: &Profile,
        opts: &ConnectOptions,
        is_target: bool,
    ) -> Result<Option<Secret>> {
        let user = profile.username.clone();
        let needs = |what: &str| Error::CredentialsRequired {
            what: what.into(),
            user: user.clone(),
            host: profile.host.clone(),
            profile_id: profile.id.clone(),
        };
        let typed: Option<Secret> = opts.typed_for(profile, is_target);

        let (authed, used_secret_kind) = match profile.auth_method {
            AuthMethod::Password => {
                let pw = match typed.clone() {
                    Some(p) => p,
                    None => secrets
                        .get(&secret_key(&profile.id, SecretKind::Password))?
                        .ok_or_else(|| needs("Password"))?,
                };
                let mut ok = handle.authenticate_password(user.clone(), pw.as_str()).await?.success();
                if !ok {
                    ok = keyboard_interactive(handle, &user, &pw).await?;
                }
                (ok, Some(SecretKind::Password))
            }
            AuthMethod::Key => {
                let path = expand_tilde(profile.key_path.as_deref().unwrap_or(""));
                let pass = match typed.clone() {
                    Some(p) => Some(p),
                    None => secrets.get(&secret_key(&profile.id, SecretKind::KeyPassphrase))?,
                };
                let key = match russh::keys::load_secret_key(&path, pass.as_ref().map(|p| p.as_str())) {
                    Ok(k) => k,
                    Err(russh::keys::Error::KeyIsEncrypted) => return Err(needs("Key passphrase")),
                    Err(e) => return Err(Error::Ssh(format!("cannot load private key {path}: {e}"))),
                };
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                let ok = handle
                    .authenticate_publickey(user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                    .await?
                    .success();
                (ok, typed.as_ref().map(|_| SecretKind::KeyPassphrase))
            }
            AuthMethod::Agent => (agent_auth(handle, &user).await?, None),
        };
        if !authed {
            return Err(Error::AuthFailed {
                user,
                host: profile.host.clone(),
                profile_id: profile.id.clone(),
            });
        }
        if opts.remember {
            if let (Some(s), Some(kind)) = (&typed, used_secret_kind) {
                secrets.set(&secret_key(&profile.id, kind), s)?;
            }
        }

        if !is_target || profile.sudo_mode != SudoMode::Password {
            return Ok(None);
        }
        Ok(match secrets.get(&secret_key(&profile.id, SecretKind::SudoPassword))? {
            Some(s) => Some(s),
            None if profile.auth_method == AuthMethod::Password => match typed {
                Some(t) => Some(t),
                None => secrets.get(&secret_key(&profile.id, SecretKind::Password))?,
            },
            None => None,
        })
    }

    /// Jump hosts this session is tunnelled through, outermost first.
    pub fn via(&self) -> Vec<String> {
        let mut hops = Vec::new();
        let mut cur = self.jump.as_deref();
        while let Some(j) = cur {
            hops.push(endpoint(&j.profile));
            cur = j.jump.as_deref();
        }
        hops.reverse();
        hops
    }

    pub fn label(&self) -> String {
        let p = &self.profile;
        if p.port == 22 {
            format!("{}@{}", p.username, p.host)
        } else {
            format!("{}@{}:{}", p.username, p.host, p.port)
        }
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    pub(crate) fn handle(&self) -> &Handle<ClientHandler> {
        &self.handle
    }

    pub async fn exec(&self, cmd: &str) -> Result<ExecOutput> {
        self.exec_with_input(cmd, None, Duration::from_secs(120)).await
    }

    pub async fn exec_with_input(&self, cmd: &str, input: Option<&[u8]>, timeout: Duration) -> Result<ExecOutput> {
        let started = Instant::now();
        let run = async {
            let mut ch = self.handle.channel_open_session().await?;
            ch.exec(true, cmd).await?;
            if let Some(data) = input {
                ch.data(data).await?;
            }
            ch.eof().await?;
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let mut code: Option<i32> = None;
            while let Some(msg) = ch.wait().await {
                match msg {
                    ChannelMsg::Data { data } if out.len() < MAX_OUTPUT => out.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } if err.len() < MAX_OUTPUT => err.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status as i32),
                    ChannelMsg::ExitSignal { .. } => code = code.or(Some(255)),
                    _ => {}
                }
            }
            Ok::<_, Error>(ExecOutput {
                stdout: String::from_utf8_lossy(&out).into_owned(),
                stderr: String::from_utf8_lossy(&err).into_owned(),
                exit_code: code.unwrap_or(-1),
                duration_ms: started.elapsed().as_millis() as i64,
            })
        };
        tokio::time::timeout(timeout, run)
            .await
            .map_err(|_| Error::Ssh(format!("command timed out after {}s", timeout.as_secs())))?
    }

    pub fn elevation_needed(&self) -> bool {
        match self.profile.sudo_mode {
            SudoMode::Never => false,
            SudoMode::Auto => self.profile.username != "root",
            SudoMode::NoPassword | SudoMode::Password => true,
        }
    }

    /// The command line that will actually be sent (for audit / preview). Never contains secrets.
    pub fn privileged_command(&self, cmd: &str) -> String {
        if !self.elevation_needed() {
            return cmd.to_string();
        }
        match self.profile.sudo_mode {
            SudoMode::Password => format!("sudo -k -S -p '' -- sh -c {}", quote(cmd)),
            _ => format!("sudo -n -- sh -c {}", quote(cmd)),
        }
    }

    /// Run with root privileges according to the profile's sudo policy. The sudo
    /// password goes through stdin (never the command line, never the audit log).
    pub async fn exec_privileged(&self, cmd: &str) -> Result<ExecOutput> {
        self.exec_privileged_with(cmd, None, Duration::from_secs(300)).await
    }

    /// Like [`exec_privileged`], with data for the command's stdin (for example a
    /// new password for `chpasswd`, which must never be on the command line) and
    /// a timeout of the caller's choosing (backups can take long).
    ///
    /// sudo reads its own password a byte at a time up to the newline, so what
    /// follows that line reaches the command untouched.
    pub async fn exec_privileged_with(&self, cmd: &str, input: Option<&[u8]>, timeout: Duration) -> Result<ExecOutput> {
        let full = self.privileged_command(cmd);
        let mut stdin: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
        match (self.elevation_needed(), self.profile.sudo_mode, &self.sudo_password) {
            (true, SudoMode::Password, Some(pw)) => {
                stdin.extend_from_slice(pw.as_bytes());
                stdin.push(b'\n');
            }
            (true, SudoMode::Password, None) => {
                return Err(Error::CredentialsRequired {
                    what: "Sudo password".into(),
                    user: self.profile.username.clone(),
                    host: self.profile.host.clone(),
                    profile_id: self.profile.id.clone(),
                })
            }
            _ => {}
        }
        if let Some(data) = input {
            stdin.extend_from_slice(data);
        }
        let out = self
            .exec_with_input(&full, (!stdin.is_empty()).then_some(stdin.as_slice()), timeout)
            .await?;
        if out.exit_code != 0 && self.elevation_needed() && is_sudo_failure(&out.stderr) {
            return Err(Error::PermissionDenied(format!("sudo refused: {}", out.stderr.trim())));
        }
        Ok(out)
    }

    /// Run a read-only inspection command as root when the profile allows it,
    /// falling back to the login user when sudo is unavailable or refused.
    /// The flag says whether it ran with root's view of the system.
    pub async fn exec_inspect(&self, cmd: &str) -> Result<(ExecOutput, bool)> {
        if self.elevation_needed() {
            if let Ok(o) = self.exec_privileged(cmd).await {
                return Ok((o, true));
            }
        }
        let root = self.profile.username == "root";
        Ok((self.exec(cmd).await?, root))
    }

    /// Disconnect this session and every jump host under it, innermost first.
    pub async fn close(&self) {
        let mut cur = Some(self);
        while let Some(s) = cur {
            let _ = s.handle.disconnect(Disconnect::ByApplication, "bye", "en").await;
            cur = s.jump.as_deref();
        }
    }
}

fn is_sudo_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("sudo:")
        && (s.contains("password is required")
            || s.contains("incorrect password")
            || s.contains("not in the sudoers")
            || s.contains("no password was provided")
            || s.contains("not allowed to execute")
            || s.contains("may not run sudo"))
}

async fn keyboard_interactive(handle: &mut Handle<ClientHandler>, user: &str, pw: &str) -> Result<bool> {
    let mut resp = handle.authenticate_keyboard_interactive_start(user.to_string(), None).await?;
    for _ in 0..5 {
        match resp {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| pw.to_string()).collect();
                resp = handle.authenticate_keyboard_interactive_respond(answers).await?;
            }
        }
    }
    Ok(false)
}

type AgentClient = russh::keys::agent::client::AgentClient<Box<dyn russh::keys::agent::client::AgentStream + Send + Unpin>>;

/// The ssh-agent named by `SSH_AUTH_SOCK` (1Password, Secretive, …).
#[cfg(not(windows))]
async fn open_agent() -> Result<AgentClient> {
    let a = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(|e| Error::Ssh(format!("ssh-agent not available (is SSH_AUTH_SOCK set?): {e}")))?;
    Ok(AgentClient::connect(a.into_inner()))
}

/// Windows: the OpenSSH agent service's pipe (or the pipe `SSH_AUTH_SOCK`
/// names, as 1Password sets), then Pageant (PuTTY's agent).
#[cfg(windows)]
async fn open_agent() -> Result<AgentClient> {
    use russh::keys::agent::client::AgentClient as Client;
    let pipe = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|p| p.starts_with(r"\\.\pipe\"))
        .unwrap_or_else(|| r"\\.\pipe\openssh-ssh-agent".into());
    if let Ok(a) = Client::connect_named_pipe(&pipe).await {
        return Ok(AgentClient::connect(a.into_inner()));
    }
    match Client::connect_pageant().await {
        Ok(a) => Ok(AgentClient::connect(a.into_inner())),
        Err(e) => Err(Error::Ssh(format!(
            "no SSH agent found: start the \"OpenSSH Authentication Agent\" service or Pageant ({e})"
        ))),
    }
}

/// Fingerprints (`SHA256:…`) of the keys the SSH agent holds; empty without an agent.
pub(crate) async fn agent_fingerprints() -> Vec<String> {
    let Ok(mut agent) = open_agent().await else { return vec![] };
    let Ok(ids) = agent.request_identities().await else { return vec![] };
    ids.into_iter()
        .filter_map(|id| match id {
            russh::keys::agent::AgentIdentity::PublicKey { key, .. } => Some(key.fingerprint(russh::keys::HashAlg::Sha256).to_string()),
            _ => None,
        })
        .collect()
}

async fn agent_auth(handle: &mut Handle<ClientHandler>, user: &str) -> Result<bool> {
    let mut agent = open_agent().await?;
    let identities = agent.request_identities().await?;
    if identities.is_empty() {
        return Err(Error::Ssh("ssh-agent has no identities (run ssh-add)".into()));
    }
    let hash = handle.best_supported_rsa_hash().await?.flatten();
    for id in identities {
        if let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = id {
            match handle.authenticate_publickey_with(user.to_string(), key, hash, &mut agent).await {
                Ok(r) if r.success() => return Ok(true),
                _ => continue,
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, host: &str, auth: AuthMethod, sudo: SudoMode) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            host: host.into(),
            port: 22,
            username: "deploy".into(),
            auth_method: auth,
            key_path: None,
            sudo_mode: sudo,
            group_name: None,
            color: None,
            initial_path: None,
            jump_profile_id: None,
            protocol: Default::default(),
            region: None,
            path_style: false,
            created_at: String::new(),
            last_connected_at: None,
        }
    }

    #[test]
    fn fingerprint_acceptance_is_bound_to_one_endpoint() {
        let bastion = profile("b", "bastion.example", AuthMethod::Agent, SudoMode::Never);
        let web = profile("w", "10.0.0.5", AuthMethod::Agent, SudoMode::Never);
        let fp = Some("SHA256:abc".to_string());

        // Confirmed for the bastion: never applied to the target behind it.
        let o = ConnectOptions {
            accept_fingerprint: fp.clone(),
            accept_host: Some(("bastion.example".into(), 22)),
            ..Default::default()
        };
        assert_eq!(o.accepted_fingerprint(&bastion, false), fp);
        assert_eq!(o.accepted_fingerprint(&web, true), None);
        // Same host, different port is a different endpoint.
        let o2 = ConnectOptions {
            accept_host: Some(("bastion.example".into(), 2222)),
            ..o.clone()
        };
        assert_eq!(o2.accepted_fingerprint(&bastion, false), None);

        // No endpoint given: target only, never a jump host.
        let legacy = ConnectOptions {
            accept_fingerprint: fp.clone(),
            ..Default::default()
        };
        assert_eq!(legacy.accepted_fingerprint(&bastion, false), None);
        assert_eq!(legacy.accepted_fingerprint(&web, true), fp);
    }

    #[test]
    fn typed_secrets_go_to_the_right_hop_and_are_not_debug_printed() {
        let bastion = profile("b", "bastion.example", AuthMethod::Password, SudoMode::Never);
        let web = profile("w", "10.0.0.5", AuthMethod::Password, SudoMode::Never);
        let o = ConnectOptions {
            secret: Some("target-pass".into()),
            hop_secrets: HashMap::from([("b".to_string(), "bastion-pass".to_string())]),
            ..Default::default()
        };
        assert_eq!(o.typed_for(&bastion, false).unwrap().as_str(), "bastion-pass");
        assert_eq!(o.typed_for(&web, true).unwrap().as_str(), "target-pass");
        // The target's password is never offered to a jump host.
        let only_target = ConnectOptions {
            secret: Some("target-pass".into()),
            ..Default::default()
        };
        assert!(only_target.typed_for(&bastion, false).is_none());

        let dbg = format!("{o:?}");
        assert!(!dbg.contains("target-pass") && !dbg.contains("bastion-pass"), "{dbg}");
    }

    #[test]
    fn saved_secrets_wanted_matches_auth_flow() {
        use SecretKind::*;
        let p = |a, s| profile("p", "h", a, s);
        assert_eq!(saved_secrets_wanted(&p(AuthMethod::Agent, SudoMode::Auto), false, true), vec![]);
        assert_eq!(
            saved_secrets_wanted(&p(AuthMethod::Password, SudoMode::Auto), false, true),
            vec![Password]
        );
        assert_eq!(saved_secrets_wanted(&p(AuthMethod::Password, SudoMode::Auto), true, true), vec![]);
        assert_eq!(
            saved_secrets_wanted(&p(AuthMethod::Key, SudoMode::Auto), false, true),
            vec![KeyPassphrase]
        );
        assert_eq!(
            saved_secrets_wanted(&p(AuthMethod::Agent, SudoMode::Password), false, true),
            vec![SudoPassword]
        );
        // The sudo password is only fetched for the target, not for jump hosts.
        assert_eq!(
            saved_secrets_wanted(&p(AuthMethod::Agent, SudoMode::Password), false, false),
            vec![]
        );
        // Typed login password still reads a saved sudo password.
        assert_eq!(
            saved_secrets_wanted(&p(AuthMethod::Password, SudoMode::Password), true, true),
            vec![SudoPassword]
        );
        assert!(saved_secrets_wanted(&p(AuthMethod::Password, SudoMode::Password), false, true).contains(&Password));
    }
}
