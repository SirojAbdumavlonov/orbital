//! Secrets (passwords, key passphrases, sudo passwords) are stored only in the
//! OS credential store — the macOS Keychain on Mac, exactly like Cyberduck.
//! They are never written to the SQLite database, logs or the audit trail.

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// Keys under this prefix hold server credentials and are subject to the
/// user-presence gate. Other keys (app settings) are not.
const CREDENTIAL_PREFIX: &str = "profile:";
/// Where the gate's on/off switch lives: in the credential store itself, so
/// flipping it takes the same access as reading the credentials it protects.
const GATE_KEY: &str = "app:presence-gate";

pub trait SecretStore: Send + Sync {
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<Secret>>;
    fn delete(&self, key: &str) -> Result<()>;
    /// Whether a secret is stored, without handing it out.
    fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.get(key)?.is_some())
    }
}

/// Wraps the real store and refuses to hand out saved server credentials
/// unless the caller holds an [`Unlock`] — which the engine only issues after
/// the user has authenticated on this Mac (Touch ID or login password).
///
/// Fail-closed: with the gate on, a `get` outside an unlock is an error, never
/// the secret. Saving, deleting and existence checks are not gated.
pub struct GatedStore {
    inner: Arc<dyn SecretStore>,
    enabled: AtomicBool,
    unlocks: AtomicUsize,
}

/// Proof that the user was just verified. Credentials are readable while at
/// least one is alive; dropping it locks the store again.
pub struct Unlock(Arc<GatedStore>);

impl Drop for Unlock {
    fn drop(&mut self) {
        self.0.unlocks.fetch_sub(1, Ordering::SeqCst);
    }
}

impl GatedStore {
    pub fn new(inner: Arc<dyn SecretStore>) -> Self {
        // If the switch can't be read, assume the gate is on.
        let enabled = !matches!(inner.get(GATE_KEY), Ok(None));
        Self {
            inner,
            enabled: AtomicBool::new(enabled),
            unlocks: AtomicUsize::new(0),
        }
    }

    pub fn gate_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Persist and apply the switch. The caller must have verified the user.
    pub fn set_gate(&self, on: bool) -> Result<()> {
        if on {
            self.inner.set(GATE_KEY, "1")?;
        } else {
            self.inner.delete(GATE_KEY)?;
        }
        self.enabled.store(on, Ordering::SeqCst);
        Ok(())
    }

    /// Only call after the user has been verified.
    pub fn unlock(self: &Arc<Self>) -> Unlock {
        self.unlocks.fetch_add(1, Ordering::SeqCst);
        Unlock(self.clone())
    }

    fn locked(&self, key: &str) -> bool {
        key.starts_with(CREDENTIAL_PREFIX) && self.gate_enabled() && self.unlocks.load(Ordering::SeqCst) == 0
    }
}

impl SecretStore for GatedStore {
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.inner.set(key, secret)
    }
    fn get(&self, key: &str) -> Result<Option<Secret>> {
        if self.locked(key) {
            // Only a stored secret is worth refusing over; "nothing saved" is not sensitive.
            return if self.inner.exists(key)? {
                Err(Error::PresenceRequired("saved credentials are locked".into()))
            } else {
                Ok(None)
            };
        }
        self.inner.get(key)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(key)
    }
    fn exists(&self, key: &str) -> Result<bool> {
        self.inner.exists(key)
    }
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

/// Moves secrets from an older store to the current one as they are read:
/// after the app was renamed, its Keychain items still sit under the old
/// service name. A secret found only in the old store is copied to the new
/// one on first use and removed from the old one.
pub struct MigratingStore {
    current: Arc<dyn SecretStore>,
    legacy: Arc<dyn SecretStore>,
}

impl MigratingStore {
    pub fn new(current: Arc<dyn SecretStore>, legacy: Arc<dyn SecretStore>) -> Self {
        Self { current, legacy }
    }
}

