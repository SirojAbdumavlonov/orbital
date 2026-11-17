//! Import hosts from OpenSSH's client configuration (`~/.ssh/config`).
//!
//! Only the config files themselves are read (the main file and whatever its
//! `Include` lines name); keys stay where they are and are referenced by path.
//! Settings follow ssh's rules: for each option the first value that applies
//! to a host wins, `Host` patterns may be negated, and `Host *` supplies
//! defaults. `Match` blocks can't be evaluated without a connection, so they
//! are skipped with a note (except `Match all`).
//!
//! The result is a list of [`Candidate`]s: one per concrete host alias, plus a
//! candidate for every `ProxyJump` hop that isn't itself a host in the file.
//! Candidates point at their jump host by key, so the engine can create them
//! in order and wire up `jump_profile_id`.

use crate::profiles::{AuthMethod, MAX_HOPS};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// ssh's own limit on nested `Include`s.
const MAX_INCLUDE_DEPTH: usize = 16;

/// Who and where the import runs as: used for `~`, `%d`, `%u` and the default user.
#[derive(Debug, Clone)]
pub struct Context {
    pub home: PathBuf,
    pub user: String,
}

impl Context {
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let user = std::env::var("USER")
            .ok()
            .filter(|u| !u.is_empty())
            .or_else(|| home.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "root".into());
        Self { home, user }
    }

    pub fn default_config(&self) -> PathBuf {
        self.home.join(".ssh").join("config")
    }

    /// `/Users/me/.ssh/x` → `~/.ssh/x`, for display and for stored key paths.
    fn tilde(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rest) if self.home != Path::new("/") => {
                if rest.as_os_str().is_empty() {
                    "~".into()
                } else {
                    format!("~/{}", rest.display())
                }
            }
            _ => p.display().to_string(),
        }
    }

    fn expand_tilde(&self, s: &str) -> PathBuf {
        if s == "~" {
            self.home.clone()
        } else if let Some(rest) = s.strip_prefix("~/") {
            self.home.join(rest)
        } else {
            PathBuf::from(s)
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NoteLevel {
    Info,
    Warn,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub level: NoteLevel,
    pub text: String,
}

fn info(text: impl Into<String>) -> Note {
    Note {
        level: NoteLevel::Info,
        text: text.into(),
    }
}
fn warn(text: impl Into<String>) -> Note {
    Note {
        level: NoteLevel::Warn,
        text: text.into(),
    }
}

/// One server the config describes, as it would be saved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Stable identity: `user@host:port`, plus ` via <jump key>` for routed hosts.
    pub key: String,
    /// The `Host` alias (or the `ProxyJump` hop as written).
    pub name: String,
    /// Other aliases that resolve to the same server.
    pub aliases: Vec<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub key_path: Option<String>,
    /// Key of the candidate this one connects through.
    pub jump_key: Option<String>,
    /// Not a `Host` entry: only here because another host jumps through it.
    pub jump_only: bool,
    pub importable: bool,
    /// `~/.ssh/config:12`
    pub source: Option<String>,
    pub notes: Vec<Note>,
    /// Filled in by the engine: a saved profile with the same host, port and user.
    pub existing_id: Option<String>,
    pub existing_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub path: String,
    pub found: bool,
    pub candidates: Vec<Candidate>,
    /// File-level notes: skipped `Match` blocks, patterns, missing includes.
    pub notes: Vec<Note>,
}

// ---------- reading the files ----------

#[derive(Debug, Clone, PartialEq)]
enum Cond {
    Host(Vec<String>),
    All,
    Never,
}

impl Cond {
    fn matches(&self, name: &str) -> bool {
        match self {
            Cond::All => true,
            Cond::Never => false,
            Cond::Host(patterns) => host_matches(patterns, name),
        }
    }
}

#[derive(Debug)]
struct Directive {
    key: String,
    args: Vec<String>,
    conds: Vec<Cond>,
    src: String,
}

#[derive(Debug, Default)]
struct Parsed {
    directives: Vec<Directive>,
    /// Concrete aliases in file order, with where they were declared.
    aliases: Vec<(String, String)>,
    notes: Vec<Note>,
    found: bool,
}

/// Split one config line into a lower-cased keyword and its arguments, the way
/// ssh does: `Key value`, `Key=value`, `Key = value`, double quotes group, an
/// unquoted `#` starts a comment. `Ok(None)` for blank and comment lines.
fn split_line(line: &str) -> Result<Option<(String, Vec<String>)>, String> {
    let s = line.trim();
    if s.is_empty() || s.starts_with('#') {
        return Ok(None);
    }
    let end = s.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(s.len());
    let key = s[..end].to_ascii_lowercase();
    let mut rest = s[end..].trim_start();
    if let Some(r) = rest.strip_prefix('=') {
        rest = r.trim_start();
    }
    let mut args = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in rest.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            '#' if !quoted && !started => break,
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("unbalanced quotes".into());
    }
    if started {
        args.push(cur);
    }
    Ok(Some((key, args)))
}

