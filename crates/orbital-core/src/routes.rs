//! nginx routes: the `location` blocks of one server block, edited in place.
//!
//! A route sends a path to an app on a port (`proxy_pass`, with WebSockets),
//! serves files from a folder, or redirects. Changes are made to the site's
//! own file only (never nginx's stock files), as text: the rest of the file is
//! kept exactly as it was, comments and formatting included. The engine writes
//! the result through safe save (backup, `nginx -t`, rollback) and reloads.
//!
//! Everything here is pure and tested; values are validated before they are
//! written into the config (nginx has no quoting we could rely on here).

use crate::error::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum RouteTarget {
    /// An app listening on `host:port`. `keep_prefix`: the app sees `/api/x`
    /// rather than `/x`.
    App {
        host: String,
        port: u16,
        keep_prefix: bool,
        websockets: bool,
    },
    /// Files from a folder.
    Folder { dir: String },
    /// Send the visitor elsewhere.
    Redirect { to: String, code: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// Starts with `/`. Prefix routes other than `/` are written with a
    /// trailing slash (`/api` → `/api/`), so `/apiv2` isn't caught by `/api`.
    pub path: String,
    /// Only this exact path (`location = /path`), for redirects.
    #[serde(default)]
    pub exact: bool,
    pub target: RouteTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "op")]
pub enum RouteChange {
    Add {
        route: Route,
    },
    /// `spec` is the existing location's arguments as written (`/api/`, `= /old`).
    Replace {
        spec: String,
        route: Route,
    },
    Remove {
        spec: String,
    },
}

/// A `location` block found in a server block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteInfo {
    /// The location's arguments as written: `/api/`, `= /old`, `~* \.php$`.
    pub spec: String,
    /// 1-based line of the `location` directive.
    pub line: usize,
    /// The route, when it's one Orbital can edit (prefix or exact path; a
    /// proxy_pass to http://host:port, an alias/root, or a return).
    pub route: Option<Route>,
    /// What it does, in a few words, for routes that can't be edited here.
    pub summary: String,
    /// The block's text as written.
    pub text: String,
}

/// A server block's routes, read from its real file (links resolved).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutesView {
    pub file: String,
    pub line: usize,
    pub routes: Vec<RouteInfo>,
}

/// What a change does (preview) or did (applied).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteChangeReport {
    pub file: String,
    pub before: String,
    pub after: String,
    pub applied: bool,
    pub backup_path: Option<String>,
    pub reloaded: bool,
    pub reload_output: String,
}

/// The file a site's routes may be changed in: under /etc/nginx and not one of nginx's own files.
pub fn check_site_file(path: &str, real: &str) -> Result<()> {
    for p in [path, real] {
        if crate::certbot::is_nginx_default_file(p) {
            return Err(Error::InvalidInput(format!(
                "{p} is one of nginx's own files; Orbital doesn't change it. Create the site with New website and add routes there."
            )));
        }
    }
    if !real.starts_with("/etc/nginx/") || real.contains("..") {
        return Err(Error::InvalidInput(format!(
            "{real} isn't a site file under /etc/nginx; edit it in the editor"
        )));
    }
    Ok(())
}

// ---------- validation ----------

