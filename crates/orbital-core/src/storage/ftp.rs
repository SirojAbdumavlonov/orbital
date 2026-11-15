//! FTP and FTPS (explicit TLS, `AUTH TLS`). Passive mode, with the usual
//! workaround for servers behind NAT that announce a private address.

use super::entry;
use crate::error::{Error, Result};
use crate::profiles::{Profile, Protocol};
use crate::secrets::Secret;
use crate::sftp::FileEntry;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use suppaftp::list::ListParser;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::FtpError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

pub struct FtpClient {
    ftp: Mutex<AsyncRustlsFtpStream>,
    home: String,
}

fn err(e: FtpError) -> Error {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("530") || lower.contains("login") && lower.contains("incorrect") {
        Error::Other(format!("the FTP server refused the login: {msg}"))
    } else if lower.contains("550") || lower.contains("permission denied") {
        Error::PermissionDenied(msg)
    } else {
        Error::Other(format!("FTP: {msg}"))
    }
}

/// TLS that trusts the public certificate authorities (as browsers do).
pub(crate) fn tls_connector() -> AsyncRustlsConnector {
    use tokio_rustls::rustls;
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    AsyncRustlsConnector::from(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

impl FtpClient {
    pub async fn connect(p: &Profile, secret: Option<Secret>) -> Result<Self> {
        let addr = format!("{}:{}", p.host, p.port);
        let mut ftp = tokio::time::timeout(Duration::from_secs(15), AsyncRustlsFtpStream::connect(&addr))
            .await
            .map_err(|_| Error::Ssh(format!("no answer from {addr}")))?
            .map_err(|e| Error::Ssh(format!("couldn't reach {addr}: {e}")))?;
        if p.protocol == Protocol::Ftps {
            ftp = ftp
                .into_secure(tls_connector(), &p.host)
                .await
                .map_err(|e| Error::Other(format!("the server didn't accept a secure (TLS) connection: {e}")))?;
        }
        let password = secret.as_ref().map(|s| s.as_str()).unwrap_or("");
        ftp.login(p.username.as_str(), password).await.map_err(|e| match e {
            FtpError::UnexpectedResponse(_) => Error::AuthFailed {
                user: p.username.clone(),
                host: p.host.clone(),
                profile_id: p.id.clone(),
            },
            e => err(e),
        })?;
        ftp.set_passive_nat_workaround(true);
        ftp.transfer_type(FileType::Binary).await.map_err(err)?;
        let home = p
            .initial_path
            .clone()
            .filter(|x| x.starts_with('/'))
            .or(ftp.pwd().await.ok())
            .unwrap_or_else(|| "/".into());
        Ok(Self {
            ftp: Mutex::new(ftp),
            home,
        })
    }

    pub async fn home(&self) -> String {
        self.home.clone()
    }

    pub async fn list(&self, path: &str) -> Result<Vec<FileEntry>> {
        let mut ftp = self.ftp.lock().await;
        // MLSD gives exact sizes and times; old servers only know LIST.
        let parsed: Vec<suppaftp::list::File> = match ftp.mlsd(Some(path)).await {
            Ok(lines) => lines.iter().filter_map(|l| ListParser::parse_mlsd(l).ok()).collect(),
            Err(_) => ftp
                .list(Some(path))
                .await
                .map_err(err)?
                .iter()
                .filter_map(|l| ListParser::parse_posix(l).or_else(|_| ListParser::parse_dos(l)).ok())
                .collect(),
        };
        Ok(parsed
            .into_iter()
            .filter(|f| f.name() != "." && f.name() != "..")
            .map(|f| {
                let mtime = f.modified().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as u32);
                entry(path, f.name(), f.is_directory(), f.size() as u64, mtime)
            })
            .collect())
    }

    pub async fn read(&self, path: &str, limit: u64) -> Result<Vec<u8>> {
        let mut ftp = self.ftp.lock().await;
        // Ask first, so a huge file isn't fetched just to be refused.
        if matches!(ftp.size(path).await, Ok(n) if n as u64 > limit) {
            return Err(Error::InvalidInput("the file is too large to open in the editor".into()));
        }
        let mut stream = ftp.retr_as_stream(path).await.map_err(err)?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        stream.finish().await.map_err(err)?;
        if buf.len() as u64 > limit {
            return Err(Error::InvalidInput("the file is too large to open in the editor".into()));
        }
        Ok(buf)
    }

    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<()> {
        let mut ftp = self.ftp.lock().await;
        let mut up = ftp.put_with_stream(path).await.map_err(err)?;
        up.write_all(&data).await?;
        up.finish().await.map_err(err)?;
        Ok(())
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.ftp.lock().await.mkdir(path).await.map_err(err)
    }

    pub async fn remove(&self, path: &str, recursive: bool) -> Result<()> {
        if !recursive {
            return self.ftp.lock().await.rm(path).await.map_err(err);
        }
        // Folders: empty them first, deepest first.
        for e in self.list(path).await? {
            if e.kind == crate::sftp::EntryKind::Dir {
                Box::pin(self.remove(&e.path, true)).await?;
            } else {
                self.ftp.lock().await.rm(&e.path).await.map_err(err)?;
            }
        }
        self.ftp.lock().await.rmdir(path).await.map_err(err)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.ftp.lock().await.rename(from, to).await.map_err(err)
    }

    pub async fn download(&self, remote: &str, local: &Path) -> Result<u64> {
        let mut ftp = self.ftp.lock().await;
        let mut stream = ftp.retr_as_stream(remote).await.map_err(err)?;
        let mut file = tokio::fs::File::create(local).await?;
        let n = tokio::io::copy(&mut stream, &mut file).await?;
        stream.finish().await.map_err(err)?;
        Ok(n)
    }

    pub async fn upload(&self, local: &Path, remote: &str) -> Result<u64> {
        let mut file = tokio::fs::File::open(local).await?;
        let mut ftp = self.ftp.lock().await;
        ftp.put_file(remote, &mut file).await.map_err(err)
    }

    pub async fn quit(&self) {
        let _ = self.ftp.lock().await.quit().await;
    }
}
