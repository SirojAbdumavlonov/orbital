//! Details about a server that change rarely: what it is, where it sits on
//! the network, and whether it needs attention (updates, a reboot, failed
//! services). Collected once per visit to the Monitor tab, unlike `metrics`.
//!
//! Read-only, run as the login user, one fixed script.

use crate::error::Result;
use crate::ssh::SshSession;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetAddress {
    pub interface: String,
    /// With prefix length, e.g. `10.0.0.5/24`.
    pub address: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub hostname: Option<String>,
    pub fqdn: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub arch: Option<String>,
    /// `kvm`, `xen`, `docker`, `none` (bare metal), … from systemd-detect-virt.
    pub virtualization: Option<String>,
    /// Hardware or cloud vendor and product, e.g. "Hetzner vServer".
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub cpu_threads: Option<u32>,
    pub boot_time: Option<String>,
    pub timezone: Option<String>,
    pub addresses: Vec<NetAddress>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub users_logged_in: Option<u32>,
    pub processes: Option<u32>,
    /// systemd units in the failed state.
    pub failed_units: Vec<String>,
    /// Pending package updates, and how many of them are security updates (Debian/Ubuntu).
    pub updates: Option<u32>,
    pub security_updates: Option<u32>,
    pub reboot_required: bool,
    pub package_manager: Option<String>,
    /// AppArmor or SELinux, and whether it is enforcing.
    pub security_module: Option<String>,
}

pub(crate) const SCRIPT: &str = r#"export LC_ALL=C
kv() { printf '%s=%s\n' "$1" "$2"; }
kv hostname "$(hostname 2>/dev/null)"
kv fqdn "$(hostname -f 2>/dev/null)"
[ -r /etc/os-release ] && kv os "$(. /etc/os-release; echo "${PRETTY_NAME:-$NAME}")"
kv kernel "$(uname -r)"
kv arch "$(uname -m)"
command -v systemd-detect-virt >/dev/null 2>&1 && kv virt "$(systemd-detect-virt 2>/dev/null)"
[ -r /sys/class/dmi/id/sys_vendor ] && kv vendor "$(cat /sys/class/dmi/id/sys_vendor)"
[ -r /sys/class/dmi/id/product_name ] && kv product "$(cat /sys/class/dmi/id/product_name)"
kv threads "$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null)"
kv boot "$(uptime -s 2>/dev/null)"
tz=$(timedatectl show -p Timezone --value 2>/dev/null)
[ -z "$tz" ] && [ -r /etc/timezone ] && tz=$(cat /etc/timezone)
[ -z "$tz" ] && tz=$(readlink /etc/localtime 2>/dev/null | sed 's|.*zoneinfo/||')
kv tz "$tz"
if command -v ip >/dev/null 2>&1; then
  ip -o addr show scope global 2>/dev/null | awk '{print "addr=" $2 " " $4}'
  kv gw "$(ip route show default 2>/dev/null | awk '{print $3; exit}')"
fi
[ -r /etc/resolv.conf ] && awk '/^nameserver/ {print "dns=" $2}' /etc/resolv.conf
kv users "$(who 2>/dev/null | wc -l)"
kv procs "$(ps -e 2>/dev/null | tail -n +2 | wc -l)"
command -v systemctl >/dev/null 2>&1 && systemctl --failed --no-legend --plain 2>/dev/null | awk '{print "failed=" $1}'
if [ -x /usr/lib/update-notifier/apt-check ]; then kv apt "$(/usr/lib/update-notifier/apt-check 2>&1)"; fi
[ -f /var/run/reboot-required ] && kv reboot yes
for m in apt dnf yum zypper pacman apk; do command -v "$m" >/dev/null 2>&1 && { kv pkg "$m"; break; }; done
if command -v aa-enabled >/dev/null 2>&1; then kv lsm "AppArmor $(aa-enabled 2>/dev/null)"
elif command -v getenforce >/dev/null 2>&1; then kv lsm "SELinux $(getenforce 2>/dev/null)"; fi
"#;

