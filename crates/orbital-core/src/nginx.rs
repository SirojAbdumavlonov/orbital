//! Read-only view of an nginx installation: every config file nginx actually
//! loads, and the sites (server blocks) defined in them — domain names, ports,
//! TLS, and where each location goes.
//!
//! The source of truth is `nginx -T`, which tests the configuration and dumps
//! each included file. Nothing is changed here; edits go through the normal
//! safe-save path (`config_edit`), which runs `nginx -t` and rolls back.

use crate::error::Result;
use crate::ssh::SshSession;
use serde::Serialize;

/// Cap on how many directives are parsed from one dump (runaway input guard).
const MAX_DIRECTIVES: usize = 200_000;
/// Blocks nested deeper than this are skipped, not parsed (bounds recursion).
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxFile {
    pub path: String,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxLocation {
    /// As written, e.g. `/api/` or `~* \.(js|css)$`.
    pub path: String,
    pub proxy_pass: Option<String>,
    pub root: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxSite {
    pub file: String,
    pub line: usize,
    /// Domain names from `server_name` (empty for a catch-all server).
    pub server_names: Vec<String>,
    /// `listen` values as written, e.g. `443 ssl http2`.
    pub listens: Vec<String>,
    pub ssl: bool,
    pub ssl_certificate: Option<String>,
    pub root: Option<String>,
    /// A server-level `return`, e.g. `301 https://$host$request_uri`.
    pub redirect: Option<String>,
    pub locations: Vec<NginxLocation>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxUpstream {
    pub name: String,
    pub servers: Vec<String>,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NginxConfig {
    /// `nginx -t` passed.
    pub test_ok: bool,
    /// What `nginx -t` said (errors with file and line when it failed).
    pub test_output: String,
    pub files: Vec<NginxFile>,
    pub sites: Vec<NginxSite>,
    pub upstreams: Vec<NginxUpstream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Directive {
    name: String,
    args: Vec<String>,
    line: usize,
    block: Option<Vec<Directive>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Open,
    Close,
    Semi,
}

/// Split nginx config text into words, `{`, `}` and `;`, with line numbers.
/// Handles comments and single/double quoted strings; never fails.
fn tokenize(src: &str) -> Vec<(Token, usize)> {
    let mut out = Vec::new();
    let mut line = 1;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            c if c.is_whitespace() => {}
            '#' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        line += 1;
                        break;
                    }
                }
            }
            '{' => out.push((Token::Open, line)),
            '}' => out.push((Token::Close, line)),
            ';' => out.push((Token::Semi, line)),
            _ => {
                let start = line;
                let mut word = String::new();
                let mut next = Some(c);
                while let Some(c) = next {
                    match c {
                        '"' | '\'' => {
                            // Quoted part: taken literally up to the matching quote.
                            while let Some(q) = chars.next() {
                                if q == '\\' {
                                    if let Some(esc) = chars.next() {
                                        if esc == '\n' {
                                            line += 1;
                                        }
                                        // Only an escaped quote or backslash loses its
                                        // backslash; regex escapes like \d stay as written.
                                        if esc != c && esc != '\\' {
                                            word.push('\\');
                                        }
                                        word.push(esc);
                                    }
                                } else if q == c {
                                    break;
                                } else {
                                    if q == '\n' {
                                        line += 1;
                                    }
                                    word.push(q);
                                }
                            }
                        }
                        '\\' => {
                            // Kept as written (location regexes are shown to the user).
                            word.push('\\');
                            if let Some(esc) = chars.next() {
                                if esc == '\n' {
                                    line += 1;
                                }
                                word.push(esc);
                            }
                        }
                        _ => word.push(c),
                    }
                    // `${var}` keeps its braces inside a word.
                    next = match chars.peek() {
                        Some(&p) if p.is_whitespace() || p == ';' => None,
                        Some('{') if !word.ends_with('$') => None,
                        Some('}') if !word.contains("${") => None,
                        Some(_) => chars.next(),
                        None => None,
                    };
                }
                out.push((Token::Word(word), start));
            }
        }
    }
    out
}

