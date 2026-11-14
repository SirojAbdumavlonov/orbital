//! File operations over SFTP.

use crate::error::{Error, Result};
use crate::shell::{join, quote, validate_path};
use crate::ssh::SshSession;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use serde::Serialize;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Files larger than this are not opened in the editor (download them instead).
pub const MAX_EDIT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// e.g. "rwxr-xr-x"
    pub mode: String,
    /// e.g. 0o644
    pub mode_octal: u32,
    pub mtime: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub size: u64,
    pub mtime: Option<u32>,
    pub mode_octal: u32,
    /// The current user cannot write this file via SFTP (sudo save offered).
    pub writable: bool,
}

pub fn mode_string(perm: u32) -> String {
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    bits.iter().map(|(b, c)| if perm & b != 0 { *c } else { '-' }).collect()
}

fn kind_of(a: &FileAttributes) -> EntryKind {
    if a.is_symlink() {
        EntryKind::Symlink
    } else if a.is_dir() {
        EntryKind::Dir
    } else if a.is_regular() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

impl SshSession {
    pub async fn sftp(&self) -> Result<Arc<SftpSession>> {
        let mut guard = self.sftp.lock().await;
        if let Some(s) = guard.as_ref() {
            return Ok(s.clone());
        }
        let ch = self.handle().channel_open_session().await?;
        ch.request_subsystem(true, "sftp").await?;
        let s = Arc::new(SftpSession::new(ch.into_stream()).await?);
        *guard = Some(s.clone());
        Ok(s)
    }

    pub async fn home_dir(&self) -> Result<String> {
        Ok(self.sftp().await?.canonicalize(".").await?)
    }

    pub async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        let mut out = Vec::new();
        for e in sftp.read_dir(path).await? {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let mut a = e.metadata();
            let mut kind = kind_of(&a);
            let full = join(path, &name);
            // Resolve symlinks to directories so they can be browsed into.
            if kind == EntryKind::Symlink {
                if let Ok(t) = sftp.metadata(full.clone()).await {
                    if t.is_dir() {
                        a = t;
                        kind = EntryKind::Dir;
                    }
                }
            }
            let perm = a.permissions.unwrap_or(0) & 0o7777;
            out.push(FileEntry {
                hidden: name.starts_with('.'),
                path: full,
                name,
                kind,
                size: a.size.unwrap_or(0),
                mode: mode_string(perm),
                mode_octal: perm,
                mtime: a.mtime,
                uid: a.uid,
                gid: a.gid,
                owner: a.user.clone(),
                group: a.group.clone(),
            });
        }
        out.sort_by(|a, b| {
            (b.kind == EntryKind::Dir)
                .cmp(&(a.kind == EntryKind::Dir))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    pub async fn stat(&self, path: &str) -> Result<FileAttributes> {
        validate_path(path)?;
        Ok(self.sftp().await?.metadata(path).await?)
    }

    /// Read a text file for the editor. Falls back to `sudo cat` when the file
    /// isn't readable by the login user (e.g. /etc/shadow-style configs).
    pub async fn read_text(&self, path: &str) -> Result<FileContent> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        let attrs = sftp.metadata(path).await?;
        if attrs.is_dir() {
            return Err(Error::InvalidInput(format!("{path} is a directory")));
        }
        let size = attrs.size.unwrap_or(0);
        if size > MAX_EDIT_BYTES {
            return Err(Error::InvalidInput(format!(
                "file is {} MB — too large to edit, download it instead",
                size / 1_048_576
            )));
        }
        let bytes = match sftp.read(path).await {
            Ok(b) => b,
            Err(e) => {
                let err: Error = e.into();
                if matches!(err, Error::PermissionDenied(_)) && self.elevation_needed() {
                    self.exec_privileged(&format!("cat -- {}", quote(path)))
                        .await?
                        .check()?
                        .stdout
                        .into_bytes()
                } else {
                    return Err(err);
                }
            }
        };
        if looks_binary(&bytes) {
            return Err(Error::InvalidInput("binary file — download it instead".into()));
        }
        let writable = self
            .exec(&format!("test -w {}", quote(path)))
            .await
            .map(|o| o.ok())
            .unwrap_or(false);
        Ok(FileContent {
            path: path.to_string(),
            content: String::from_utf8_lossy(&bytes).into_owned(),
            size,
            mtime: attrs.mtime,
            mode_octal: attrs.permissions.unwrap_or(0) & 0o7777,
            writable,
        })
    }

    /// Write a file in place (truncate + write), preserving inode, owner and mode
    /// — important for bind-mounted configs. If `expected_mtime` is set and the
    /// file changed on the server since it was opened, refuses with `Conflict`.
    pub async fn write_file(&self, path: &str, data: &[u8], expected_mtime: Option<u32>) -> Result<Option<u32>> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        if let Some(expected) = expected_mtime {
            if let Ok(a) = sftp.metadata(path).await {
                if a.mtime.is_some() && a.mtime != Some(expected) {
                    return Err(Error::Conflict { remote_mtime: a.mtime });
                }
            }
        }
        let mut f = sftp
            .open_with_flags(path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
            .await?;
        f.write_all(data).await?;
        f.flush().await?;
        f.shutdown().await?;
        Ok(sftp.metadata(path).await.ok().and_then(|a| a.mtime))
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        validate_path(path)?;
        Ok(self.sftp().await?.create_dir(path).await?)
    }

    pub async fn create_file(&self, path: &str) -> Result<()> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        if sftp.try_exists(path).await? {
            return Err(Error::InvalidInput(format!("{path} already exists")));
        }
        let mut f = sftp.create(path).await?;
        f.shutdown().await?;
        Ok(())
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        validate_path(from)?;
        validate_path(to)?;
        Ok(self.sftp().await?.rename(from, to).await?)
    }

    /// Delete a file, or a directory recursively (via `rm -rf`, quoted).
    pub async fn remove(&self, path: &str, recursive: bool) -> Result<()> {
        validate_path(path)?;
        if matches!(path.trim_end_matches('/'), "" | "/") {
            return Err(Error::InvalidInput("refusing to delete /".into()));
        }
        let sftp = self.sftp().await?;
        let a = sftp.symlink_metadata(path).await?;
        if a.is_dir() {
            if recursive {
                self.exec(&format!("rm -rf -- {}", quote(path))).await?.check()?;
            } else {
                sftp.remove_dir(path).await?;
            }
        } else {
            sftp.remove_file(path).await?;
        }
        Ok(())
    }

    pub async fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        let mut a = FileAttributes::empty();
        a.permissions = Some(mode & 0o7777);
        sftp.set_metadata(path, a).await?;
        Ok(())
    }

    /// Stream a remote file to a local path. Returns bytes copied.
    pub async fn download(&self, remote: &str, local: &std::path::Path) -> Result<u64> {
        validate_path(remote)?;
        let mut src = self.sftp().await?.open(remote).await?;
        let mut dst = tokio::fs::File::create(local).await?;
        let n = tokio::io::copy(&mut src, &mut dst).await?;
        dst.flush().await?;
        Ok(n)
    }

    /// Stream a local file to the server. Returns bytes copied.
    pub async fn upload(&self, local: &std::path::Path, remote: &str) -> Result<u64> {
        validate_path(remote)?;
        let mut src = tokio::fs::File::open(local).await?;
        let mut dst = self
            .sftp()
            .await?
            .open_with_flags(remote, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
            .await?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = src.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await?;
            total += n as u64;
        }
        dst.shutdown().await?;
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o640), "rw-r-----");
        assert!(looks_binary(&[0x7f, b'E', b'L', b'F', 0]));
        assert!(!looks_binary(b"server { listen 80; }"));
    }
}
