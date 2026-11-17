//! Agentless server metrics. One round-trip runs a POSIX script that dumps
//! /proc and friends in tagged sections; parsing happens locally, so nothing
//! has to be installed on the server.

use crate::error::Result;
use crate::ssh::SshSession;
use serde::Serialize;
use std::collections::HashMap;

pub const SNAPSHOT_SCRIPT: &str = r#"LC_ALL=C; export LC_ALL
echo '@@stat'; grep '^cpu' /proc/stat 2>/dev/null
echo '@@meminfo'; cat /proc/meminfo 2>/dev/null
echo '@@physmem'; b=$(cat /sys/devices/system/memory/block_size_bytes 2>/dev/null) && echo "$b $(cat /sys/devices/system/memory/memory*/online 2>/dev/null | grep -c 1)"
echo '@@loadavg'; cat /proc/loadavg 2>/dev/null
echo '@@uptime'; cat /proc/uptime 2>/dev/null
echo '@@nproc'; nproc 2>/dev/null || getconf _NPROCESSORS_ONLN
echo '@@cpuinfo'; grep -m1 -E 'model name|Hardware|Model' /proc/cpuinfo 2>/dev/null
echo '@@os'; cat /etc/os-release 2>/dev/null
echo '@@uname'; uname -srm
echo '@@hostname'; hostname 2>/dev/null || cat /etc/hostname
echo '@@df'; df -PkT 2>/dev/null | grep -vE '^(tmpfs|devtmpfs|overlay|squashfs|udev|none|shm)' 
echo '@@net'; cat /proc/net/dev 2>/dev/null
echo '@@ps'; ps -eo pid=,user=,pcpu=,pmem=,rss=,comm= --sort=-pcpu 2>/dev/null | head -n 15
echo '@@end'
"#;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CpuTimes {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub filesystem: String,
    pub fs_type: String,
    pub mount: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetInterface {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: u32,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub rss_kb: u64,
    pub command: String,
}

/// Where the memory goes, from /proc/meminfo (bytes). `used + buff_cache +
/// free == total`, like `free`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDetail {
    /// RAM in the machine (online memory blocks); None where /sys doesn't say.
    pub installed: Option<u64>,
    /// Used by programs and the kernel, not counting cache.
    pub used: u64,
    /// Page cache, buffers and reclaimable kernel caches: given back when programs need it.
    pub buff_cache: u64,
    pub free: u64,
    pub buffers: u64,
    pub page_cache: u64,
    /// Programs' own memory (heap, stacks).
    pub anon: u64,
    /// Shared memory and tmpfs (/dev/shm, /run).
    pub shared: u64,
    pub slab_reclaimable: u64,
    pub slab_unreclaimable: u64,
    pub kernel_stack: u64,
    pub page_tables: u64,
    /// Changed file data not yet written to disk.
    pub dirty: u64,
    /// Memory programs were promised, and how much the kernel allows with its overcommit setting.
    pub committed: u64,
    pub commit_limit: u64,
    pub active: u64,
    pub inactive: u64,
    pub swap_cached: u64,
    pub huge_pages: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub hostname: String,
    pub os_name: String,
    pub kernel: String,
    pub cpu_model: String,
    pub cores: u32,
    pub cpu: CpuTimes,
    pub per_core: Vec<CpuTimes>,
    pub load: [f64; 3],
    pub mem_total: u64,
    pub mem_available: u64,
    pub mem_used: u64,
    pub mem_cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub memory: MemoryDetail,
    pub uptime_secs: u64,
    pub disks: Vec<Disk>,
    pub net: Vec<NetInterface>,
    pub processes: Vec<Process>,
}

/// A snapshot plus rates derived from the previous one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub cpu_percent: Option<f64>,
    pub per_core_percent: Vec<f64>,
    pub net_rx_per_sec: Option<f64>,
    pub net_tx_per_sec: Option<f64>,
    pub sampled_at_ms: i64,
}