fn is_pattern(s: &str) -> bool {
    s.contains(['*', '?'])
}

/// `*` and `?` globbing, case-insensitive (ssh lower-cases host names).
fn wild(pattern: &str, s: &str) -> bool {
    glob_match(&pattern.to_lowercase(), &s.to_lowercase())
}

fn glob_match(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = s.chars().collect();
    let (mut pi, mut si) = (0, 0);
    let mut back: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            back = Some((pi, si));
            pi += 1;
        } else if let Some((bp, bs)) = back {
            pi = bp + 1;
            si = bs + 1;
            back = Some((bp, bs + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// A `Host` line matches when any positive pattern does and no negated one does.
fn host_matches(patterns: &[String], name: &str) -> bool {
    let mut hit = false;
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if wild(neg, name) {
                return false;
            }
        } else if wild(p, name) {
            hit = true;
        }
    }
    hit
}

/// Expand an `Include` argument: `~`, relative to `~/.ssh`, globs in any
/// path component. Results are sorted, as glob(3) returns them.
fn expand_include(arg: &str, ctx: &Context) -> Vec<PathBuf> {
    let p = ctx.expand_tilde(arg);
    let p = if p.is_absolute() { p } else { ctx.home.join(".ssh").join(p) };
    let mut out = vec![PathBuf::from("/")];
    for comp in p.components().skip(1) {
        let comp = comp.as_os_str().to_string_lossy().into_owned();
        if !is_pattern(&comp) {
            out = out.into_iter().map(|d| d.join(&comp)).collect();
            continue;
        }
        let mut next = Vec::new();
        for dir in out {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let mut names: Vec<String> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| (!n.starts_with('.') || comp.starts_with('.')) && glob_match(&comp, n))
                .collect();
            names.sort();
            next.extend(names.into_iter().map(|n| dir.join(n)));
        }
        out = next;
    }
    out.into_iter().filter(|p| p.is_file()).collect()
}

impl Parsed {
    fn read(&mut self, path: &Path, outer: &[Cond], depth: usize, ctx: &Context) {
        let shown = ctx.tilde(path);
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                if depth > 0 {
                    self.notes.push(warn(format!("Couldn't read {shown}: {e}")));
                }
                return;
            }
        };
        if depth == 0 {
            self.found = true;
        }
        let mut current: Option<Cond> = None;
        let mut match_noted = false;
        for (i, line) in text.lines().enumerate() {
            let src = format!("{shown}:{}", i + 1);
            let (key, args) = match split_line(line) {
                Ok(Some(x)) => x,
                Ok(None) => continue,
                Err(e) => {
                    self.notes.push(warn(format!("{src}: {e}; line skipped")));
                    continue;
                }
            };
            let mut conds: Vec<Cond> = outer.to_vec();
            conds.extend(current.clone());
            match key.as_str() {
                "host" => {
                    match_noted = false;
                    let live = !outer.contains(&Cond::Never);
                    let concrete: Vec<&String> = args.iter().filter(|a| !a.starts_with('!') && !is_pattern(a)).collect();
                    if live {
                        for a in &concrete {
                            self.aliases.push((a.to_string(), src.clone()));
                        }
                        if concrete.is_empty() && args.iter().any(|a| a != "*") {
                            self.notes.push(info(format!(
                                "Host {} ({src}) is a pattern, not a server: its settings are applied to the hosts it matches",
                                args.join(" ")
                            )));
                        }
                    }
                    current = Some(Cond::Host(args));
                }
                "match" => {
                    if args.len() == 1 && args[0].eq_ignore_ascii_case("all") {
                        current = Some(Cond::All);
                    } else {
                        current = Some(Cond::Never);
                        if !match_noted {
                            self.notes.push(warn(format!(
                                "Match {} ({src}) was skipped: Match conditions can't be checked before connecting",
                                args.join(" ")
                            )));
                            match_noted = true;
                        }
                    }
                }
                "include" => {
                    if depth + 1 > MAX_INCLUDE_DEPTH {
                        self.notes.push(warn(format!("{src}: Include nested too deeply; skipped")));
                        continue;
                    }
                    for arg in &args {
                        let files = expand_include(arg, ctx);
                        if files.is_empty() && !is_pattern(arg) {
                            self.notes.push(info(format!("{src}: Include {arg} matched no file")));
                        }
                        for f in files {
                            self.read(&f, &conds, depth + 1, ctx);
                        }
                    }
                }
                _ => self.directives.push(Directive { key, args, conds, src }),
            }
        }
    }

    fn effective(&self, name: &str) -> Effective {
        let mut e = Effective::default();
        for d in &self.directives {
            if !d.conds.iter().all(|c| c.matches(name)) {
                continue;
            }
            let first = || d.args.first().cloned();
            match d.key.as_str() {
                "hostname" if e.hostname.is_none() => e.hostname = first(),
                "user" if e.user.is_none() => e.user = first(),
                "port" if e.port.is_none() => e.port = first().map(|p| (p, d.src.clone())),
                "identityfile" => e.identity_files.extend(first()),
                "proxyjump" if e.proxy.is_none() => e.proxy = Some(Proxy::Jump(d.args.join(","), d.src.clone())),
                "proxycommand" if e.proxy.is_none() => e.proxy = Some(Proxy::Command(d.args.clone(), d.src.clone())),
                "localforward" | "remoteforward" | "dynamicforward" => e.forwards = true,
                "certificatefile" => e.certificate = true,
                _ => {}
            }
        }
        e
    }
}

