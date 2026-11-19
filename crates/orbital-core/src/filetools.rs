//! Files tab helpers that run a command on the server: finding files by name
//! below a folder, and downloading a whole folder as one .tar.gz streamed
//! straight into a local file (with progress, stoppable).

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::ssh::SshSession;
use russh::ChannelMsg;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

/// The most matches returned; the UI says when there were more.
pub const MAX_FOUND: usize = 200;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FoundFile {
    pub path: String,
    pub dir: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindReport {
    pub files: Vec<FoundFile>,
    /// More matched than [`MAX_FOUND`].
    pub truncated: bool,
}

/// The `find` command: names containing `query` (case-insensitive) below
/// `dir`, on the same filesystem, skipping .git and node_modules. Portable
/// (GNU and BSD find): no -printf.
pub fn find_command(dir: &str, query: &str) -> Result<String> {
    if !dir.starts_with('/') || dir.contains('\0') {
        return Err(Error::InvalidInput("search inside an absolute folder".into()));
    }
    let q = query.trim();
    if q.is_empty() || q.len() > 100 || q.contains('/') || q.contains('\0') {
        return Err(Error::InvalidInput("type part of a file or folder name (no /)".into()));
    }
    Ok(format!(
        "find {} -xdev \\( -name .git -o -name node_modules \\) -prune -o -iname {} \\( -type d -exec printf 'd\\t%s\\n' {{}} + -o -type f -exec printf 'f\\t%s\\n' {{}} + \\) 2>/dev/null | head -n {}",
        quote(dir),
        quote(&format!("*{q}*")),
        MAX_FOUND + 1
    ))
}

pub fn parse_find(out: &str) -> FindReport {
    let mut files: Vec<FoundFile> = out
        .lines()
        .filter_map(|l| {
            let (kind, path) = l.split_once('\t')?;
            (!path.is_empty()).then(|| FoundFile {
                path: path.to_string(),
                dir: kind == "d",
            })
        })
        .collect();
    let truncated = files.len() > MAX_FOUND;
    files.truncate(MAX_FOUND);
    // Shallow matches first, then by path.
    files.sort_by(|a, b| {
        a.path
            .matches('/')
            .count()
            .cmp(&b.path.matches('/').count())
            .then(a.path.cmp(&b.path))
    });
    FindReport { files, truncated }
}

/// `tar czf -` of `remote`, run from its parent so the archive holds the folder by name.
pub fn tar_command(remote: &str) -> Result<String> {
    crate::shell::validate_path(remote)?;
    let trimmed = remote.trim_end_matches('/');
    let (parent, name) = trimmed
        .rsplit_once('/')
        .ok_or_else(|| Error::InvalidInput("give an absolute folder".into()))?;
    if name.is_empty() {
        return Err(Error::InvalidInput(
            "the whole filesystem can't be downloaded as one archive".into(),
        ));
    }
    let parent = if parent.is_empty() { "/" } else { parent };
    Ok(format!("tar czf - -C {} -- {}", quote(parent), quote(name)))
}

impl SshSession {
    pub async fn find_files(&self, dir: &str, query: &str) -> Result<FindReport> {
        let out = self.exec(&find_command(dir, query)?).await?;
        Ok(parse_find(&out.stdout))
    }

    /// Stream `tar czf -` of a folder into `local`. Returns the archive's size.
    /// tar's exit code 1 ("some files changed while being read") still gives a
    /// usable archive; 2 or more doesn't.
    pub async fn download_folder(&self, remote: &str, local: &std::path::Path) -> Result<u64> {
        let cmd = tar_command(remote)?;
        let name = remote.trim_end_matches('/').rsplit('/').next().unwrap_or(remote).to_string();
        crate::progress::step(format!("Packing {remote} on the server and downloading it …"));
        let mut ch = self.handle().channel_open_session().await?;
        ch.exec(true, cmd).await?;
        ch.eof().await?;
        let mut file = tokio::fs::File::create(local).await?;
        let (mut total, mut err, mut code) = (0u64, Vec::new(), None);
        let mut last = std::time::Instant::now();
        loop {
            let msg = tokio::select! {
                m = ch.wait() => m,
                _ = crate::progress::cancelled() => {
                    let _ = ch.signal(russh::Sig::TERM).await;
                    let _ = ch.close().await;
                    drop(file);
                    let _ = tokio::fs::remove_file(local).await;
                    return Err(Error::Cancelled);
                }
            };
            let Some(msg) = msg else { break };
            match msg {
                ChannelMsg::Data { data } => {
                    file.write_all(&data).await?;
                    total += data.len() as u64;
                    if last.elapsed() >= std::time::Duration::from_millis(250) {
                        last = std::time::Instant::now();
                        crate::progress::transfer(1, total, &name);
                    }
                }
                ChannelMsg::ExtendedData { data, .. } if err.len() < 16_384 => err.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                _ => {}
            }
        }
        file.flush().await?;
        crate::progress::transfer(1, total, &name);
        match code {
            Some(0) | Some(1) if total > 0 => Ok(total),
            _ => {
                let _ = tokio::fs::remove_file(local).await;
                let why = String::from_utf8_lossy(&err).trim().lines().last().unwrap_or("").to_string();
                Err(Error::Remote {
                    code: code.map(|c| c as i32).unwrap_or(-1),
                    stderr: if why.is_empty() {
                        "tar couldn't pack the folder".into()
                    } else {
                        why
                    },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_command_quotes_and_refuses_bad_input() {
        let c = find_command("/srv/my app", "conf'ig").unwrap();
        assert!(c.starts_with("find '/srv/my app' -xdev"), "{c}");
        assert!(c.contains(r"-iname '*conf'\''ig*'"), "{c}");
        assert!(c.ends_with(&format!("head -n {}", MAX_FOUND + 1)));
        assert!(find_command("relative", "x").is_err());
        assert!(find_command("/", "").is_err());
        assert!(find_command("/", "a/b").is_err());
    }

    #[test]
    fn parses_matches_shallow_first() {
        let r = parse_find("f\t/srv/app/deep/nginx.conf\nd\t/srv/app/nginx\nf\t/srv/nginx.conf\nnoise\n");
        assert_eq!(
            r.files.iter().map(|f| (f.path.as_str(), f.dir)).collect::<Vec<_>>(),
            [
                ("/srv/nginx.conf", false),
                ("/srv/app/nginx", true),
                ("/srv/app/deep/nginx.conf", false)
            ]
        );
        assert!(!r.truncated);
        let many: String = (0..=MAX_FOUND).map(|i| format!("f\t/x/{i}\n")).collect();
        let r = parse_find(&many);
        assert!(r.truncated && r.files.len() == MAX_FOUND);
    }

    #[test]
    fn tar_runs_from_the_parent() {
        assert_eq!(tar_command("/var/www/site/").unwrap(), "tar czf - -C /var/www -- site");
        assert_eq!(tar_command("/srv").unwrap(), "tar czf - -C / -- srv");
        assert_eq!(tar_command("/home/me/my app").unwrap(), "tar czf - -C /home/me -- 'my app'");
        assert!(tar_command("/").is_err());
    }
}
