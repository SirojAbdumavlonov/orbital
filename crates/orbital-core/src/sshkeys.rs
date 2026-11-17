//! SSH keys: the key pairs on this computer (`~/.ssh`), and the keys that may
//! sign in to a server account (`~/.ssh/authorized_keys`).
//!
//! New keys are ed25519, made here (no `ssh-keygen` needed, so it works on
//! Windows too) and written in OpenSSH format with the usual permissions. A
//! passphrase, when given, encrypts the private key file; it is never stored.

use crate::error::{Error, Result};
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::{Cipher, HashAlg, Kdf, LineEnding, PrivateKey, PublicKey};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A key pair in `~/.ssh` (a `.pub` file with its private key beside it).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalKey {
    /// File name without `.pub`, e.g. `id_ed25519`.
    pub name: String,
    /// The private key file, as `~/.ssh/…` when it is there.
    pub path: String,
    pub algorithm: String,
    pub comment: String,
    pub fingerprint: String,
    /// The whole public key line, ready to paste into authorized_keys.
    pub public_line: String,
    /// The private key needs a passphrase.
    pub encrypted: bool,
}

/// One line of authorized_keys.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedKey {
    /// The line exactly as it is in the file (used to remove it).
    pub line: String,
    /// `command="…",no-pty`, … before the key, if any.
    pub options: Option<String>,
    pub algorithm: String,
    pub comment: String,
    /// `SHA256:…`; `None` when the key part doesn't decode.
    pub fingerprint: Option<String>,
    /// The same key is on this computer (a file in ~/.ssh, the profile's key or the agent).
    pub on_this_computer: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedKeys {
    pub user: String,
    pub path: String,
    pub keys: Vec<AuthorizedKey>,
    /// The file doesn't exist yet (no key can sign in).
    pub missing: bool,
}

const KEY_TYPES: &[&str] = &[
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
    "ssh-dss",
];

