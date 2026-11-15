//! Live logs: follow a log file, a systemd service's journal or a container's
//! output as it is written. Read-only; each source is one streaming channel.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_container, validate_path, validate_unit};
use crate::ssh::SshSession;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum LogSource {
    File { path: String },
    Unit { unit: String },
    Container { id: String },
}

impl LogSource {
    pub fn label(&self) -> String {
        match self {
            LogSource::File { path } => path.clone(),
            LogSource::Unit { unit } => unit.trim_end_matches(".service").to_string(),
            LogSource::Container { id } => id.clone(),
        }
    }
}

/// The command that prints the last `lines` and then follows new output.
pub fn follow_command(src: &LogSource, lines: u32) -> Result<String> {
    let n = lines.clamp(0, 5000);
    Ok(match src {
        LogSource::File { path } => format!("tail -n {n} -F -- {} 2>&1", quote(validate_path(path)?)),
        LogSource::Unit { unit } => format!(
            "journalctl -f -n {n} -o short-iso --no-pager -u {} 2>&1",
            quote(validate_unit(unit)?)
        ),
        LogSource::Container { id } => format!("docker logs -f --tail {n} --timestamps -- {} 2>&1", quote(validate_container(id)?)),
    })
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    pub path: String,
    pub size: u64,
}

const FILES_SCRIPT: &str = r#"export LC_ALL=C
find /var/log -maxdepth 3 -type f \( -name '*.log' -o -name syslog -o -name messages -o -name secure -o -name kern.log -o -name auth.log -o -name '*_log' \) -size +0 2>/dev/null \
  | grep -Ev '\.(gz|xz|bz2|zst)$|\.[0-9]+(\.log)?$' | head -n 300 \
  | while IFS= read -r f; do printf '%s\t%s\n' "$(wc -c < "$f" 2>/dev/null || echo 0)" "$f"; done
"#;

/// Log files under /var/log worth following (rotated and compressed copies left out).
pub fn parse_files(out: &str) -> Vec<LogFile> {
    let mut v: Vec<LogFile> = out
        .lines()
        .filter_map(|l| {
            let (size, path) = l.split_once('\t')?;
            path.starts_with('/').then(|| LogFile {
                path: path.to_string(),
                size: size.trim().parse().unwrap_or(0),
            })
        })
        .collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

impl SshSession {
    pub async fn log_files(&self) -> Result<Vec<LogFile>> {
        let (o, _) = self.exec_inspect(FILES_SCRIPT).await?;
        Ok(parse_files(&o.stdout))
    }
}

pub fn not_found(src: &LogSource) -> Error {
    Error::NotFound(format!("no log at {}", src.label()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_commands_are_quoted_and_validated() {
        assert_eq!(
            follow_command(
                &LogSource::File {
                    path: "/var/log/nginx/access.log".into()
                },
                200
            )
            .unwrap(),
            "tail -n 200 -F -- /var/log/nginx/access.log 2>&1"
        );
        assert_eq!(
            follow_command(
                &LogSource::Unit {
                    unit: "nginx.service".into()
                },
                9999
            )
            .unwrap(),
            "journalctl -f -n 5000 -o short-iso --no-pager -u nginx.service 2>&1"
        );
        assert!(follow_command(&LogSource::Container { id: "shop-web-1".into() }, 50)
            .unwrap()
            .contains("-- shop-web-1"));
        assert!(follow_command(
            &LogSource::File {
                path: "relative.log".into()
            },
            1
        )
        .is_err());
        assert!(follow_command(&LogSource::Unit { unit: "x; reboot".into() }, 1).is_err());
        assert!(follow_command(&LogSource::Container { id: "--privileged".into() }, 1).is_err());
        assert_eq!(
            follow_command(
                &LogSource::File {
                    path: "/var/log/my app.log".into()
                },
                1
            )
            .unwrap(),
            "tail -n 1 -F -- '/var/log/my app.log' 2>&1"
        );
    }

    #[test]
    fn lists_current_logs_only() {
        let files = parse_files("120\t/var/log/syslog\n0\t/var/log/nginx/error.log\n5000\t/var/log/nginx/access.log\nnoise\n");
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["/var/log/nginx/access.log", "/var/log/nginx/error.log", "/var/log/syslog"]
        );
        let out = std::process::Command::new("sh").arg("-c").arg(FILES_SCRIPT).output().unwrap();
        assert!(out.status.success());
    }
}
