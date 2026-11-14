//! Secrets (passwords, key passphrases, sudo passwords) are stored only in the
//! OS credential store — the macOS Keychain on Mac, exactly like Cyberduck.
//! They are never written to the SQLite database, logs or the audit trail.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use zeroize::Zeroizing;

pub type Secret = Zeroizing<String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    Password,
    KeyPassphrase,
    SudoPassword,
}

impl SecretKind {
    fn as_str(self) -> &'static str {
        match self {
            SecretKind::Password => "password",
            SecretKind::KeyPassphrase => "passphrase",
            SecretKind::SudoPassword => "sudo",
        }
    }
}

pub fn secret_key(profile_id: &str, kind: SecretKind) -> String {
    format!("profile:{profile_id}:{}", kind.as_str())
}

pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<Secret>>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// OS keychain (macOS Keychain / Windows Credential Manager / Linux keyutils).
pub struct KeychainStore {
    service: String,
}

impl KeychainStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }
}

impl SecretStore for KeychainStore {
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        keyring::Entry::new(&self.service, key)?.set_password(secret)?;
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Secret>> {
        match keyring::Entry::new(&self.service, key)?.get_password() {
            Ok(s) => Ok(Some(Zeroizing::new(s))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Secret(e.to_string())),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        match keyring::Entry::new(&self.service, key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Secret(e.to_string())),
        }
    }
}

/// In-memory store for tests and "don't remember" sessions.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<HashMap<String, Secret>>,
}

impl SecretStore for MemoryStore {
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .insert(key.to_string(), Zeroizing::new(secret.to_string()));
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Secret>> {
        Ok(self.inner.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.inner.lock().unwrap().remove(key);
        Ok(())
    }
}
