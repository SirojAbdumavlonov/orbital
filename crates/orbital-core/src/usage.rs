//! What uses the server's resources: which folders take the disk space, and
//! which programs, services and users use the CPU and memory right now.
//!
//! Read-only. Disk usage runs `du` one level deep at a time (the UI drills
//! down), at low priority and staying on one filesystem. CPU is measured over
//! one second from /proc, because `ps` on Linux reports a lifetime average.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_path};
use crate::ssh::SshSession;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiskEntry {
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub path: String,
    /// Everything under `path` on this filesystem.
    pub total: u64,
    /// Sub-folders, and files over 1 MB, largest first.
    pub entries: Vec<DiskEntry>,
    pub fs_size: Option<u64>,
    pub fs_used: Option<u64>,
    pub fs_free: Option<u64>,
    pub mount: Option<String>,
    /// Some folders couldn't be read (no permission), so sizes are lower bounds.
    pub partial: bool,
}

pub fn disk_script(path: &str) -> Result<String> {
    let p = quote(validate_path(path)?);
    Ok(format!(
        r#"export LC_ALL=C
cd -- {p} || exit 3
echo "@@fs $(df -kP . | awk 'NR==2 {{print $2, $3, $4, $6}}')"
N=""; command -v nice >/dev/null 2>&1 && N="nice -n 19"; command -v ionice >/dev/null 2>&1 && N="$N ionice -c3"
echo @@du; $N du -xk -d 1 . 2>&1
echo @@files; find . -maxdepth 1 -type f -size +1024k -exec du -k {{}} + 2>/dev/null
"#
    ))
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn parse_disk(path: &str, out: &str) -> DiskUsage {
    let mut u = DiskUsage {
        path: path.to_string(),
        ..Default::default()
    };
    let mut section = "";
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("@@fs ") {
            let f: Vec<&str> = v.split_whitespace().collect();
            let kb = |i: usize| f.get(i).and_then(|x| x.parse::<u64>().ok()).map(|k| k * 1024);
            u.fs_size = kb(0);
            u.fs_used = kb(1);
            u.fs_free = kb(2);
            u.mount = f.get(3).map(|m| m.to_string());
            continue;
        }
        if let Some(s) = line.strip_prefix("@@") {
            section = if s == "du" || s == "files" { s } else { "" };
            continue;
        }
        if section.is_empty() {
            continue;
        }
        let Some((size, name)) = line.split_once('\t') else {
            // du prints "du: cannot read directory …" for folders it can't enter.
            if section == "du" && !line.trim().is_empty() {
                u.partial = true;
            }
            continue;
        };
        let Ok(kb) = size.trim().parse::<u64>() else {
            u.partial = true;
            continue;
        };
        let name = name.strip_prefix("./").unwrap_or(name);
        if name == "." {
            u.total = kb * 1024;
            continue;
        }
        if name.contains('/') || name.is_empty() {
            continue;
        }
        u.entries.push(DiskEntry {
            path: join(path, name),
            name: name.to_string(),
            bytes: kb * 1024,
            is_dir: section == "du",
        });
    }
    u.entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    u
}

// ---------- CPU and memory by program ----------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcUsage {
    pub pid: u32,
    pub user: String,
    pub command: String,
    /// Percent of one core over the last second (can exceed 100 on several cores).
    pub cpu: f64,
    pub mem_percent: f64,
    pub rss_bytes: u64,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub name: String,
    pub processes: u32,
    pub cpu: f64,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessReport {
    pub cores: u32,
    /// How CPU was measured: `sampled` (1 s from /proc) or `ps` (its own average).
    pub method: String,
    pub processes: Vec<ProcUsage>,
    pub by_program: Vec<UsageGroup>,
    pub by_service: Vec<UsageGroup>,
    pub by_user: Vec<UsageGroup>,
}

pub(crate) const PROC_SCRIPT: &str = r#"export LC_ALL=C
echo "@@cores $(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null)"
if [ -r /proc/stat ]; then
  echo "@@hz $(getconf CLK_TCK)"
  echo @@s1; cat /proc/[0-9]*/stat 2>/dev/null
  sleep 1
  echo @@s2; cat /proc/[0-9]*/stat 2>/dev/null
  echo @@units; grep -H -o '[^/]*\.service$' /proc/[0-9]*/cgroup 2>/dev/null
