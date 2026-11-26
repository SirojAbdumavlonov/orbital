//! Disks: block devices (disks, partitions, LVM, RAID), mounted filesystems
//! with space and inode use, what /etc/fstab mounts at boot, and each disk's
//! SMART health when smartmontools is installed. Read-only; a fixed script.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlockDevice {
    pub name: String,
    /// `disk`, `part`, `lvm`, `raid1`, `rom`, `loop`, …
    pub kind: String,
    pub size: u64,
    pub fstype: Option<String>,
    pub mountpoint: Option<String>,
    pub model: Option<String>,
    /// Spinning disk (`ROTA` 1) rather than SSD/NVMe.
    pub rotational: bool,
    /// The device this one is on (a partition's disk).
    pub parent: Option<String>,
    /// SMART overall health for whole disks: `Some(true)` passed.
    pub healthy: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Filesystem {
    pub source: String,
    pub fstype: String,
    pub mountpoint: String,
    pub size: u64,
    pub used: u64,
    pub inodes: Option<u64>,
    pub inodes_used: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FstabEntry {
    pub source: String,
    pub mountpoint: String,
    pub fstype: String,
    pub options: String,
    /// It's mounted right now.
    pub mounted: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiskReport {
    pub devices: Vec<BlockDevice>,
    pub filesystems: Vec<Filesystem>,
    pub fstab: Vec<FstabEntry>,
    /// smartctl isn't installed (or couldn't read the disks: needs root).
    pub smart_missing: bool,
    pub warnings: Vec<String>,
}

pub const SCRIPT: &str = r#"export LC_ALL=C
echo @@lsblk
lsblk -P -b -o NAME,TYPE,SIZE,FSTYPE,MOUNTPOINT,MODEL,ROTA,PKNAME 2>/dev/null
echo @@df
df -P -B1 -T -x tmpfs -x devtmpfs -x squashfs -x overlay -x efivarfs 2>/dev/null | tail -n +2
echo @@inodes
df -P -i -x tmpfs -x devtmpfs -x squashfs -x overlay -x efivarfs 2>/dev/null | tail -n +2
echo @@fstab
grep -v '^[[:space:]]*#' /etc/fstab 2>/dev/null | grep -v '^[[:space:]]*$'
echo @@smart
if command -v smartctl >/dev/null 2>&1; then
  for d in $(lsblk -dno NAME,TYPE 2>/dev/null | awk '$2 == "disk" { print $1 }'); do
    r=$(smartctl -H "/dev/$d" 2>/dev/null | grep -Ei 'overall-health|health status' | head -1)
    [ -n "$r" ] && echo "$d $r"
  done
else echo @@nosmart; fi
echo @@end
"#;

/// `KEY="value" KEY2="value"` lines from `lsblk -P`.
fn pairs(l: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = l.trim();
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.push((key, after[..end].to_string()));
        rest = &after[end + 1..];
    }
    out
}

fn section<'a>(out: &'a str, name: &str) -> Vec<&'a str> {
    let head = format!("@@{name}");
    let mut on = false;
    let mut v = Vec::new();
    for l in out.lines() {
        if l == head {
            on = true;
        } else if on && l.starts_with("@@") && l != "@@nosmart" {
            break;
        } else if on && !l.trim().is_empty() {
            v.push(l);
        }
    }
    v
}