fn sections(out: &str) -> HashMap<&str, Vec<&str>> {
    let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut cur = "";
    for line in out.lines() {
        if let Some(tag) = line.strip_prefix("@@") {
            cur = tag.trim();
            map.entry(cur).or_default();
        } else if !cur.is_empty() {
            map.entry(cur).or_default().push(line);
        }
    }
    map
}

fn parse_cpu_line(l: &str) -> CpuTimes {
    let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    // user nice system idle iowait irq softirq steal (guest fields are already in user)
    let total: u64 = v.iter().take(8).sum();
    let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
    CpuTimes { total, idle }
}

pub fn cpu_percent(prev: &CpuTimes, cur: &CpuTimes) -> Option<f64> {
    let dt = cur.total.checked_sub(prev.total)?;
    if dt == 0 {
        return None;
    }
    let di = cur.idle.saturating_sub(prev.idle);
    Some(((dt.saturating_sub(di)) as f64 / dt as f64 * 100.0).clamp(0.0, 100.0))
}

/// `block_size_hex online_blocks` from /sys/devices/system/memory.
fn installed_memory(line: &str) -> Option<u64> {
    let mut it = line.split_whitespace();
    let size = u64::from_str_radix(it.next()?.trim_start_matches("0x"), 16).ok()?;
    let blocks: u64 = it.next()?.parse().ok()?;
    (size > 0 && blocks > 0).then(|| size.saturating_mul(blocks))
}

fn memory_detail(mem: &HashMap<&str, u64>, physmem: Option<&str>) -> MemoryDetail {
    let g = |k: &str| mem.get(k).copied().unwrap_or(0);
    let total = g("MemTotal");
    let free = g("MemFree");
    let buff_cache = g("Buffers") + g("Cached") + g("SReclaimable");
    MemoryDetail {
        // Online blocks can't be less than what the kernel manages; otherwise /sys is lying (or partial).
        installed: physmem.and_then(installed_memory).filter(|&b| b >= total),
        used: total.saturating_sub(free).saturating_sub(buff_cache),
        buff_cache: buff_cache.min(total.saturating_sub(free)),
        free,
        buffers: g("Buffers"),
        page_cache: g("Cached"),
        anon: g("AnonPages"),
        shared: g("Shmem"),
        slab_reclaimable: g("SReclaimable"),
        slab_unreclaimable: g("SUnreclaim"),
        kernel_stack: g("KernelStack"),
        page_tables: g("PageTables"),
        dirty: g("Dirty") + g("Writeback"),
        committed: g("Committed_AS"),
        commit_limit: g("CommitLimit"),
        active: g("Active"),
        inactive: g("Inactive"),
        swap_cached: g("SwapCached"),
        // HugePages_* are counts, not kB; Hugetlb is the total in kB (×1024 above).
        huge_pages: g("Hugetlb"),
    }
}