fi
echo @@ps; ps -A -o pid=,user=,pcpu=,pmem=,rss=,comm= 2>/dev/null
"#;

/// pid → utime + stime ticks, from `/proc/<pid>/stat` lines (the name may contain spaces).
fn ticks(lines: &[&str]) -> HashMap<u32, u64> {
    let mut m = HashMap::new();
    for l in lines {
        let Some(close) = l.rfind(')') else { continue };
        let pid = l.split_whitespace().next().and_then(|p| p.parse().ok());
        // After ") ": state is field 3; utime and stime are fields 14 and 15.
        let rest: Vec<&str> = l[close + 1..].split_whitespace().collect();
        if let (Some(pid), Some(u), Some(s)) = (pid, rest.get(11), rest.get(12)) {
            if let (Ok(u), Ok(s)) = (u.parse::<u64>(), s.parse::<u64>()) {
                m.insert(pid, u + s);
            }
        }
    }
    m
}

fn group<F: Fn(&ProcUsage) -> Option<String>>(procs: &[ProcUsage], key: F) -> Vec<UsageGroup> {
    let mut g: HashMap<String, UsageGroup> = HashMap::new();
    for p in procs {
        let Some(k) = key(p) else { continue };
        let e = g.entry(k.clone()).or_insert(UsageGroup {
            name: k,
            processes: 0,
            cpu: 0.0,
            rss_bytes: 0,
        });
        e.processes += 1;
        e.cpu += p.cpu;
        e.rss_bytes += p.rss_bytes;
    }
    let mut v: Vec<UsageGroup> = g.into_values().collect();
    v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.rss_bytes.cmp(&a.rss_bytes)));
    v
}