/// Advance past the block whose `{` was just consumed, without recursing.
fn skip_block(tokens: &[(Token, usize)], pos: &mut usize) {
    let mut open = 1usize;
    while *pos < tokens.len() && open > 0 {
        match tokens[*pos].0 {
            Token::Open => open += 1,
            Token::Close => open -= 1,
            _ => {}
        }
        *pos += 1;
    }
}

fn parse_block(tokens: &[(Token, usize)], pos: &mut usize, budget: &mut usize, depth: usize) -> Vec<Directive> {
    let mut out = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut first_line = 0;
    while *pos < tokens.len() && *budget > 0 {
        let (tok, line) = &tokens[*pos];
        *pos += 1;
        match tok {
            Token::Word(w) => {
                if words.is_empty() {
                    first_line = *line;
                }
                words.push(w.clone());
            }
            Token::Semi => {
                if !words.is_empty() {
                    *budget -= 1;
                    let name = words.remove(0);
                    out.push(Directive {
                        name,
                        args: std::mem::take(&mut words),
                        line: first_line,
                        block: None,
                    });
                }
            }
            Token::Open => {
                *budget = budget.saturating_sub(1);
                let block = if depth >= MAX_DEPTH {
                    skip_block(tokens, pos);
                    Vec::new()
                } else {
                    parse_block(tokens, pos, budget, depth + 1)
                };
                let name = if words.is_empty() { String::new() } else { words.remove(0) };
                out.push(Directive {
                    name,
                    args: std::mem::take(&mut words),
                    line: if first_line == 0 { *line } else { first_line },
                    block: Some(block),
                });
                first_line = 0;
            }
            Token::Close => return out,
        }
    }
    out
}

fn parse_directives(src: &str, budget: &mut usize) -> Vec<Directive> {
    let tokens = tokenize(src);
    let mut pos = 0;
    let mut out = Vec::new();
    // A stray `}` at top level ends parse_block early; keep going after it.
    while pos < tokens.len() && *budget > 0 {
        out.extend(parse_block(&tokens, &mut pos, budget, 0));
    }
    out
}

fn first_arg(block: &[Directive], name: &str) -> Option<String> {
    block.iter().find(|d| d.name == name && d.block.is_none()).map(|d| d.args.join(" "))
}

fn site_from(file: &str, d: &Directive) -> NginxSite {
    let block = d.block.as_deref().unwrap_or(&[]);
    let listens: Vec<String> = block.iter().filter(|x| x.name == "listen").map(|x| x.args.join(" ")).collect();
    let ssl_certificate = first_arg(block, "ssl_certificate");
    let ssl = ssl_certificate.is_some()
        || listens.iter().any(|l| l.split_whitespace().any(|w| w == "ssl" || w == "quic"))
        || block.iter().any(|x| x.name == "ssl" && x.args.first().is_some_and(|a| a == "on"));
    NginxSite {
        file: file.to_string(),
        line: d.line,
        server_names: block
            .iter()
            .filter(|x| x.name == "server_name")
            .flat_map(|x| x.args.iter().cloned())
            .filter(|n| n != "_" && !n.is_empty())
            .collect(),
        listens,
        ssl,
        ssl_certificate,
        root: first_arg(block, "root"),
        redirect: first_arg(block, "return"),
        locations: block
            .iter()
            .filter(|x| x.name == "location" && x.block.is_some())
            .map(|x| {
                let inner = x.block.as_deref().unwrap_or(&[]);
                NginxLocation {
                    path: x.args.join(" "),
                    proxy_pass: ["proxy_pass", "fastcgi_pass", "uwsgi_pass", "grpc_pass"]
                        .iter()
                        .find_map(|n| first_arg(inner, n)),
                    root: first_arg(inner, "root").or_else(|| first_arg(inner, "alias")),
                    line: x.line,
                }
            })
            .collect(),
    }
}