static PATH: Lazy<Regex> = Lazy::new(|| Regex::new(r"^/[A-Za-z0-9._~@%+\-/]*$").unwrap());
static HOST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(localhost|\d{1,3}(\.\d{1,3}){3}|[A-Za-z0-9]([A-Za-z0-9\-]{0,61}[A-Za-z0-9])?(\.[A-Za-z0-9]([A-Za-z0-9\-]{0,61}[A-Za-z0-9])?)*)$").unwrap()
});
static DIR: Lazy<Regex> = Lazy::new(|| Regex::new(r"^/[A-Za-z0-9._@+\-/]*$").unwrap());
static URL: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^(https?://[^\s;{}"'`$\\#]+|/[^\s;{}"'`$\\#]*)$"#).unwrap());

/// The route as it will be written, or why not.
pub fn validate(r: &Route) -> Result<Route> {
    let bad = |m: String| Err(Error::InvalidInput(m));
    let path = r.path.trim();
    if !PATH.is_match(path) || path.contains("//") || path.split('/').any(|p| p == ".." || p == ".") {
        return bad(format!("{path:?} isn't a path; use something like /api or /static/"));
    }
    let exact = r.exact && matches!(r.target, RouteTarget::Redirect { .. });
    let path = if !exact && path != "/" && !path.ends_with('/') {
        format!("{path}/")
    } else {
        path.to_string()
    };
    let target = match &r.target {
        RouteTarget::App {
            host,
            port,
            keep_prefix,
            websockets,
        } => {
            let host = host.trim();
            let host = if host.is_empty() { "127.0.0.1" } else { host };
            if !HOST.is_match(host) {
                return bad(format!("{host:?} isn't an address; the app usually listens on 127.0.0.1"));
            }
            if *port == 0 {
                return bad("choose the port the app listens on (1–65535)".into());
            }
            RouteTarget::App {
                host: host.to_string(),
                port: *port,
                keep_prefix: *keep_prefix,
                websockets: *websockets,
            }
        }
        RouteTarget::Folder { dir } => {
            let d = dir.trim().trim_end_matches('/');
            let d = if d.is_empty() { "/" } else { d };
            if !DIR.is_match(d) || d.split('/').any(|p| p == "..") || d == "/" {
                return bad(format!(
                    "{dir:?} isn't a folder; give its full path, e.g. /var/www/example.com/static"
                ));
            }
            RouteTarget::Folder { dir: d.to_string() }
        }
        RouteTarget::Redirect { to, code } => {
            let to = to.trim();
            if !URL.is_match(to) {
                return bad(format!(
                    "{to:?} isn't an address to redirect to; use https://… or a path starting with /"
                ));
            }
            if ![301, 302, 307, 308].contains(code) {
                return bad("the redirect code is 301, 302, 307 or 308".into());
            }
            RouteTarget::Redirect {
                to: to.to_string(),
                code: *code,
            }
        }
    };
    Ok(Route { path, exact, target })
}

/// The location's arguments for a route: `/api/` or `= /old`.
pub fn spec_of(r: &Route) -> String {
    if r.exact {
        format!("= {}", r.path)
    } else {
        r.path.clone()
    }
}

// ---------- rendering ----------

/// The `location` block for a validated route, indented by `indent`.
pub fn render(r: &Route, indent: &str) -> String {
    let i2 = format!("{indent}    ");
    let mut body: Vec<String> = Vec::new();
    match &r.target {
        RouteTarget::App {
            host,
            port,
            keep_prefix,
            websockets,
        } => {
            // With a URI part ("/") nginx swaps the location's prefix for it; without, the path goes as is.
            let uri = if *keep_prefix || r.path == "/" { "" } else { "/" };
            body.push(format!("proxy_pass http://{host}:{port}{uri};"));
            body.push("proxy_http_version 1.1;".into());
            body.push("proxy_set_header Host $host;".into());
            body.push("proxy_set_header X-Real-IP $remote_addr;".into());
            body.push("proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;".into());
            body.push("proxy_set_header X-Forwarded-Proto $scheme;".into());
            if *websockets {
                body.push("proxy_set_header Upgrade $http_upgrade;".into());
                body.push("proxy_set_header Connection \"upgrade\";".into());
                body.push("proxy_read_timeout 300s;".into());
            }
        }
        RouteTarget::Folder { dir } => {
            if r.path == "/" {
                body.push(format!("root {dir};"));
            } else {
                body.push(format!("alias {dir}/;"));
            }
            body.push("try_files $uri $uri/ =404;".into());
        }
        RouteTarget::Redirect { to, code } => body.push(format!("return {code} {to};")),
    }
    let mut s = format!("{indent}# Route added by Orbital\n{indent}location {} {{\n", spec_of(r));
    for b in body {
        s.push_str(&format!("{i2}{b}\n"));
    }
    s.push_str(&format!("{indent}}}\n"));
    s
}

// ---------- reading the file ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Block {
    /// Byte offset of the directive's first character.
    start: usize,
    /// Byte offset of its `{`.
    open: usize,
    /// Byte offset of the matching `}`.
    close: usize,
}

