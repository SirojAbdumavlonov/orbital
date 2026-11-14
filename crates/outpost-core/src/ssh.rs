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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;

/// Hard cap on captured command output, so a runaway command can't exhaust memory.
const MAX_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// Fingerprint the user explicitly confirmed in the trust dialog.
    pub accept_fingerprint: Option<String>,
    /// Password / passphrase typed into the login prompt.
    pub secret: Option<String>,
    /// Save `secret` to the Keychain on successful login.
    pub remember: bool,
    pub timeout: Option<Duration>,
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
    sudo_password: Option<Secret>,
    pub(crate) sftp: AsyncMutex<Option<Arc<russh_sftp::client::SftpSession>>>,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{}", home.to_string_lossy(), rest);
        }
    }
    p.to_string()
}

impl SshSession {
    pub async fn connect(db: Arc<Database>, secrets: &dyn SecretStore, profile: Profile, opts: ConnectOptions) -> Result<Self> {
        let config = Arc::new(client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(20)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let rejection = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            db: db.clone(),
            host: profile.host.clone(),
            port: profile.port,
            accept_fingerprint: opts.accept_fingerprint.clone(),
            rejection: rejection.clone(),
        };
        let timeout = opts.timeout.unwrap_or(Duration::from_secs(15));
        let connect = client::connect(config, (profile.host.as_str(), profile.port), handler);
        let mut handle = match tokio::time::timeout(timeout, connect).await {
            Err(_) => return Err(Error::Ssh(format!("connection to {}:{} timed out", profile.host, profile.port))),
            Ok(Err(e)) => {
                if let Some(rej) = rejection.lock().unwrap().take() {
                    return Err(rej);
                }
                return Err(e.into());
            }
            Ok(Ok(h)) => h,
        };

        let user = profile.username.clone();
        let auth_err = || Error::AuthFailed {
            user: user.clone(),
            host: profile.host.clone(),
        };
        let typed: Option<Secret> = opts.secret.clone().map(Zeroizing::new);

        let (authed, used_secret_kind) = match profile.auth_method {
            AuthMethod::Password => {
                let pw = match typed.clone() {
                    Some(p) => p,
                    None => secrets
                        .get(&secret_key(&profile.id, SecretKind::Password))?
                        .ok_or_else(|| Error::CredentialsRequired {
                            what: "Password".into(),
                            user: user.clone(),
                            host: profile.host.clone(),
                        })?,
                };
                let mut ok = handle.authenticate_password(user.clone(), pw.as_str()).await?.success();
                if !ok {
                    ok = keyboard_interactive(&mut handle, &user, &pw).await?;
                }
                (ok, Some(SecretKind::Password))
            }
            AuthMethod::Key => {
                let path = expand_tilde(profile.key_path.as_deref().unwrap_or(""));
                let stored = secrets.get(&secret_key(&profile.id, SecretKind::KeyPassphrase))?;
                let pass = typed.clone().or(stored);
                let key = match russh::keys::load_secret_key(&path, pass.as_ref().map(|p| p.as_str())) {
                    Ok(k) => k,
                    Err(russh::keys::Error::KeyIsEncrypted) => {
                        return Err(Error::CredentialsRequired {
                            what: "Key passphrase".into(),
                            user: user.clone(),
                            host: profile.host.clone(),
                        })
                    }
                    Err(e) => return Err(Error::Ssh(format!("cannot load private key {path}: {e}"))),
                };
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                let ok = handle
                    .authenticate_publickey(user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                    .await?
                    .success();
                (ok, typed.as_ref().map(|_| SecretKind::KeyPassphrase))
            }
            AuthMethod::Agent => (agent_auth(&mut handle, &user).await?, None),
        };
        if !authed {
            return Err(auth_err());
        }
        if opts.remember {
            if let (Some(s), Some(kind)) = (&typed, used_secret_kind) {
                secrets.set(&secret_key(&profile.id, kind), s)?;
            }
        }

        let sudo_password = if profile.sudo_mode == SudoMode::Password {
            match secrets.get(&secret_key(&profile.id, SecretKind::SudoPassword))? {
                Some(s) => Some(s),
                None if profile.auth_method == AuthMethod::Password => {
                    typed.clone().or(secrets.get(&secret_key(&profile.id, SecretKind::Password))?)
                }
                None => None,
            }
        } else {
            None
        };

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            profile,
            handle,
            sudo_password,
            sftp: AsyncMutex::new(None),
            connected_at: chrono::Utc::now(),
        })
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
        let full = self.privileged_command(cmd);
        let input = match (self.elevation_needed(), self.profile.sudo_mode, &self.sudo_password) {
            (true, SudoMode::Password, Some(pw)) => Some(Zeroizing::new(format!("{}\n", pw.as_str()))),
            (true, SudoMode::Password, None) => {
                return Err(Error::CredentialsRequired {
                    what: "Sudo password".into(),
                    user: self.profile.username.clone(),
                    host: self.profile.host.clone(),
                })
            }
            _ => None,
        };
        let out = self
            .exec_with_input(&full, input.as_ref().map(|s| s.as_bytes()), Duration::from_secs(300))
            .await?;
        if out.exit_code != 0 && self.elevation_needed() && is_sudo_failure(&out.stderr) {
            return Err(Error::PermissionDenied(format!("sudo refused: {}", out.stderr.trim())));
        }
        Ok(out)
    }

    pub async fn close(&self) {
        let _ = self.handle.disconnect(Disconnect::ByApplication, "bye", "en").await;
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

async fn agent_auth(handle: &mut Handle<ClientHandler>, user: &str) -> Result<bool> {
    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(|e| Error::Ssh(format!("ssh-agent not available (is SSH_AUTH_SOCK set?): {e}")))?;
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
