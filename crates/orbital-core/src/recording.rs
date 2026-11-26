//! Terminal recordings in asciinema's v2 format (`.cast`): a JSON header,
//! then one `[seconds, "o", text]` line per piece of output and
//! `[seconds, "r", "COLSxROWS"]` per resize. Only output is recorded, never
//! keystrokes, and the sudo password Orbital types is already removed from
//! the output by the terminal. Files live in the app's data folder (0600)
//! and play back in Orbital or any asciinema player.

use crate::error::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static LIVE: Lazy<Mutex<HashMap<String, Arc<Recorder>>>> = Lazy::new(Default::default);
static NAME: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{8}T\d{6}-[0-9a-f]{8}\.cast$").unwrap());

/// Where recordings go (the app's data folder). Set once at startup.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn dir() -> Result<&'static Path> {
    DIR.get()
        .map(PathBuf::as_path)
        .ok_or_else(|| Error::Other("recordings aren't set up in this build".into()))
}

pub struct Recorder {
    pub name: String,
    pub path: PathBuf,
    started: Instant,
    inner: Mutex<(BufWriter<std::fs::File>, Vec<u8>)>,
}

impl Recorder {
    /// Start a recording in `dir`: the file is created readable by its owner only.
    pub fn create(dir: &Path, title: &str, cols: u32, rows: u32) -> Result<Arc<Self>> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let now = chrono::Utc::now();
        let name = format!(
            "{}-{}.cast",
            now.format("%Y%m%dT%H%M%S"),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let path = dir.join(&name);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut w = BufWriter::new(opts.open(&path)?);
        let header = serde_json::json!({
            "version": 2, "width": cols, "height": rows, "timestamp": now.timestamp(),
            "title": title, "env": { "TERM": "xterm-256color" },
        });
        writeln!(w, "{header}")?;
        w.flush()?;
        Ok(Arc::new(Self {
            name,
            path,
            started: Instant::now(),
            inner: Mutex::new((w, Vec::new())),
        }))
    }

    fn event(&self, kind: &str, text: &str) {
        let t = self.started.elapsed().as_secs_f64();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let line = serde_json::json!([(t * 1e6).round() / 1e6, kind, text]);
        let _ = writeln!(g.0, "{line}");
        let _ = g.0.flush();
    }

    /// Terminal output. A UTF-8 character split between two chunks waits for its end.
    pub fn output(&self, data: &[u8]) {
        let text = {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            g.1.extend_from_slice(data);
            let cut = complete_utf8(&g.1);
            let rest = g.1.split_off(cut);
            let done = std::mem::replace(&mut g.1, rest);
            String::from_utf8_lossy(&done).into_owned()
        };
        if !text.is_empty() {
            self.event("o", &text);
        }
    }

    pub fn resize(&self, cols: u32, rows: u32) {
        self.event("r", &format!("{cols}x{rows}"));
    }
}

/// How many leading bytes form whole UTF-8 characters (an incomplete one at
/// the very end is held back; anything else invalid is passed on as is).
pub fn complete_utf8(b: &[u8]) -> usize {
    let n = b.len();
    for back in 1..=3.min(n) {
        let i = n - back;
        let c = b[i];
        if c & 0xC0 == 0x80 {
            continue; // continuation byte: look further back
        }
        let need = if c >= 0xF0 {
            4
        } else if c >= 0xE0 {
            3
        } else if c >= 0xC0 {
            2
        } else {
            1
        };
        return if back < need { i } else { n };
    }
    n
}

/// Keep `r` as the recording of terminal `tid` (for resizes and the end).
pub fn register(tid: &str, r: Arc<Recorder>) {
    LIVE.lock().unwrap_or_else(|p| p.into_inner()).insert(tid.to_string(), r);
}

pub fn resized(tid: &str, cols: u32, rows: u32) {
    if let Some(r) = LIVE.lock().unwrap_or_else(|p| p.into_inner()).get(tid) {
        r.resize(cols, rows);
    }
}

/// The terminal closed: stop keeping its recording.
pub fn finish(tid: &str) -> Option<String> {
    LIVE.lock().unwrap_or_else(|p| p.into_inner()).remove(tid).map(|r| r.name.clone())
}

pub fn is_recording(tid: &str) -> bool {
    LIVE.lock().unwrap_or_else(|p| p.into_inner()).contains_key(tid)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingInfo {
    pub name: String,
    pub title: String,
    pub started: i64,
    pub seconds: f64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}

pub fn check_name(name: &str) -> Result<&str> {
    if NAME.is_match(name) {
        Ok(name)
    } else {
        Err(Error::InvalidInput(format!("{name:?} isn't a recording")))
    }
}

/// Header and length of one recording file.
pub fn info(path: &Path) -> Option<RecordingInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let h: serde_json::Value = serde_json::from_str(lines.next()?).ok()?;
    let seconds = lines
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()?.get(0)?.as_f64())
        .unwrap_or(0.0);
    Some(RecordingInfo {
        name: path.file_name()?.to_string_lossy().into_owned(),
        title: h["title"].as_str().unwrap_or("").to_string(),
        started: h["timestamp"].as_i64().unwrap_or(0),
        seconds,
        bytes: text.len() as u64,
        width: h["width"].as_u64().unwrap_or(80) as u32,
        height: h["height"].as_u64().unwrap_or(24) as u32,
    })
}

pub fn list(dir: &Path) -> Vec<RecordingInfo> {
    let mut v: Vec<RecordingInfo> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| NAME.is_match(&e.file_name().to_string_lossy()))
                .filter_map(|e| info(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.name.cmp(&a.name));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_utf8_only_at_character_ends() {
        let s = "añ€😀".as_bytes();
        assert_eq!(complete_utf8(s), s.len());
        assert_eq!(complete_utf8(&s[..2]), 1, "half of ñ waits");
        assert_eq!(complete_utf8(&s[..5]), 3, "two thirds of € wait");
        assert_eq!(complete_utf8(&s[..8]), 6, "most of 😀 waits");
        assert_eq!(complete_utf8(b"ab"), 2);
        assert_eq!(complete_utf8(b""), 0);
    }

    #[test]
    fn records_a_cast_file_and_lists_it() {
        let d = tempfile::tempdir().unwrap();
        let r = Recorder::create(d.path(), "deploy@203.0.113.10: ~", 100, 30).unwrap();
        r.output("$ ls\r\n".as_bytes());
        let e = "é".as_bytes();
        r.output(&e[..1]);
        r.output(&e[1..]);
        r.resize(120, 40);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&r.path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let text = std::fs::read_to_string(&r.path).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["version"], 2);
        assert_eq!(lines[0]["width"], 100);
        assert_eq!(lines[1][1], "o");
        assert_eq!(lines[1][2], "$ ls\r\n");
        assert_eq!(lines[2][2], "é", "the split character is one event");
        assert_eq!(lines[3][1], "r");
        assert_eq!(lines[3][2], "120x40");
        let all = list(d.path());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "deploy@203.0.113.10: ~");
        assert!(check_name(&all[0].name).is_ok());
        assert!(check_name("../vault.bin").is_err());
    }

    #[test]
    fn registry_follows_the_terminal() {
        let d = tempfile::tempdir().unwrap();
        let r = Recorder::create(d.path(), "t", 80, 24).unwrap();
        register("tid-1", r.clone());
        assert!(is_recording("tid-1"));
        resized("tid-1", 90, 20);
        assert_eq!(finish("tid-1").as_deref(), Some(r.name.as_str()));
        assert!(!is_recording("tid-1"));
        assert!(std::fs::read_to_string(&r.path).unwrap().contains("\"90x20\""));
    }
}