pub fn parse(out: &str) -> DiskReport {
    let mut r = DiskReport::default();
    let opt = |s: String| (!s.is_empty()).then_some(s);
    for l in section(out, "lsblk") {
        let m: std::collections::HashMap<String, String> = pairs(l).into_iter().collect();
        let g = |k: &str| m.get(k).cloned().unwrap_or_default();
        r.devices.push(BlockDevice {
            name: g("NAME"),
            kind: g("TYPE"),
            size: g("SIZE").parse().unwrap_or(0),
            fstype: opt(g("FSTYPE")),
            mountpoint: opt(g("MOUNTPOINT")),
            model: opt(g("MODEL").trim().to_string()),
            rotational: g("ROTA") == "1",
            parent: opt(g("PKNAME")),
            healthy: None,
        });
    }
    let mut inodes = std::collections::HashMap::new();
    for l in section(out, "inodes") {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() >= 6 {
            inodes.insert(f[5].to_string(), (f[1].parse::<u64>().ok(), f[2].parse::<u64>().ok()));
        }
    }
    for l in section(out, "df") {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 7 {
            continue;
        }
        let mp = f[6..].join(" ");
        let (i, iu) = inodes.get(&mp).cloned().unwrap_or((None, None));
        r.filesystems.push(Filesystem {
            source: f[0].into(),
            fstype: f[1].into(),
            size: f[2].parse().unwrap_or(0),
            used: f[3].parse().unwrap_or(0),
            mountpoint: mp,
            inodes: i.filter(|n| *n > 0),
            inodes_used: iu,
        });
    }
    let mounted: Vec<&str> = r.filesystems.iter().map(|f| f.mountpoint.as_str()).collect();
    for l in section(out, "fstab") {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let swap = f[2] == "swap" || f[1] == "none";
        r.fstab.push(FstabEntry {
            source: f[0].into(),
            mountpoint: f[1].into(),
            fstype: f[2].into(),
            options: f[3].into(),
            mounted: swap || mounted.contains(&f[1]),
        });
    }
    let smart = section(out, "smart");
    r.smart_missing = smart.contains(&"@@nosmart") || smart.is_empty();
    for l in smart {
        let Some((dev, text)) = l.split_once(' ') else { continue };
        let ok = text.to_uppercase().contains("PASSED") || text.to_uppercase().ends_with(": OK");
        if let Some(d) = r.devices.iter_mut().find(|d| d.name == dev) {
            d.healthy = Some(ok);
        }
    }

    for d in &r.devices {
        if d.healthy == Some(false) {
            r.warnings.push(format!(
                "SMART says /dev/{} is failing: copy your data off it and replace it",
                d.name
            ));
        }
    }
    for f in &r.filesystems {
        if f.size > 0 && f.used * 100 / f.size >= 90 {
            r.warnings.push(format!("{} is {}% full", f.mountpoint, f.used * 100 / f.size));
        }
        if let (Some(t), Some(u)) = (f.inodes, f.inodes_used) {
            if t > 0 && u * 100 / t >= 90 {
                r.warnings.push(format!(
                    "{} has used {}% of its inodes: it can't create new files even with space left (often millions of small cache or session files)",
                    f.mountpoint,
                    u * 100 / t
                ));
            }
        }
    }
    for e in &r.fstab {
        if !e.mounted && !e.options.split(',').any(|o| o == "noauto") {
            r.warnings.push(format!(
                "{} should be mounted at {} (in /etc/fstab) but isn't",
                e.source, e.mountpoint
            ));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = r#"@@lsblk
NAME="sda" TYPE="disk" SIZE="107374182400" FSTYPE="" MOUNTPOINT="" MODEL="QEMU HARDDISK   " ROTA="1" PKNAME=""
NAME="sda1" TYPE="part" SIZE="107373133824" FSTYPE="ext4" MOUNTPOINT="/" MODEL="" ROTA="1" PKNAME="sda"
NAME="nvme0n1" TYPE="disk" SIZE="500107862016" FSTYPE="" MOUNTPOINT="" MODEL="Samsung SSD 980" ROTA="0" PKNAME=""
NAME="nvme0n1p1" TYPE="part" SIZE="500106813440" FSTYPE="xfs" MOUNTPOINT="/var/lib/docker" MODEL="" ROTA="0" PKNAME="nvme0n1"
@@df
/dev/sda1      ext4  105000000000 99000000000  6000000000  95% /
/dev/nvme0n1p1 xfs   500000000000 100000000000 400000000000 20% /var/lib/docker
@@inodes
/dev/sda1       6553600 6400000 153600  98% /
/dev/nvme0n1p1  0 0 0 - /var/lib/docker
@@fstab
UUID=abc / ext4 defaults 0 1
/dev/sdb1 /backup ext4 defaults,nofail 0 2
/dev/sdc1 /mnt/usb ext4 noauto 0 0
/swapfile none swap sw 0 0
@@smart
sda SMART overall-health self-assessment test result: FAILED!
nvme0n1 SMART overall-health self-assessment test result: PASSED
@@end
"#;

    #[test]
    fn reads_devices_filesystems_fstab_and_health() {
        let r = parse(OUT);
        assert_eq!(r.devices.len(), 4);
        let sda = &r.devices[0];
        assert_eq!(
            (sda.kind.as_str(), sda.model.as_deref(), sda.rotational, sda.healthy),
            ("disk", Some("QEMU HARDDISK"), true, Some(false))
        );
        assert_eq!(r.devices[1].parent.as_deref(), Some("sda"));
        assert_eq!(r.devices[2].healthy, Some(true));
        assert_eq!(r.filesystems.len(), 2);
        assert_eq!(r.filesystems[0].inodes_used, Some(6400000));
        assert_eq!(r.filesystems[1].inodes, None, "xfs reports 0 inodes: unknown");
        assert!(!r.smart_missing);
        assert_eq!(r.fstab.len(), 4);
        assert!(!r.fstab[1].mounted);
        assert!(r.fstab[3].mounted, "swap counts as mounted");
        let w = r.warnings.join("\n");
        assert!(w.contains("/dev/sda is failing"));
        assert!(w.contains("/ is 94% full"));
        assert!(w.contains("inodes"));
        assert!(w.contains("/dev/sdb1 should be mounted at /backup"));
        assert!(!w.contains("/mnt/usb"), "noauto isn't expected to be mounted");
    }

    #[test]
    fn without_smartctl() {
        let r = parse("@@lsblk\n@@df\n@@inodes\n@@fstab\n@@smart\n@@nosmart\n@@end\n");
        assert!(r.smart_missing);
        assert!(r.warnings.is_empty());
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn script_is_valid_sh() {
        let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(o.status.success());
    }
}
