//! Shell-safety helpers. Every value interpolated into a remote command goes
//! through [`quote`]; identifiers (unit names, container ids) are additionally
//! validated against a strict allow-list so a crafted name can never become a
//! second command.

use crate::error::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;

/// POSIX single-quote escaping: `it's` -> `'it'\''s'`.
pub fn quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:@%+,".contains(&b)) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

static UNIT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9:_.@\\\-]{0,254}$").unwrap());
static CONTAINER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_.\-]{0,254}$").unwrap());

pub fn validate_unit(name: &str) -> Result<&str> {
    if UNIT_RE.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!("invalid systemd unit name: {name:?}")))
    }
}

pub fn validate_container(id: &str) -> Result<&str> {
    if CONTAINER_RE.is_match(id) {
        Ok(id)
    } else {
        Err(Error::InvalidInput(format!("invalid container reference: {id:?}")))
    }
}

/// Remote paths must be absolute and free of control characters.
pub fn validate_path(path: &str) -> Result<&str> {
    if !path.starts_with('/') {
        return Err(Error::InvalidInput(format!("path must be absolute: {path:?}")));
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(Error::InvalidInput("path contains control characters".into()));
    }
    Ok(path)
}

/// Join a directory and a child name the way a POSIX server expects.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("nginx.service"), "nginx.service");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn identifiers() {
        assert!(validate_unit("nginx.service").is_ok());
        assert!(validate_unit("getty@tty1.service").is_ok());
        assert!(validate_unit("foo;rm -rf /").is_err());
        assert!(validate_unit("-H").is_err());
        assert!(validate_container("web_1").is_ok());
        assert!(validate_container("3f2a9c").is_ok());
        assert!(validate_container("x && reboot").is_err());
        assert!(validate_container("--privileged").is_err());
    }

    #[test]
    fn paths() {
        assert!(validate_path("/etc/nginx/nginx.conf").is_ok());
        assert!(validate_path("etc/passwd").is_err());
        assert!(validate_path("/tmp/a\nb").is_err());
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/etc", "nginx"), "/etc/nginx");
        assert_eq!(parent("/etc/nginx/nginx.conf"), "/etc/nginx");
        assert_eq!(parent("/etc"), "/");
        assert_eq!(parent("/"), "/");
    }
}
