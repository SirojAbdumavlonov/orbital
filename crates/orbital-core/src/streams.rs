//! Long-running commands whose output streams to the UI as it arrives (log
//! following, deploy steps). Each runs on its own channel of the existing SSH
//! connection. A sudo password, when needed, is written to stdin before
//! anything else and never appears in the output (there is no terminal to echo it).

use crate::error::{Error, Result};
use crate::profiles::SudoMode;
use crate::ssh::SshSession;
use russh::client::Msg;
use russh::{ChannelMsg, ChannelWriteHalf};
use tokio::task::JoinHandle;

pub enum StreamEvent {
    Data(Vec<u8>),
    Exit(Option<i32>),
}

pub struct Stream {
    pub id: String,
    writer: ChannelWriteHalf<Msg>,
    task: JoinHandle<Option<i32>>,
}

impl Stream {
    /// Stop the command (closing its channel ends `tail -F`, `journalctl -f`, …).
    pub async fn stop(&self) {
        let _ = self.writer.close().await;
        self.task.abort();
    }

    /// Wait for the command to finish; its exit code, if the server sent one.
    pub async fn wait(self) -> Option<i32> {
        self.task.await.ok().flatten()
    }
}

impl SshSession {
    /// Whether sudo works on this connection right now (configured, and not refused).
    pub async fn sudo_works(&self) -> bool {
        self.elevation_needed() && matches!(self.exec_privileged("true").await, Ok(o) if o.ok())
    }

    /// Start `cmd`, as root when `privileged` (and the profile can), streaming
    /// stdout and stderr to `sink` as they arrive.
    pub async fn open_stream<F>(&self, cmd: &str, privileged: bool, sink: F) -> Result<Stream>
    where
        F: Fn(StreamEvent) + Send + Sync + 'static,
    {
        let elevate = privileged && self.elevation_needed();
        let full = if elevate { self.privileged_command(cmd) } else { cmd.to_string() };
        let password = match (elevate, self.profile.sudo_mode, &self.sudo_password) {
            (true, SudoMode::Password, Some(pw)) => Some(zeroize::Zeroizing::new(format!("{}\n", pw.as_str()))),
            (true, SudoMode::Password, None) => {
                return Err(Error::CredentialsRequired {
                    what: "Sudo password".into(),
                    user: self.profile.username.clone(),
                    host: self.profile.host.clone(),
                    profile_id: self.profile.id.clone(),
                })
            }
            _ => None,
        };
        let ch = self.handle().channel_open_session().await?;
        ch.exec(true, full).await?;
        if let Some(pw) = &password {
            ch.data(pw.as_bytes()).await?;
        }
        ch.eof().await?;
        let (mut read, writer) = ch.split();
        let task = tokio::spawn(async move {
            let mut code = None;
            while let Some(msg) = read.wait().await {
                match msg {
                    ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => sink(StreamEvent::Data(data.to_vec())),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status as i32),
                    ChannelMsg::ExitSignal { .. } => code = code.or(Some(255)),
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            sink(StreamEvent::Exit(code));
            code
        });
        Ok(Stream {
            id: uuid::Uuid::new_v4().to_string(),
            writer,
            task,
        })
    }
}
