//! Unified error type. Serialized to the UI as `{ kind, message, data }` so the
//! frontend can react to specific cases (e.g. show the host-key trust dialog).

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SSH error: {0}")]
    Ssh(String),
    #[error("SFTP error: {0}")]
    Sftp(String),
    #[error("Authentication failed for {user}@{host}")]
    AuthFailed { user: String, host: String },
    #[error("The authenticity of host {host}:{port} can't be established ({algorithm} {fingerprint})")]
    UnknownHostKey {
        host: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
    },
    #[error("HOST KEY FOR {host}:{port} HAS CHANGED (expected {expected}, got {actual}). Someone could be eavesdropping on you (man-in-the-middle attack). Connection refused.")]
    HostKeyMismatch {
        host: String,
        port: u16,
        expected: String,
        actual: String,
    },
    #[error("{what} required for {user}@{host}")]
    CredentialsRequired { what: String, user: String, host: String },
    #[error("Secret store error: {0}")]
    Secret(String),
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Remote command failed (exit {code}): {stderr}")]
    Remote { code: i32, stderr: String },
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("The file was modified on the server since you opened it")]
    Conflict { remote_mtime: Option<u32> },
    #[error("Validation failed, change was rolled back: {output}")]
    ValidationFailed { validator: String, output: String },
    #[error("Pre-flight check failed, action aborted: {output}")]
    PreflightFailed { check: String, output: String },
    #[error("Not connected (session {0})")]
    NotConnected(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Ssh(_) => "Ssh",
            Error::Sftp(_) => "Sftp",
            Error::AuthFailed { .. } => "AuthFailed",
            Error::UnknownHostKey { .. } => "UnknownHostKey",
            Error::HostKeyMismatch { .. } => "HostKeyMismatch",
            Error::CredentialsRequired { .. } => "CredentialsRequired",
            Error::Secret(_) => "Secret",
            Error::Db(_) => "Db",
            Error::InvalidInput(_) => "InvalidInput",
            Error::NotFound(_) => "NotFound",
            Error::Remote { .. } => "Remote",
            Error::PermissionDenied(_) => "PermissionDenied",
            Error::Conflict { .. } => "Conflict",
            Error::ValidationFailed { .. } => "ValidationFailed",
            Error::PreflightFailed { .. } => "PreflightFailed",
            Error::NotConnected(_) => "NotConnected",
            Error::Io(_) => "Io",
            Error::Other(_) => "Other",
        }
    }

    fn data(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Error::UnknownHostKey {
                host,
                port,
                algorithm,
                fingerprint,
            } => {
                json!({ "host": host, "port": port, "algorithm": algorithm, "fingerprint": fingerprint })
            }
            Error::HostKeyMismatch {
                host,
                port,
                expected,
                actual,
            } => {
                json!({ "host": host, "port": port, "expected": expected, "actual": actual })
            }
            Error::CredentialsRequired { what, user, host } => json!({ "what": what, "user": user, "host": host }),
            Error::Remote { code, stderr } => json!({ "code": code, "stderr": stderr }),
            Error::Conflict { remote_mtime } => json!({ "remoteMtime": remote_mtime }),
            Error::ValidationFailed { validator, output } => json!({ "validator": validator, "output": output }),
            Error::PreflightFailed { check, output } => json!({ "check": check, "output": output }),
            _ => serde_json::Value::Null,
        }
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Error", 3)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.serialize_field("data", &self.data())?;
        st.end()
    }
}

impl From<russh::Error> for Error {
    fn from(e: russh::Error) -> Self {
        Error::Ssh(e.to_string())
    }
}

impl From<russh::keys::Error> for Error {
    fn from(e: russh::keys::Error) -> Self {
        Error::Ssh(format!("key error: {e}"))
    }
}

impl From<russh_sftp::client::error::Error> for Error {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        let msg = e.to_string();
        if msg.to_lowercase().contains("permission denied") {
            Error::PermissionDenied(msg)
        } else {
            Error::Sftp(msg)
        }
    }
}

impl From<keyring::Error> for Error {
    fn from(e: keyring::Error) -> Self {
        Error::Secret(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Other(format!("json: {e}"))
    }
}