/// Tokens that matter for structure: words (outside comments and quotes) and braces.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(usize, usize),
    Open(usize),
    Close(usize),
    Semi(usize),
}

fn tokens(text: &str) -> Vec<Tok> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                out.push(Tok::Open(i));
                i += 1;
            }
            b'}' => {
                out.push(Tok::Close(i));
                i += 1;
            }
            b';' => {
                out.push(Tok::Semi(i));
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            b'"' | b'\'' => {
                let q = c;
                let s = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.push(Tok::Word(s, i));
            }
            _ => {
                let s = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'{' | b'}' | b';' | b'#') {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                out.push(Tok::Word(s, i.min(b.len())));
            }
        }
    }
    out
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].bytes().filter(|&c| c == b'\n').count() + 1
}

/// The `server` block whose directive is on `line` (1-based).
fn server_block(text: &str, toks: &[Tok], line: usize) -> Result<Block> {
    for (k, t) in toks.iter().enumerate() {
        if let Tok::Word(s, e) = *t {
            if &text[s..e] == "server" && line_of(text, s) == line {
                if let Some(Tok::Open(open)) = toks.get(k + 1) {
                    let close = matching(toks, k + 1).ok_or_else(|| Error::InvalidInput("the server block isn't closed".into()))?;
                    return Ok(Block {
                        start: s,
                        open: *open,
                        close,
                    });
                }
            }
        }
    }
    Err(Error::NotFound(format!(
        "no server block starts on line {line}; the file changed, reload the nginx tab"
    )))
}

/// Offset of the `}` matching the `{` token at `k`.
fn matching(toks: &[Tok], k: usize) -> Option<usize> {
    let mut depth = 0i32;
    for t in &toks[k..] {
        match t {
            Tok::Open(_) => depth += 1,
            Tok::Close(c) => {
                depth -= 1;
                if depth == 0 {
                    return Some(*c);
                }
            }
            _ => {}
        }
    }
    None
}

