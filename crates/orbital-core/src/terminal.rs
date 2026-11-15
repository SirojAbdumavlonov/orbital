//! Interactive PTY shells over the existing SSH connection — no separate
//! Terminal.app / iTerm needed. Output is pushed to a sink (the UI channel).
//!
//! When the profile can use sudo, a terminal can open as root: the shell is
//! `sudo -i` (or `sudo -s` to stay in a chosen folder). With a saved sudo
//! password, sudo asks with a marker prompt; the reader hides the marker and
//! answers once with the password, which sudo reads with echo off, so it is
//! never shown on screen, sent through the UI, or logged.

use crate::error::Result;
use crate::profiles::SudoMode;
use crate::secrets::Secret;
use crate::shell::quote;
use crate::ssh::SshSession;
use russh::client::Msg;
use russh::{ChannelMsg, ChannelWriteHalf};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

pub enum TermEvent {
    Data(Vec<u8>),
    Exit(Option<u32>),
}

pub struct Terminal {
    pub id: String,
    pub cwd: Option<String>,
    /// The shell runs as root (through sudo).
    pub root: bool,
    writer: Arc<ChannelWriteHalf<Msg>>,
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

/// The prompt sudo prints when Orbital will answer it. Never shown to the user.
pub(crate) const SUDO_MARKER: &str = "[orbital:sudo]";
/// What the user sees instead, if they have to type the password themselves.
const SUDO_PROMPT: &str = "[sudo] password: ";

/// The command a terminal runs. `None` is the account's own login shell.
pub(crate) fn shell_command(cwd: Option<&str>, root: Option<SudoMode>) -> Option<String> {
    let cd = cwd.map(|d| format!("cd -- {} 2>/dev/null || cd; ", quote(d)));
    // -s keeps the folder for "Open terminal here"; -i is a full root login otherwise.
    let flag = if cwd.is_some() { "-s" } else { "-i" };
    match root {
        None => cd.map(|cd| format!("{cd}exec \"${{SHELL:-/bin/sh}}\" -l")),
        Some(SudoMode::Password) => Some(format!("{}exec sudo -p {} {flag}", cd.unwrap_or_default(), quote(SUDO_MARKER))),
        // Passwordless sudo: if it doesn't work after all, say so and stay as the user.
        Some(_) => Some(format!(
            "{}if sudo -n true 2>/dev/null; then exec sudo -n {flag}; fi; \
             printf '%s\\r\\n' 'Orbital: sudo needs a password here, so this shell runs as your own user.'; \
             exec \"${{SHELL:-/bin/sh}}\" -l",
            cd.unwrap_or_default()
        )),
    }
}

/// Holds back output until sudo's marker prompt appears (or it is clear none
/// will), so the marker is never displayed.
pub(crate) struct PromptFilter {
    pending: Vec<u8>,
    waiting: bool,
}

pub(crate) enum Filtered {
    /// Show this; keep waiting.
    Hold,
    /// Show this.
    Show(Vec<u8>),
    /// The marker was found: show this, and answer the prompt.
    Answer(Vec<u8>),
}

impl PromptFilter {
    pub(crate) fn new(waiting: bool) -> Self {
        Self {
            pending: Vec::new(),
            waiting,
        }
    }

    pub(crate) fn feed(&mut self, data: &[u8]) -> Filtered {
        if !self.waiting {
            return Filtered::Show(data.to_vec());
        }
        self.pending.extend_from_slice(data);
        let marker = SUDO_MARKER.as_bytes();
        if let Some(i) = self.pending.windows(marker.len()).position(|w| w == marker) {
            self.waiting = false;
            let mut out = self.pending[..i].to_vec();
            out.extend_from_slice(&self.pending[i + marker.len()..]);
            self.pending.clear();
            return Filtered::Answer(out);
        }
        // A part of the marker may still be on its way; anything long is not a prompt.
        if self.pending.len() > 4096 {
            return Filtered::Show(self.flush());
        }
        Filtered::Hold
    }

    /// Stop waiting (no prompt came) and release what was held back.
    pub(crate) fn flush(&mut self) -> Vec<u8> {
        self.waiting = false;
        std::mem::take(&mut self.pending)
    }

    /// Later prompts (a wrong password) are the user's to answer: show sudo's usual text.
    pub(crate) fn relabel(data: &[u8]) -> Vec<u8> {
        let marker = SUDO_MARKER.as_bytes();
        let mut out = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if data[i..].starts_with(marker) {
                out.extend_from_slice(SUDO_PROMPT.as_bytes());
                i += marker.len();
            } else {
                out.push(data[i]);
                i += 1;
            }
        }
        out
    }
}

impl SshSession {
    /// Whether terminals on this connection can open as root.
    pub fn terminal_root_available(&self) -> bool {
        self.elevation_needed()
    }

