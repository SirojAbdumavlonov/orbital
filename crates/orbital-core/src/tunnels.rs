//! Port forwarding: a port on this Mac (127.0.0.1 only) that leads to a port
//! on, or reachable from, the server, through the SSH connection already
//! open. So a database, Redis or an admin page that listens only on the
//! server's localhost can be used from Mac apps without opening the firewall.

use crate::error::{Error, Result};
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TunnelInfo {
    pub id: String,
    pub session_id: String,
    /// Always 127.0.0.1: tunnels never listen on the Mac's network interfaces.
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub open_connections: usize,
    pub total_connections: u64,
    pub bytes: u64,
    pub label: Option<String>,
    /// Why the server refused the last connection, in plain words (None while it works).
    pub last_error: Option<String>,
}

pub struct Tunnel {
    pub info: TunnelInfo,
    open: Arc<AtomicUsize>,
    total: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    last_error: Arc<std::sync::Mutex<Option<String>>>,
    task: JoinHandle<()>,
}

impl Tunnel {
    pub fn info(&self) -> TunnelInfo {
        TunnelInfo {
            open_connections: self.open.load(Ordering::Relaxed),
            total_connections: self.total.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            last_error: self.last_error.lock().map(|e| e.clone()).unwrap_or(None),
            ..self.info.clone()
        }
    }

    pub fn close(&self) {
        self.task.abort();
    }
}

static HOST_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9]([A-Za-z0-9.\-]{0,252})$").unwrap());

/// Where the server should connect: `localhost`, a host name or an IP address.
pub fn validate_target(host: &str) -> Result<String> {
    let h = host.trim();
    if h.parse::<IpAddr>().is_ok() || HOST_RE.is_match(h) {
        Ok(h.to_string())
    } else {
        Err(Error::InvalidInput(format!("{host:?} is not a host name or IP address")))
    }
}

/// Bind 127.0.0.1 on `wanted`, or on the same number as the remote port when
/// that is free and unprivileged, or on any free port.
pub async fn bind_local(wanted: Option<u16>, remote_port: u16) -> Result<TcpListener> {
    if let Some(p) = wanted {
        return TcpListener::bind(("127.0.0.1", p))
            .await
            .map_err(|e| Error::InvalidInput(format!("port {p} on this Mac is not available: {e}")));
    }
    if remote_port >= 1024 {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", remote_port)).await {
            return Ok(l);
        }
    }
    Ok(TcpListener::bind(("127.0.0.1", 0)).await?)
}

/// Why the server refused a `direct-tcpip` channel, in plain words.
pub fn refusal(e: &Error, host: &str, port: u16) -> String {
    let t = e.to_string();
    if t.contains("AdministrativelyProhibited") || t.contains("administratively") || t.contains("prohibited") {
        "the server's SSH settings don't allow port forwarding (AllowTcpForwarding is off in /etc/ssh/sshd_config)".into()
    } else if t.contains("ConnectFailed") || t.contains("Connect failed") || t.contains("connect failed") || t.contains("refused") {
        format!("nothing answers on {host}:{port} on the server: is the program running, and listening on that address?")
    } else if t.contains("UnknownChannelType") || t.contains("ResourceShortage") {
        format!("the server couldn't open the connection ({t})")
    } else {
        format!("the server refused the connection to {host}:{port} ({t})")
    }
}

/// Open one connection to `host:port` through the server and close it again,
/// so a tunnel that can't work says why when it's opened, not later in a browser.
pub async fn probe(session: &SshSession, host: &str, port: u16) -> Result<()> {
    match session
        .handle()
        .channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1".to_string(), 0)
        .await
    {
        Ok(ch) => {
            let _ = ch.close().await;
            Ok(())
        }
        Err(e) => Err(Error::InvalidInput(refusal(&e.into(), host, port))),
    }
}

/// Start forwarding: every connection to the local listener becomes a
/// `direct-tcpip` channel to `remote_host:remote_port` as seen from the server.
pub fn start(
    session: Arc<SshSession>,
    listener: TcpListener,
    remote_host: String,
    remote_port: u16,
    label: Option<String>,
) -> Result<Tunnel> {
    let local_port = listener.local_addr()?.port();
    let open = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let last_error = Arc::new(std::sync::Mutex::new(None));
    let (o, t, b, host, le) = (open.clone(), total.clone(), bytes.clone(), remote_host.clone(), last_error.clone());
    let task = tokio::spawn(async move {
        while let Ok((mut sock, peer)) = listener.accept().await {
            let (s, o, b, host, le) = (session.clone(), o.clone(), b.clone(), host.clone(), le.clone());
            t.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let ch = match s
                    .handle()
                    .channel_open_direct_tcpip(host.clone(), remote_port as u32, peer.ip().to_string(), peer.port() as u32)
                    .await
                {
                    Ok(ch) => {
                        if let Ok(mut l) = le.lock() {
                            *l = None;
                        }
                        ch
                    }
                    Err(e) => {
                        let why = refusal(&e.into(), &host, remote_port);
                        tracing::warn!("tunnel to {host}:{remote_port}: {why}");
                        if let Ok(mut l) = le.lock() {
                            *l = Some(why);
                        }
                        return;
                    }
                };
                o.fetch_add(1, Ordering::Relaxed);
                let mut remote = ch.into_stream();
                if let Ok((up, down)) = tokio::io::copy_bidirectional(&mut sock, &mut remote).await {
                    b.fetch_add(up + down, Ordering::Relaxed);
                }
                o.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    Ok(Tunnel {
        info: TunnelInfo {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: String::new(),
            local_port,
            remote_host,
            remote_port,
            open_connections: 0,
            total_connections: 0,
            bytes: 0,
            label,
            last_error: None,
        },
        open,
        total,
        bytes,
        last_error,
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_in_plain_words() {
        let e = |t: &str| Error::Ssh(t.into());
        assert!(refusal(&e("Channel open failure: AdministrativelyProhibited"), "127.0.0.1", 9000).contains("AllowTcpForwarding"));
        assert!(refusal(&e("Channel open failure: ConnectFailed"), "127.0.0.1", 9000).starts_with("nothing answers on 127.0.0.1:9000"));
        assert!(refusal(&e("something odd"), "db", 5432).contains("db:5432"));
    }

    #[test]
    fn targets() {
        assert_eq!(validate_target(" localhost ").unwrap(), "localhost");
        assert!(validate_target("10.0.0.5").is_ok());
        assert!(validate_target("::1").is_ok());
        assert!(validate_target("db.internal.example.com").is_ok());
        for bad in ["", "-oProxyCommand=x", "a b", "host;id", "$(id)"] {
            assert!(validate_target(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn binds_only_to_loopback() {
        let l = bind_local(None, 22).await.unwrap();
        let addr = l.local_addr().unwrap();
        assert!(addr.ip().is_loopback() && addr.port() != 22);
        let taken = addr.port();
        assert!(bind_local(Some(taken), 5432).await.is_err(), "a port in use is refused");
    }
}
