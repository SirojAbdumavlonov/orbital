//! Live progress for long operations ("jobs"): renewing a certificate,
//! copying a folder between servers, creating a website, `docker compose
//! pull`. The UI shows what is happening while it happens, instead of a
//! spinner for a minute.
//!
//! A caller (the Tauri command) runs the operation inside [`scope`] with a
//! sink. Engine code anywhere below it reports with [`step`] (what Orbital is
//! doing, in words) and [`transfer`] (files and bytes so far), without the sink
//! being threaded through every signature. Commands started inside
//! [`with_output`] also stream each line they print (`SshSession::exec_with_input`
//! does that); other commands, such as the inspection that precedes a change,
//! stay quiet so the log isn't flooded with `nginx -T`.
//!
//! Outside a scope every call here does nothing. The sink only ever sees text
//! a command printed or Orbital wrote: never stdin, so never a sudo password,
//! and lines that contain the session's sudo password are redacted anyway.

use serde::Serialize;
use std::future::Future;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProgressEvent {
    /// What Orbital is doing now, in words ("Asking Let's Encrypt for a certificate…").
    Step { text: String },
    /// A line a command printed.
    Line { text: String },
    /// A transfer so far: files and bytes done, and the file being copied.
    Transfer { files: u64, bytes: u64, current: String },
}

pub type Sink = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

tokio::task_local! {
    static SINK: Sink;
    static LIVE: bool;
}

/// Run `fut` with `sink` receiving its progress; without a sink, just run it.
pub async fn scope<F: Future>(sink: Option<Sink>, fut: F) -> F::Output {
    match sink {
        Some(s) => SINK.scope(s, fut).await,
        None => fut.await,
    }
}

/// Run `fut` so that the commands it starts stream their output lines.
pub async fn with_output<F: Future>(fut: F) -> F::Output {
    LIVE.scope(true, fut).await
}

fn emit(ev: ProgressEvent) {
    let _ = SINK.try_with(|s| s(ev));
}

/// Whether anyone is listening (a job's progress is shown).
pub fn active() -> bool {
    SINK.try_with(|_| ()).is_ok()
}

/// Say what is happening now.
pub fn step(text: impl Into<String>) {
    if active() {
        emit(ProgressEvent::Step { text: text.into() });
    }
}

/// Report a transfer's progress.
pub fn transfer(files: u64, bytes: u64, current: &str) {
    if active() {
        emit(ProgressEvent::Transfer {
            files,
            bytes,
            current: current.to_string(),
        });
    }
}

/// Whether commands started now should stream their output lines.
pub(crate) fn live() -> bool {
    LIVE.try_with(|v| *v).unwrap_or(false) && active()
}

/// The longest line passed on; longer ones are cut (progress bars, minified JSON).
const MAX_LINE: usize = 300;

/// Turns a command's output, as it arrives in pieces, into clean lines for
/// [`ProgressEvent::Line`]. `\r` ends a line too, so progress bars that
/// redraw themselves (`docker pull`, `apt`) show their latest state.
pub(crate) struct Lines {
    buf: Vec<u8>,
    redact: Option<String>,
}

impl Lines {
    pub(crate) fn new(redact: Option<&str>) -> Self {
        Self {
            buf: Vec::new(),
            redact: redact.filter(|p| !p.is_empty()).map(String::from),
        }
    }

    pub(crate) fn push(&mut self, data: &[u8]) {
        for &b in data {
            if b == b'\n' || b == b'\r' {
                self.flush();
            } else if self.buf.len() < MAX_LINE * 4 {
                self.buf.push(b);
            }
        }
    }

    pub(crate) fn finish(&mut self) {
        self.flush();
    }

    fn flush(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let raw = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        if let Some(text) = clean(&raw, self.redact.as_deref()) {
            emit(ProgressEvent::Line { text });
        }
    }
}

/// Strip terminal escape sequences and control characters, trim, cut long
/// lines, hide the password. `None` for a line with nothing left to show.
pub(crate) fn clean(raw: &str, redact: Option<&str>) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI (ESC [ … final byte) and two-character escapes.
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else if c == '\t' {
            out.push(' ');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    let mut text = out.trim().to_string();
    if let Some(pw) = redact {
        if text.contains(pw) {
            text = text.replace(pw, "••••");
        }
    }
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > MAX_LINE {
        text = text.chars().take(MAX_LINE).collect::<String>() + "…";
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn collect() -> (Sink, Arc<Mutex<Vec<ProgressEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (Arc::new(move |e| s.lock().unwrap().push(e)), seen)
    }

    #[test]
    fn cleans_escapes_controls_and_passwords() {
        assert_eq!(clean("\u{1b}[1;32mSuccess\u{1b}[0m", None).as_deref(), Some("Success"));
        assert_eq!(clean("  a\tb  ", None).as_deref(), Some("a b"));
        assert_eq!(clean("\u{7}\u{1b}[2K", None), None);
        assert_eq!(clean("password is hunter2!", Some("hunter2")).as_deref(), Some("password is ••••!"));
        let long = "x".repeat(400);
        assert_eq!(clean(&long, None).unwrap().chars().count(), MAX_LINE + 1);
    }

    #[tokio::test]
    async fn lines_split_on_newlines_and_carriage_returns() {
        let (sink, seen) = collect();
        scope(Some(sink), async {
            let mut l = Lines::new(None);
            l.push(b"Pulling web ... ");
            l.push(b"10%\rPulling web ... done\n\nSaving debug log to /var/log/letsencrypt");
            l.finish();
        })
        .await;
        let lines: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|e| match e {
                ProgressEvent::Line { text } => text.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            lines,
            [
                "Pulling web ... 10%",
                "Pulling web ... done",
                "Saving debug log to /var/log/letsencrypt"
            ]
        );
    }

    #[tokio::test]
    async fn quiet_outside_a_scope_and_output_only_inside_with_output() {
        // No scope: nothing is listening and nothing panics.
        assert!(!active());
        step("ignored");
        transfer(1, 2, "/x");

        let (sink, seen) = collect();
        scope(Some(sink), async {
            assert!(active());
            assert!(!live(), "commands stay quiet unless asked");
            step("Renewing example.com…");
            with_output(async { assert!(live()) }).await;
            transfer(3, 4096, "/srv/a.txt");
        })
        .await;
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ProgressEvent::Step {
                    text: "Renewing example.com…".into()
                },
                ProgressEvent::Transfer {
                    files: 3,
                    bytes: 4096,
                    current: "/srv/a.txt".into()
                },
            ]
        );
    }
}
