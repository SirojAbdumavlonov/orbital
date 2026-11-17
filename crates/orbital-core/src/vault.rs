//! Saved credentials in Orbital's own encrypted file, instead of the macOS
//! Keychain. The Keychain ties access to the app's code signature, so an
//! unsigned or rebuilt app asks the user for permission again and again; the
//! vault never asks.
//!
//! The vault is one file encrypted with ChaCha20-Poly1305. Its key is derived
//! from a random per-install key file and this machine's hardware ID, so the
//! folder copied to another computer can't be read. Both files are 0600 in a
//! 0700 folder. Like the Keychain, this does not stop malware running as the
//! same user; the optional Touch ID gate (`GatedStore`) still applies on top.

use crate::error::{Error, Result};
use crate::secrets::{Secret, SecretStore};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"ORV1";
const VAULT_FILE: &str = "vault.bin";
const KEY_FILE: &str = ".install-key";

pub struct VaultStore {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    items: Mutex<BTreeMap<String, Zeroizing<String>>>,
}

/// A stable ID for this computer: the hardware UUID on macOS, the MachineGuid
/// Windows sets at install, machine-id on Linux.
pub fn machine_id() -> String {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        if let Ok(o) = std::process::Command::new(format!(r"{root}\System32\reg.exe"))
            .args(["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if let Some(id) = parse_machine_guid(&String::from_utf8_lossy(&o.stdout)) {
                return id;
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(o) = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            let text = String::from_utf8_lossy(&o.stdout);
            if let Some(id) = text
                .lines()
                .find(|l| l.contains("IOPlatformUUID"))
                .and_then(|l| l.split('"').nth(3))
            {
                return id.to_string();
            }
        }
    }
    // Linux (and the BSDs with dbus): systemd's machine-id, then dbus's copy.
    for f in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Some(id) = std::fs::read_to_string(f).ok().as_deref().and_then(parse_machine_id) {
            return id;
        }
    }
    String::new()
}

/// The contents of a machine-id file → the ID. An empty file or systemd's
/// "uninitialized" placeholder (first boot, some containers) is no ID: using
/// it would give every such machine the same vault key.
fn parse_machine_id(text: &str) -> Option<String> {
    let id = text.trim();
    (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| id.to_ascii_lowercase())
}

/// `reg query … /v MachineGuid` output → the GUID.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_machine_guid(out: &str) -> Option<String> {
    out.lines()
        .find(|l| l.trim_start().starts_with("MachineGuid"))
        .and_then(|l| l.split_whitespace().last())
        .filter(|g| g.len() >= 32)
        .map(String::from)
}

#[cfg(unix)]
fn private(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}
#[cfg(not(unix))]
fn private(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    // Write beside it and rename, so a crash never leaves half a vault.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    private(&tmp, 0o600)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl VaultStore {
    /// Open (or create) the vault in `dir`, keyed to this machine.
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with(dir, &machine_id())
    }

    pub fn open_with(dir: &Path, machine: &str) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        private(dir, 0o700)?;
        let key_path = dir.join(KEY_FILE);
        let install = match std::fs::read(&key_path) {
            Ok(k) if k.len() == 32 => Zeroizing::new(k),
            _ => {
                let k = ChaCha20Poly1305::generate_key(&mut OsRng);
                write_private(&key_path, k.as_slice())?;
                Zeroizing::new(k.to_vec())
            }
        };
        let mut h = Sha256::new();
        h.update(b"orbital-vault-v1\0");
        h.update(&install[..]);
        h.update(machine.as_bytes());
        let key = Zeroizing::new(<[u8; 32]>::from(h.finalize()));
        let vault = Self {
            path: dir.join(VAULT_FILE),
            key,
            items: Mutex::new(BTreeMap::new()),
        };
        vault.load();
        Ok(vault)
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.key[..]))
    }

    /// Read the file. One that can't be decrypted (copied from another Mac,
    /// damaged) is set aside rather than overwritten, and the vault starts empty.
    fn load(&self) {
        let Ok(data) = std::fs::read(&self.path) else { return };
        let plain = (|| {
            if data.len() < 4 + 12 || &data[..4] != MAGIC {
                return None;
            }
            let nonce = Nonce::from_slice(&data[4..16]);
            let plain = Zeroizing::new(self.cipher().decrypt(nonce, &data[16..]).ok()?);
            serde_json::from_slice::<BTreeMap<String, String>>(&plain).ok()
        })();
        match plain {
            Some(map) => {
                *self.items.lock().unwrap() = map.into_iter().map(|(k, v)| (k, Zeroizing::new(v))).collect();
            }
            None => {
                let aside = self.path.with_extension("unreadable");
                let _ = std::fs::rename(&self.path, &aside);
                tracing::warn!("the credentials vault couldn't be read; moved it to {}", aside.display());
            }
        }
    }

    fn save(&self, items: &BTreeMap<String, Zeroizing<String>>) -> Result<()> {
        let plain: BTreeMap<&str, &str> = items.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let json = Zeroizing::new(serde_json::to_vec(&plain)?);
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = self
            .cipher()
            .encrypt(&nonce, json.as_slice())
            .map_err(|_| Error::Secret("couldn't encrypt the vault".into()))?;
        let mut out = Vec::with_capacity(4 + 12 + sealed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        write_private(&self.path, &out)
    }
}