fn collect(file: &str, dirs: &[Directive], sites: &mut Vec<NginxSite>, upstreams: &mut Vec<NginxUpstream>, depth: usize) {
    if depth > 16 {
        return;
    }
    for d in dirs {
        let Some(block) = d.block.as_deref() else { continue };
        match d.name.as_str() {
            "server" if d.args.is_empty() => sites.push(site_from(file, d)),
            "upstream" => upstreams.push(NginxUpstream {
                name: d.args.first().cloned().unwrap_or_default(),
                servers: block
                    .iter()
                    .filter(|x| x.name == "server" && x.block.is_none())
                    .map(|x| x.args.join(" "))
                    .collect(),
                file: file.to_string(),
                line: d.line,
            }),
            // http { }, stream { }, and anything else that can hold servers.
            // Location and *_by_lua_block bodies never do, so they are skipped.
            "location" | "types" | "map" | "geo" | "events" => {}
            n if n.ends_with("_by_lua_block") => {}
            _ => collect(file, block, sites, upstreams, depth + 1),
        }
    }
}

const FILE_MARKER: &str = "# configuration file ";

/// Parse the combined output of `nginx -T` (stderr first, then the dump).
pub fn parse_dump(out: &str, test_ok: bool) -> NginxConfig {
    let mut files: Vec<(String, String)> = Vec::new();
    let mut test_output = String::new();
    for line in out.lines() {
        if let Some(path) = line.strip_prefix(FILE_MARKER).and_then(|r| r.strip_suffix(':')) {
            files.push((path.to_string(), String::new()));
        } else if let Some((_, body)) = files.last_mut() {
            body.push_str(line);
            body.push('\n');
        } else {
            test_output.push_str(line);
            test_output.push('\n');
        }
    }
    let mut cfg = NginxConfig {
        test_ok,
        test_output: test_output.trim().to_string(),
        files: Vec::new(),
        sites: Vec::new(),
        upstreams: Vec::new(),
    };
    let mut budget = MAX_DIRECTIVES;
    for (path, body) in &files {
        // nginx separates dumped files with a blank line; don't count it.
        cfg.files.push(NginxFile {
            path: path.clone(),
            lines: body.trim_end().lines().count(),
        });
        // The marker line itself isn't part of the file, so line 1 is the first body line.
        let dirs = parse_directives(body, &mut budget);
        collect(path, &dirs, &mut cfg.sites, &mut cfg.upstreams, 0);
    }
    cfg
}