impl SecretStore for MigratingStore {
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.current.set(key, secret)?;
        // A stale copy under the old name must not come back later.
        let _ = self.legacy.delete(key);
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Secret>> {
        if let Some(s) = self.current.get(key)? {
            return Ok(Some(s));
        }
        // An unreadable old store (denied Keychain prompt) just means nothing to migrate.
        let Ok(Some(old)) = self.legacy.get(key) else { return Ok(None) };
        self.current.set(key, &old)?;
        let _ = self.legacy.delete(key);
        Ok(Some(old))
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.current.delete(key)?;
        let _ = self.legacy.delete(key);
        Ok(())
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

#[cfg(test)]
mod tests {

    #[test]
    fn migrating_store_moves_old_secrets_on_first_use() {
        let current = Arc::new(MemoryStore::default());
        let legacy = Arc::new(MemoryStore::default());
        legacy.set("p1:password", "old-secret").unwrap();
        legacy.set("p2:password", "stale").unwrap();
        let store = MigratingStore::new(current.clone(), legacy.clone());

        assert!(store.exists("p1:password").unwrap());
        assert_eq!(store.get("p1:password").unwrap().unwrap().as_str(), "old-secret");
        assert_eq!(current.get("p1:password").unwrap().unwrap().as_str(), "old-secret");
        assert!(legacy.get("p1:password").unwrap().is_none(), "moved, not copied");

        // Saving replaces the old copy; deleting removes both.
        store.set("p2:password", "new").unwrap();
        assert!(legacy.get("p2:password").unwrap().is_none());
        assert_eq!(store.get("p2:password").unwrap().unwrap().as_str(), "new");
        legacy.set("p3:password", "x").unwrap();
        store.delete("p3:password").unwrap();
        assert!(store.get("p3:password").unwrap().is_none());
        assert!(store.get("missing").unwrap().is_none());
    }
    use super::*;

    #[test]
    fn gate_locks_saved_credentials_only() {
        let inner: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        let key = secret_key("p1", SecretKind::Password);
        inner.set(&key, "hunter2").unwrap();
        let store = Arc::new(GatedStore::new(inner.clone()));

        // Gate off by default: behaves like the inner store.
        assert!(!store.gate_enabled());
        assert_eq!(store.get(&key).unwrap().unwrap().as_str(), "hunter2");

        store.set_gate(true).unwrap();
        assert!(matches!(store.get(&key), Err(Error::PresenceRequired(_))));
        // Existence, absent keys, writes and deletes don't need the user.
        assert!(store.exists(&key).unwrap());
        assert!(store.get(&secret_key("p1", SecretKind::SudoPassword)).unwrap().is_none());
        store.set(&secret_key("p2", SecretKind::Password), "x").unwrap();
        assert!(matches!(
            store.get(&secret_key("p2", SecretKind::Password)),
            Err(Error::PresenceRequired(_))
        ));

        // Readable only while an unlock is alive; nested unlocks are counted.
        let a = store.unlock();
        let b = store.unlock();
        assert_eq!(store.get(&key).unwrap().unwrap().as_str(), "hunter2");
        drop(a);
        assert!(store.get(&key).is_ok());
        drop(b);
        assert!(matches!(store.get(&key), Err(Error::PresenceRequired(_))));

        // The switch persists in the store and is picked up by a new instance.
        assert!(GatedStore::new(inner.clone()).gate_enabled());
        store.set_gate(false).unwrap();
        assert!(!GatedStore::new(inner).gate_enabled());
    }

    #[test]
    fn unreadable_switch_means_gate_on() {
        struct Broken;
        impl SecretStore for Broken {
            fn set(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn get(&self, _: &str) -> Result<Option<Secret>> {
                Err(Error::Secret("keychain unavailable".into()))
            }
            fn delete(&self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        assert!(GatedStore::new(Arc::new(Broken)).gate_enabled());
    }
}