#[derive(Debug, Clone)]
enum Proxy {
    Jump(String, String),
    Command(Vec<String>, String),
}

#[derive(Debug, Default)]
struct Effective {
    hostname: Option<String>,
    user: Option<String>,
    port: Option<(String, String)>,
    identity_files: Vec<String>,
    proxy: Option<Proxy>,
    forwards: bool,
    certificate: bool,
}

// ---------- turning hosts into candidates ----------

#[derive(Debug, Clone, PartialEq)]
struct Hop {
    user: Option<String>,
    host: String,
    port: Option<u16>,
    written: String,
}

/// `[user@]host[:port]`, `[user@][v6]:port` or `ssh://[user@]host[:port]`.
fn parse_hop(s: &str) -> Option<Hop> {
    let written = s.trim();
    let rest = written.strip_prefix("ssh://").unwrap_or(written).trim_end_matches('/');
    let (user, hp) = match rest.rsplit_once('@') {
        Some((u, h)) => (Some(u.to_string()), h),
        None => (None, rest),
    };
    let (host, port) = if let Some(v6) = hp.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p.parse::<u16>().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        (h.to_string(), port)
    } else {
        match hp.split_once(':') {
            Some((h, p)) if !p.contains(':') => (h.to_string(), Some(p.parse().ok()?)),
            Some(_) => (hp.to_string(), None), // bare IPv6
            None => (hp.to_string(), None),
        }
    };
    if host.is_empty() || user.as_deref() == Some("") {
        return None;
    }
    Some(Hop {
        user,
        host,
        port,
        written: written.to_string(),
    })
}

/// `ssh [-q] [-p N] [-l U] [user@]bastion -W %h:%p` (in any order) is a jump
/// host spelled the old way.
fn proxy_command_hop(args: &[String]) -> Option<Hop> {
    let words: Vec<String> = args
        .iter()
        .flat_map(|a| a.split_whitespace().map(String::from).collect::<Vec<_>>())
        .collect();
    let prog = words.first()?;
    if prog != "ssh" && !prog.ends_with("/ssh") {
        return None;
    }
    let (mut host, mut port, mut user, mut forward) = (None, None, None, false);
    let mut it = words[1..].iter();
    while let Some(w) = it.next() {
        match w.as_str() {
            "-W" => forward = it.next().is_some_and(|t| t == "%h:%p"),
            "-p" => port = Some(it.next()?.parse::<u16>().ok()?),
            "-l" => user = Some(it.next()?.clone()),
            "-q" | "-T" | "-x" | "-a" | "-4" | "-6" | "-C" => {}
            w if w.starts_with('-') => return None,
            w if host.is_none() => host = Some(w.to_string()),
            _ => return None,
        }
    }
    let mut hop = parse_hop(&host?)?;
    if !forward {
        return None;
    }
    hop.user = hop.user.or(user);
    hop.port = hop.port.or(port);
    Some(hop)
}

#[derive(Debug, Clone)]
struct Endpoint {
    key: String,
    name: String,
    aliases: Vec<String>,
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    jump: Option<usize>,
    listed: bool,
    source: Option<String>,
    notes: Vec<Note>,
    importable: bool,
}

