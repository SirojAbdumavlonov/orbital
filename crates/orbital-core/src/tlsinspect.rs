//! TLS inspector: connect from this computer to `host:port`, finish a TLS
//! handshake whatever the certificate, and report the chain (names, issuer,
//! validity, key), the protocol and cipher, and whether a browser would
//! trust it (checked against the public certificate authorities, with the
//! host name). Nothing is sent after the handshake.

use crate::error::{Error, Result};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_rustls::rustls;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{DigitallySignedStruct, SignatureScheme};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub subject: String,
    pub common_name: Option<String>,
    pub issuer: String,
    pub names: Vec<String>,
    pub not_before: i64,
    pub not_after: i64,
    pub days_left: i64,
    pub serial: String,
    /// `ECDSA P-256`, `RSA 2048`, …
    pub key: String,
    pub self_signed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TlsReport {
    pub host: String,
    pub port: u16,
    pub address: Option<String>,
    /// A browser would accept this certificate for this name.
    pub trusted: bool,
    /// Why not, when it isn't.
    pub problem: Option<String>,
    pub protocol: Option<String>,
    pub cipher: Option<String>,
    /// Server certificate first, then the intermediates it sent.
    pub chain: Vec<CertInfo>,
}

pub fn validate_target(host: &str, port: u16) -> Result<()> {
    let ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        && !host.starts_with('-');
    if !ok || port == 0 {
        return Err(Error::InvalidInput(format!("{host}:{port} isn't a host and port")));
    }
    Ok(())
}

/// Readable details of one DER certificate.
pub fn describe(der: &[u8], now: i64) -> Option<CertInfo> {
    use x509_parser::prelude::*;
    let (_, c) = X509Certificate::from_der(der).ok()?;
    let common_name = c
        .subject()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .map(String::from);
    let mut names = Vec::new();
    if let Ok(Some(san)) = c.subject_alternative_name() {
        for n in &san.value.general_names {
            match n {
                GeneralName::DNSName(d) => names.push(d.to_string()),
                GeneralName::IPAddress(b) if b.len() == 4 => names.push(std::net::Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string()),
                GeneralName::IPAddress(b) if b.len() == 16 => {
                    let a: [u8; 16] = (*b).try_into().ok()?;
                    names.push(std::net::Ipv6Addr::from(a).to_string())
                }
                _ => {}
            }
        }
    }
    let key = match c.public_key().parsed() {
        Ok(x509_parser::public_key::PublicKey::RSA(r)) => format!("RSA {}", r.key_size()),
        Ok(x509_parser::public_key::PublicKey::EC(e)) => match e.key_size() {
            256 => "ECDSA P-256".into(),
            384 => "ECDSA P-384".into(),
            521 => "ECDSA P-521".into(),
            n => format!("ECDSA {n}"),
        },
        Ok(_) => {
            let oid = c.public_key().algorithm.algorithm.to_id_string();
            if oid == "1.3.101.112" {
                "Ed25519".into()
            } else {
                oid
            }
        }
        Err(_) => "unknown".into(),
    };
    let not_after = c.validity().not_after.timestamp();
    Some(CertInfo {
        subject: c.subject().to_string(),
        common_name,
        issuer: c.issuer().to_string(),
        names,
        not_before: c.validity().not_before.timestamp(),
        not_after,
        days_left: (not_after - now).div_euclid(86400),
        serial: c.raw_serial_as_string(),
        key,
        self_signed: c.subject() == c.issuer(),
    })
}

/// Accepts any certificate so the handshake completes, but records the
/// chain and what the real (public CA) check said.
#[derive(Debug)]
struct Recorder {
    inner: Arc<rustls::client::WebPkiServerVerifier>,
    chain: Mutex<Vec<Vec<u8>>>,
    verdict: Mutex<Option<String>>,
}

impl ServerCertVerifier for Recorder {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let mut chain = vec![end_entity.to_vec()];
        chain.extend(intermediates.iter().map(|c| c.to_vec()));
        *self.chain.lock().unwrap_or_else(|p| p.into_inner()) = chain;
        let v = self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp, now);
        *self.verdict.lock().unwrap_or_else(|p| p.into_inner()) = v.err().map(|e| explain(&e));
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// rustls's verdict in words.
fn explain(e: &rustls::Error) -> String {
    use rustls::CertificateError as C;
    match e {
        rustls::Error::InvalidCertificate(c) => match c {
            C::Expired | C::ExpiredContext { .. } => "the certificate has expired".into(),
            C::NotValidYet | C::NotValidYetContext { .. } => "the certificate isn't valid yet (or this computer's clock is wrong)".into(),
            C::UnknownIssuer => {
                "it isn't signed by a public certificate authority (self-signed, or the server doesn't send its intermediate certificate)"
                    .into()
            }
            C::NotValidForName | C::NotValidForNameContext { .. } => "the certificate is for other names than this host".into(),
            C::Revoked => "the certificate has been revoked".into(),
            C::Other(o) if format!("{o:?}").contains("CaUsedAsEndEntity") => {
                "the server uses a certificate authority's own certificate (typical of a self-signed certificate)".into()
            }
            other => format!("the certificate was refused: {other:?}"),
        },
        other => other.to_string(),
    }
}

