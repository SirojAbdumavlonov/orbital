//! Interactive PTY shells over the existing SSH connection — no separate
//! Terminal.app / iTerm needed. Output is pushed to a sink (the UI channel).

use crate::error::Result;
use crate::shell::quote;
use crate::ssh::SshSession;
use russh::client::Msg;
use russh::{ChannelMsg, ChannelWriteHalf};
use tokio::task::JoinHandle;

pub enum TermEvent {
    Data(Vec<u8>),
    Exit(Option<u32>),
}

pub struct Terminal {
    pub id: String,
    pub cwd: Option<String>,
    writer: ChannelWriteHalf<Msg>,
    reader_task: JoinHandle<()>,
}

impl Terminal {
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        self.writer.data(data).await?;
        Ok(())
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.writer.window_change(cols.max(1), rows.max(1), 0, 0).await?;
        Ok(())
    }

    pub async fn close(&self) {
        let _ = self.writer.eof().await;
        let _ = self.writer.close().await;
        self.reader_task.abort();
    }
}

impl SshSession {
    pub async fn open_terminal<F>(&self, cols: u32, rows: u32, cwd: Option<String>, sink: F) -> Result<Terminal>
    where
        F: Fn(TermEvent) + Send + Sync + 'static,
    {
        let ch = self.handle().channel_open_session().await?;
        ch.request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[]).await?;
        ch.set_env(true, "COLORTERM", "truecolor").await.ok();
        match &cwd {
            Some(dir) => {
                // Login shell started in the requested directory ("Open terminal here").
                let cmd = format!("cd -- {} 2>/dev/null || cd; exec \"${{SHELL:-/bin/sh}}\" -l", quote(dir));
                ch.exec(true, cmd).await?;
            }
            None => ch.request_shell(true).await?,
        }
        let (mut read, writer) = ch.split();
        let reader_task = tokio::spawn(async move {
            let mut code = None;
            while let Some(msg) = read.wait().await {
                match msg {
                    ChannelMsg::Data { data } => sink(TermEvent::Data(data.to_vec())),
                    ChannelMsg::ExtendedData { data, .. } => sink(TermEvent::Data(data.to_vec())),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            sink(TermEvent::Exit(code));
        });
        Ok(Terminal {
            id: uuid::Uuid::new_v4().to_string(),
            cwd,
            writer,
            reader_task,
        })
    }
}