/// Split a key line into (options, type, base64, comment). Options may
/// contain quoted spaces (`command="a b"`), so the type is found as the
/// first token outside quotes that is a key type.
pub fn split_line(line: &str) -> Option<(Option<String>, String, String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut tokens: Vec<(usize, &str)> = Vec::new();
    let (mut start, mut quoted) = (None, false);
    for (i, c) in line.char_indices() {
        match c {
            '"' => {
                quoted = !quoted;
                start.get_or_insert(i);
            }
            c if c.is_whitespace() && !quoted => {
                if let Some(s) = start.take() {
                    tokens.push((s, &line[s..i]));
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        tokens.push((s, &line[s..]));
    }
    let k = tokens.iter().position(|(_, t)| KEY_TYPES.contains(t))?;
    let (_, algorithm) = tokens[k];
    let (_, b64) = *tokens.get(k + 1)?;
    let options = (k > 0).then(|| line[..tokens[k].0].trim().to_string());
    let comment = tokens.get(k + 2).map(|(s, _)| line[*s..].trim().to_string()).unwrap_or_default();
    Some((options, algorithm.to_string(), b64.to_string(), comment))
}

/// `SHA256:…` of a key given as type and base64.
pub fn fingerprint(algorithm: &str, b64: &str) -> Option<String> {
    PublicKey::from_openssh(&format!("{algorithm} {b64}"))
        .ok()
        .map(|k| k.fingerprint(HashAlg::Sha256).to_string())
}

pub fn parse_authorized(text: &str, mine: &[String]) -> Vec<AuthorizedKey> {
    text.lines()
        .filter_map(|l| {
            let (options, algorithm, b64, comment) = split_line(l)?;
            let fingerprint = fingerprint(&algorithm, &b64);
            Some(AuthorizedKey {
                line: l.to_string(),
                on_this_computer: fingerprint.as_ref().is_some_and(|f| mine.contains(f)),
                options,
                algorithm,
                comment,
                fingerprint,
            })
        })
        .collect()
}

/// Check a key someone pasted: one line, a known type, a key that decodes.
/// Returns it without surrounding whitespace.
pub fn validate_public_line(line: &str) -> Result<String> {
    let line = line.trim();
    if line.contains('\n') || line.contains('\r') {
        return Err(Error::InvalidInput("paste one key, on one line".into()));
    }
    let bad = || {
        Error::InvalidInput(
            "that isn't an SSH public key: it should look like \"ssh-ed25519 AAAA… name\" (the .pub file, never the private key)".into(),
        )
    };
    if line.contains("PRIVATE KEY") {
        return Err(Error::InvalidInput(
            "that's a private key: paste the public key (the .pub file) instead, and keep the private one secret".into(),
        ));
    }
    let (_, algorithm, b64, _) = split_line(line).ok_or_else(bad)?;
    fingerprint(&algorithm, &b64).ok_or_else(bad)?;
    Ok(line.to_string())
}

/// The file with `line` added at the end, unless the same key is already there.
pub fn with_key(text: &str, line: &str) -> Result<String> {
    let (_, algorithm, b64, _) = split_line(line).ok_or_else(|| Error::InvalidInput("not a key line".into()))?;
    if text.lines().filter_map(split_line).any(|(_, a, b, _)| a == algorithm && b == b64) {
        return Err(Error::InvalidInput("this key can already sign in".into()));
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    Ok(out)
}

/// The file without the line that is exactly `line` (the first one, if repeated).
pub fn without_line(text: &str, line: &str) -> Result<String> {
    let mut found = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            if !found && *l == line {
                found = true;
                false
            } else {
                true
            }
        })
        .collect();
    if !found {
        return Err(Error::NotFound(
            "that key is no longer in authorized_keys (the file changed); reload".into(),
        ));
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

/// Would removing `line` leave no key from this computer that can sign in?
/// `None` when it's safe. Only matters when this connection signs in with a key.
pub fn lockout_reason(keys: &[AuthorizedKey], line: &str, key_login: bool) -> Option<String> {
    let target = keys.iter().find(|k| k.line == line)?;
    if !key_login || !target.on_this_computer {
        return None;
    }
    let others = keys.iter().filter(|k| k.line != line && k.on_this_computer).count();
    (others == 0).then(|| {
        format!(
            "This is the only key from this computer that can sign in to this account ({}). \
             This connection signs in with a key, so after removing it you could only get back in with a password, \
             if the server allows one.",
            target.fingerprint.as_deref().unwrap_or("this key")
        )
    })
}

// ---------- keys on this computer ----------

pub fn ssh_dir() -> Option<PathBuf> {
    crate::ssh::home_dir().map(|h| h.join(".ssh"))
}

fn display_path(p: &Path) -> String {
    match crate::ssh::home_dir().and_then(|h| p.strip_prefix(h).ok().map(|r| r.to_path_buf())) {
        Some(rel) => format!("~/{}", rel.to_string_lossy().replace('\\', "/")),
        None => p.to_string_lossy().into_owned(),
    }
}

/// Key pairs in `dir`: every `X.pub` whose private key `X` is beside it.
pub fn local_keys(dir: &Path) -> Vec<LocalKey> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<LocalKey> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".pub")?.to_string();
            let private = dir.join(&stem);
            if !private.is_file() {
                return None;
            }
            let text = std::fs::read_to_string(e.path()).ok()?;
            let line = text.lines().next()?.trim().to_string();
            let (_, algorithm, b64, comment) = split_line(&line)?;
            let fingerprint = fingerprint(&algorithm, &b64)?;
            let encrypted = std::fs::read_to_string(&private)
                .ok()
                .and_then(|pem| PrivateKey::from_openssh(pem).ok())
                .map(|k| k.is_encrypted())
                // Old PEM keys: "Proc-Type: 4,ENCRYPTED" marks a passphrase.
                .unwrap_or_else(|| std::fs::read_to_string(&private).is_ok_and(|t| t.contains("ENCRYPTED")));
            Some(LocalKey {
                name: stem,
                path: display_path(&private),
                algorithm,
                comment,
                fingerprint,
                public_line: line,
                encrypted,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.ends_with(".pub")
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}
#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> std::io::Result<()> {
    Ok(())
}

/// Make a new ed25519 key pair `dir/name` and `dir/name.pub`. Never overwrites.
pub fn generate(dir: &Path, name: &str, comment: &str, passphrase: Option<&str>) -> Result<LocalKey> {
    use chacha20poly1305::aead::rand_core::RngCore;
    use chacha20poly1305::aead::OsRng;
    if !valid_file_name(name) {
        return Err(Error::InvalidInput(
            "use a file name of letters, digits, '-', '_' or '.', like id_ed25519_work".into(),
        ));
    }
    if comment.chars().any(|c| c.is_control()) || comment.len() > 200 {
        return Err(Error::InvalidInput("the comment must be one short line".into()));
    }
    let private = dir.join(name);
    let public = dir.join(format!("{name}.pub"));
    if private.exists() || public.exists() {
        return Err(Error::InvalidInput(format!(
            "{} already exists; choose another name",
            display_path(&private)
        )));
    }
    std::fs::create_dir_all(dir)?;
    set_mode(dir, 0o700).ok();

    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut seed[..]);
    let mut key = PrivateKey::from(Ed25519Keypair::from_seed(&seed));
    key.set_comment(comment);
    let key = match passphrase.filter(|p| !p.is_empty()) {
        Some(pass) => {
            let mut salt = vec![0u8; 16];
            OsRng.fill_bytes(&mut salt);
            let kdf = Kdf::Bcrypt { salt, rounds: 16 };
            key.encrypt_with(Cipher::Aes256Ctr, kdf, OsRng.next_u32(), pass)
                .map_err(|e| Error::Other(format!("couldn't encrypt the key: {e}")))?
        }
        None => key,
    };
    let pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| Error::Other(format!("couldn't write the key: {e}")))?;
    let line = key
        .public_key()
        .to_openssh()
        .map_err(|e| Error::Other(format!("couldn't write the public key: {e}")))?;

    // The private key is created readable by its owner only, before any data is in it.
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&private)?;
        f.write_all(pem.as_bytes())?;
    }
    std::fs::write(&public, format!("{line}\n"))?;
    set_mode(&public, 0o644).ok();
    local_keys(dir)
        .into_iter()
        .find(|k| k.name == name)
        .ok_or_else(|| Error::Other("the new key couldn't be read back".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl anna@mac";

    #[test]
    fn splits_lines_with_options_and_comments() {
        let (o, a, b, c) = split_line(ED).unwrap();
        assert_eq!((o, a.as_str(), c.as_str()), (None, "ssh-ed25519", "anna@mac"));
        assert!(b.starts_with("AAAAC3"));
        let with = format!(r#"command="/usr/bin/rsync --server -e.",no-pty,from="10.0.0.0/8" {ED}"#);
        let (o, a, _, c) = split_line(&with).unwrap();
        assert_eq!(
            o.as_deref(),
            Some(r#"command="/usr/bin/rsync --server -e.",no-pty,from="10.0.0.0/8""#)
        );
        assert_eq!((a.as_str(), c.as_str()), ("ssh-ed25519", "anna@mac"));
        let (_, _, _, c) = split_line("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl").unwrap();
        assert_eq!(c, "");
        assert!(split_line("# a comment").is_none());
        assert!(split_line("   ").is_none());
        assert!(split_line("garbage here").is_none());
    }

    #[test]
    fn fingerprints_and_marks_keys_from_this_computer() {
        let fp = fingerprint(
            "ssh-ed25519",
            "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl",
        )
        .unwrap();
        assert!(fp.starts_with("SHA256:"));
        let text = format!("# keys\n{ED}\nssh-rsa AAAAnotbase64!! bob\n");
        let keys = parse_authorized(&text, std::slice::from_ref(&fp));
        assert_eq!(keys.len(), 2);
        assert!(keys[0].on_this_computer);
        assert_eq!(keys[1].fingerprint, None, "a broken key is listed, without a fingerprint");
    }

    #[test]
    fn validates_pasted_keys() {
        assert_eq!(validate_public_line(&format!("  {ED}\n")).unwrap(), ED);
        assert!(validate_public_line("-----BEGIN OPENSSH PRIVATE KEY-----").is_err());
        assert!(validate_public_line("ssh-ed25519 AAAA").is_err());
        assert!(validate_public_line(&format!("{ED}\n{ED}")).is_err());
    }

    #[test]
    fn adds_and_removes_exact_lines() {
        let one = with_key("", ED).unwrap();
        assert_eq!(one, format!("{ED}\n"));
        assert!(with_key(&one, &ED.replace("anna@mac", "other comment")).is_err(), "same key twice");
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBXd8sq7wFq4JnB3vJrQe2c3Pq6l1m8T0Q2N5yQx0d1K bob";
        let two = with_key("# mine\nno newline at end", other).unwrap();
        assert_eq!(two, format!("# mine\nno newline at end\n{other}\n"));
        let both = format!("{ED}\n{other}\n");
        assert_eq!(without_line(&both, ED).unwrap(), format!("{other}\n"));
        assert_eq!(without_line(&format!("{ED}\n"), ED).unwrap(), "");
        assert!(matches!(without_line(&both, "ssh-ed25519 AAAA gone"), Err(Error::NotFound(_))));
    }

    #[test]
    fn refuses_to_remove_the_last_key_from_here() {
        let mine = fingerprint("ssh-ed25519", &split_line(ED).unwrap().2).unwrap();
        let other = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBXd8sq7wFq4JnB3vJrQe2c3Pq6l1m8T0Q2N5yQx0d1K bob";
        let keys = parse_authorized(&format!("{ED}\n{other}\n"), std::slice::from_ref(&mine));
        assert!(lockout_reason(&keys, ED, true).is_some());
        assert!(lockout_reason(&keys, ED, false).is_none(), "password logins aren't cut off");
        assert!(lockout_reason(&keys, other, true).is_none(), "someone else's key");
        let twice = parse_authorized(&format!("{ED}\n{}\n", ED.replace("anna@mac", "copy")), &[mine]);
        assert!(lockout_reason(&twice, ED, true).is_none(), "another copy remains");
    }

    #[test]
    fn generates_keys_that_load_and_lists_them() {
        let dir = tempfile::tempdir().unwrap();
        let k = generate(dir.path(), "id_ed25519_test", "me@orbital", None).unwrap();
        assert_eq!(k.algorithm, "ssh-ed25519");
        assert!(!k.encrypted);
        assert!(k.public_line.ends_with(" me@orbital"));
        assert!(russh::keys::load_secret_key(dir.path().join("id_ed25519_test"), None).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("id_ed25519_test")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let e = generate(dir.path(), "locked", "", Some("correct horse")).unwrap();
        assert!(e.encrypted);
        let p = dir.path().join("locked");
        assert!(russh::keys::load_secret_key(&p, Some("correct horse")).is_ok());
        assert!(russh::keys::load_secret_key(&p, Some("wrong")).is_err());

        assert!(generate(dir.path(), "locked", "", None).is_err(), "never overwrites");
        assert!(generate(dir.path(), "../evil", "", None).is_err());
        assert!(generate(dir.path(), "x", "two\nlines", None).is_err());
        let names: Vec<_> = local_keys(dir.path()).into_iter().map(|k| k.name).collect();
        assert_eq!(names, ["id_ed25519_test", "locked"]);
    }
}
