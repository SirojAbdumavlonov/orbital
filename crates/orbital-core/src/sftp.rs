//! File operations over SFTP.

use crate::error::{Error, Result};
use crate::shell::{join, quote, validate_path};
use crate::ssh::SshSession;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
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

/// What a recursive transfer did.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferReport {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Symlinks and special files are not followed or copied.
    pub skipped: u64,
}

/// Upper bound on entries in one recursive transfer (guards against runaway trees).
const MAX_TRANSFER_ENTRIES: u64 = 200_000;

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

/// SFTP or a shell command refused for lack of permission (a sudo retry may work).
pub(crate) fn denied(e: &Error) -> bool {
    match e {
        Error::PermissionDenied(_) => true,
        Error::Remote { stderr, .. } => {
            let t = stderr.to_lowercase();
            t.contains("permission denied") || t.contains("operation not permitted")
        }
        _ => false,
    }
}

/// One folder's entries as root: type, target type, mode, size, mtime, uid, gid, owner, group, name.
pub fn list_command(path: &str) -> String {
    format!(
        "find {} -mindepth 1 -maxdepth 1 -printf '%y\\t%Y\\t%m\\t%s\\t%T@\\t%U\\t%G\\t%u\\t%g\\t%f\\0'",
        quote(path)
    )
}

pub fn parse_listing(dir: &str, out: &str) -> Vec<FileEntry> {
    let mut v: Vec<FileEntry> = out
        .split('\0')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_start_matches('\n').splitn(10, '\t').collect();
            if f.len() != 10 || f[9].is_empty() || f[9] == "." || f[9] == ".." {
                return None;
            }
            let kind = match (f[0], f[1]) {
                ("d", _) | ("l", "d") => EntryKind::Dir,
                ("f", _) => EntryKind::File,
                ("l", _) => EntryKind::Symlink,
                _ => EntryKind::Other,
            };
            let perm = u32::from_str_radix(f[2], 8).unwrap_or(0) & 0o7777;
            let name = f[9].to_string();
            Some(FileEntry {
                hidden: name.starts_with('.'),
                path: join(dir, &name),
                kind,
                size: f[3].parse().unwrap_or(0),
                mode: mode_string(perm),
                mode_octal: perm,
                mtime: f[4].split('.').next().and_then(|t| t.parse().ok()),
                uid: f[5].parse().ok(),
                gid: f[6].parse().ok(),
                owner: Some(f[7].to_string()),
                group: Some(f[8].to_string()),
                name,
            })
        })
        .collect();
    sort_entries(&mut v);
    v
}

