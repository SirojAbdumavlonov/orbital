//! New timer: a scheduled job as a systemd timer (a modern cron). Orbital
//! writes `NAME.service` (Type=oneshot, the command through /bin/sh -c) and
//! `NAME.timer` (OnCalendar=…) marked as its own, checks the schedule with
//! `systemd-analyze calendar` and the units with `systemd-analyze verify`,
//! then enables the timer. Logs go to the journal; missed runs (the server
//! was off) catch up at boot when "persistent" is on.

use crate::error::{Error, Result};
use crate::shell::quote;
use crate::units::{systemd_quote, MARK, UNIT_DIR};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    /// Run as this user (root when empty).
    #[serde(default)]
    pub user: Option<String>,
    /// systemd OnCalendar, e.g. `daily`, `Mon..Fri 09:00`, `*:0/15`.
    pub schedule: String,
    /// Run a missed job at boot.
    #[serde(default)]
    pub persistent: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleCheck {
    /// systemd's normalised form.
    pub normalized: String,
    /// The next run in words, as systemd prints it.
    pub next: String,
}

static NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9_.-]{0,62}$").unwrap());
static SCHEDULE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9 *:/,.~\-+]{1,100}$").unwrap());

pub fn validate(t: &TimerSpec) -> Result<TimerSpec> {
    let bad = |m: &str| Err(Error::InvalidInput(m.into()));
    let name = t.name.trim().trim_end_matches(".timer").trim_end_matches(".service").to_string();
    if !NAME.is_match(&name) {
        return bad("the name has lowercase letters, digits and - _ . (e.g. nightly-report)");
    }
    let command = t.command.trim();
    if command.is_empty() || command.len() > 4000 || command.chars().any(|c| c.is_control()) {
        return bad("give the command to run, on one line");
    }
    let schedule = t.schedule.split_whitespace().collect::<Vec<_>>().join(" ");
    if !SCHEDULE.is_match(&schedule) {
        return bad("write the schedule like daily, hourly, Mon..Fri 09:00 or *:0/15");
    }
    let description = t.description.trim();
    if description.chars().any(|c| c.is_control()) || description.chars().count() > 120 {
        return bad("the description is one short line");
    }
    let user = match t.user.as_deref().map(str::trim).filter(|u| !u.is_empty() && *u != "root") {
        Some(u) => Some(crate::users::validate_user(u)?.to_string()),
        None => None,
    };
    Ok(TimerSpec {
        name,
        description: description.into(),
        command: command.into(),
        user,
        schedule,
        persistent: t.persistent,
    })
}

pub fn service_path(name: &str) -> String {
    format!("{UNIT_DIR}/{name}.service")
}
pub fn timer_path(name: &str) -> String {
    format!("{UNIT_DIR}/{name}.timer")
}

pub fn render_service(t: &TimerSpec) -> String {
    let mut u = format!(
        "{MARK}; the job behind {name}.timer.\n[Unit]\nDescription={desc}\n\n[Service]\nType=oneshot\n",
        name = t.name,
        desc = if t.description.is_empty() {
            t.name.clone()
        } else {
            t.description.replace('%', "%%")
        }
    );
    if let Some(user) = &t.user {
        u.push_str(&format!("User={user}\n"));
    }
    u.push_str(&format!("ExecStart=/bin/sh -c {}\n", systemd_quote(&t.command)));
    u
}

pub fn render_timer(t: &TimerSpec) -> String {
    format!(
        "{MARK}; change it in Orbital (Cron → timers) or here.\n[Unit]\nDescription=Runs {name}.service: {sched}\n\n[Timer]\nOnCalendar={sched}\nPersistent={p}\nRandomizedDelaySec=0\n\n[Install]\nWantedBy=timers.target\n",
        name = t.name,
        sched = t.schedule.replace('%', "%%"),
        p = if t.persistent { "true" } else { "false" }
    )
}

/// `systemd-analyze calendar` for a schedule (validated first).
pub fn calendar_command(schedule: &str) -> Result<String> {
    let s = schedule.split_whitespace().collect::<Vec<_>>().join(" ");
    if !SCHEDULE.is_match(&s) {
        return Err(Error::InvalidInput(
            "write the schedule like daily, hourly, Mon..Fri 09:00 or *:0/15".into(),
        ));
    }
    Ok(format!("LC_ALL=C systemd-analyze calendar -- {} 2>&1", quote(&s)))
}

pub fn parse_calendar(out: &str, code: i32) -> Result<ScheduleCheck> {
    let get = |k: &str| {
        out.lines()
            .find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim_start_matches(':').trim().to_string()))
    };
    match (code, get("Normalized form"), get("Next elapse")) {
        (0, Some(n), next) => Ok(ScheduleCheck {
            normalized: n,
            next: next.unwrap_or_else(|| "never".into()),
        }),
        _ => Err(Error::InvalidInput(format!(
            "systemd doesn't understand this schedule: {}",
            out.lines().last().unwrap_or("").trim()
        ))),
    }
}

