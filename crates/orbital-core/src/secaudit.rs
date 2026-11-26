//! Security audit: one read-only scan for the problems attackers look for
//! first. Extra root accounts, empty passwords, readable shadow files,
//! unexpected setuid programs, world-writable files, databases listening on
//! every address, passwordless sudo and loose SSH key permissions.
//!
//! The script is fixed (nothing interpolated). It runs as root when sudo
//! works; otherwise the report says it's partial.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Bad,
    Warn,
    Info,
    Ok,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub level: Level,
    pub title: String,
    pub detail: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    pub findings: Vec<Finding>,
    /// Ran without root: shadow and other users' files couldn't be read.
    pub partial: bool,
}

pub const SCRIPT: &str = r#"export LC_ALL=C
echo @@uid0; awk -F: '$3 == 0 && $1 != "root" { print $1 }' /etc/passwd 2>/dev/null
echo @@shadow; if [ -r /etc/shadow ]; then awk -F: '$2 == "" { print $1 }' /etc/shadow; else echo @@unreadable; fi
echo @@perms; stat -c '%a %U %n' /etc/shadow /etc/gshadow /etc/passwd /etc/sudoers 2>/dev/null
echo @@suid; find /usr /bin /sbin /opt /srv /home /var /tmp -xdev -type f \( -perm -4000 -o -perm -2000 \) 2>/dev/null | head -n 300
echo @@wwfiles; find /etc /usr /opt /srv /var/www -xdev -type f -perm -0002 2>/dev/null | head -n 50
echo @@wwdirs; find / -xdev -type d -perm -0002 ! -perm -1000 2>/dev/null | head -n 50
echo @@nopasswd; grep -rhsE '^[^#]*NOPASSWD' /etc/sudoers /etc/sudoers.d 2>/dev/null | head -n 20
echo @@docker; getent group docker 2>/dev/null | cut -d: -f4
echo @@listen; (ss -Htln 2>/dev/null || netstat -tln 2>/dev/null | tail -n +3) | awk '{ print $4 }'
echo @@keys; for h in /root /home/*; do f="$h/.ssh/authorized_keys"; [ -f "$f" ] && stat -c '%a %U %n' "$f"; done 2>/dev/null
echo @@end
"#;

/// setuid/setgid programs a normal Debian, Ubuntu or RHEL system ships.
const KNOWN_SUID: &[&str] = &[
    "sudo",
    "su",
    "passwd",
    "chsh",
    "chfn",
    "gpasswd",
    "newgrp",
    "mount",
    "umount",
    "pkexec",
    "ssh-keysign",
    "dbus-daemon-launch-helper",
    "unix_chkpwd",
    "pam_extrausers_chkpwd",
    "crontab",
    "at",
    "fusermount",
    "fusermount3",
    "ping",
    "ping6",
    "chage",
    "expiry",
    "wall",
    "write",
    "write.ul",
    "ssh-agent",
    "mount.nfs",
    "mount.cifs",
    "newuidmap",
    "newgidmap",
    "snap-confine",
    "polkit-agent-helper-1",
    "Xorg.wrap",
    "ntfs-3g",
    "sg",
    "staprun",
    "utempter",
    "vmware-user-suid-wrapper",
    "plocate",
    "mlocate",
    "locate",
    "dotlockfile",
    "bsd-write",
    "netreport",
    "usernetctl",
    "userhelper",
    "lockdev",
    "chromium-sandbox",
    "chrome-sandbox",
    "traceroute6.iputils",
    "mail-lock",
    "mail-touchlock",
    "mail-unlock",
    "screen",
    "Xorg",
    "doas",
    "sudoedit",
    "pppd",
    "exim4",
    "postdrop",
    "postqueue",
    "sendmail",
    "ksu",
    "suexec",
    "cockpit-session",
    "agetty",
    "lppasswd",
    "uuidd",
    "dmcrypt-get-device",
    "mount.ecryptfs_private",
];

/// Ports that shouldn't face the internet: (port, what).
const RISKY_PORTS: &[(u16, &str, bool)] = &[
    (2375, "Docker API without TLS (anyone reaching it controls the server)", true),
    (6379, "Redis", true),
    (11211, "Memcached", true),
    (27017, "MongoDB", true),
    (9200, "Elasticsearch", true),
    (3306, "MySQL/MariaDB", false),
    (5432, "PostgreSQL", false),
    (5984, "CouchDB", false),
    (8086, "InfluxDB", false),
];

fn section<'a>(out: &'a str, name: &str) -> Vec<&'a str> {
    let start = format!("@@{name}");
    let mut on = false;
    let mut v = Vec::new();
    for l in out.lines() {
        if l == start {
            on = true;
        } else if l.starts_with("@@") && on && !l.starts_with("@@unreadable") {
            break;
        } else if on && !l.trim().is_empty() {
            v.push(l.trim());
        }
    }
    v
}

fn f(id: &str, level: Level, title: &str, detail: &str, items: Vec<String>) -> Finding {
    Finding {
        id: id.into(),
        level,
        title: title.into(),
        detail: detail.into(),
        items,
    }
}

/// Mode (octal string like "640") lets group or others do something.
fn mode_bits(mode: &str) -> Option<u32> {
    u32::from_str_radix(mode, 8).ok()
}

pub fn parse(out: &str, root: bool) -> AuditReport {
    let mut fs = Vec::new();

    let uid0: Vec<String> = section(out, "uid0").iter().map(|s| s.to_string()).collect();
    fs.push(if uid0.is_empty() {
        f(
            "uid0",
            Level::Ok,
            "Only root has root's user ID",
            "No other account has UID 0.",
            vec![],
        )
    } else {
        f(
            "uid0",
            Level::Bad,
            "Other accounts with root's user ID",
            "These accounts are root under another name. Unless you made them on purpose, it's a classic backdoor: remove them.",
            uid0,
        )
    });

    let shadow = section(out, "shadow");
    let unreadable = shadow.contains(&"@@unreadable");
    if !unreadable {
        let empty: Vec<String> = shadow.iter().map(|s| s.to_string()).collect();
        fs.push(if empty.is_empty() {
            f("empty-passwords", Level::Ok, "No accounts with an empty password", "Every account has a password or is locked.", vec![])
        } else {
            f("empty-passwords", Level::Bad, "Accounts with an empty password", "Anyone on the server can switch to these accounts without a password (and SSH allows it if PermitEmptyPasswords is on). Lock them or set a password in the Users tab.", empty)
        });
    }

    let mut loose = Vec::new();
    for l in section(out, "perms") {
        let mut p = l.splitn(3, ' ');
        if let (Some(mode), Some(_owner), Some(path)) = (p.next(), p.next(), p.next()) {
            let Some(bits) = mode_bits(mode) else { continue };
            let bad = match path {
                "/etc/shadow" | "/etc/gshadow" => bits & 0o004 != 0,
                "/etc/sudoers" => bits & 0o022 != 0,
                "/etc/passwd" => bits & 0o022 != 0,
                _ => false,
            };
            if bad {
                loose.push(format!("{path} ({mode})"));
            }
        }
    }
    if !loose.is_empty() {
        fs.push(f(
            "system-file-perms",
            Level::Bad,
            "Account files with loose permissions",
            "Password hashes or sudo rules can be read or changed by other users. Usual modes: shadow 640, passwd 644, sudoers 440.",
            loose,
        ));
    }

    let mut odd_suid = Vec::new();
    let mut user_suid = Vec::new();
    for p in section(out, "suid") {
        let name = p.rsplit('/').next().unwrap_or(p);
        if ["/home/", "/tmp/", "/var/tmp/", "/srv/"].iter().any(|d| p.starts_with(d)) {
            user_suid.push(p.to_string());
        } else if !KNOWN_SUID.contains(&name) && !name.starts_with("pt_chown") {
            odd_suid.push(p.to_string());
        }
    }
    if !user_suid.is_empty() {
        fs.push(f("suid-user-dirs", Level::Bad, "Setuid programs in home or temp folders", "A program that runs with its owner's rights from a writable folder is how attackers keep root. Check and remove them (chmod u-s).", user_suid));
    }
    fs.push(if odd_suid.is_empty() {
        f("suid", Level::Ok, "Only the usual setuid programs", "Every program that runs with extra rights is one the system ships.", vec![])
    } else {
        f("suid", Level::Warn, "Unusual setuid or setgid programs", "These run with their owner's rights (often root). Some come with installed software; anything you don't recognise deserves a look.", odd_suid)
    });

    let ww: Vec<String> = section(out, "wwfiles").iter().map(|s| s.to_string()).collect();
    fs.push(if ww.is_empty() {
        f(
            "world-writable",
            Level::Ok,
            "No world-writable system files",
            "Nobody but their owners can change files in /etc, /usr, /opt, /srv or /var/www.",
            vec![],
        )
    } else {
        f(
            "world-writable",
            Level::Bad,
            "World-writable system files",
            "Any user (or a hacked web app) can change these files. Remove write access for others (chmod o-w).",
            ww,
        )
    });

    let wd: Vec<String> = section(out, "wwdirs").iter().map(|s| s.to_string()).collect();
    if !wd.is_empty() {
        fs.push(f(
            "sticky",
            Level::Warn,
            "Shared folders without the sticky bit",
            "Everyone can write here and also delete other people's files. Folders like /tmp should have the sticky bit (chmod +t).",
            wd,
        ));
    }

    let nopw: Vec<String> = section(out, "nopasswd").iter().map(|s| s.to_string()).collect();
    if !nopw.is_empty() {
        fs.push(f("nopasswd", Level::Info, "sudo without a password", "These rules let users or programs become root without typing a password. Fine for automation accounts; for people, a stolen SSH key then means root.", nopw));
    }

    let docker: Vec<String> = section(out, "docker")
        .first()
        .map(|l| l.split(',').filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    if !docker.is_empty() {
        fs.push(f(
            "docker-group",
            Level::Info,
            "Members of the docker group",
            "Anyone in the docker group can become root (by starting a container that mounts /). Keep it to admins.",
            docker,
        ));
    }

    let mut exposed = Vec::new();
    let mut worst = Level::Info;
    for addr in section(out, "listen") {
        let Some((host, port)) = addr.rsplit_once(':') else { continue };
        let public = matches!(host, "0.0.0.0" | "*" | "[::]" | "::");
        let Ok(port) = port.parse::<u16>() else { continue };
        if let Some((_, what, severe)) = RISKY_PORTS.iter().find(|(p, ..)| *p == port) {
            if public {
                let item = format!("{what} on port {port}");
                if !exposed.contains(&item) {
                    exposed.push(item);
                }
                worst = worst.min(if *severe { Level::Bad } else { Level::Warn });
            }
        }
    }
    fs.push(if exposed.is_empty() {
        f("exposed-db", Level::Ok, "Databases listen only privately", "No database, cache or Docker API listens on every address.", vec![])
    } else {
        f("exposed-db", worst, "Databases listening on every address", "Unless a firewall blocks them (see Open ports), these can be reached from the internet, and bots scan for them constantly. Bind them to 127.0.0.1 or allow only your addresses.", exposed)
    });

    let mut keyperm = Vec::new();
    for l in section(out, "keys") {
        let mut p = l.splitn(3, ' ');
        if let (Some(mode), Some(_), Some(path)) = (p.next(), p.next(), p.next()) {
            if mode_bits(mode).is_some_and(|b| b & 0o022 != 0) {
                keyperm.push(format!("{path} ({mode})"));
            }
        }
    }
    if !keyperm.is_empty() {
        fs.push(f(
            "authorized-keys",
            Level::Warn,
            "authorized_keys others can change",
            "Anyone who can write these files can add their own key and sign in as that user. sshd may also ignore them. chmod 600 them.",
            keyperm,
        ));
    }

    fs.sort_by_key(|x| x.level);
    AuditReport {
        findings: fs,
        partial: !root || unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@uid0
toor
@@shadow
guest
@@perms
640 root /etc/shadow
644 root /etc/gshadow
644 root /etc/passwd
440 root /etc/sudoers
@@suid
/usr/bin/sudo
/usr/bin/passwd
/usr/local/bin/backup-helper
/home/anna/bin/rootme
@@wwfiles
/etc/cron.d/oops
@@wwdirs
/srv/shared
@@nopasswd
deploy ALL=(ALL) NOPASSWD: ALL
@@docker
deploy,anna
@@listen
0.0.0.0:22
127.0.0.1:5432
0.0.0.0:6379
[::]:3306
@@keys
600 deploy /home/deploy/.ssh/authorized_keys
664 anna /home/anna/.ssh/authorized_keys
@@end
";

    fn by<'a>(r: &'a AuditReport, id: &str) -> &'a Finding {
        r.findings
            .iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("{id} missing: {:?}", r.findings))
    }

    #[test]
    fn finds_the_usual_problems() {
        let r = parse(OUT, true);
        assert!(!r.partial);
        assert_eq!(by(&r, "uid0").items, ["toor"]);
        assert_eq!(by(&r, "empty-passwords").level, Level::Bad);
        assert_eq!(by(&r, "system-file-perms").items, ["/etc/gshadow (644)"]);
        assert_eq!(by(&r, "suid").items, ["/usr/local/bin/backup-helper"]);
        assert_eq!(by(&r, "suid-user-dirs").items, ["/home/anna/bin/rootme"]);
        assert_eq!(by(&r, "world-writable").level, Level::Bad);
        assert_eq!(by(&r, "sticky").items, ["/srv/shared"]);
        assert_eq!(by(&r, "docker-group").items, ["deploy", "anna"]);
        let ex = by(&r, "exposed-db");
        assert_eq!(ex.level, Level::Bad, "Redis on every address");
        assert_eq!(ex.items, ["Redis on port 6379", "MySQL/MariaDB on port 3306"]);
        assert_eq!(by(&r, "authorized-keys").items, ["/home/anna/.ssh/authorized_keys (664)"]);
        assert_eq!(r.findings[0].level, Level::Bad, "worst first");
    }

    #[test]
    fn clean_server_and_no_root() {
        let r = parse("@@uid0\n@@shadow\n@@unreadable\n@@perms\n@@suid\n/usr/bin/sudo\n@@wwfiles\n@@wwdirs\n@@nopasswd\n@@docker\n\n@@listen\n127.0.0.1:6379\n@@keys\n@@end\n", false);
        assert!(r.partial);
        assert!(r.findings.iter().all(|x| x.level == Level::Ok), "{:?}", r.findings);
        assert!(r.findings.iter().all(|x| x.id != "empty-passwords"), "shadow unreadable: no claim");
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn script_is_valid_sh() {
        let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}