struct Builder<'a> {
    parsed: &'a Parsed,
    ctx: &'a Context,
    endpoints: Vec<Endpoint>,
    by_key: HashMap<String, usize>,
    stack: Vec<String>,
}

impl Builder<'_> {
    /// Expand ssh's `%` tokens and `~` in an IdentityFile path.
    fn expand_identity(&self, raw: &str, host: &str, user: &str, port: u16, alias: &str, notes: &mut Vec<Note>) -> String {
        let mut out = String::new();
        let mut chars = raw.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => out.push('%'),
                Some('d') => out.push_str(&self.ctx.home.to_string_lossy()),
                Some('u') => out.push_str(&self.ctx.user),
                Some('h') => out.push_str(host),
                Some('r') => out.push_str(user),
                Some('p') => out.push_str(&port.to_string()),
                Some('n') => out.push_str(alias),
                Some(t) => {
                    notes.push(warn(format!("IdentityFile {raw}: %{t} isn't expanded; check the key path")));
                    out.push('%');
                    out.push(t);
                }
                None => out.push('%'),
            }
        }
        if out.contains("${") {
            notes.push(warn(format!(
                "IdentityFile {raw}: environment variables aren't expanded; check the key path"
            )));
        }
        let path = self.ctx.expand_tilde(&out);
        let path = if path.is_absolute() || out.contains("${") {
            path
        } else {
            self.ctx.home.join(path)
        };
        self.ctx.tilde(&path)
    }

    /// Resolve `name` (an alias or a hop's host) into an endpoint index.
    /// `jump_override`: `Some(j)` when the hop's predecessor is given by the
    /// route (ProxyJump a,b: b goes through a whatever b's own config says).
    fn resolve(
        &mut self,
        name: &str,
        user: Option<String>,
        port: Option<u16>,
        jump_override: Option<Option<usize>>,
        display: String,
    ) -> Result<usize, String> {
        let lname = name.to_lowercase();
        if self.stack.contains(&lname) {
            let mut route = self.stack.clone();
            route.push(lname);
            return Err(format!("ProxyJump loop: {}", route.join(" → ")));
        }
        let eff = self.parsed.effective(name);
        let mut notes = Vec::new();
        let host = eff
            .hostname
            .as_deref()
            .map(|h| h.replace("%h", name).replace("%%", "%"))
            .unwrap_or_else(|| name.to_string());
        let user = user.or(eff.user.clone()).unwrap_or_else(|| self.ctx.user.clone());
        let port = match port {
            Some(p) => p,
            None => match &eff.port {
                None => 22,
                Some((p, src)) => p.parse().unwrap_or_else(|_| {
                    notes.push(warn(format!("Port {p} ({src}) isn't a port number; using 22")));
                    22
                }),
            },
        };

        self.stack.push(lname);
        let jump = match jump_override {
            Some(j) => Ok(j),
            None => self.resolve_proxy(&eff, &mut notes),
        };
        self.stack.pop();
        let jump = jump?;

        let key_path = eff
            .identity_files
            .first()
            .map(|k| self.expand_identity(k, &host, &user, port, name, &mut notes));
        if eff.identity_files.len() > 1 {
            notes.push(info(format!(
                "{} identity files are listed; Orbital uses the first",
                eff.identity_files.len()
            )));
        }
        if key_path.is_none() {
            notes.push(info("No IdentityFile: signs in with ssh-agent"));
        }
        if eff.certificate {
            notes.push(warn("CertificateFile isn't supported: certificate sign-in won't be used"));
        }
        if eff.forwards {
            notes.push(info("Port forwards aren't imported; open them from the Tunnels tab"));
        }

        let key = match jump {
            Some(j) => format!("{user}@{host}:{port} via {}", self.endpoints[j].key),
            None => format!("{user}@{host}:{port}"),
        };
        if let Some(&i) = self.by_key.get(&key) {
            return Ok(i);
        }
        let depth = 1 + std::iter::successors(jump, |&j| self.endpoints[j].jump).count();
        let mut importable = jump.is_none_or(|j| self.endpoints[j].importable);
        if depth > MAX_HOPS {
            importable = false;
            notes.push(warn(format!(
                "The route has {} jump hosts; Orbital allows at most {}",
                depth - 1,
                MAX_HOPS - 1
            )));
        }
        self.endpoints.push(Endpoint {
            key: key.clone(),
            name: display,
            aliases: Vec::new(),
            host,
            port,
            user,
            key_path,
            jump,
            listed: false,
            source: None,
            notes,
            importable,
        });
        let i = self.endpoints.len() - 1;
        self.by_key.insert(key, i);
        Ok(i)
    }

    fn resolve_proxy(&mut self, eff: &Effective, notes: &mut Vec<Note>) -> Result<Option<usize>, String> {
        let hops = match &eff.proxy {
            None => return Ok(None),
            Some(Proxy::Jump(list, src)) => {
                if list.eq_ignore_ascii_case("none") {
                    return Ok(None);
                }
                let mut hops = Vec::new();
                for h in list.split(',').filter(|h| !h.trim().is_empty()) {
                    hops.push(parse_hop(h).ok_or_else(|| format!("ProxyJump {h} ({src}) isn't user@host:port"))?);
                }
                hops
            }
            Some(Proxy::Command(args, src)) => {
                if args.len() == 1 && args[0].eq_ignore_ascii_case("none") {
                    return Ok(None);
                }
                match proxy_command_hop(args) {
                    Some(h) => {
                        notes.push(info(format!("ProxyCommand ({src}) is read as jump host {}", h.host)));
                        vec![h]
                    }
                    None => {
                        notes.push(warn(format!(
                            "ProxyCommand ({src}) isn't supported: this server is imported without it. Set a jump host after importing if it needs one"
                        )));
                        return Ok(None);
                    }
                }
            }
        };
        let mut prev: Option<usize> = None;
        for (n, hop) in hops.into_iter().enumerate() {
            let route = if n == 0 { None } else { Some(prev) };
            prev = Some(self.resolve(&hop.host.clone(), hop.user.clone(), hop.port, route, hop.written.clone())?);
        }
        Ok(prev)
    }
}

