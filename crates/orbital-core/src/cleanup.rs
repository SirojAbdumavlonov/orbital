//! Disk cleanup: the usual places a server's disk fills up with things that
//! can go, how much each holds, and a fixed command to free it. Nothing here
//! deletes user data: only the system journal beyond a week, the package
//! manager's download cache, rotated old logs, and Docker leftovers (stopped
//! containers, unused networks, dangling images, build cache; images no
//! container uses only when asked separately; volumes never).
//!
//! The scan script and every command are fixed text.

use crate::error::{Error, Result};
use crate::ssh::SshSession;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub id: String,
    pub title: String,
    pub detail: String,
    /// Bytes it would free, when known.
    pub bytes: Option<u64>,
}

struct Kind {
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    command: &'static str,
}

const KINDS: &[Kind] = &[
    Kind {
        id: "journal",
        title: "System journal older than a week",
        detail: "systemd's log of everything. The last 7 days are kept.",
        command: "journalctl --vacuum-time=7d",
    },
    Kind {
        id: "apt",
        title: "Downloaded package files (apt)",
        detail: "Installers apt kept after installing updates. Downloaded again if ever needed.",
        command: "apt-get clean",
    },
    Kind {
        id: "dnf",
        title: "Downloaded package files (dnf)",
        detail: "Installers dnf kept after installing updates. Downloaded again if ever needed.",
        command: "dnf clean packages",
    },
    Kind {
        id: "oldlogs",
        title: "Rotated old log files",
        detail: "Compressed and numbered logs in /var/log (*.gz, *.1, *.old). Current logs are kept.",
        command: r"find /var/log -type f \( -name '*.gz' -o -name '*.[0-9]' -o -name '*.old' \) -delete",
    },
    Kind {
        id: "docker",
        title: "Docker leftovers",
        detail: "Stopped containers, unused networks, dangling images and build cache. Running containers and volumes (data) are kept.",
        command: "docker system prune -f",
    },
    Kind {
        id: "dockerimages",
        title: "Docker images no container uses",
        detail: "Pulled again when a container needs one. Can take a while to download later.",
        command: "docker image prune -a -f",
    },
];

/// Prints `id=bytes` for every place that exists on this server.
pub const SCAN_SCRIPT: &str = r#"
sz() { du -sb "$@" 2>/dev/null | awk '{s+=$1} END {print s+0}'; }
[ -d /var/log/journal ] && command -v journalctl >/dev/null 2>&1 && echo "journal=$(sz /var/log/journal)"
command -v apt-get >/dev/null 2>&1 && echo "apt=$(sz /var/cache/apt/archives)"
command -v dnf >/dev/null 2>&1 && echo "dnf=$(sz /var/cache/dnf)"
echo "oldlogs=$(find /var/log -type f \( -name '*.gz' -o -name '*.[0-9]' -o -name '*.old' \) -printf '%s\n' 2>/dev/null | awk '{s+=$1} END {print s+0}')"
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  docker system df --format '{{.Type}}|{{.Reclaimable}}' 2>/dev/null | sed 's/^/docker:/'
fi
true
"#;

/// "1.2GB (40%)", "512.3MB", "0B" → bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.split_whitespace().next()?.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mul = match unit.to_ascii_uppercase().as_str() {
        "B" => 1.0,
        "KB" | "K" => 1e3,
        "MB" | "M" => 1e6,
        "GB" | "G" => 1e9,
        "TB" | "T" => 1e12,
        _ => return None,
    };
    Some((n * mul).round() as u64)
}

pub fn parse_scan(out: &str) -> Vec<CleanupItem> {
    let mut sizes: Vec<(&str, Option<u64>)> = Vec::new();
    let (mut docker_all, mut docker_images, mut docker_seen) = (0u64, 0u64, false);
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("docker:") {
            let Some((kind, size)) = rest.split_once('|') else { continue };
            let b = parse_size(size).unwrap_or(0);
            docker_seen = true;
            if kind == "Images" {
                docker_images = b;
            } else if kind != "Local Volumes" {
                docker_all += b;
            }
        } else if let Some((k, v)) = line.split_once('=') {
            if let Some(kind) = KINDS.iter().find(|x| x.id == k) {
                sizes.push((kind.id, v.trim().parse().ok()));
            }
        }
    }
    if docker_seen {
        // `docker system prune` frees the non-image leftovers plus dangling
        // images; the rest of the reclaimable image space needs `image prune -a`.
        sizes.push(("docker", Some(docker_all)));
        sizes.push(("dockerimages", Some(docker_images)));
    }
    KINDS
        .iter()
        .filter_map(|k| {
            let (_, bytes) = sizes.iter().find(|(id, _)| *id == k.id)?;
            Some(CleanupItem {
                id: k.id.into(),
                title: k.title.into(),
                detail: k.detail.into(),
                bytes: *bytes,
            })
        })
        .collect()
}

/// The fixed command for a cleanup item.
pub fn command(id: &str) -> Result<&'static str> {
    KINDS
        .iter()
        .find(|k| k.id == id)
        .map(|k| k.command)
        .ok_or_else(|| Error::InvalidInput(format!("unknown cleanup: {id}")))
}

impl SshSession {
    /// What could be freed (read-only; root's view when sudo works).
    pub async fn cleanup_scan(&self) -> Result<Vec<CleanupItem>> {
        let (o, _) = self.exec_inspect(SCAN_SCRIPT).await?;
        Ok(parse_scan(&o.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1.2GB (40%)"), Some(1_200_000_000));
        assert_eq!(parse_size("512.3MB"), Some(512_300_000));
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("n/a"), None);
    }

    #[test]
    fn scan_lists_what_exists_with_sizes() {
        let out = "journal=734003200\napt=52428800\noldlogs=10485760\ndocker:Images|3.1GB (62%)\ndocker:Containers|12.5MB (100%)\ndocker:Local Volumes|2GB (50%)\ndocker:Build Cache|800MB\n";
        let items = parse_scan(out);
        let ids: Vec<_> = items.iter().map(|i| (i.id.as_str(), i.bytes)).collect();
        assert_eq!(
            ids,
            [
                ("journal", Some(734003200)),
                ("apt", Some(52428800)),
                ("oldlogs", Some(10485760)),
                ("docker", Some(812_500_000)),
                ("dockerimages", Some(3_100_000_000)),
            ],
            "volumes are never counted"
        );
        assert!(parse_scan("oldlogs=0\n").iter().all(|i| i.id == "oldlogs"));
    }

    #[test]
    fn commands_are_fixed_and_never_touch_volumes() {
        assert_eq!(command("apt").unwrap(), "apt-get clean");
        assert!(command("rm -rf /").is_err());
        for k in KINDS {
            assert!(!k.command.contains("volume"), "{}", k.id);
            assert!(!k.command.contains("--volumes"), "{}", k.id);
        }
        assert!(command("journal").unwrap().contains("7d"));
    }
}