    pub async fn open_terminal<F>(&self, cols: u32, rows: u32, cwd: Option<String>, as_root: bool, sink: F) -> Result<Terminal>
    where
        F: Fn(TermEvent) + Send + Sync + 'static,
    {
        let root_mode = (as_root && self.elevation_needed()).then_some(self.profile.sudo_mode);
        // Logged in as root already: a plain shell is root.
        let root = root_mode.is_some() || self.profile.username == "root";
        let ch = self.handle().channel_open_session().await?;
        ch.request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[]).await?;
        ch.set_env(true, "COLORTERM", "truecolor").await.ok();
        match shell_command(cwd.as_deref(), root_mode) {
            Some(cmd) => ch.exec(true, cmd).await?,
            None => ch.request_shell(true).await?,
        }
        let (mut read, writer) = ch.split();
        let writer = Arc::new(writer);
        let answer: Option<Secret> = match root_mode {
            Some(SudoMode::Password) => self.sudo_password.clone(),
            _ => None,
        };
        let mut filter = PromptFilter::new(root_mode == Some(SudoMode::Password));
        let w = writer.clone();
        let reader_task = tokio::spawn(async move {
            let mut answer = answer;
            let mut code = None;
            loop {
                // While waiting for sudo's prompt, give up after a moment (cached
                // credentials or a NOPASSWD rule mean there won't be one).
                let msg = if filter.waiting {
                    match tokio::time::timeout(Duration::from_secs(3), read.wait()).await {
                        Ok(m) => m,
                        Err(_) => {
                            let held = filter.flush();
                            if !held.is_empty() {
                                sink(TermEvent::Data(held));
                            }
                            continue;
                        }
                    }
                } else {
                    read.wait().await
                };
                let Some(msg) = msg else { break };
                match msg {
                    ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => match filter.feed(&data) {
                        Filtered::Hold => {}
                        Filtered::Show(out) => sink(TermEvent::Data(PromptFilter::relabel(&out))),
                        Filtered::Answer(out) => {
                            if !out.is_empty() {
                                sink(TermEvent::Data(out));
                            }
                            match answer.take() {
                                Some(pw) => {
                                    let mut line = zeroize::Zeroizing::new(pw.as_bytes().to_vec());
                                    line.push(b'\n');
                                    let _ = w.data(&line[..]).await;
                                }
                                // No saved password: let the user type it.
                                None => sink(TermEvent::Data(SUDO_PROMPT.as_bytes().to_vec())),
                            }
                        }
                    },
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
            root,
            writer,
            reader_task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_commands() {
        assert_eq!(shell_command(None, None), None);
        assert_eq!(
            shell_command(Some("/srv/my app"), None).unwrap(),
            "cd -- '/srv/my app' 2>/dev/null || cd; exec \"${SHELL:-/bin/sh}\" -l"
        );
        assert_eq!(
            shell_command(None, Some(SudoMode::Password)).unwrap(),
            "exec sudo -p '[orbital:sudo]' -i"
        );
        assert_eq!(
            shell_command(Some("/etc"), Some(SudoMode::Password)).unwrap(),
            "cd -- /etc 2>/dev/null || cd; exec sudo -p '[orbital:sudo]' -s"
        );
        let nopw = shell_command(None, Some(SudoMode::NoPassword)).unwrap();
        assert!(nopw.starts_with("if sudo -n true 2>/dev/null; then exec sudo -n -i; fi;"));
        assert!(nopw.ends_with("exec \"${SHELL:-/bin/sh}\" -l"));
    }

    #[test]
    fn prompt_filter_hides_the_marker_even_when_split() {
        let mut f = PromptFilter::new(true);
        assert!(matches!(f.feed(b"Last login: today\r\n[orbital"), Filtered::Hold));
        match f.feed(b":sudo]") {
            Filtered::Answer(out) => assert_eq!(out, b"Last login: today\r\n"),
            _ => panic!("marker not found"),
        }
        // After answering, everything passes; a second prompt (wrong password) is relabelled.
        match f.feed(b"Sorry, try again.\r\n[orbital:sudo]") {
            Filtered::Show(out) => assert_eq!(PromptFilter::relabel(&out), b"Sorry, try again.\r\n[sudo] password: "),
            _ => panic!(),
        }
        // Not waiting at all: straight through.
        let mut plain = PromptFilter::new(false);
        assert!(matches!(plain.feed(b"$ "), Filtered::Show(_)));
        // A long output without a prompt is released.
        let mut long = PromptFilter::new(true);
        assert!(matches!(long.feed(&[b'x'; 5000]), Filtered::Show(v) if v.len() == 5000));
        let mut held = PromptFilter::new(true);
        assert!(matches!(held.feed(b"root@web:~# "), Filtered::Hold));
        assert_eq!(held.flush(), b"root@web:~# ");
    }
}
