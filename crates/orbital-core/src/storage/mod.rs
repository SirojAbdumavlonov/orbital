//! File connections other than SFTP: FTP and FTPS, WebDAV (HTTP/HTTPS), and
//! S3 with every S3-compatible service. They offer the Files tab only; the
//! server tools (services, terminal, …) need SSH.
//!
//! All paths the UI sees look like Unix paths starting at `/`. For S3 the
//! first segment is the bucket (`/` lists buckets). Credentials come from the
//! profile's saved password (the S3 secret key) and are never logged.

pub mod dav;
pub mod ftp;
pub mod s3;

use crate::error::{Error, Result};
use crate::profiles::{Profile, Protocol};
use crate::secrets::Secret;
use crate::sftp::{EntryKind, FileContent, FileEntry, MAX_EDIT_BYTES};
use std::path::Path;

pub(crate) enum Client {
    Ftp(ftp::FtpClient),
    Dav(dav::DavClient),
    S3(s3::S3Client),
}

pub struct StorageSession {
    pub id: String,
    pub profile: Profile,
    pub home: String,
    client: Client,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

/// A file listing entry with the fields these protocols can tell us.
pub(crate) fn entry(dir: &str, name: &str, is_dir: bool, size: u64, mtime: Option<u32>) -> FileEntry {
    FileEntry {
        path: join(dir, name),
        name: name.to_string(),
        kind: if is_dir { EntryKind::Dir } else { EntryKind::File },
        size,
        mode: String::new(),
        mode_octal: 0,
        mtime,
        uid: None,
        gid: None,
        owner: None,
        group: None,
        hidden: name.starts_with('.'),
    }
}

pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Normalise a UI path: absolute, no `.`/`..`, no trailing slash (except `/`).
pub fn clean(path: &str) -> Result<String> {
    if !path.starts_with('/') || path.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidInput(format!("invalid path: {path:?}")));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

pub(crate) fn sort(v: &mut [FileEntry]) {
    v.sort_by(|a, b| {
        (b.kind == EntryKind::Dir)
            .cmp(&(a.kind == EntryKind::Dir))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

impl StorageSession {
    pub async fn connect(profile: Profile, secret: Option<Secret>) -> Result<Self> {
        let client = match profile.protocol {
            Protocol::Ftp | Protocol::Ftps => Client::Ftp(ftp::FtpClient::connect(&profile, secret).await?),
            Protocol::Webdav | Protocol::Webdavs => Client::Dav(dav::DavClient::connect(&profile, secret).await?),
            Protocol::S3 => Client::S3(s3::S3Client::connect(&profile, secret).await?),
            Protocol::Sftp => return Err(Error::InvalidInput("SFTP connects over SSH".into())),
        };
        let home = match &client {
            Client::Ftp(c) => c.home().await,
            // For S3 the profile's path is a bucket (and maybe a folder in it);
            // for WebDAV it is the server's WebDAV folder, which is shown as `/`.
            Client::S3(_) => profile
                .initial_path
                .as_deref()
                .map(|p| format!("/{}", p.trim().trim_matches('/')))
                .unwrap_or_else(|| "/".into()),
            Client::Dav(_) => "/".into(),
        };
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            home: clean(&home).unwrap_or_else(|_| "/".into()),
            profile,
            client,
            connected_at: chrono::Utc::now(),
        })
    }

    pub fn label(&self) -> String {
        let scheme = match self.profile.protocol {
            Protocol::Ftp => "ftp",
            Protocol::Ftps => "ftps",
            Protocol::Webdav => "http",
            Protocol::Webdavs => "https",
            Protocol::S3 => "s3",
            Protocol::Sftp => "sftp",
        };
        format!("{scheme}://{}@{}", self.profile.username, self.profile.host)
    }

    pub async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>> {
        let path = clean(path)?;
        let mut v = match &self.client {
            Client::Ftp(c) => c.list(&path).await?,
            Client::Dav(c) => c.list(&path).await?,
            Client::S3(c) => c.list(&path).await?,
        };
        sort(&mut v);
        Ok(v)
    }

    async fn read_bytes(&self, path: &str, limit: u64) -> Result<Vec<u8>> {
        match &self.client {
            Client::Ftp(c) => c.read(path, limit).await,
            Client::Dav(c) => c.read(path, limit).await,
            Client::S3(c) => c.read(path, limit).await,
        }
    }

    pub async fn read_text(&self, path: &str) -> Result<FileContent> {
        let path = clean(path)?;
        let data = self.read_bytes(&path, MAX_EDIT_BYTES).await?;
        let content =
            String::from_utf8(data).map_err(|_| Error::InvalidInput("this file isn't text, so it can't be edited here".into()))?;
        Ok(FileContent {
            size: content.len() as u64,
            path,
            content,
            mtime: None,
            mode_octal: 0o644,
            writable: true,
        })
    }

    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<()> {
        let path = clean(path)?;
        match &self.client {
            Client::Ftp(c) => c.write(&path, data).await,
            Client::Dav(c) => c.write(&path, data).await,
            Client::S3(c) => c.write(&path, data).await,
        }
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        let path = clean(path)?;
        match &self.client {
            Client::Ftp(c) => c.mkdir(&path).await,
            Client::Dav(c) => c.mkdir(&path).await,
            Client::S3(c) => c.mkdir(&path).await,
        }
    }

    pub async fn remove(&self, path: &str, recursive: bool) -> Result<()> {
        let path = clean(path)?;
        if path == "/" {
            return Err(Error::InvalidInput("the top folder can't be deleted".into()));
        }
        match &self.client {
            Client::Ftp(c) => c.remove(&path, recursive).await,
            Client::Dav(c) => c.remove(&path).await,
            Client::S3(c) => c.remove(&path, recursive).await,
        }
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let (from, to) = (clean(from)?, clean(to)?);
        match &self.client {
            Client::Ftp(c) => c.rename(&from, &to).await,
            Client::Dav(c) => c.rename(&from, &to).await,
            Client::S3(c) => c.rename(&from, &to).await,
        }
    }

    /// Download a file to `local`. Returns bytes written.
    pub async fn download(&self, remote: &str, local: &Path) -> Result<u64> {
        let remote = clean(remote)?;
        match &self.client {
            Client::Ftp(c) => c.download(&remote, local).await,
            Client::Dav(c) => c.download(&remote, local).await,
            Client::S3(c) => c.download(&remote, local).await,
        }
    }

    /// Upload a file, or a folder with everything in it. Returns bytes sent.
    pub async fn upload(&self, local: &Path, remote: &str) -> Result<u64> {
        let remote = clean(remote)?;
        let meta = tokio::fs::symlink_metadata(local).await?;
        if meta.is_dir() {
            self.mkdir(&remote).await.ok();
            let mut total = 0;
            let mut dir = tokio::fs::read_dir(local).await?;
            while let Some(e) = dir.next_entry().await? {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().await?.is_symlink() {
                    continue;
                }
                total += Box::pin(self.upload(&e.path(), &join(&remote, &name))).await?;
            }
            return Ok(total);
        }
        match &self.client {
            Client::Ftp(c) => c.upload(local, &remote).await,
            Client::Dav(c) => c.upload(local, &remote).await,
            Client::S3(c) => c.upload(local, &remote).await,
        }
    }

    pub async fn close(&self) {
        if let Client::Ftp(c) = &self.client {
            c.quit().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_paths() {
        assert_eq!(clean("/").unwrap(), "/");
        assert_eq!(clean("/a/b/").unwrap(), "/a/b");
        assert_eq!(clean("/a/./b/../c").unwrap(), "/a/c");
        assert_eq!(clean("/../../etc").unwrap(), "/etc");
        assert!(clean("relative").is_err());
        assert!(clean("/a\nb").is_err());
        assert_eq!(join("/", "x"), "/x");
        assert_eq!(join("/a", "x"), "/a/x");
    }
}
