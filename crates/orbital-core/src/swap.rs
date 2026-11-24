//! Add swap to a server that has none (Monitor → Add swap): a /swapfile of
//! the chosen size, turned on, kept on after reboots (/etc/fstab), with a low
//! swappiness (/etc/sysctl.d/99-orbital-swap.conf) so memory is preferred.
//! The script checks first and changes nothing when swap is already on, the
//! file exists, or the disk hasn't room for it plus 1 GB.

use crate::error::{Error, Result};

pub const MIN_MB: u32 = 256;
pub const MAX_MB: u32 = 64 * 1024;

/// A sensible size for a server with this much memory: twice small memory,
/// the same as medium memory, 4 GB beyond that.
pub fn suggest_mb(mem_bytes: u64) -> u32 {
    let mb = (mem_bytes / (1024 * 1024)) as u32;
    let s = if mb < 2048 {
        mb * 2
    } else if mb <= 8192 {
        mb
    } else {
        4096
    };
    // Whole 256 MB steps, within limits.
    (s.div_ceil(256) * 256).clamp(1024, 8192)
}

pub fn validate_mb(mb: u32) -> Result<u32> {
    if (MIN_MB..=MAX_MB).contains(&mb) {
        Ok(mb)
    } else {
        Err(Error::InvalidInput(format!(
            "choose between {MIN_MB} MB and {} GB of swap",
            MAX_MB / 1024
        )))
    }
}

/// The script (as root). Only the size goes in, as a number.
pub fn add_script(mb: u32) -> Result<String> {
    let mb = validate_mb(mb)?;
    Ok(format!(
        r#"set -e
if [ -n "$(swapon --show --noheadings 2>/dev/null)" ]; then echo "swap is already on:" >&2; swapon --show >&2; exit 3; fi
if [ -e /swapfile ]; then echo "/swapfile already exists; Orbital doesn't overwrite it" >&2; exit 3; fi
free=$(df -Pk / | awk 'NR==2 {{print $4}}')
need=$(( {mb} * 1024 + 1048576 ))
if [ "$free" -lt "$need" ]; then echo "not enough free disk on /: {mb} MB of swap needs $(( need / 1024 )) MB free, there are $(( free / 1024 )) MB" >&2; exit 4; fi
echo "Creating a {mb} MB /swapfile …"
fallocate -l {mb}M /swapfile 2>/dev/null || dd if=/dev/zero of=/swapfile bs=1M count={mb} status=none
chmod 600 /swapfile
mkswap /swapfile >/dev/null
swapon /swapfile
grep -qs '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
echo 'vm.swappiness=10' > /etc/sysctl.d/99-orbital-swap.conf
sysctl -q -w vm.swappiness=10 || true
echo "@@swap $(swapon --show=SIZE --noheadings --bytes /swapfile)""#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_and_validates_sizes() {
        let gb = 1024 * 1024 * 1024;
        assert_eq!(suggest_mb(gb), 2048, "1 GB of memory → 2 GB swap");
        assert_eq!(suggest_mb(4 * gb), 4096);
        assert_eq!(suggest_mb(64 * gb), 4096);
        assert_eq!(suggest_mb(256 * 1024 * 1024), 1024, "at least 1 GB");
        assert!(validate_mb(100).is_err() && validate_mb(70_000).is_err() && validate_mb(2048).is_ok());
    }

    #[test]
    fn the_script_checks_before_it_changes_anything() {
        let s = add_script(2048).unwrap();
        let checks = s.find("swapon --show --noheadings").unwrap();
        let first_change = s.find("fallocate").unwrap();
        assert!(checks < first_change && s.find("df -Pk /").unwrap() < first_change);
        assert!(s.contains("chmod 600 /swapfile") && s.contains("/etc/fstab"));
        let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&s).output().unwrap();
        assert!(o.status.success());
        assert!(add_script(1).is_err());
    }
}