pub fn parse_processes(out: &str) -> ProcessReport {
    let mut r = ProcessReport::default();
    let mut section = "";
    let (mut s1, mut s2, mut ps) = (Vec::new(), Vec::new(), Vec::new());
    let mut units: HashMap<u32, String> = HashMap::new();
    let mut hz = 100u64;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("@@cores ") {
            r.cores = v.trim().parse().unwrap_or(1);
            continue;
        }
        if let Some(v) = line.strip_prefix("@@hz ") {
            hz = v.trim().parse().unwrap_or(100);
            continue;
        }
        if let Some(s) = line.strip_prefix("@@") {
            section = match s {
                "s1" | "s2" | "units" | "ps" => s,
                _ => "",
            };
            continue;
        }
        match section {
            "s1" => s1.push(line),
            "s2" => s2.push(line),
            "ps" => ps.push(line),
            // "/proc/123/cgroup:nginx.service"
            "units" => {
                if let Some((p, u)) = line.split_once(":") {
                    if let Some(pid) = p
                        .strip_prefix("/proc/")
                        .and_then(|x| x.strip_suffix("/cgroup"))
                        .and_then(|x| x.parse().ok())
                    {
                        units.insert(pid, u.trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let (t1, t2) = (ticks(&s1), ticks(&s2));
    let sampled = !t2.is_empty();
    r.method = if sampled { "sampled" } else { "ps" }.into();
    for l in ps {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 6 {
            continue;
        }
        let Ok(pid) = f[0].parse::<u32>() else { continue };
        let num = |s: &str| s.parse::<f64>().unwrap_or(0.0);
        let cpu = if sampled {
            match (t1.get(&pid), t2.get(&pid)) {
                (Some(a), Some(b)) => (b.saturating_sub(*a)) as f64 * 100.0 / hz as f64,
                // Started during the second: count what it used since.
                (None, Some(b)) => (*b as f64 * 100.0 / hz as f64).min(100.0 * r.cores.max(1) as f64),
                _ => 0.0,
            }
        } else {
            num(f[2])
        };
        let command = f[5..].join(" ");
        let command = command.rsplit('/').next().unwrap_or(&command).to_string();
        r.processes.push(ProcUsage {
            pid,
            user: f[1].to_string(),
            cpu: (cpu * 10.0).round() / 10.0,
            mem_percent: num(f[3]),
            rss_bytes: f[4].parse::<u64>().unwrap_or(0) * 1024,
            unit: units.get(&pid).cloned(),
            command,
        });
    }
    r.processes
        .sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.rss_bytes.cmp(&a.rss_bytes)));
    r.by_program = group(&r.processes, |p| Some(p.command.clone()));
    r.by_service = group(&r.processes, |p| p.unit.clone());
    r.by_user = group(&r.processes, |p| Some(p.user.clone()));
    r
}

impl SshSession {
    pub async fn disk_usage(&self, path: &str) -> Result<DiskUsage> {
        let (o, _) = self.exec_inspect(&disk_script(path)?).await?;
        if o.exit_code == 3 {
            return Err(Error::NotFound(format!("{path} doesn't exist or can't be opened")));
        }
        Ok(parse_disk(path, &o.stdout))
    }

    pub async fn process_usage(&self) -> Result<ProcessReport> {
        let (o, _) = self.exec_inspect(PROC_SCRIPT).await?;
        Ok(parse_processes(&o.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disk_usage() {
        let out = "@@fs 41152736 20480000 18563464 /\n@@du\n1048576\t./var\n2097152\t./usr\n4\t./lost+found\ndu: cannot read directory './root': Permission denied\n3200000\t.\n@@files\n51200\t./swap.img\n";
        let u = parse_disk("/", out);
        assert_eq!(u.total, 3200000 * 1024);
        assert!(u.partial);
        let names: Vec<&str> = u.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["usr", "var", "swap.img", "lost+found"]);
        assert_eq!(u.entries[0].path, "/usr");
        assert!(!u.entries[2].is_dir);
        assert_eq!((u.fs_size, u.mount.as_deref()), (Some(41152736 * 1024), Some("/")));
        assert_eq!(parse_disk("/srv", "@@du\n10\t./my app\n20\t.\n").entries[0].path, "/srv/my app");
        assert!(disk_script("relative").is_err());
        assert!(disk_script("/srv/$(id)").unwrap().contains("cd -- '/srv/$(id)'"));
    }

    #[test]
    fn measures_cpu_over_a_second_and_groups() {
        let out = "@@cores 4\n@@hz 100\n@@s1\n812 (nginx) S 1 812 812 0 -1 4194560 0 0 0 0 1000 200 0 0\n900 (my worker) S 1 900 900 0 -1 0 0 0 0 0 5000 0 0 0\n@@s2\n812 (nginx) S 1 812 812 0 -1 4194560 0 0 0 0 1030 220 0 0\n900 (my worker) S 1 900 900 0 -1 0 0 0 0 0 5090 0 0 0\n@@units\n/proc/812/cgroup:nginx.service\n@@ps\n  812 www-data  9.9  1.2 98304 nginx\n  900 deploy   80.0  4.0 409600 /usr/bin/my worker\n  1 root 0.0 0.1 8000 systemd\n";
        let r = parse_processes(out);
        assert_eq!((r.cores, r.method.as_str()), (4, "sampled"));
        let w = &r.processes[0];
        assert_eq!((w.pid, w.cpu, w.command.as_str()), (900, 90.0, "my worker"));
        let n = r.processes.iter().find(|p| p.pid == 812).unwrap();
        assert_eq!((n.cpu, n.rss_bytes, n.unit.as_deref()), (50.0, 98304 * 1024, Some("nginx.service")));
        assert_eq!(r.by_service.len(), 1);
        assert_eq!(r.by_user[0].name, "deploy");
        // Without /proc (macOS), ps's own figure is used.
        let mac = parse_processes("@@cores 8\n@@ps\n  10 root 12.5 0.3 2048 /usr/sbin/sshd\n");
        assert_eq!(
            (mac.method.as_str(), mac.processes[0].cpu, mac.processes[0].command.as_str()),
            ("ps", 12.5, "sshd")
        );
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn scripts_run_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(PROC_SCRIPT).output().unwrap();
        let r = parse_processes(&String::from_utf8_lossy(&out.stdout));
        assert!(r.cores > 0 && !r.processes.is_empty());
        let tmp = std::env::temp_dir();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(disk_script(tmp.to_str().unwrap()).unwrap())
            .output()
            .unwrap();
        let u = parse_disk(tmp.to_str().unwrap(), &String::from_utf8_lossy(&out.stdout));
        assert!(u.fs_size.is_some());
    }
}