/// The `location` blocks directly inside a server block: (spec, block).
fn locations(text: &str, toks: &[Tok], server: Block) -> Vec<(String, Block)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut k = 0;
    while k < toks.len() {
        let t = &toks[k];
        let pos = match t {
            Tok::Word(s, _) | Tok::Open(s) | Tok::Close(s) | Tok::Semi(s) => *s,
        };
        if pos <= server.open || pos >= server.close {
            k += 1;
            continue;
        }
        match t {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => depth -= 1,
            Tok::Word(s, e) if depth == 0 && &text[*s..*e] == "location" => {
                // Arguments up to the `{`.
                let mut j = k + 1;
                let mut args = Vec::new();
                while let Some(Tok::Word(a, b)) = toks.get(j) {
                    args.push(&text[*a..*b]);
                    j += 1;
                }
                if let Some(Tok::Open(open)) = toks.get(j) {
                    if let Some(close) = matching(toks, j) {
                        out.push((
                            args.join(" "),
                            Block {
                                start: *s,
                                open: *open,
                                close,
                            },
                        ));
                        // Skip the location's own contents.
                        while k < toks.len() {
                            if let Tok::Close(c) = toks[k] {
                                if c == close {
                                    break;
                                }
                            }
                            k += 1;
                        }
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    out
}

/// Start of the line holding `at`, and that line's leading whitespace.
fn line_start(text: &str, at: usize) -> (usize, String) {
    let s = text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent: String = text[s..at].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    (s, indent)
}

/// The byte range a location occupies as whole lines, including one
/// "# Route added by Orbital" comment right above it.
fn whole_lines(text: &str, b: Block) -> (usize, usize) {
    let (mut s, _) = line_start(text, b.start);
    let prev_end = s.saturating_sub(1);
    if s > 0 {
        let (ps, _) = line_start(text, prev_end);
        if text[ps..prev_end].trim() == "# Route added by Orbital" {
            s = ps;
        }
    }
    let mut e = b.close + 1;
    if let Some(nl) = text[e..].find('\n') {
        if text[e..e + nl].trim().is_empty() {
            e += nl + 1;
        }
    }
    (s, e)
}

/// Reads an existing location back into a route, when it's one of ours (or written like one).
fn parse_route(spec: &str, body: &str) -> Option<Route> {
    let (exact, path) = match spec.split_once(' ') {
        Some(("=", p)) => (true, p.trim()),
        Some(_) => return None,
        None => (false, spec),
    };
    if !PATH.is_match(path) {
        return None;
    }
    let dir = |d: &str| d.trim_end_matches(';').trim().to_string();
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let find = |k: &str| lines.iter().find_map(|l| l.strip_prefix(k).map(|v| v.trim()));
    let target = if let Some(pp) = find("proxy_pass ") {
        let pp = dir(pp);
        let rest = pp.strip_prefix("http://")?;
        let (hostport, uri) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let (host, port) = hostport.rsplit_once(':')?;
        let port: u16 = port.parse().ok()?;
        RouteTarget::App {
            host: host.to_string(),
            port,
            keep_prefix: uri.is_empty() && path != "/",
            websockets: lines.iter().any(|l| l.contains("$http_upgrade")),
        }
    } else if let Some(a) = find("alias ") {
        RouteTarget::Folder {
            dir: dir(a).trim_end_matches('/').to_string(),
        }
    } else if let Some(r) = find("root ") {
        RouteTarget::Folder {
            dir: dir(r).trim_end_matches('/').to_string(),
        }
    } else {
        let ret = find("return ")?;
        let (code, to) = dir(ret).split_once(' ').map(|(c, t)| (c.to_string(), t.trim().to_string()))?;
        RouteTarget::Redirect {
            to,
            code: code.parse().ok()?,
        }
    };
    validate(&Route {
        path: path.to_string(),
        exact,
        target,
    })
    .ok()
}

fn summary(body: &str) -> String {
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("proxy_set_header") && !l.starts_with("proxy_http_version"))
        .unwrap_or("");
    first.trim_end_matches(';').to_string()
}

/// The routes of the server block on `line`.
pub fn list(text: &str, line: usize) -> Result<Vec<RouteInfo>> {
    let toks = tokens(text);
    let server = server_block(text, &toks, line)?;
    Ok(locations(text, &toks, server)
        .into_iter()
        .map(|(spec, b)| {
            let body = &text[b.open + 1..b.close];
            RouteInfo {
                route: parse_route(&spec, body),
                summary: summary(body),
                line: line_of(text, b.start),
                text: text[b.start..=b.close].to_string(),
                spec,
            }
        })
        .collect())
}

/// The file's text with `change` made to the server block on `line`.
pub fn apply(text: &str, line: usize, change: &RouteChange) -> Result<String> {
    let toks = tokens(text);
    let server = server_block(text, &toks, line)?;
    let locs = locations(text, &toks, server);
    let find = |spec: &str| {
        let want = norm(spec);
        locs.iter().find(|(s, _)| norm(s) == want).map(|(_, b)| *b)
    };
    // The server's own indentation, plus one level for what's inside it.
    let (_, server_indent) = line_start(text, server.start);
    let inner = locs
        .first()
        .map(|(_, b)| line_start(text, b.start).1)
        .unwrap_or_else(|| format!("{server_indent}    "));
    match change {
        RouteChange::Add { route } => {
            let r = validate(route)?;
            if find(&spec_of(&r)).is_some() {
                return Err(Error::InvalidInput(format!(
                    "this site already has a route for {}; change that one instead",
                    spec_of(&r)
                )));
            }
            // Before the server's closing brace, on a line of its own.
            let (close_line, _) = line_start(text, server.close);
            let before_close = text[close_line..server.close].trim().is_empty();
            let mut out = String::with_capacity(text.len() + 512);
            if before_close {
                out.push_str(&text[..close_line]);
                if !out.ends_with("\n\n") && !out.ends_with("{\n") {
                    out.push('\n');
                }
                out.push_str(&render(&r, &inner));
                out.push_str(&text[close_line..]);
            } else {
                out.push_str(&text[..server.close]);
                out.push('\n');
                out.push_str(&render(&r, &inner));
                out.push_str(&server_indent);
                out.push_str(&text[server.close..]);
            }
            Ok(out)
        }
        RouteChange::Replace { spec, route } => {
            let r = validate(route)?;
            let b = find(spec).ok_or_else(|| Error::NotFound(format!("no route {spec} in this site any more; reload the nginx tab")))?;
            if norm(&spec_of(&r)) != norm(spec) && find(&spec_of(&r)).is_some() {
                return Err(Error::InvalidInput(format!("this site already has a route for {}", spec_of(&r))));
            }
            let (s, e) = whole_lines(text, b);
            let indent = line_start(text, b.start).1;
            Ok(format!("{}{}{}", &text[..s], render(&r, &indent), &text[e..]))
        }
        RouteChange::Remove { spec } => {
            let b = find(spec).ok_or_else(|| Error::NotFound(format!("no route {spec} in this site any more; reload the nginx tab")))?;
            let (mut s, e) = whole_lines(text, b);
            // Take one blank line above with it, so adding and removing leave the file as it was.
            if s >= 1 && text[..s - 1].ends_with('\n') {
                s -= 1;
            }
            Ok(format!("{}{}", &text[..s], &text[e..]))
        }
    }
}

/// `= /x` and `=  /x` are the same location.
fn norm(spec: &str) -> String {
    spec.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "# shop.example.com: created by Orbital.\nserver {\n    listen 80;\n    server_name shop.example.com; # main\n\n    location / {\n        proxy_pass http://127.0.0.1:3000;\n        proxy_set_header Host $host;\n    }\n\n    location ~* \\.(png|jpg)$ {\n        expires 30d;\n    }\n}\n\nserver {\n    listen 443 ssl;\n    server_name shop.example.com;\n    ssl_certificate /etc/letsencrypt/live/shop.example.com/fullchain.pem; # managed by Certbot\n    location / {\n        root /var/www/shop;\n        add_header X-Note \"a { brace } and ; inside\";\n    }\n}\n";

    fn app(path: &str, port: u16) -> Route {
        Route {
            path: path.into(),
            exact: false,
            target: RouteTarget::App {
                host: "127.0.0.1".into(),
                port,
                keep_prefix: false,
                websockets: true,
            },
        }
    }

    #[test]
    fn lists_the_routes_of_one_server_block() {
        let r = list(SITE, 2).unwrap();
        assert_eq!(r.iter().map(|x| x.spec.as_str()).collect::<Vec<_>>(), ["/", "~* \\.(png|jpg)$"]);
        assert_eq!(r[0].line, 6);
        assert_eq!(
            r[0].route,
            Some(Route {
                path: "/".into(),
                exact: false,
                target: RouteTarget::App {
                    host: "127.0.0.1".into(),
                    port: 3000,
                    keep_prefix: false,
                    websockets: false
                }
            })
        );
        assert!(r[1].route.is_none());
        assert_eq!(r[1].summary, "expires 30d");
        // The HTTPS block, whose quoted header holds braces and a semicolon.
        let s = list(SITE, 16).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].route.as_ref().unwrap().target,
            RouteTarget::Folder {
                dir: "/var/www/shop".into()
            }
        );
        assert!(list(SITE, 3).is_err(), "line 3 isn't a server block");
    }

    #[test]
    fn adds_a_route_before_the_closing_brace_and_keeps_the_rest() {
        let out = apply(SITE, 2, &RouteChange::Add { route: app("/api", 4000) }).unwrap();
        assert!(
            out.starts_with(&SITE[..SITE.find("    location ~*").unwrap()]),
            "earlier text untouched"
        );
        assert!(
            out.contains("    # Route added by Orbital\n    location /api/ {\n        proxy_pass http://127.0.0.1:4000/;\n"),
            "{out}"
        );
        assert!(out.contains("proxy_set_header Upgrade $http_upgrade;"));
        // Read back: three routes now, and the new one is editable.
        let r = list(&out, 2).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[2].route.as_ref().unwrap().path, "/api/");
        // The second server block is unchanged.
        assert!(out.ends_with(&SITE[SITE.find("\nserver {\n    listen 443").unwrap()..]));
        // Adding it twice is refused.
        assert!(apply(&out, 2, &RouteChange::Add { route: app("/api/", 5000) })
            .unwrap_err()
            .to_string()
            .contains("already has a route"));
    }

    #[test]
    fn replaces_and_removes_whole_routes() {
        let added = apply(SITE, 2, &RouteChange::Add { route: app("/api", 4000) }).unwrap();
        let changed = apply(
            &added,
            2,
            &RouteChange::Replace {
                spec: "/api/".into(),
                route: Route {
                    path: "/static".into(),
                    exact: false,
                    target: RouteTarget::Folder {
                        dir: "/var/www/shop/static/".into(),
                    },
                },
            },
        )
        .unwrap();
        assert!(
            changed.contains("location /static/ {\n        alias /var/www/shop/static/;\n        try_files $uri $uri/ =404;\n    }"),
            "{changed}"
        );
        assert!(!changed.contains("4000"));
        assert_eq!(
            changed.matches("# Route added by Orbital").count(),
            1,
            "the comment is replaced, not repeated"
        );
        let removed = apply(&changed, 2, &RouteChange::Remove { spec: "/static/".into() }).unwrap();
        assert_eq!(removed, SITE, "removing what was added gives the original file back");
        assert!(apply(SITE, 2, &RouteChange::Remove { spec: "/nope/".into() }).is_err());
        // An existing route written by hand can be removed too.
        let no_images = apply(
            SITE,
            2,
            &RouteChange::Remove {
                spec: "~* \\.(png|jpg)$".into(),
            },
        )
        .unwrap();
        assert!(!no_images.contains("expires 30d") && no_images.contains("location / {"));
    }

    #[test]
    fn renders_each_kind() {
        let keep = Route {
            path: "/api".into(),
            exact: false,
            target: RouteTarget::App {
                host: "".into(),
                port: 8080,
                keep_prefix: true,
                websockets: false,
            },
        };
        let r = render(&validate(&keep).unwrap(), "    ");
        assert!(r.contains("proxy_pass http://127.0.0.1:8080;") && !r.contains("Upgrade"), "{r}");
        let root = Route {
            path: "/".into(),
            exact: false,
            target: RouteTarget::Folder {
                dir: "/var/www/site".into(),
            },
        };
        assert!(render(&validate(&root).unwrap(), "").contains("root /var/www/site;"));
        let redirect = Route {
            path: "/old".into(),
            exact: true,
            target: RouteTarget::Redirect {
                to: "https://example.com/new".into(),
                code: 301,
            },
        };
        let r = render(&validate(&redirect).unwrap(), "  ");
        assert!(
            r.contains("  location = /old {\n      return 301 https://example.com/new;\n  }"),
            "{r}"
        );
    }

    #[test]
    fn refuses_values_that_would_break_or_inject_config() {
        let bad_paths = ["api", "/a b", "/a;b", "/a{b", "/../etc", "//x", "/x\"y", "/$host"];
        for p in bad_paths {
            assert!(validate(&app(p, 3000)).is_err(), "{p}");
        }
        assert!(validate(&app("/api", 0)).is_err());
        let host = |h: &str| Route {
            path: "/x".into(),
            exact: false,
            target: RouteTarget::App {
                host: h.into(),
                port: 1,
                keep_prefix: false,
                websockets: false,
            },
        };
        assert!(validate(&host("10.0.0.5")).is_ok() && validate(&host("app.internal")).is_ok());
        assert!(validate(&host("a;b")).is_err() && validate(&host("a b")).is_err());
        let dir = |d: &str| Route {
            path: "/s".into(),
            exact: false,
            target: RouteTarget::Folder { dir: d.into() },
        };
        assert!(
            validate(&dir("var/www")).is_err()
                && validate(&dir("/var/../etc")).is_err()
                && validate(&dir("/")).is_err()
                && validate(&dir("/a;b")).is_err()
        );
        let to = |t: &str, c: u16| Route {
            path: "/o".into(),
            exact: true,
            target: RouteTarget::Redirect { to: t.into(), code: c },
        };
        assert!(validate(&to("https://a.example/x?y=1", 302)).is_ok() && validate(&to("/new", 308)).is_ok());
        for t in [
            "javascript:alert(1)",
            "https://a b",
            "https://a;b",
            "https://a/$uri",
            "https://a/{x}",
        ] {
            assert!(validate(&to(t, 301)).is_err(), "{t}");
        }
        assert!(validate(&to("/x", 200)).is_err());
        // Exact matches are only for redirects.
        assert!(
            !validate(&Route {
                exact: true,
                ..app("/api", 1)
            })
            .unwrap()
            .exact
        );
    }

    #[test]
    fn reads_the_apps_json() {
        let c: RouteChange = serde_json::from_str(
            r#"{"op":"replace","spec":"/api/","route":{"path":"/api","exact":false,"target":{"kind":"app","host":"127.0.0.1","port":3000,"keepPrefix":true,"websockets":false}}}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            RouteChange::Replace {
                spec: "/api/".into(),
                route: Route {
                    path: "/api".into(),
                    exact: false,
                    target: RouteTarget::App {
                        host: "127.0.0.1".into(),
                        port: 3000,
                        keep_prefix: true,
                        websockets: false
                    }
                }
            }
        );
        let r = serde_json::to_value(RouteTarget::Redirect {
            to: "/n".into(),
            code: 301,
        })
        .unwrap();
        assert_eq!(r, serde_json::json!({"kind": "redirect", "to": "/n", "code": 301}));
    }

    #[test]
    fn only_a_sites_own_file_is_changed() {
        assert!(check_site_file("/etc/nginx/sites-enabled/shop", "/etc/nginx/sites-available/shop").is_ok());
        assert!(check_site_file("/etc/nginx/sites-enabled/default", "/etc/nginx/sites-available/default").is_err());
        assert!(
            check_site_file("/etc/nginx/sites-enabled/shop", "/etc/nginx/sites-available/default").is_err(),
            "a link to default"
        );
        assert!(check_site_file("/etc/nginx/nginx.conf", "/etc/nginx/nginx.conf").is_err());
        assert!(
            check_site_file("/etc/nginx/sites-enabled/x", "/opt/app/nginx.conf").is_err(),
            "outside /etc/nginx"
        );
    }

    #[test]
    fn works_with_tabs_and_a_one_line_server() {
        let t = "server {\n\tlisten 80;\n\tlocation / {\n\t\treturn 200;\n\t}\n}\n";
        let out = apply(t, 1, &RouteChange::Add { route: app("/x", 1) }).unwrap();
        assert!(
            out.contains("\t# Route added by Orbital\n\tlocation /x/ {\n\t    proxy_pass"),
            "{out:?}"
        );
        let one = "server { listen 80; }\n";
        let out = apply(one, 1, &RouteChange::Add { route: app("/x", 1) }).unwrap();
        assert_eq!(list(&out, 1).unwrap().len(), 1, "{out}");
    }
}