/// Connect and look. `roots` are the trusted authorities (the public ones in the app).
pub async fn inspect_with(host: &str, port: u16, roots: rustls::RootCertStore, timeout: Duration) -> Result<TlsReport> {
    validate_target(host, port)?;
    let bare = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| Error::Other(e.to_string()))?;
    let rec = Arc::new(Recorder {
        inner,
        chain: Mutex::new(vec![]),
        verdict: Mutex::new(None),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Other(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(rec.clone())
        .with_no_client_auth();
    let name = ServerName::try_from(bare.clone()).map_err(|_| Error::InvalidInput(format!("{host} isn't a host name")))?;
    let tcp = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((bare.as_str(), port)))
        .await
        .map_err(|_| Error::Other(format!("no answer from {host}:{port}")))?
        .map_err(|e| Error::Other(format!("couldn't connect to {host}:{port}: {e}")))?;
    let address = tcp.peer_addr().ok().map(|a| a.ip().to_string());
    let tls = tokio::time::timeout(timeout, tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp))
        .await
        .map_err(|_| Error::Other(format!("{host}:{port} didn't finish a TLS handshake (is it TLS on this port?)")))?;
    let (protocol, cipher, handshake_error) = match &tls {
        Ok(s) => {
            let c = s.get_ref().1;
            (
                c.protocol_version()
                    .map(|v| format!("{v:?}").replace("TLSv1_", "TLS 1.").replace('_', ".")),
                c.negotiated_cipher_suite().map(|s| format!("{:?}", s.suite())),
                None,
            )
        }
        Err(e) => (None, None, Some(e.to_string())),
    };
    let now = chrono::Utc::now().timestamp();
    let chain: Vec<CertInfo> = rec
        .chain
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter_map(|d| describe(d, now))
        .collect();
    if chain.is_empty() {
        return Err(Error::Other(format!(
            "{host}:{port} didn't present a certificate{}",
            handshake_error.map(|e| format!(": {e}")).unwrap_or_default()
        )));
    }
    let verdict = rec.verdict.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Ok(TlsReport {
        host: host.into(),
        port,
        address,
        trusted: verdict.is_none() && handshake_error.is_none(),
        problem: verdict.or(handshake_error),
        protocol,
        cipher,
        chain,
    })
}

pub async fn inspect(host: &str, port: u16) -> Result<TlsReport> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    inspect_with(host, port, roots, Duration::from_secs(10)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &[u8] = include_bytes!("../tests/fixtures/tls-test-cert.der");
    const KEY: &[u8] = include_bytes!("../tests/fixtures/tls-test-key.der");

    #[test]
    fn describes_a_certificate() {
        let c = describe(CERT, 0).unwrap();
        assert_eq!(c.common_name.as_deref(), Some("test.example.com"));
        assert_eq!(c.names, ["test.example.com", "www.example.com", "127.0.0.1"]);
        assert_eq!(c.key, "ECDSA P-256");
        assert!(c.self_signed);
        assert!(c.not_after > c.not_before);
        assert!(describe(b"not a certificate", 0).is_none());
    }

    #[test]
    fn refuses_bad_targets() {
        assert!(validate_target("example.com", 443).is_ok());
        assert!(validate_target("[2001:db8::1]", 443).is_ok());
        assert!(validate_target("example.com", 0).is_err());
        assert!(validate_target("-x", 443).is_err());
        assert!(validate_target("a b", 443).is_err());
    }

    /// A local TLS server with the self-signed test certificate: the
    /// handshake completes, the chain is read, and it isn't trusted.
    #[tokio::test]
    async fn inspects_a_local_server() {
        use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..2 {
                if let Ok((s, _)) = listener.accept().await {
                    let a = acceptor.clone();
                    tokio::spawn(async move {
                        let _ = a.accept(s).await;
                    });
                }
            }
        });
        // Public roots: self-signed is refused, but everything is read.
        let public = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let r = inspect_with("127.0.0.1", port, public, Duration::from_secs(5)).await.unwrap();
        assert!(!r.trusted);
        let why = r.problem.clone().unwrap_or_default();
        assert!(why.contains("certificate authority"), "{why}");
        assert_eq!(r.protocol.as_deref(), Some("TLS 1.3"));
        assert_eq!(r.chain.len(), 1);
        assert_eq!(r.chain[0].common_name.as_deref(), Some("test.example.com"));
        // Trusting the test certificate itself, the same server passes (the IP is in its names).
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(CERT.to_vec())).unwrap();
        let ok = inspect_with("127.0.0.1", port, roots, Duration::from_secs(5)).await.unwrap();
        assert!(ok.trusted, "{:?}", ok.problem);
    }
}