/// Read `path` (following `Include`) and describe every host in it.
pub fn preview(path: &Path, ctx: &Context) -> Preview {
    let mut parsed = Parsed::default();
    parsed.read(path, &[], 0, ctx);
    let mut b = Builder {
        parsed: &parsed,
        ctx,
        endpoints: Vec::new(),
        by_key: HashMap::new(),
        stack: Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut failed: Vec<Endpoint> = Vec::new();
    for (alias, src) in &parsed.aliases {
        if !seen.insert(alias.to_lowercase()) {
            continue; // a later Host line for the same alias only adds settings
        }
        match b.resolve(alias, None, None, None, alias.clone()) {
            Ok(i) => {
                let e = &mut b.endpoints[i];
                if !e.listed {
                    e.listed = true;
                    e.name = alias.clone();
                    e.source = Some(src.clone());
                } else if !e.name.eq_ignore_ascii_case(alias) {
                    e.aliases.push(alias.clone());
                }
            }
            Err(msg) => {
                let eff = parsed.effective(alias);
                failed.push(Endpoint {
                    key: format!("!{alias}"),
                    name: alias.clone(),
                    aliases: Vec::new(),
                    host: eff.hostname.unwrap_or_else(|| alias.clone()),
                    port: eff.port.and_then(|(p, _)| p.parse().ok()).unwrap_or(22),
                    user: eff.user.unwrap_or_else(|| ctx.user.clone()),
                    key_path: None,
                    jump: None,
                    listed: true,
                    source: Some(src.clone()),
                    notes: vec![warn(msg)],
                    importable: false,
                });
            }
        }
    }
    let keys: Vec<String> = b.endpoints.iter().map(|e| e.key.clone()).collect();
    let mut candidates: Vec<Candidate> = b
        .endpoints
        .into_iter()
        .chain(failed)
        .map(|e| {
            let mut notes = e.notes;
            if !e.aliases.is_empty() {
                notes.insert(0, info(format!("Also listed as {}", e.aliases.join(", "))));
            }
            Candidate {
                jump_key: e.jump.map(|j| keys[j].clone()),
                key: e.key,
                name: e.name,
                aliases: e.aliases,
                auth_method: if e.key_path.is_some() { AuthMethod::Key } else { AuthMethod::Agent },
                host: e.host,
                port: e.port,
                username: e.user,
                key_path: e.key_path,
                jump_only: !e.listed,
                importable: e.importable,
                source: e.source,
                notes,
                existing_id: None,
                existing_name: None,
            }
        })
        .collect();
    // Listed hosts in file order, then the jump hosts that only appear in ProxyJump.
    candidates.sort_by_key(|c| c.jump_only);
    Preview {
        path: ctx.tilde(path),
        found: parsed.found,
        candidates,
        notes: parsed.notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Context) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let p = dir.path().join(".ssh").join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        let ctx = Context {
            home: dir.path().to_path_buf(),
            user: "me".into(),
        };
        (dir, ctx)
    }

    fn run(files: &[(&str, &str)]) -> Preview {
        let (_d, ctx) = setup(files);
        preview(&ctx.default_config(), &ctx)
    }

    fn get<'a>(p: &'a Preview, name: &str) -> &'a Candidate {
        p.candidates
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no candidate {name}: {:#?}", p.candidates))
    }

    fn jump_of<'a>(p: &'a Preview, c: &Candidate) -> Option<&'a Candidate> {
        c.jump_key.as_ref().map(|k| p.candidates.iter().find(|x| &x.key == k).unwrap())
    }

    #[test]
    fn tokenizer_handles_equals_quotes_and_comments() {
        assert_eq!(
            split_line("  HostName=example.com").unwrap(),
            Some(("hostname".into(), vec!["example.com".into()]))
        );
        assert_eq!(split_line("Port = 2222").unwrap(), Some(("port".into(), vec!["2222".into()])));
        assert_eq!(
            split_line("Port=  2222  # admin").unwrap(),
            Some(("port".into(), vec!["2222".into()]))
        );
        assert_eq!(
            split_line("IdentityFile \"~/My Keys/id_ed25519\"").unwrap(),
            Some(("identityfile".into(), vec!["~/My Keys/id_ed25519".into()]))
        );
        assert_eq!(
            split_line("Host a\t b").unwrap(),
            Some(("host".into(), vec!["a".into(), "b".into()]))
        );
        assert_eq!(split_line("   # comment").unwrap(), None);
        assert_eq!(split_line("").unwrap(), None);
        assert!(split_line("HostName \"oops").is_err());
    }

    #[test]
    fn patterns_and_negation() {
        assert!(wild("*.example.com", "web.EXAMPLE.com"));
        assert!(wild("db?", "db1"));
        assert!(!wild("db?", "db12"));
        assert!(wild("*", ""));
        assert!(wild("a*b*c", "axxbyyc"));
        let p = vec!["*.prod".to_string(), "!bastion.prod".to_string()];
        assert!(host_matches(&p, "web.prod"));
        assert!(!host_matches(&p, "bastion.prod"));
        assert!(!host_matches(&p, "web.dev"));
    }

    #[test]
    fn hops_parse() {
        let h = parse_hop("admin@10.0.0.1:2200").unwrap();
        assert_eq!(
            (h.user.as_deref(), h.host.as_str(), h.port),
            (Some("admin"), "10.0.0.1", Some(2200))
        );
        let h = parse_hop("ssh://root@[fd00::1]:22").unwrap();
        assert_eq!((h.user.as_deref(), h.host.as_str(), h.port), (Some("root"), "fd00::1", Some(22)));
        assert_eq!(parse_hop("bastion").unwrap().port, None);
        assert!(parse_hop("bastion:nope").is_none());
        let h = proxy_command_hop(&["ssh".into(), "-q".into(), "-W".into(), "%h:%p".into(), "ops@gw".into()]).unwrap();
        assert_eq!((h.user.as_deref(), h.host.as_str()), (Some("ops"), "gw"));
        assert!(proxy_command_hop(&["nc".into(), "-X".into(), "5".into(), "%h".into(), "%p".into()]).is_none());
        assert!(proxy_command_hop(&["ssh gw nc %h %p".into()]).is_none());
    }

    const REALISTIC: &str = r#"