impl SshSession {
    /// Test and dump the live nginx configuration. Uses sudo when the profile
    /// allows it (nginx usually needs root to read TLS keys), and falls back
    /// to the login user otherwise.
    pub async fn nginx_config(&self) -> Result<NginxConfig> {
        const CMD: &str = "nginx -T 2>&1";
        let out = if self.elevation_needed() {
            match self.exec_privileged(CMD).await {
                Ok(o) => o,
                Err(_) => self.exec(CMD).await?,
            }
        } else {
            self.exec(CMD).await?
        };
        Ok(parse_dump(&out.stdout, out.ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"nginx: the configuration file /etc/nginx/nginx.conf syntax is ok
nginx: configuration file /etc/nginx/nginx.conf test is successful
# configuration file /etc/nginx/nginx.conf:
user www-data;
events { worker_connections 2048; }
http {
    # a comment with { braces } and ; semicolons
    log_format main '$remote_addr - "$request" { not a block }';
    map $http_upgrade $connection_upgrade { default upgrade; '' close; }
    upstream shop_api {
        server 127.0.0.1:8081 max_fails=3;
        server 127.0.0.1:8082 backup;
    }
    include /etc/nginx/sites-enabled/*;
}

# configuration file /etc/nginx/sites-enabled/shop:
server {
    listen 80;
    listen [::]:80;
    server_name shop.example.com www.shop.example.com;
    return 301 https://$host$request_uri;
}
server {
    listen 443 ssl http2;
    server_name shop.example.com;
    ssl_certificate     /etc/letsencrypt/live/shop.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/shop.example.com/privkey.pem;
    root /srv/shop/public;

    location /api/ {
        proxy_pass http://shop_api;
        proxy_set_header Host $host;
    }
    location ~* \.(js|css|png)$ {
        expires 30d;
    }
    location = /health { return 200 "ok"; }
    location /media/ { alias /srv/media/; }
    location ~ "^/v\d{1,2}/" { proxy_pass http://127.0.0.1:9000; }
    if ($host = old.example.com) { return 301 https://shop.example.com${request_uri}; }
}

# configuration file /etc/nginx/conf.d/default.conf:
server {
    listen 8080 default_server;
    server_name _;
    location / { root /usr/share/nginx/html; }
}
"#;

    #[test]
    fn parses_nginx_dump() {
        let cfg = parse_dump(DUMP, true);
        assert!(cfg.test_ok);
        assert!(cfg.test_output.contains("test is successful"));
        assert_eq!(
            cfg.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            [
                "/etc/nginx/nginx.conf",
                "/etc/nginx/sites-enabled/shop",
                "/etc/nginx/conf.d/default.conf"
            ]
        );

        assert_eq!(cfg.upstreams.len(), 1);
        assert_eq!(cfg.upstreams[0].name, "shop_api");
        assert_eq!(cfg.upstreams[0].servers, ["127.0.0.1:8081 max_fails=3", "127.0.0.1:8082 backup"]);
        // Line numbers are relative to the file, not the dump.
        assert_eq!(cfg.upstreams[0].line, 7);

        assert_eq!(cfg.sites.len(), 3);
        let redirect = &cfg.sites[0];
        assert_eq!(redirect.file, "/etc/nginx/sites-enabled/shop");
        assert_eq!(redirect.line, 1);
        assert_eq!(redirect.server_names, ["shop.example.com", "www.shop.example.com"]);
        assert_eq!(redirect.listens, ["80", "[::]:80"]);
        assert!(!redirect.ssl);
        assert_eq!(redirect.redirect.as_deref(), Some("301 https://$host$request_uri"));

        let shop = &cfg.sites[1];
        assert_eq!(shop.line, 7);
        assert!(shop.ssl);
        assert_eq!(
            shop.ssl_certificate.as_deref(),
            Some("/etc/letsencrypt/live/shop.example.com/fullchain.pem")
        );
        assert_eq!(shop.root.as_deref(), Some("/srv/shop/public"));
        assert_eq!(
            shop.locations.iter().map(|l| l.path.as_str()).collect::<Vec<_>>(),
            ["/api/", r"~* \.(js|css|png)$", "= /health", "/media/", r"~ ^/v\d{1,2}/"]
        );
        assert_eq!(shop.locations[0].proxy_pass.as_deref(), Some("http://shop_api"));
        assert_eq!(shop.locations[0].line, 14);
        assert_eq!(shop.locations[3].root.as_deref(), Some("/srv/media/"));
        assert_eq!(shop.locations[4].proxy_pass.as_deref(), Some("http://127.0.0.1:9000"));
        // The `if` block's return is not the server's own redirect.
        assert_eq!(shop.redirect, None);

        // `_` is a catch-all, not a domain.
        let default = &cfg.sites[2];
        assert!(default.server_names.is_empty());
        assert_eq!(default.listens, ["8080 default_server"]);
        assert_eq!(default.locations[0].root.as_deref(), Some("/usr/share/nginx/html"));
    }

    #[test]
    fn failed_test_and_garbage_never_panic() {
        let failed = "nginx: [emerg] unexpected \"}\" in /etc/nginx/sites-enabled/shop:41\nnginx: configuration file /etc/nginx/nginx.conf test failed\n";
        let cfg = parse_dump(failed, false);
        assert!(!cfg.test_ok);
        assert!(cfg.test_output.contains("unexpected"));
        assert!(cfg.files.is_empty() && cfg.sites.is_empty());

        for junk in [
            "",
            "}}}}",
            "{{{{",
            "server {",
            "# configuration file /x:\nserver { listen 80; server_name a.example \"unterminated",
            "# configuration file /x:\n} server { listen 1; } }",
            "sudo: a password is required",
        ] {
            let _ = parse_dump(junk, false);
        }
        // An unbalanced file still yields the server it does contain.
        let cfg = parse_dump("# configuration file /x:\n} server { listen 1; }\n", true);
        assert_eq!(cfg.sites.len(), 1);
        assert_eq!(cfg.sites[0].listens, ["1"]);

        // Deep nesting is bounded, not a stack overflow.
        let deep = format!("# configuration file /x:\n{}", "a {".repeat(100_000));
        let _ = parse_dump(&deep, true);
    }
}