fn sort_entries(v: &mut [FileEntry]) {
    v.sort_by(|a, b| {
        (b.kind == EntryKind::Dir)
            .cmp(&(a.kind == EntryKind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// New empty file as root; refuses if something is already there.
pub fn create_command(path: &str) -> String {
    let q = quote(path);
    format!("if [ -e {q} ] || [ -L {q} ]; then echo 'already exists' >&2; exit 1; fi; : > {q}")
}

/// Rename as root; refuses to replace something already at `to`.
pub fn rename_command(from: &str, to: &str) -> String {
    let t = quote(to);
    format!(
        "if [ -e {t} ] || [ -L {t} ]; then echo 'already exists' >&2; exit 1; fi; mv -- {} {t}",
        quote(from)
    )
}

/// Copy a file or folder on the same server (paste in Files), keeping modes
/// and times and copying links as links; refused when the target exists.
pub fn copy_command(from: &str, to: &str) -> String {
    let t = quote(to);
    format!(
        "if [ -e {t} ] || [ -L {t} ]; then echo 'already exists' >&2; exit 1; fi; cp -pRP -- {} {t}",
        quote(from)
    )
}

/// Paths for a copy on one server: both absolute, different, and not a folder into itself.
pub fn check_copy(from: &str, to: &str) -> Result<()> {
    validate_path(from)?;
    validate_path(to)?;
    let f = from.trim_end_matches('/');
    let t = to.trim_end_matches('/');
    if f.is_empty() || f == t {
        return Err(Error::InvalidInput("choose a different name or folder for the copy".into()));
    }
    if t.starts_with(&format!("{f}/")) {
        return Err(Error::InvalidInput("a folder can't be copied into itself".into()));
    }
    Ok(())
}

/// Delete as root: a file or link, an empty folder, or a folder with everything in it.
pub fn remove_command(path: &str, recursive: bool) -> String {
    let q = quote(path);
    if recursive {
        format!("rm -rf -- {q}")
    } else {
        format!("if [ -d {q} ] && [ ! -L {q} ]; then rmdir -- {q}; else rm -f -- {q}; fi")
    }
}

/// Put a staged upload in place as root (merging into an existing folder).
pub fn place_command(staged: &str, remote: &str) -> String {
    let (s, r) = (quote(staged), quote(remote));
    format!("if [ -d {s} ]; then mkdir -p -- {r} && cp -R -- {s}/. {r}; else mkdir -p -- \"$(dirname -- {r})\" && cp -- {s} {r}; fi")
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
        let entries = match sftp.read_dir(path).await {
            Ok(v) => v,
            Err(e) => {
                let e: Error = e.into();
                if denied(&e) && self.elevation_needed() {
                    let o = self.exec_privileged(&list_command(path)).await?.check()?;
                    return Ok(parse_listing(path, &o.stdout));
                }
                return Err(e);
            }
        };
        for e in entries {
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
        sort_entries(&mut out);
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
        let attrs = match sftp.metadata(path).await {
            Ok(a) => a,
            Err(e) => {
                let e: Error = e.into();
                if !(denied(&e) && self.elevation_needed()) {
                    return Err(e);
                }
                // The folder isn't readable by the login user: size and time from root's view.
                let o = self
                    .exec_privileged(&format!("stat -c '%s %Y %a %F' -- {}", quote(path)))
                    .await?
                    .check()?;
                let mut it = o.stdout.split_whitespace();
                let mut a = FileAttributes::empty();
                a.size = it.next().and_then(|v| v.parse().ok());
                a.mtime = it.next().and_then(|v| v.parse().ok());
                a.permissions = it.next().and_then(|v| u32::from_str_radix(v, 8).ok());
                if o.stdout.contains("directory") {
                    return Err(Error::InvalidInput(format!("{path} is a directory")));
                }
                a
            }
        };
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
        match sftp
            .open_with_flags(path, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
            .await
        {
            Ok(mut f) => {
                f.write_all(data).await?;
                f.flush().await?;
                f.shutdown().await?;
            }
            Err(e) => {
                let e: Error = e.into();
                if !(denied(&e) && self.elevation_needed()) {
                    return Err(e);
                }
                // In place as root: same inode, owner and mode.
                self.exec_privileged_with(&format!("cat > {}", quote(path)), Some(data), std::time::Duration::from_secs(300))
                    .await?
                    .check()?;
            }
        }
        Ok(self.mtime_of(path).await)
    }

    /// A file's mtime, as root if the login user can't see it.
    async fn mtime_of(&self, path: &str) -> Option<u32> {
        if let Ok(a) = self.sftp().await.ok()?.metadata(path).await {
            return a.mtime;
        }
        let o = self.exec_privileged(&format!("stat -c %Y -- {}", quote(path))).await.ok()?;
        o.stdout.trim().parse().ok()
    }

    /// Run `cmd` as root (after SFTP was refused), failing with its own message.
    async fn as_root(&self, cmd: &str) -> Result<()> {
        self.exec_privileged(cmd).await?.check()?;
        Ok(())
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        validate_path(path)?;
        match self.sftp().await?.create_dir(path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let e: Error = e.into();
                if denied(&e) && self.elevation_needed() {
                    self.as_root(&format!("mkdir -- {}", quote(path))).await
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn create_file(&self, path: &str) -> Result<()> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        let tried = async {
            if sftp.try_exists(path).await? {
                return Err(Error::InvalidInput(format!("{path} already exists")));
            }
            let mut f = sftp.create(path).await?;
            f.shutdown().await?;
            Ok(())
        }
        .await;
        match tried {
            Err(e) if denied(&e) && self.elevation_needed() => self.as_root(&create_command(path)).await,
            r => r,
        }
    }

    /// Copy within this server (`cp`), with sudo when the login user is refused.
    /// Up to an hour, for big folders.
    pub async fn copy_within(&self, from: &str, to: &str) -> Result<()> {
        check_copy(from, to)?;
        let cmd = copy_command(from, to);
        let limit = Duration::from_secs(3600);
        match self.exec_with_input(&cmd, None, limit).await?.check() {
            Ok(_) => Ok(()),
            Err(e) if denied(&e) && self.elevation_needed() => {
                self.exec_privileged_with(&cmd, None, limit).await?.check()?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        validate_path(from)?;
        validate_path(to)?;
        match self.sftp().await?.rename(from, to).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let e: Error = e.into();
                if denied(&e) && self.elevation_needed() {
                    self.as_root(&rename_command(from, to)).await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Delete a file, or a directory recursively (via `rm -rf`, quoted).
    pub async fn remove(&self, path: &str, recursive: bool) -> Result<()> {
        validate_path(path)?;
        if matches!(path.trim_end_matches('/'), "" | "/") {
            return Err(Error::InvalidInput("refusing to delete /".into()));
        }
        let sftp = self.sftp().await?;
        let tried = async {
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
        .await;
        match tried {
            Err(e) if denied(&e) && self.elevation_needed() => self.as_root(&remove_command(path, recursive)).await,
            r => r,
        }
    }

    pub async fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        validate_path(path)?;
        let sftp = self.sftp().await?;
        let mut a = FileAttributes::empty();
        a.permissions = Some(mode & 0o7777);
        match sftp.set_metadata(path, a).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let e: Error = e.into();
                if denied(&e) && self.elevation_needed() {
                    self.as_root(&format!("chmod {:o} -- {}", mode & 0o7777, quote(path))).await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Stream a remote file to a local path. Returns bytes copied.
    pub async fn download(&self, remote: &str, local: &std::path::Path) -> Result<u64> {
        validate_path(remote)?;
        let mut src = match self.sftp().await?.open(remote).await {
            Ok(f) => f,
            Err(e) => {
                let e: Error = e.into();
                if !(denied(&e) && self.elevation_needed()) {
                    return Err(e);
                }
                let name = remote.rsplit('/').next().unwrap_or(remote);
                let (n, code, err) = self.stream_to_file(&format!("cat -- {}", quote(remote)), true, local, name).await?;
                if code != Some(0) {
                    let _ = tokio::fs::remove_file(local).await;
                    return Err(crate::filetools::stream_error(code, &err, "couldn't read the file"));
                }
                return Ok(n);
            }
        };
        let mut dst = tokio::fs::File::create(local).await?;
        let n = tokio::io::copy(&mut src, &mut dst).await?;
        dst.flush().await?;
        Ok(n)
    }

    /// Like [`copy_to`], into a place the destination login can't write:
    /// staged in /tmp on `dst`, then put in place as root there.
    pub async fn copy_to_as_root(&self, src_path: &str, dst: &SshSession, dst_path: &str, overwrite: bool) -> Result<TransferReport> {
        validate_path(dst_path)?;
        if !overwrite && dst.exec_privileged(&format!("[ -e {} ]", quote(dst_path))).await?.ok() {
            return Err(Error::InvalidInput(format!("{dst_path} already exists on {}", dst.label())));
        }
        let o = dst.exec("mktemp -d /tmp/orbital-copy-XXXXXX").await?.check()?;
        let tmp_dir = o.stdout.trim().to_string();
        validate_path(&tmp_dir)?;
        let name = dst_path.trim_end_matches('/').rsplit('/').next().unwrap_or("copy");
        let staged = join(&tmp_dir, name);
        let r = async {
            let rep = self.copy_to(src_path, dst, &staged, true).await?;
            crate::progress::step(format!("Putting it in place with sudo: {dst_path} …"));
            dst.as_root(&place_command(&staged, dst_path)).await?;
            Ok(rep)
        }
        .await;
        let _ = dst.exec(&format!("rm -rf -- {}", quote(&tmp_dir))).await;
        r
    }

    /// Upload into a place the login user can't write: to a private folder in
    /// /tmp first, then copied into place as root (owned by root, like any
    /// file made with sudo) and the temporary copy removed.
    pub async fn upload_as_root(&self, local: &std::path::Path, remote: &str) -> Result<TransferReport> {
        validate_path(remote)?;
        let o = self.exec("mktemp -d /tmp/orbital-upload-XXXXXX").await?.check()?;
        let tmp_dir = o.stdout.trim().to_string();
        validate_path(&tmp_dir)?;
        let name = remote.trim_end_matches('/').rsplit('/').next().unwrap_or("upload");
        let staged = join(&tmp_dir, name);
        let r = async {
            let rep = self.upload_tree(local, &staged).await?;
            crate::progress::step(format!("Putting it in place with sudo: {remote} …"));
            self.as_root(&place_command(&staged, remote)).await?;
            Ok(rep)
        }
        .await;
        let _ = self.exec(&format!("rm -rf -- {}", quote(&tmp_dir))).await;
        r
    }

    /// Upload a local file, or a folder with everything in it, to `remote`.
    /// Symlinks inside a folder are skipped rather than followed.
    pub async fn upload_tree(&self, local: &std::path::Path, remote: &str) -> Result<TransferReport> {
        validate_path(remote)?;
        let mut report = TransferReport::default();
        if !tokio::fs::metadata(local).await?.is_dir() {
            report.bytes = self.upload(local, remote).await?;
            report.files = 1;
            return Ok(report);
        }
        let mut meter = Meter::default();
        crate::progress::step(format!("Uploading {} …", local.display()));
        let sftp = self.sftp().await?;
        let mut stack = vec![(local.to_path_buf(), remote.to_string())];
        while let Some((src_dir, dst_dir)) = stack.pop() {
            ensure_dir(&sftp, &dst_dir).await?;
            report.dirs += 1;
            let mut rd = tokio::fs::read_dir(&src_dir).await?;
            while let Some(entry) = rd.next_entry().await? {
                if report.files + report.dirs + report.skipped >= MAX_TRANSFER_ENTRIES {
                    return Err(Error::InvalidInput(format!(
                        "more than {MAX_TRANSFER_ENTRIES} items — transfer stopped"
                    )));
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let dst = join(&dst_dir, &name);
                validate_path(&dst)?;
                // `file_type` does not follow symlinks.
                let ft = entry.file_type().await?;
                if ft.is_dir() {
                    stack.push((entry.path(), dst));
                } else if ft.is_file() {
                    // New files get the local permission bits (scripts stay
                    // executable); a file that is overwritten keeps its own.
                    let is_new = !sftp.try_exists(&dst).await?;
                    report.bytes += self.upload_metered(&entry.path(), &dst, &mut meter).await?;
                    if is_new {
                        set_mode(&sftp, &dst, local_mode(&entry.metadata().await?)).await;
                    }
                    report.files += 1;
                } else {
                    report.skipped += 1;
                }
            }
        }
        meter.finish(remote);
        Ok(report)
    }

    /// Copy a file or folder from this server to `dst_path` on another
    /// connected server. Data is streamed through this Mac over the two
    /// existing SFTP sessions: the servers never talk to each other, and no
    /// credentials or keys are sent to either. File modes are carried over;
    /// ownership is whoever the destination login is. Symlinks and special
    /// files are skipped.
    pub async fn copy_to(&self, src_path: &str, dst: &SshSession, dst_path: &str, overwrite: bool) -> Result<TransferReport> {
        validate_path(src_path)?;
        validate_path(dst_path)?;
        let from = self.sftp().await?;
        let to = dst.sftp().await?;
        if !overwrite && to.try_exists(dst_path).await? {
            return Err(Error::InvalidInput(format!("{dst_path} already exists on {}", dst.label())));
        }
        let mut report = TransferReport::default();
        // Follows a symlink given as the source itself (the user picked it).
        let top = from.metadata(src_path).await?;
        if !top.is_dir() {
            let mut meter = Meter::default();
            report.bytes = copy_file(&from, src_path, &to, dst_path, top.permissions, &mut meter).await?;
            report.files = 1;
            meter.finish(src_path);
            return Ok(report);
        }
        let mut meter = Meter::default();
        crate::progress::step(format!("Copying {src_path} to {} …", dst.label()));
        let mut stack = vec![(src_path.to_string(), dst_path.to_string(), top.permissions)];
        while let Some((src_dir, dst_dir, perm)) = stack.pop() {
            ensure_dir(&to, &dst_dir).await?;
            // Keep the folder writable for us so its contents can be created.
            set_mode(&to, &dst_dir, perm.map(|p| p | 0o700)).await;
            report.dirs += 1;
            for e in from.read_dir(&src_dir).await? {
                let name = e.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                if report.files + report.dirs + report.skipped >= MAX_TRANSFER_ENTRIES {
                    return Err(Error::InvalidInput(format!(
                        "more than {MAX_TRANSFER_ENTRIES} items — transfer stopped"
                    )));
                }
                let (src, dst_child) = (join(&src_dir, &name), join(&dst_dir, &name));
                validate_path(&dst_child)?;
                // read_dir reports the link itself, so symlinks are not followed.
                let a = e.metadata();
                match kind_of(&a) {
                    EntryKind::Dir => stack.push((src, dst_child, a.permissions)),
                    EntryKind::File => {
                        report.bytes += copy_file(&from, &src, &to, &dst_child, a.permissions, &mut meter).await?;
                        report.files += 1;
                    }
                    _ => report.skipped += 1,
                }
            }
        }
        meter.finish(src_path);
        Ok(report)
    }

    /// Stream a local file to the server. Returns bytes copied.
    pub async fn upload(&self, local: &std::path::Path, remote: &str) -> Result<u64> {
        let mut meter = Meter::default();
        let n = self.upload_metered(local, remote, &mut meter).await?;
        meter.finish(remote);
        Ok(n)
    }

    async fn upload_metered(&self, local: &std::path::Path, remote: &str, meter: &mut Meter) -> Result<u64> {
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
            crate::progress::check()?;
            dst.write_all(&buf[..n]).await?;
            total += n as u64;
            meter.add(n as u64, remote);
        }
        dst.shutdown().await?;
        meter.file_done(remote);
        Ok(total)
    }
}

/// Create `path` as a directory unless it already is one.
async fn ensure_dir(sftp: &SftpSession, path: &str) -> Result<()> {
    match sftp.metadata(path).await {
        Ok(a) if a.is_dir() => Ok(()),
        Ok(_) => Err(Error::InvalidInput(format!("{path} exists and is not a folder"))),
        Err(_) => Ok(sftp.create_dir(path).await?),
    }
}

#[cfg(unix)]
fn local_mode(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(m.permissions().mode())
}

#[cfg(not(unix))]
fn local_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Best effort: apply the source's permission bits (never setuid/setgid/sticky).
async fn set_mode(sftp: &SftpSession, path: &str, perm: Option<u32>) {
    if let Some(p) = perm {
        let mut a = FileAttributes::empty();
        a.permissions = Some(p & 0o777);
        let _ = sftp.set_metadata(path, a).await;
    }
}

async fn copy_file(from: &SftpSession, src: &str, to: &SftpSession, dst: &str, perm: Option<u32>, meter: &mut Meter) -> Result<u64> {
    let mut reader = from.open(src).await?;
    let mut writer = to
        .open_with_flags(dst, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE)
        .await?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        crate::progress::check()?;
        writer.write_all(&buf[..n]).await?;
        total += n as u64;
        meter.add(n as u64, src);
    }
    writer.shutdown().await?;
    set_mode(to, dst, perm).await;
    meter.file_done(src);
    Ok(total)
}

/// A transfer's progress for the job that runs it (see `progress`): files
/// and bytes so far, reported at most four times a second.
#[derive(Default)]
struct Meter {
    files: u64,
    bytes: u64,
    last: Option<std::time::Instant>,
}

impl Meter {
    fn add(&mut self, n: u64, path: &str) {
        self.bytes += n;
        self.tick(path, false);
    }

    fn file_done(&mut self, path: &str) {
        self.files += 1;
        self.tick(path, false);
    }

    fn finish(&mut self, path: &str) {
        self.tick(path, true);
    }

    fn tick(&mut self, path: &str, force: bool) {
        if !crate::progress::active() {
            return;
        }
        let now = std::time::Instant::now();
        if force
            || self
                .last
                .is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_millis(250))
        {
            self.last = Some(now);
            crate::progress::transfer(self.files, self.bytes, path);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_fallback_commands_and_listing() {
        use super::*;
        assert!(denied(&Error::PermissionDenied("x".into())));
        assert!(denied(&Error::Remote {
            code: 1,
            stderr: "rm: cannot remove 'x': Permission denied".into()
        }));
        assert!(!denied(&Error::NotFound("x".into())));
        assert_eq!(
            list_command("/opt/my app"),
            r"find '/opt/my app' -mindepth 1 -maxdepth 1 -printf '%y\t%Y\t%m\t%s\t%T@\t%U\t%G\t%u\t%g\t%f\0'"
        );
        let out = "f\tf\t644\t12\t1763700000.5\t0\t0\troot\troot\tb.conf\0d\td\t755\t4096\t1763700001.0\t0\t0\troot\troot\tsites\0l\td\t777\t9\t1763700002.0\t0\t0\troot\troot\tlink\0l\tf\t777\t9\t1\t0\t0\troot\troot\t.hidden\0";
        let v = parse_listing("/etc/x", out);
        let names: Vec<_> = v.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            names,
            [
                ("link", EntryKind::Dir),
                ("sites", EntryKind::Dir),
                (".hidden", EntryKind::Symlink),
                ("b.conf", EntryKind::File)
            ]
        );
        let b = v.iter().find(|e| e.name == "b.conf").unwrap();
        assert_eq!(
            (b.path.as_str(), b.size, b.mode.as_str(), b.mtime, b.owner.as_deref()),
            ("/etc/x/b.conf", 12, "rw-r--r--", Some(1763700000), Some("root"))
        );
        assert!(v[2].hidden);
        assert_eq!(
            create_command("/opt/a b"),
            "if [ -e '/opt/a b' ] || [ -L '/opt/a b' ]; then echo 'already exists' >&2; exit 1; fi; : > '/opt/a b'"
        );
        assert!(rename_command("/a", "/b").ends_with("mv -- /a /b"));
        assert!(copy_command("/a b", "/c").ends_with("cp -pRP -- '/a b' /c"));
        assert!(copy_command("/a", "/c d").starts_with("if [ -e '/c d' ]"));
        assert!(check_copy("/etc/nginx", "/etc/nginx copy").is_ok());
        assert!(check_copy("/srv/app", "/srv/app/app").is_err(), "into itself");
        assert!(check_copy("/srv/app", "/srv/app/").is_err(), "same path");
        assert!(check_copy("srv", "/x").is_err(), "relative");
        assert!(
            check_copy("/srv/app", "/srv/application").is_ok(),
            "a sibling that only shares a prefix"
        );
        assert_eq!(remove_command("/a", true), "rm -rf -- /a");
        assert!(remove_command("/a", false).contains("rmdir -- /a; else rm -f -- /a"));
        assert!(place_command("/tmp/x/s", "/opt/s").contains("cp -R -- /tmp/x/s/. /opt/s"));
        // Every command is valid sh.
        for c in [
            list_command("/x"),
            create_command("/x"),
            rename_command("/x", "/y"),
            copy_command("/x", "/y"),
            remove_command("/x", false),
            place_command("/a", "/b"),
        ] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&c).output().unwrap();
            assert!(o.status.success(), "{c}");
        }
    }

    use super::*;
    #[test]
    fn modes() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o640), "rw-r-----");
        assert!(looks_binary(&[0x7f, b'E', b'L', b'F', 0]));
        assert!(!looks_binary(b"server { listen 80; }"));
    }
}