pub fn parse_snapshot(out: &str) -> Snapshot {
    let s = sections(out);
    let get = |k: &str| s.get(k).cloned().unwrap_or_default();
    let mut snap = Snapshot::default();

    for l in get("stat") {
        if l.starts_with("cpu ") {
            snap.cpu = parse_cpu_line(l);
        } else if l.starts_with("cpu") {
            snap.per_core.push(parse_cpu_line(l));
        }
    }

    let mem: HashMap<&str, u64> = get("meminfo")
        .iter()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim(), v.split_whitespace().next()?.parse::<u64>().ok()? * 1024))
        })
        .collect();
    snap.mem_total = *mem.get("MemTotal").unwrap_or(&0);
    snap.mem_available = mem
        .get("MemAvailable")
        .copied()
        .unwrap_or_else(|| mem.get("MemFree").unwrap_or(&0) + mem.get("Buffers").unwrap_or(&0) + mem.get("Cached").unwrap_or(&0));
    snap.mem_used = snap.mem_total.saturating_sub(snap.mem_available);
    snap.mem_cached = mem.get("Cached").unwrap_or(&0) + mem.get("Buffers").unwrap_or(&0);
    snap.swap_total = *mem.get("SwapTotal").unwrap_or(&0);
    snap.swap_used = snap.swap_total.saturating_sub(*mem.get("SwapFree").unwrap_or(&0));
    snap.memory = memory_detail(&mem, get("physmem").first().copied());

    if let Some(l) = get("loadavg").first() {
        let v: Vec<f64> = l.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
        if v.len() == 3 {
            snap.load = [v[0], v[1], v[2]];
        }
    }
    if let Some(l) = get("uptime").first() {
        snap.uptime_secs = l.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0) as u64;
    }
    snap.cores = get("nproc")
        .first()
        .and_then(|l| l.trim().parse().ok())
        .unwrap_or(snap.per_core.len() as u32);
    snap.cpu_model = get("cpuinfo")
        .first()
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();
    let os: HashMap<&str, String> = get("os")
        .iter()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k, v.trim_matches('"').to_string()))
        .collect();
    snap.os_name = os
        .get("PRETTY_NAME")
        .or(os.get("NAME"))
        .cloned()
        .unwrap_or_else(|| "Unknown".into());
    snap.kernel = get("uname").first().map(|s| s.trim().to_string()).unwrap_or_default();
    snap.hostname = get("hostname").first().map(|s| s.trim().to_string()).unwrap_or_default();

    for l in get("df").iter().skip(1) {
        let c: Vec<&str> = l.split_whitespace().collect();
        if c.len() < 7 {
            continue;
        }
        let (total, used, avail) = (c[2].parse::<u64>(), c[3].parse::<u64>(), c[4].parse::<u64>());
        if let (Ok(t), Ok(u), Ok(a)) = (total, used, avail) {
            if t == 0 {
                continue;
            }
            snap.disks.push(Disk {
                filesystem: c[0].into(),
                fs_type: c[1].into(),
                mount: c[6..].join(" "),
                total_bytes: t * 1024,
                used_bytes: u * 1024,
                available_bytes: a * 1024,
                used_percent: if u + a > 0 { u as f64 / (u + a) as f64 * 100.0 } else { 0.0 },
            });
        }
    }

    for l in get("net").iter().skip(2) {
        if let Some((name, rest)) = l.split_once(':') {
            let name = name.trim();
            if name == "lo" {
                continue;
            }
            let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 9 {
                snap.net.push(NetInterface {
                    name: name.into(),
                    rx_bytes: v[0],
                    tx_bytes: v[8],
                });
            }
        }
    }

    for l in get("ps") {
        let c: Vec<&str> = l.split_whitespace().collect();
        if c.len() >= 6 {
            if let Ok(pid) = c[0].parse() {
                snap.processes.push(Process {
                    pid,
                    user: c[1].into(),
                    cpu_percent: c[2].parse().unwrap_or(0.0),
                    mem_percent: c[3].parse().unwrap_or(0.0),
                    rss_kb: c[4].parse().unwrap_or(0),
                    command: c[5..].join(" "),
                });
            }
        }
    }
    snap
}

pub fn derive(prev: Option<&Metrics>, snap: Snapshot, now_ms: i64) -> Metrics {
    let (mut cpu, mut per_core, mut rx, mut tx) = (None, Vec::new(), None, None);
    if let Some(p) = prev {
        cpu = cpu_percent(&p.snapshot.cpu, &snap.cpu);
        per_core = p
            .snapshot
            .per_core
            .iter()
            .zip(&snap.per_core)
            .map(|(a, b)| cpu_percent(a, b).unwrap_or(0.0))
            .collect();
        let dt = (now_ms - p.sampled_at_ms) as f64 / 1000.0;
        if dt > 0.0 {
            let sum = |s: &Snapshot| s.net.iter().fold((0u64, 0u64), |(r, t), n| (r + n.rx_bytes, t + n.tx_bytes));
            let (pr, pt) = sum(&p.snapshot);
            let (cr, ct) = sum(&snap);
            rx = Some(cr.saturating_sub(pr) as f64 / dt);
            tx = Some(ct.saturating_sub(pt) as f64 / dt);
        }
    }
    Metrics {
        snapshot: snap,
        cpu_percent: cpu,
        per_core_percent: per_core,
        net_rx_per_sec: rx,
        net_tx_per_sec: tx,
        sampled_at_ms: now_ms,
    }
}