impl SecretStore for VaultStore {
    fn set(&self, key: &str, secret: &str) -> Result<()> {
        let mut items = self.items.lock().unwrap();
        items.insert(key.to_string(), Zeroizing::new(secret.to_string()));
        self.save(&items)
    }

    fn get(&self, key: &str) -> Result<Option<Secret>> {
        Ok(self.items.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let mut items = self.items.lock().unwrap();
        if items.remove(key).is_some() {
            self.save(&items)?;
        }
        Ok(())
    }

    fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.items.lock().unwrap().contains_key(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_linux_machine_id() {
        assert_eq!(
            parse_machine_id("4C4C4544004E3510804BB4C04F563232\n").as_deref(),
            Some("4c4c4544004e3510804bb4c04f563232")
        );
        assert_eq!(parse_machine_id(""), None);
        assert_eq!(parse_machine_id("uninitialized\n"), None);
        assert_eq!(parse_machine_id("not-a-machine-id-but-32-chars-xx"), None);
    }

    #[test]
    fn reads_the_windows_machine_guid() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    6f1e0c2b-9a3d-4c55-8e21-0b7d9f3a1c42\r\n\r\n";
        assert_eq!(parse_machine_guid(out).as_deref(), Some("6f1e0c2b-9a3d-4c55-8e21-0b7d9f3a1c42"));
        assert_eq!(
            parse_machine_guid("ERROR: The system was unable to find the specified registry key"),
            None
        );
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("orbital-vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn keeps_secrets_encrypted_across_restarts() {
        let d = dir("roundtrip");
        let v = VaultStore::open_with(&d, "mac-1").unwrap();
        v.set("profile:a:password", "hunter2 with spaces").unwrap();
        v.set("profile:a:sudo", "s3cret").unwrap();
        v.delete("profile:a:sudo").unwrap();
        drop(v);

        let raw = std::fs::read(d.join(VAULT_FILE)).unwrap();
        assert!(raw.starts_with(MAGIC));
        assert!(!String::from_utf8_lossy(&raw).contains("hunter2"), "plaintext on disk");

        let v = VaultStore::open_with(&d, "mac-1").unwrap();
        assert_eq!(v.get("profile:a:password").unwrap().unwrap().as_str(), "hunter2 with spaces");
        assert!(!v.exists("profile:a:sudo").unwrap());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&d), 0o700);
            assert_eq!(mode(&d.join(VAULT_FILE)), 0o600);
            assert_eq!(mode(&d.join(KEY_FILE)), 0o600);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copied_to_another_machine_it_is_unreadable_and_set_aside() {
        let d = dir("othermac");
        VaultStore::open_with(&d, "mac-1").unwrap().set("k", "v").unwrap();
        let elsewhere = VaultStore::open_with(&d, "mac-2").unwrap();
        assert!(elsewhere.get("k").unwrap().is_none());
        assert!(d.join("vault.unreadable").exists(), "kept, not overwritten");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every supported desktop has an ID to bind the vault to (CI runs this on
    /// macOS, Windows and Linux).
    #[test]
    fn has_a_machine_id() {
        assert!(
            !machine_id().is_empty(),
            "no machine ID: the vault key would not be tied to this computer"
        );
    }
}