pub fn parse(out: &str) -> SystemInfo {
    let mut i = SystemInfo::default();
    let some = |v: &str| (!v.trim().is_empty()).then(|| v.trim().to_string());
    for line in out.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "hostname" => i.hostname = some(v),
            "fqdn" => i.fqdn = some(v),
            "os" => i.os = some(v),
            "kernel" => i.kernel = some(v),
            "arch" => i.arch = some(v),
            "virt" => i.virtualization = some(v),
            "vendor" => i.vendor = some(v),
            "product" => i.product = some(v),
            "threads" => i.cpu_threads = v.trim().parse().ok(),
            "boot" => i.boot_time = some(v),
            "tz" => i.timezone = some(v),
            "addr" => {
                if let Some((iface, addr)) = v.split_once(' ') {
                    i.addresses.push(NetAddress {
                        interface: iface.to_string(),
                        address: addr.trim().to_string(),
                    });
                }
            }
            "gw" => i.gateway = some(v),
            "dns" => i.dns.extend(some(v)),
            "users" => i.users_logged_in = v.trim().parse().ok(),
            "procs" => i.processes = v.trim().parse().ok(),
            "failed" => i.failed_units.extend(some(v)),
            // apt-check prints "<updates>;<security updates>".
            "apt" => {
                if let Some((u, s)) = v.trim().split_once(';') {
                    i.updates = u.parse().ok();
                    i.security_updates = s.parse().ok();
                }
            }
            "reboot" => i.reboot_required = true,
            "pkg" => i.package_manager = some(v),
            // aa-enabled answers "Yes"/"No"; getenforce answers "Enforcing", "Permissive" or "Disabled".
            "lsm" => {
                i.security_module = some(v).map(|m| match m.split_once(' ') {
                    Some((name, state)) if state.eq_ignore_ascii_case("yes") => format!("{name}, on"),
                    Some((name, state)) if state.eq_ignore_ascii_case("no") => format!("{name}, off"),
                    Some((name, state)) => format!("{name}, {}", state.to_lowercase()),
                    None => m,
                })
            }
            _ => {}
        }
    }
    i
}

impl SshSession {
    pub async fn system_info(&self) -> Result<SystemInfo> {
        Ok(parse(&self.exec(SCRIPT).await?.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_details() {
        let out = "hostname=prod-web-01\nfqdn=prod-web-01.example.com\nos=Ubuntu 24.04.1 LTS\nkernel=6.8.0-45-generic\narch=x86_64\nvirt=kvm\nvendor=Hetzner\nproduct=vServer\nthreads=8\nboot=2026-10-06 09:14:02\ntz=Europe/Berlin\naddr=eth0 203.0.113.10/32\naddr=eth0 2001:db8::1/64\naddr=docker0 172.17.0.1/16\ngw=172.31.1.1\ndns=127.0.0.53\nusers=2\nprocs=214\nfailed=certbot.service\napt=12;3\nreboot=yes\npkg=apt\nlsm=AppArmor yes\nnoise\n";
        let i = parse(out);
        assert_eq!(i.hostname.as_deref(), Some("prod-web-01"));
        assert_eq!(i.os.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(i.cpu_threads, Some(8));
        assert_eq!(i.addresses.len(), 3);
        assert_eq!(
            i.addresses[1],
            NetAddress {
                interface: "eth0".into(),
                address: "2001:db8::1/64".into()
            }
        );
        assert_eq!(i.dns, ["127.0.0.53"]);
        assert_eq!(i.failed_units, ["certbot.service"]);
        assert_eq!((i.updates, i.security_updates), (Some(12), Some(3)));
        assert!(i.reboot_required);
        assert_eq!(i.security_module.as_deref(), Some("AppArmor, on"));
        assert_eq!(
            parse("lsm=SELinux Enforcing\n").security_module.as_deref(),
            Some("SELinux, enforcing")
        );
        // Empty values are absent, not "".
        let i = parse("tz=\nvirt=\n");
        assert_eq!((i.timezone, i.virtualization), (None, None));
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(SCRIPT).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let i = parse(&String::from_utf8_lossy(&out.stdout));
        assert!(i.hostname.is_some() && i.kernel.is_some());
    }
}