impl SshSession {
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let out = self.exec(SNAPSHOT_SCRIPT).await?;
        Ok(parse_snapshot(&out.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const FIXTURE: &str = include_str!("../tests/fixtures/snapshot.txt");

    #[test]
    fn parses_fixture() {
        let s = parse_snapshot(FIXTURE);
        assert_eq!(s.hostname, "web-01");
        assert_eq!(s.os_name, "Ubuntu 24.04.1 LTS");
        assert_eq!(s.cores, 4);
        assert_eq!(s.per_core.len(), 4);
        assert!(s.cpu_model.contains("EPYC"));
        assert_eq!(s.mem_total, 8_143_288 * 1024);
        assert_eq!(s.mem_used, (8_143_288 - 5_120_000) * 1024);
        assert_eq!(s.load, [0.42, 0.37, 0.30]);
        let m = &s.memory;
        assert_eq!(m.installed, Some(8 * 1024 * 1024 * 1024));
        assert_eq!(m.free, 912_004 * 1024);
        assert_eq!(m.buff_cache, (200_000 + 3_500_000 + 150_000) * 1024);
        assert_eq!(m.used + m.buff_cache + m.free, s.mem_total, "adds up like `free`");
        assert_eq!((m.anon, m.shared, m.dirty), (2_400_000 * 1024, 64_000 * 1024, (1_200 + 8) * 1024));
        assert_eq!((m.committed, m.commit_limit), (4_100_000 * 1024, 6_168_792 * 1024));
        assert_eq!(s.uptime_secs, 864_123);
        assert_eq!(s.disks.len(), 2);
        assert_eq!(s.disks[0].mount, "/");
        assert_eq!(s.net.len(), 1);
        assert_eq!(s.net[0].name, "eth0");
        assert_eq!(s.processes[0].command, "nginx");
    }

    #[test]
    fn installed_memory_from_sys() {
        assert_eq!(installed_memory("8000000 64"), Some(64 * 128 * 1024 * 1024));
        assert_eq!(installed_memory("0x8000000 2"), Some(256 * 1024 * 1024));
        assert_eq!(installed_memory(""), None);
        assert_eq!(installed_memory("zz 3"), None);
        // An old kernel without /sys/devices/system/memory: nothing after the tag.
        let s = parse_snapshot("@@meminfo\nMemTotal: 1024 kB\nMemFree: 512 kB\n@@physmem\n@@end\n");
        assert_eq!(s.memory.installed, None);
        assert_eq!(s.memory.used, 512 * 1024);
    }

    #[test]
    fn cpu_rates() {
        let a = CpuTimes { total: 1000, idle: 800 };
        let b = CpuTimes { total: 1100, idle: 850 };
        assert_eq!(cpu_percent(&a, &b), Some(50.0));
        assert_eq!(cpu_percent(&b, &b), None);
        let s1 = parse_snapshot(FIXTURE);
        let m1 = derive(None, s1.clone(), 0);
        assert!(m1.cpu_percent.is_none());
        let mut s2 = s1;
        s2.cpu.total += 200;
        s2.cpu.idle += 150;
        s2.net[0].rx_bytes += 2048;
        let m2 = derive(Some(&m1), s2, 2000);
        assert_eq!(m2.cpu_percent, Some(25.0));
        assert_eq!(m2.net_rx_per_sec, Some(1024.0));
    }
}
