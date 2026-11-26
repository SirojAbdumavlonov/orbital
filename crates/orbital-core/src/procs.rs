//! Processes: every process with its parent (for a tree), CPU, memory, run
//! time and priority; details for one (threads, open files, listening ports,
//! working folder, service); and sending signals or changing priority.
//!
//! PIDs and signal names are numbers and fixed words, never free text. PID 1
//! and sshd (which carries this connection) are protected.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub nice: i32,
    /// `R` running, `S` sleeping, `D` waiting for disk, `Z` zombie, `T` stopped.
    pub state: String,
    pub cpu: f64,
    pub mem: f64,
    pub rss_bytes: u64,
    pub elapsed_secs: u64,
    /// Program name (kernel threads in brackets).
    pub name: String,
    pub args: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcDetail {
    pub pid: u32,
    pub threads: Option<u32>,
    pub swap_bytes: Option<u64>,
    pub open_files: Option<u32>,
    pub cwd: Option<String>,
    pub exe: Option<String>,
    pub unit: Option<String>,
    /// `address:port` it listens on.
    pub listening: Vec<String>,
    /// Max open files (soft limit).
    pub max_files: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Signal {
    Term,
    Kill,
    Hup,
    Int,
    Stop,
    Cont,
    Usr1,
    Usr2,
}

impl Signal {
    pub fn name(self) -> &'static str {
        match self {
            Signal::Term => "TERM",
            Signal::Kill => "KILL",
            Signal::Hup => "HUP",
            Signal::Int => "INT",
            Signal::Stop => "STOP",
            Signal::Cont => "CONT",
            Signal::Usr1 => "USR1",
            Signal::Usr2 => "USR2",
        }
    }
}

pub const LIST_SCRIPT: &str =
    "LC_ALL=C ps -eo pid=,ppid=,user:32=,ni=,stat=,pcpu=,pmem=,rss=,etimes=,args= 2>/dev/null || LC_ALL=C ps -eo pid,ppid,user,nice,stat,pcpu,pmem,rss,etime,args\n";

/// `[[DD-]HH:]MM:SS` (busybox/BSD `etime`) or plain seconds (`etimes`).
fn elapsed(s: &str) -> u64 {
    if let Ok(n) = s.parse() {
        return n;
    }
    let (days, rest) = s.split_once('-').map(|(d, r)| (d.parse().unwrap_or(0), r)).unwrap_or((0, s));
    let parts: Vec<u64> = rest.split(':').filter_map(|p| p.parse().ok()).collect();
    let secs = parts.iter().fold(0, |acc, p| acc * 60 + p);
    days * 86400 + secs
}

pub fn parse_list(out: &str) -> Vec<Proc> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let user = f.next()?.to_string();
            let nice = f.next()?.parse().unwrap_or(0);
            let state = f.next()?.to_string();
            let cpu = f.next()?.parse().unwrap_or(0.0);
            let mem = f.next()?.parse().unwrap_or(0.0);
            let rss: u64 = f.next()?.parse().unwrap_or(0);
            let elapsed_secs = elapsed(f.next()?);
            let args = f.collect::<Vec<_>>().join(" ");
            if args.is_empty() {
                return None;
            }
            let first = args.split_whitespace().next().unwrap_or("");
            let name = if first.starts_with('[') {
                args.clone()
            } else {
                first.rsplit('/').next().unwrap_or(first).trim_end_matches(':').to_string()
            };
            Some(Proc {
                pid,
                ppid,
                user,
                nice,
                state,
                cpu,
                mem,
                rss_bytes: rss * 1024,
                elapsed_secs,
                name,
                args,
            })
        })
        .collect()
}

/// Details of one process; nothing but the number goes into the script.
pub fn detail_script(pid: u32) -> String {
    format!(
        "p=/proc/{pid}; [ -d $p ] || {{ echo @@gone; exit 0; }}\n\
         grep -E '^(Threads|VmSwap):' $p/status 2>/dev/null\n\
         echo \"@@fds $(ls $p/fd 2>/dev/null | wc -l)\"\n\
         echo \"@@cwd $(readlink $p/cwd 2>/dev/null)\"\n\
         echo \"@@exe $(readlink $p/exe 2>/dev/null)\"\n\
         echo \"@@unit $(grep -o '[^/]*\\.service' $p/cgroup 2>/dev/null | tail -1)\"\n\
         echo \"@@limit $(awk '/Max open files/ {{ print $4 }}' $p/limits 2>/dev/null)\"\n\
         ss -Htlnp 2>/dev/null | grep \"pid={pid},\" | awk '{{ print \"@@listen \" $4 }}'\n"
    )
}