/// `@@none`, `@@ours` or `@@theirs <path>` for either unit name.
pub fn existing_check(name: &str) -> String {
    let (s, t) = (quote(&service_path(name)), quote(&timer_path(name)));
    format!(
        "for f in {s} {t}; do if [ -e \"$f\" ] && ! head -1 \"$f\" | grep -q '^{MARK}'; then echo \"@@theirs $f\"; exit 0; fi; done; \
         n=$(systemctl show -p FragmentPath --value -- {u} 2>/dev/null); \
         if [ -n \"$n\" ] && [ \"$n\" != {sp} ]; then echo \"@@theirs $n\"; exit 0; fi; \
         if [ -e {t} ]; then echo @@ours; else echo @@none; fi",
        u = quote(&format!("{name}.service")),
        sp = s
    )
}

pub fn enable_command(name: &str) -> String {
    let t = quote(&format!("{name}.timer"));
    format!("systemctl daemon-reload && systemctl enable --now -- {t} 2>&1 && systemctl list-timers --no-pager --all -- {t} 2>/dev/null | sed -n 2p")
}

pub fn remove_command(name: &str) -> String {
    let (s, t) = (quote(&service_path(name)), quote(&timer_path(name)));
    format!(
        "if ! head -1 {t} 2>/dev/null | grep -q '^{MARK}'; then echo 'only timers created by Orbital can be removed here' >&2; exit 3; fi; \
         systemctl disable --now -- {tu} 2>&1; rm -f -- {s} {t}; systemctl daemon-reload",
        tu = quote(&format!("{name}.timer"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TimerSpec {
        TimerSpec {
            name: "nightly-report.timer".into(),
            description: "Mail the sales report".into(),
            command: "cd /srv/shop && ./bin/report --since \"1 day ago\" > /var/log/report.log 2>&1".into(),
            user: Some("deploy".into()),
            schedule: "Mon..Fri   07:30".into(),
            persistent: true,
        }
    }

    #[test]
    fn validates_and_renders() {
        let t = validate(&spec()).unwrap();
        assert_eq!((t.name.as_str(), t.schedule.as_str()), ("nightly-report", "Mon..Fri 07:30"));
        let s = render_service(&t);
        assert!(s.starts_with(MARK));
        assert!(s.contains("Type=oneshot\nUser=deploy\nExecStart=/bin/sh -c \"cd /srv/shop && ./bin/report --since \\\"1 day ago\\\""));
        let tm = render_timer(&t);
        assert!(tm.contains("OnCalendar=Mon..Fri 07:30\nPersistent=true"));
        assert!(tm.contains("WantedBy=timers.target"));
        for bad in [
            TimerSpec {
                name: "Bad Name".into(),
                ..spec()
            },
            TimerSpec {
                schedule: "daily; reboot".into(),
                ..spec()
            },
            TimerSpec {
                schedule: "$(id)".into(),
                ..spec()
            },
            TimerSpec {
                command: "a\nb".into(),
                ..spec()
            },
            TimerSpec {
                user: Some("x;y".into()),
                ..spec()
            },
        ] {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            validate(&TimerSpec {
                user: Some("root".into()),
                ..spec()
            })
            .unwrap()
            .user,
            None
        );
    }

    #[test]
    fn reads_systemd_calendar_answers() {
        let ok = "  Original form: Mon..Fri 07:30\nNormalized form: Mon..Fri *-*-* 07:30:00\n    Next elapse: Fri 2026-11-27 07:30:00 UTC\n       From now: 8h left\n";
        let c = parse_calendar(ok, 0).unwrap();
        assert_eq!(c.normalized, "Mon..Fri *-*-* 07:30:00");
        assert_eq!(c.next, "Fri 2026-11-27 07:30:00 UTC");
        assert!(parse_calendar("Failed to parse calendar specification 'Fooday': Invalid argument", 1).is_err());
        assert!(calendar_command("daily").unwrap().ends_with("-- daily 2>&1"));
        assert!(calendar_command("daily && id").is_err());
    }

    // Runs the script meant for Linux servers through a local sh.
    #[cfg(unix)]
    #[test]
    fn scripts_are_valid_sh() {
        for s in [
            existing_check("x"),
            enable_command("x"),
            remove_command("x"),
            calendar_command("*:0/15").unwrap(),
        ] {
            let o = std::process::Command::new("sh").arg("-n").arg("-c").arg(&s).output().unwrap();
            assert!(o.status.success(), "{s}");
        }
    }
}
