//! The server's shell history (bash, zsh, fish), for the terminal's command
//! suggestions. Read-only and kept in memory by the UI: Orbital never writes
//! it anywhere. Lines that look like they carry a secret (`PASSWORD=…`,
//! `mysql -pSECRET`, tokens) are left out, so a suggestion never shows one on
//! screen.

use crate::error::Result;
use crate::ssh::SshSession;
use std::collections::HashSet;

/// The most commands returned, newest first.
pub const MAX_COMMANDS: usize = 1000;
const MAX_LEN: usize = 300;

/// The script that prints the history files, each after a `### <shell>` line.
/// Fixed text: nothing from the user or the server is interpolated into it.
pub fn script(root: bool) -> &'static str {
    if root {
        r#"h=/root; for s in bash:.bash_history zsh:.zsh_history fish:.local/share/fish/fish_history; do f="$h/${s#*:}"; [ -r "$f" ] && { printf '### %s\n' "${s%%:*}"; tail -n 3000 "$f"; printf '\n'; }; done; true"#
    } else {
        r#"h="$HOME"; for s in bash:.bash_history zsh:.zsh_history fish:.local/share/fish/fish_history; do f="$h/${s#*:}"; [ -r "$f" ] && { printf '### %s\n' "${s%%:*}"; tail -n 3000 "$f"; printf '\n'; }; done; true"#
    }
}

/// Parse the script's output into distinct commands, newest first.
pub fn parse(out: &str) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    let mut shell = "";
    for line in out.lines() {
        if let Some(s) = line.strip_prefix("### ") {
            shell = match s.trim() {
                "zsh" => "zsh",
                "fish" => "fish",
                _ => "bash",
            };
            continue;
        }
        let cmd = match shell {
            // Extended format ": 1700000000:0;command"; plain lines otherwise.
            "zsh" => match line.strip_prefix(": ").and_then(|r| r.split_once(';')) {
                Some((_, c)) => c,
                None => line,
            },
            // "- cmd: command" lines; "  when: …" and "  paths:" are metadata.
            "fish" => match line.strip_prefix("- cmd: ") {
                Some(c) => c,
                None => continue,
            },
            // bash: "#1700000000" timestamps when HISTTIMEFORMAT is set.
            _ => {
                if line.starts_with('#') && line[1..].chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                line
            }
        };
        let cmd = cmd.trim();
        // Multi-line commands (a trailing backslash) aren't useful as one suggestion.
        if cmd.is_empty() || cmd.len() > MAX_LEN || cmd.ends_with('\\') || cmd.chars().any(|c| c.is_control()) || looks_secret(cmd) {
            continue;
        }
        all.push(cmd.to_string());
    }
    // Newest first, each command once (where it was last used).
    let mut seen = HashSet::new();
    let mut out: Vec<String> = all.into_iter().rev().filter(|c| seen.insert(c.clone())).collect();
    out.truncate(MAX_COMMANDS);
    out
}

/// A command that seems to contain a password, token or key.
pub fn looks_secret(cmd: &str) -> bool {
    let l = cmd.to_ascii_lowercase();
    const WORDS: [&str; 8] = [
        "passw",
        "secret",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "private_key",
        "authorization:",
    ];
    if WORDS.iter().any(|w| l.contains(w)) {
        return true;
    }
    // mysql/mariadb -pSECRET (a password glued to -p), and sshpass -p.
    let words: Vec<&str> = l.split_whitespace().collect();
    let tool = words.first().map(|w| w.rsplit('/').next().unwrap_or(w)).unwrap_or("");
    let tool = if tool == "sudo" {
        words.get(1).copied().unwrap_or("")
    } else {
        tool
    };
    if matches!(tool, "mysql" | "mariadb" | "mysqldump" | "mysqladmin") && words.iter().any(|w| w.starts_with("-p") && w.len() > 2) {
        return true;
    }
    if tool == "sshpass" {
        return true;
    }
    // URLs with credentials: scheme://user:pass@host
    l.split_whitespace().any(|w| {
        w.split_once("://")
            .and_then(|(_, rest)| rest.split('/').next())
            .is_some_and(|auth| auth.contains('@') && auth.split('@').next().is_some_and(|u| u.contains(':')))
    })
}

impl SshSession {
    /// The login user's shell history, or root's (with sudo) for root terminals.
    pub async fn shell_history(&self, root: bool) -> Result<Vec<String>> {
        let script = script(root && self.elevation_needed());
        let out = if root && self.elevation_needed() {
            self.exec_privileged(script).await?
        } else {
            self.exec(script).await?
        };
        Ok(parse(&out.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bash_zsh_and_fish_newest_first() {
        let out = "### bash\n#1700000000\nls -la\ncd /etc/nginx\nnginx -t\nls -la\n\n### zsh\n: 1700000100:0;git pull\n: 1700000200:0;docker compose up -d\nplain zsh line\n\n### fish\n- cmd: htop\n  when: 1700000300\n- cmd: df -h\n  when: 1700000400\n  paths:\n    - /\n";
        assert_eq!(
            parse(out),
            [
                "df -h",
                "htop",
                "plain zsh line",
                "docker compose up -d",
                "git pull",
                "ls -la",
                "nginx -t",
                "cd /etc/nginx"
            ]
        );
    }

    #[test]
    fn leaves_out_secrets_multiline_and_noise() {
        let out = "### bash\nexport DB_PASSWORD=hunter2\nmysql -u root -phunter2\nsudo mysql -pX db\nmysql -u root -p\ncurl -H 'Authorization: Bearer abc'\ngit clone https://me:tok@github.com/x/y\ngit clone https://github.com/x/y\nsshpass -p x ssh host\nfor i in 1 2; do \\\necho ok\n";
        assert_eq!(parse(out), ["echo ok", "git clone https://github.com/x/y", "mysql -u root -p"]);
    }

    #[test]
    fn secret_detection() {
        assert!(looks_secret("TOKEN=abc ./deploy.sh"));
        assert!(looks_secret("/usr/bin/mysql -pS3cret"));
        assert!(looks_secret("psql postgres://app:pw@db/app"));
        assert!(!looks_secret("mysql -u root -p"));
        assert!(!looks_secret("psql postgres://db/app"));
        assert!(!looks_secret("ssh me@host"));
    }

    #[test]
    fn script_is_fixed_and_reads_only_history_files() {
        for root in [false, true] {
            let s = script(root);
            assert!(s.contains(".bash_history") && s.contains(".zsh_history") && s.contains("fish_history"));
            assert!(!s.contains('>'), "the script writes nothing");
        }
        assert!(script(true).starts_with("h=/root;"));
    }
}