# Global defaults first, then hosts; ssh takes the first value it sees.
Include config.d/*
Include ~/.ssh/missing.conf

Host bastion
    HostName bastion.example.com
    User ops
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host web web.example.com
  HostName=10.0.1.10
  ProxyJump bastion

Host "db-primary"
  HostName 10.0.4.21
  User postgres
  ProxyJump bastion,admin@10.0.0.5:2200
  IdentityFile %d/.ssh/keys/%h_%r
  IdentityFile ~/.ssh/fallback

Host legacy
  HostName legacy.internal
  ProxyCommand ssh -q -W %h:%p bastion

Host weird
  HostName weird.internal
  ProxyCommand nc -X 5 -x proxy:1080 %h %p

Host *.prod !bastion.prod
  User deploy

Host app.prod
  LocalForward 8080 localhost:80

Host web
  User www

Match host *.corp exec "test -f /tmp/x"
  User nobody

Host loop-a
  ProxyJump loop-b
Host loop-b
  ProxyJump loop-a

Host deep
  ProxyJump h1,h2,h3,h4

Host *
  User fallback
  IdentityFile ~/.ssh/id_rsa
  ServerAliveInterval 30
"#;

    const INCLUDED: &str = "Host staging\n  HostName staging.example.dev\n  Port 2222\n  User me\n";

    #[test]
    fn realistic_config() {
        let p = run(&[
            ("config", REALISTIC),
            ("config.d/10-staging", INCLUDED),
            ("config.d/.hidden", "Host hidden\n"),
        ]);
        assert!(p.found);

        // Included file comes first (it's included at the top) and keeps its own settings.
        let staging = get(&p, "staging");
        assert_eq!(
            (staging.host.as_str(), staging.port, staging.username.as_str()),
            ("staging.example.dev", 2222, "me")
        );
        assert_eq!(staging.source.as_deref(), Some("~/.ssh/config.d/10-staging:1"));
        assert!(!p.candidates.iter().any(|c| c.name == "hidden"), "dotfiles aren't globbed");
        assert_eq!(p.candidates[0].name, "staging");

        let bastion = get(&p, "bastion");
        assert_eq!(
            (bastion.host.as_str(), bastion.port, bastion.username.as_str()),
            ("bastion.example.com", 2222, "ops")
        );
        assert_eq!(bastion.auth_method, AuthMethod::Key);
        assert_eq!(bastion.key_path.as_deref(), Some("~/.ssh/id_ed25519"));
        assert!(bastion.jump_key.is_none());

        // Multiple patterns on one Host line; the `Host *` User only applies when nothing earlier set it.
        // The later `Host web` (User www) precedes `Host *`, so it wins for web only.
        let web = get(&p, "web");
        assert_eq!((web.host.as_str(), web.username.as_str()), ("10.0.1.10", "www"));
        assert_eq!(jump_of(&p, web).unwrap().name, "bastion");
        assert_eq!(web.key_path.as_deref(), Some("~/.ssh/id_rsa"));
        let web2 = get(&p, "web.example.com");
        assert_eq!((web2.host.as_str(), web2.username.as_str()), ("10.0.1.10", "fallback"));

        // Multi-hop chain: target → admin@10.0.0.5:2200 → bastion.
        let db = get(&p, "db-primary");
        assert_eq!(db.username, "postgres");
        assert_eq!(db.key_path.as_deref(), Some("~/.ssh/keys/10.0.4.21_postgres"));
        assert!(db.notes.iter().any(|n| n.text.contains("3 identity files")));
        let hop = jump_of(&p, db).unwrap();
        assert_eq!(hop.name, "admin@10.0.0.5:2200");
        assert!(hop.jump_only);
        assert_eq!((hop.host.as_str(), hop.port, hop.username.as_str()), ("10.0.0.5", 2200, "admin"));
        assert_eq!(jump_of(&p, hop).unwrap().name, "bastion");

        // Old-style ProxyCommand is a jump host; an unknown one is a warning.
        assert_eq!(jump_of(&p, get(&p, "legacy")).unwrap().name, "bastion");
        let weird = get(&p, "weird");
        assert!(weird.jump_key.is_none());
        assert!(weird
            .notes
            .iter()
            .any(|n| n.level == NoteLevel::Warn && n.text.contains("ProxyCommand")));

        // Negated pattern: deploy applies to *.prod except bastion.prod; forwards get a note.
        let app = get(&p, "app.prod");
        assert_eq!(app.username, "deploy");
        assert!(app.notes.iter().any(|n| n.text.contains("Tunnels")));

        // Loops and too-long routes are listed but can't be imported.
        let a = get(&p, "loop-a");
        assert!(!a.importable);
        assert!(a.notes[0].text.contains("loop"), "{:?}", a.notes);
        let deep = get(&p, "deep");
        assert!(!deep.importable);
        // The deep route's hops themselves are fine.
        assert!(get(&p, "h1").importable);

        // File-level notes: Match skipped, the pattern Host line, missing include.
        assert!(p.notes.iter().any(|n| n.text.starts_with("Match host *.corp")));
        assert!(p.notes.iter().any(|n| n.text.contains("Host *.prod !bastion.prod")));
        assert!(p.notes.iter().any(|n| n.text.contains("missing.conf")));
        assert!(
            !p.notes.iter().any(|n| n.text.starts_with("Host * ")),
            "Host * alone is just defaults"
        );
        // Match's User never applies.
        assert!(!p.candidates.iter().any(|c| c.username == "nobody"));
    }

    #[test]
    fn first_value_wins_across_blocks_and_duplicates() {
        let cfg = "Host web web.example.com\n  HostName 10.0.1.10\nHost web\n  User www\n  HostName ignored\nHost *\n  User fallback\n";
        let p = run(&[("config", cfg)]);
        let web = get(&p, "web");
        assert_eq!((web.host.as_str(), web.username.as_str()), ("10.0.1.10", "www"));
        let other = get(&p, "web.example.com");
        assert_eq!(other.username, "fallback");
        // Same alias twice → listed once.
        assert_eq!(p.candidates.iter().filter(|c| c.name == "web").count(), 1);
    }

    #[test]
    fn aliases_for_the_same_server_are_merged() {
        let cfg = "Host a b\n  HostName 1.2.3.4\n  User root\nHost c\n  HostName 1.2.3.4\n  User root\n  Port 22\n";
        let p = run(&[("config", cfg)]);
        assert_eq!(p.candidates.len(), 1);
        assert_eq!(p.candidates[0].name, "a");
        assert_eq!(p.candidates[0].aliases, ["b", "c"]);
        assert!(p.candidates[0].notes[0].text.contains("Also listed as b, c"));
    }

    #[test]
    fn hop_that_is_an_alias_reuses_it_and_route_overrides_own_jump() {
        // ProxyJump a,b: b is reached through a even though b's own config says c.
        let cfg = "Host t\n  ProxyJump a,b\nHost a\n  HostName a.example\nHost b\n  HostName b.example\n  ProxyJump c\nHost c\n  HostName c.example\n";
        let p = run(&[("config", cfg)]);
        let t = get(&p, "t");
        let b_via_a = jump_of(&p, t).unwrap();
        assert_eq!(b_via_a.host, "b.example");
        assert_eq!(jump_of(&p, b_via_a).unwrap().name, "a");
        // The listed `b` goes through c, so it's a different server entry.
        let b = p.candidates.iter().find(|c| c.name == "b" && !c.jump_only).unwrap();
        assert_eq!(jump_of(&p, b).unwrap().name, "c");
        assert_ne!(b.key, b_via_a.key);
    }

    #[test]
    fn include_inside_host_block_is_conditional() {
        let cfg = "Host special\n  Include extra\nHost other\n  HostName other.example\n";
        let p = run(&[("config", cfg), ("extra", "User inner\n")]);
        assert_eq!(get(&p, "special").username, "inner");
        assert_eq!(get(&p, "other").username, "me");
    }

    #[test]
    fn include_inside_match_contributes_nothing() {
        let cfg = "Match exec true\n  Include extra\nHost x\n";
        let p = run(&[("config", cfg), ("extra", "Host hidden\nUser inner\n")]);
        assert!(!p.candidates.iter().any(|c| c.name == "hidden"));
        assert_eq!(get(&p, "x").username, "me");
    }

    #[test]
    fn match_all_applies_everywhere() {
        let p = run(&[("config", "Host x\nMatch all\n  User everyone\n")]);
        assert_eq!(get(&p, "x").username, "everyone");
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn proxyjump_none_and_hostname_token() {
        let cfg = "Host *.lan\n  HostName %h.example.com\nHost gw.lan\n  ProxyJump none\nHost *\n  ProxyJump bastion\nHost bastion\n";
        let p = run(&[("config", cfg)]);
        let gw = get(&p, "gw.lan");
        assert_eq!(gw.host, "gw.lan.example.com");
        assert!(gw.jump_key.is_none());
    }

    #[test]
    fn bad_values_are_noted() {
        let p = run(&[("config", "Host x\n  Port ssh\n  IdentityFile ~/.ssh/%l_key\n  HostName \"broken\n")]);
        let x = get(&p, "x");
        assert_eq!(x.port, 22);
        assert!(x.notes.iter().any(|n| n.text.contains("isn't a port")));
        assert!(x.notes.iter().any(|n| n.text.contains("%l")));
        assert!(p.notes.iter().any(|n| n.text.contains("unbalanced quotes")));
    }

    #[test]
    fn missing_config_is_not_found() {
        let (_d, ctx) = setup(&[]);
        let p = preview(&ctx.default_config(), &ctx);
        assert!(!p.found);
        assert!(p.candidates.is_empty());
        assert_eq!(p.path, "~/.ssh/config");
    }

    #[test]
    fn include_recursion_is_bounded() {
        let p = run(&[("config", "Include config\nHost x\n")]);
        assert!(p.notes.iter().any(|n| n.text.contains("nested too deeply")));
        assert_eq!(p.candidates.iter().filter(|c| c.name == "x").count(), 1);
    }
}