pub fn parse_detail(pid: u32, out: &str) -> Result<ProcDetail> {
    if out.lines().any(|l| l.trim() == "@@gone") {
        return Err(Error::NotFound(format!("process {pid} has ended")));
    }
    let mut d = ProcDetail { pid, ..Default::default() };
    let val = |l: &str, p: &str| l.strip_prefix(p).map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    for l in out.lines() {
        if let Some(v) = l.strip_prefix("Threads:") {
            d.threads = v.trim().parse().ok();
        } else if let Some(v) = l.strip_prefix("VmSwap:") {
            d.swap_bytes = v.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).map(|kb| kb * 1024);
        } else if let Some(v) = val(l, "@@fds") {
            d.open_files = v.parse().ok();
        } else if let Some(v) = val(l, "@@cwd") {
            d.cwd = Some(v);
        } else if let Some(v) = val(l, "@@exe") {
            d.exe = Some(v);
        } else if let Some(v) = val(l, "@@unit") {
            d.unit = Some(v);
        } else if let Some(v) = val(l, "@@limit") {
            d.max_files = Some(v);
        } else if let Some(v) = val(l, "@@listen") {
            if !d.listening.contains(&v) {
                d.listening.push(v);
            }
        }
    }
    Ok(d)
}

/// Refuse what would hurt the system or cut this connection.
pub fn check_target(pid: u32, name: &str, signal: Option<Signal>) -> Result<()> {
    if pid <= 1 {
        return Err(Error::InvalidInput("PID 1 is the system itself".into()));
    }
    let sshd = name == "sshd" || name.starts_with("sshd:");
    if sshd && signal.is_some_and(|s| !matches!(s, Signal::Hup)) {
        return Err(Error::InvalidInput(
            "sshd carries this connection; restart or reload ssh from Services instead".into(),
        ));
    }
    Ok(())
}

pub fn signal_command(pid: u32, s: Signal) -> String {
    format!("kill -{} {pid}", s.name())
}

pub fn renice_command(pid: u32, nice: i32) -> Result<String> {
    if !(-20..=19).contains(&nice) {
        return Err(Error::InvalidInput("priority (nice) goes from -20 (first) to 19 (last)".into()));
    }
    Ok(format!("renice -n {nice} -p {pid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "      1       0 root                               0 Ss    0.0  0.1 12345      86400 /sbin/init splash
      2       0 root                               0 S     0.0  0.0     0      86400 [kthreadd]
    812       1 root                               0 Ss    0.0  0.1  7000      3600 sshd: /usr/sbin/sshd -D [listener] 0 of 10-100 startups
   1201       1 www-data                          -5 Sl    2.5  1.2 99000       120 /usr/bin/node /srv/app/server.js --port 3000
";

    #[test]
    fn parses_the_process_list() {
        let p = parse_list(OUT);
        assert_eq!(p.len(), 4);
        assert_eq!((p[0].pid, p[0].ppid, p[0].name.as_str()), (1, 0, "init"));
        assert_eq!(p[1].name, "[kthreadd]");
        assert_eq!(p[2].name, "sshd");
        let node = &p[3];
        assert_eq!(
            (node.user.as_str(), node.nice, node.cpu, node.rss_bytes),
            ("www-data", -5, 2.5, 99000 * 1024)
        );
        assert_eq!(node.args, "/usr/bin/node /srv/app/server.js --port 3000");
        assert_eq!(elapsed("1-02:03:04"), 93784);
        assert_eq!(elapsed("05:06"), 306);
    }

    #[test]
    fn parses_details() {
        let d = parse_detail(1201, "Threads:\t11\nVmSwap:\t    2048 kB\n@@fds 37\n@@cwd /srv/app\n@@exe /usr/bin/node\n@@unit shop.service\n@@limit 1048576\n@@listen 0.0.0.0:3000\n@@listen 0.0.0.0:3000\n").unwrap();
        assert_eq!((d.threads, d.swap_bytes, d.open_files), (Some(11), Some(2048 * 1024), Some(37)));
        assert_eq!(d.unit.as_deref(), Some("shop.service"));
        assert_eq!(d.listening, ["0.0.0.0:3000"]);
        assert_eq!(parse_detail(9, "@@cwd \n@@unit \n").unwrap().cwd, None);
        assert!(matches!(parse_detail(9, "@@gone\n"), Err(Error::NotFound(_))));
    }

    #[test]
    fn protects_init_and_sshd() {
        assert!(check_target(1, "init", Some(Signal::Term)).is_err());
        assert!(check_target(812, "sshd", Some(Signal::Kill)).is_err());
        assert!(check_target(812, "sshd", Some(Signal::Hup)).is_ok(), "HUP reloads sshd");
        assert!(check_target(812, "sshd", None).is_ok(), "renice is harmless");
        assert!(check_target(1201, "node", Some(Signal::Kill)).is_ok());
        assert_eq!(signal_command(1201, Signal::Usr1), "kill -USR1 1201");
        assert_eq!(renice_command(1201, 10).unwrap(), "renice -n 10 -p 1201");
        assert!(renice_command(1201, 25).is_err());
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn scripts_are_valid_sh() {
        for s in [LIST_SCRIPT.to_string(), detail_script(42)] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&s).output().unwrap();
            assert!(o.status.success(), "{s}");
        }
    }
}
