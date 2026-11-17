//! "New website": one nginx server block for one domain, from a small
//! template (a folder of static files, or a reverse proxy to a local port).
//!
//! The block is written through safe save (`config_edit`): on a Debian-style
//! layout it goes to `sites-available/<domain>` and is linked from
//! `sites-enabled/`, elsewhere to `conf.d/<domain>.conf`. `nginx -t` runs
//! once the site is live in the configuration, and a failure removes both the
//! file and the link again. DNS and the certificate are separate steps in the
//! UI, built on `dns` and `certbot`.

use crate::certbot;
use crate::config_edit::{SafeWriteReport, ValidateStage, Validator};
use crate::error::{Error, Result};
use crate::nginx::NginxConfig;
use crate::shell::quote;
use crate::ssh::SshSession;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const NGINX_DIR: &str = "/etc/nginx";

/// What the site serves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SiteKind {
    /// Files from a folder on the server.
    Static { root: String },
    /// An app listening on this port on the server itself (127.0.0.1).
    Proxy { port: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteInput {
    pub domain: String,
    /// Also answer for `www.<domain>`.
    #[serde(default)]
    pub include_www: bool,
    pub site: SiteKind,
}

/// Where this nginx keeps its sites.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    /// Debian/Ubuntu: `sites-available/` plus a link in `sites-enabled/`.
    SitesAvailable,
    /// nginx.org packages, RHEL, Alpine, …: `conf.d/*.conf`.
    ConfD,
}

/// What "Create" would do, worked out without changing anything.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebsitePlan {
    pub domain: String,
    /// `server_name`s: the domain, and `www.` when asked for.
    pub names: Vec<String>,
    pub layout: Layout,
    /// The server block's file.
    pub file: String,
    /// The `sites-enabled` link (Debian layout only).
    pub enabled_link: Option<String>,
    /// Static sites: the folder, and whether it exists yet (if not, it's
    /// created with a placeholder page).
    pub root: Option<String>,
    pub root_exists: bool,
    pub content: String,
    /// Why it can't be created as it stands (file exists, name already served,
    /// nginx's configuration already broken). Empty: good to go.
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteReport {
    pub plan: WebsitePlan,
    pub write: SafeWriteReport,
    pub root_created: bool,
    /// nginx picked the new site up (reload succeeded).
    pub reloaded: bool,
    pub reload_output: String,
}

// ---------- validation ----------

/// A domain the site can be named after: lowercase, no wildcard, at least one dot.
pub fn validate_domain(d: &str) -> Result<String> {
    let d = certbot::validate_domain(d)?;
    if d.parse::<std::net::IpAddr>().is_ok() {
        return Err(Error::InvalidInput(format!("{d} is an address, not a domain name")));
    }
    Ok(d)
}

/// The local port a reverse proxy forwards to.
pub fn validate_port(port: u16) -> Result<u16> {
    match port {
        0 => Err(Error::InvalidInput("choose the port your app listens on (1–65535)".into())),
        80 | 443 => Err(Error::InvalidInput(format!(
            "port {port} is nginx itself; use the port your app listens on, for example 3000 or 8080"
        ))),
        22 => Err(Error::InvalidInput("port 22 is SSH, not a web app".into())),
        p => Ok(p),
    }
}

static ROOT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(/[A-Za-z0-9._@+-]+)+$").unwrap());
/// Folders a site must never be served from (they'd publish the system).
const SYSTEM_DIRS: &[&str] = &[
    "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/tmp", "/usr",
    "/var", "/var/lib", "/var/log", "/var/www",
];

/// The folder a static site serves. It ends up inside the nginx config, so
/// only plain path characters are allowed (no spaces, quotes, `;`, `$`, `{`).
pub fn validate_root(root: &str) -> Result<String> {
    let r = root.trim();
    let r = if r.len() > 1 { r.trim_end_matches('/') } else { r };
    if !ROOT_RE.is_match(r) || r.len() > 255 {
        return Err(Error::InvalidInput(format!(
            "{r:?}: use an absolute folder path made of letters, digits and . _ - @ +, for example /var/www/example.com"
        )));
    }
    if r.split('/').any(|c| c == "." || c == "..") {
        return Err(Error::InvalidInput("the folder path can't contain . or .. parts".into()));
    }
    let top = ["/etc", "/proc", "/sys", "/dev", "/boot", "/root", "/run"];
    if SYSTEM_DIRS.contains(&r) || top.iter().any(|t| r.starts_with(&format!("{t}/"))) {
        return Err(Error::InvalidInput(format!(
            "{r} is a system folder; serve the site from its own folder, for example /var/www/<domain>"
        )));
    }
    Ok(r.to_string())
}

/// The input with every value checked and normalized.
pub fn validate(input: &WebsiteInput) -> Result<WebsiteInput> {
    Ok(WebsiteInput {
        domain: validate_domain(&input.domain)?,
        include_www: input.include_www,
        site: match &input.site {
            SiteKind::Static { root } => SiteKind::Static {
                root: validate_root(root)?,
            },
            SiteKind::Proxy { port } => SiteKind::Proxy {
                port: validate_port(*port)?,
            },
        },
    })
}

/// `server_name`s for a (validated) input. `www.` isn't doubled.
pub fn names(input: &WebsiteInput) -> Vec<String> {
    let mut n = vec![input.domain.clone()];
    if input.include_www && !input.domain.starts_with("www.") {
        n.push(format!("www.{}", input.domain));
    }
    n
}

// ---------- the template ----------

/// The server block for a validated input.
pub fn render(input: &WebsiteInput) -> String {
    let names = names(input).join(" ");
    let body = match &input.site {
        SiteKind::Static { root } => {
            format!("    root {root};\n    index index.html index.htm;\n\n    location / {{\n        try_files $uri $uri/ =404;\n    }}\n")
        }
        SiteKind::Proxy { port } => format!(
            "    location / {{\n        proxy_pass http://127.0.0.1:{port};\n        proxy_http_version 1.1;\n        \
             proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        \
             proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        \
             proxy_set_header X-Forwarded-Proto $scheme;\n        \
             # WebSockets\n        proxy_set_header Upgrade $http_upgrade;\n        \
             proxy_set_header Connection \"upgrade\";\n        proxy_read_timeout 300s;\n    }}\n"
        ),
    };
    format!(
        "# {domain}: created by Orbital. Edit it like any other nginx file;\n\
         # HTTPS is added by certbot once the certificate is issued.\n\
         server {{\n    listen 80;\n    listen [::]:80;\n    server_name {names};\n\n{body}}}\n",
        domain = input.domain
    )
}

/// The page a new, empty static folder gets, so the site shows something.
pub fn placeholder_page(domain: &str) -> String {
    format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>{domain}</title>\n\
         <p>{domain} is served by nginx. Replace this file with your site.</p>\n"
    )
}

/// Paths for a domain in a layout: (server block file, sites-enabled link).
pub fn paths(domain: &str, layout: Layout) -> (String, Option<String>) {
    match layout {
        Layout::SitesAvailable => (
            format!("{NGINX_DIR}/sites-available/{domain}"),
            Some(format!("{NGINX_DIR}/sites-enabled/{domain}")),
        ),
        Layout::ConfD => (format!("{NGINX_DIR}/conf.d/{domain}.conf"), None),
    }
}

// ---------- probing the server ----------

/// Read-only: which layout, which of our paths exist, whether the folder exists.
/// `domain` and `root` must be validated.
pub fn probe_script(domain: &str, root: Option<&str>) -> String {
    let conf = quote(&format!("{NGINX_DIR}/nginx.conf"));
    let mut s = format!(
        "if [ -d {d}/sites-available ] && [ -d {d}/sites-enabled ] && grep -Eqs '^[[:space:]]*include[[:space:]][^;]*sites-enabled' {conf}; then echo LAYOUT=sites; \
         elif [ -d {d}/conf.d ] && grep -Eqs '^[[:space:]]*include[[:space:]][^;]*conf\\.d' {conf}; then echo LAYOUT=confd; \
         else echo LAYOUT=none; fi; ",
        d = quote(NGINX_DIR)
    );
    for layout in [Layout::SitesAvailable, Layout::ConfD] {
        let (file, link) = paths(domain, layout);
        for p in std::iter::once(file).chain(link) {
            s.push_str(&format!("if [ -e {q} ] || [ -L {q} ]; then echo EXISTS={p}; fi; ", q = quote(&p)));
        }
    }
    if let Some(r) = root {
        s.push_str(&format!("if [ -d {} ]; then echo ROOT=yes; fi; ", quote(r)));
    }
    s.push_str("true");
    s
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    pub layout: Option<Layout>,
    pub existing: Vec<String>,
    pub root_exists: bool,
}

pub fn parse_probe(out: &str) -> Probe {
    let mut p = Probe::default();
    for line in out.lines().map(str::trim) {
        if let Some(l) = line.strip_prefix("LAYOUT=") {
            p.layout = match l {
                "sites" => Some(Layout::SitesAvailable),
                "confd" => Some(Layout::ConfD),
                _ => None,
            };
        } else if let Some(path) = line.strip_prefix("EXISTS=") {
            p.existing.push(path.to_string());
        } else if line == "ROOT=yes" {
            p.root_exists = true;
        }
    }
    p
}

/// Sites nginx already serves under any of `names`, as "name (file:line)".
pub fn conflicts(cfg: &NginxConfig, names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for site in &cfg.sites {
        for n in &site.server_names {
            if names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                out.push(format!("{n} is already served by {}:{}", site.file, site.line));
            }
        }
    }
    out
}

/// Put the plan together from a validated input and what the server said.
pub fn plan(input: &WebsiteInput, probe: &Probe, cfg: &NginxConfig) -> Result<WebsitePlan> {
    let layout = probe.layout.ok_or_else(|| {
        Error::InvalidInput(format!(
            "{NGINX_DIR}/nginx.conf includes neither sites-enabled/ nor conf.d/, so Orbital doesn't know where a new site goes. Add the server block by hand."
        ))
    })?;
    let (file, enabled_link) = paths(&input.domain, layout);
    let names = names(input);
    let mut blockers = Vec::new();
    if !cfg.test_ok {
        blockers.push(format!(
            "nginx's configuration already has errors; fix them first:\n{}",
            cfg.test_output.trim()
        ));
    }
    for p in &probe.existing {
        if *p == file || Some(p) == enabled_link.as_ref() {
            blockers.push(format!("{p} already exists"));
        }
    }
    blockers.extend(conflicts(cfg, &names));
    let root = match &input.site {
        SiteKind::Static { root } => Some(root.clone()),
        SiteKind::Proxy { .. } => None,
    };
    Ok(WebsitePlan {
        domain: input.domain.clone(),
        names,
        layout,
        file,
        enabled_link,
        root_exists: root.is_some() && probe.root_exists,
        root,
        content: render(input),
        blockers,
    })
}

fn nginx_test() -> Validator {
    Validator {
        name: "nginx".into(),
        command: "nginx -t".into(),
        stage: ValidateStage::PostInstall,
    }
}

impl SshSession {
    /// Work out what creating the site would do (read-only).
    pub async fn website_plan(&self, input: &WebsiteInput) -> Result<WebsitePlan> {
        let v = validate(input)?;
        let root = match &v.site {
            SiteKind::Static { root } => Some(root.as_str()),
            SiteKind::Proxy { .. } => None,
        };
        let (o, _) = self.exec_inspect(&probe_script(&v.domain, root)).await?;
        let cfg = self.nginx_config().await?;
        plan(&v, &parse_probe(&o.stdout), &cfg)
    }

    /// Write a new server block and make it live, as root. With a `link`
    /// (Debian layout) the file is written first and `nginx -t` runs once it's
    /// linked; otherwise safe save runs `nginx -t` itself. Either way, a failed
    /// test leaves neither the file nor the link behind.
    pub async fn install_site(&self, file: &str, link: Option<&str>, content: &str) -> Result<SafeWriteReport> {
        let Some(link) = link else {
            return self.safe_write(file, content, Some(nginx_test()), true, None).await;
        };
        let mut rep = self.safe_write(file, content, None, true, None).await?;
        let (f, l) = (quote(file), quote(link));
        let ln = self.exec_privileged(&format!("ln -s -- {f} {l} 2>&1")).await?;
        if !ln.ok() {
            // The link isn't ours (it appeared meanwhile, or the folder is
            // read-only): take back only the file.
            let _ = self.exec_privileged(&format!("rm -f -- {f}")).await;
            return Err(Error::Remote {
                code: ln.exit_code,
                stderr: ln.combined(),
            });
        }
        let o = self.exec_privileged("nginx -t 2>&1").await?;
        if !o.ok() {
            let rb = self.exec_privileged(&format!("rm -f -- {l} {f}")).await?;
            let mut output = o.combined();
            if !rb.ok() {
                output.push_str(&format!("\n\n⚠ Couldn't remove {file} and {link} again: {}", rb.combined()));
            }
            return Err(Error::ValidationFailed {
                validator: "nginx".into(),
                output,
            });
        }
        rep.validator = Some("nginx".into());
        rep.validator_output = Some(o.combined());
        rep.suggested_reload = Some("nginx.service".into());
        Ok(rep)
    }

    /// Create a static site's folder with a placeholder page (root, 755).
    pub async fn create_site_root(&self, root: &str, domain: &str) -> Result<()> {
        let index = quote(&format!("{root}/index.html"));
        let cmd = format!(
            "mkdir -p -- {r} && chmod 755 -- {r} && printf '%s' {page} > {index} && chmod 644 -- {index}",
            r = quote(root),
            page = quote(&placeholder_page(domain)),
        );
        self.exec_privileged(&cmd).await?.check().map(|_| ())
    }

    /// Tell nginx to load the new configuration.
    pub async fn reload_nginx(&self) -> Result<(bool, String)> {
        let o = self
            .exec_privileged("if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet nginx; then systemctl reload nginx 2>&1; else nginx -s reload 2>&1; fi")
            .await?;
        Ok((o.ok(), o.combined()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nginx::NginxSite;

    fn proxy(domain: &str, port: u16, www: bool) -> WebsiteInput {
        WebsiteInput {
            domain: domain.into(),
            include_www: www,
            site: SiteKind::Proxy { port },
        }
    }

    #[test]
    fn domains() {
        assert_eq!(validate_domain(" Shop.Example.COM. ").unwrap(), "shop.example.com");
        for bad in [
            "",
            "localhost",
            "*.example.com",
            "exa mple.com",
            "example.com;id",
            "-x.example.com",
            "1.2.3.4",
            "a..b.com",
            "ex$ample.com",
        ] {
            assert!(validate_domain(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn ports() {
        assert_eq!(validate_port(3000).unwrap(), 3000);
        assert_eq!(validate_port(65535).unwrap(), 65535);
        for bad in [0, 22, 80, 443] {
            assert!(validate_port(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn roots() {
        assert_eq!(validate_root("/var/www/example.com/").unwrap(), "/var/www/example.com");
        assert_eq!(validate_root("/srv/sites/shop_2").unwrap(), "/srv/sites/shop_2");
        for bad in [
            "var/www/x",
            "/",
            "/etc",
            "/etc/nginx/html",
            "/var/www",
            "/home",
            "/var/www/a b",
            "/var/www/x;",
            "/var/www/$x",
            "/var/www/../etc",
            "/var/www/{x}",
            "/var/www/'x'",
            "/root/site",
        ] {
            assert!(validate_root(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn validates_and_names() {
        let v = validate(&proxy("Example.com", 8080, true)).unwrap();
        assert_eq!(v.domain, "example.com");
        assert_eq!(names(&v), ["example.com", "www.example.com"]);
        assert_eq!(
            names(&validate(&proxy("www.example.com", 8080, true)).unwrap()),
            ["www.example.com"]
        );
        assert!(validate(&proxy("example.com", 80, false)).is_err());
        let bad_root = WebsiteInput {
            domain: "example.com".into(),
            include_www: false,
            site: SiteKind::Static { root: "/etc".into() },
        };
        assert!(validate(&bad_root).is_err());
    }

    #[test]
    fn renders_a_static_site() {
        let v = validate(&WebsiteInput {
            domain: "example.com".into(),
            include_www: true,
            site: SiteKind::Static {
                root: "/var/www/example.com".into(),
            },
        })
        .unwrap();
        let c = render(&v);
        assert!(c.contains("server_name example.com www.example.com;\n"));
        assert!(c.contains("    listen 80;\n    listen [::]:80;\n"));
        assert!(c.contains("    root /var/www/example.com;\n"));
        assert!(c.contains("try_files $uri $uri/ =404;"));
        assert!(!c.contains("proxy_pass"));
        // The parser Orbital reads nginx with sees exactly one site, as meant.
        let dump = format!("# configuration file /etc/nginx/conf.d/example.com.conf:\n{c}");
        let cfg = crate::nginx::parse_dump(&dump, true);
        assert_eq!(cfg.sites.len(), 1);
        assert_eq!(cfg.sites[0].server_names, ["example.com", "www.example.com"]);
        assert_eq!(cfg.sites[0].root.as_deref(), Some("/var/www/example.com"));
        assert_eq!(cfg.sites[0].listens, ["80", "[::]:80"]);
    }

    #[test]
    fn renders_a_reverse_proxy() {
        let c = render(&validate(&proxy("app.example.com", 3000, false)).unwrap());
        assert!(c.contains("server_name app.example.com;\n"));
        assert!(c.contains("proxy_pass http://127.0.0.1:3000;"));
        assert!(c.contains("proxy_set_header Host $host;"));
        assert!(c.contains("proxy_set_header X-Forwarded-Proto $scheme;"));
        assert!(!c.contains("root "));
        let cfg = crate::nginx::parse_dump(&format!("# configuration file /etc/nginx/sites-enabled/x:\n{c}"), true);
        assert_eq!(cfg.sites.len(), 1);
        assert_eq!(cfg.sites[0].locations[0].proxy_pass.as_deref(), Some("http://127.0.0.1:3000"));
        // Braces balance (nginx -t would catch it, but cheaper here).
        assert_eq!(c.matches('{').count(), c.matches('}').count());
    }

    #[test]
    fn probes_and_plans() {
        let s = probe_script("example.com", Some("/var/www/example.com"));
        assert!(s.contains("/etc/nginx/sites-available/example.com"));
        assert!(s.contains("/etc/nginx/conf.d/example.com.conf"));
        assert!(s.contains("[ -d /var/www/example.com ]"));

        let p = parse_probe("LAYOUT=sites\nEXISTS=/etc/nginx/conf.d/example.com.conf\nROOT=yes\n");
        assert_eq!(p.layout, Some(Layout::SitesAvailable));
        assert!(p.root_exists);
        assert_eq!(parse_probe("LAYOUT=none\n").layout, None);

        let ok = NginxConfig {
            test_ok: true,
            test_output: String::new(),
            files: vec![],
            sites: vec![],
            upstreams: vec![],
        };
        let input = validate(&WebsiteInput {
            domain: "example.com".into(),
            include_www: true,
            site: SiteKind::Static {
                root: "/var/www/example.com".into(),
            },
        })
        .unwrap();
        let pl = plan(&input, &p, &ok).unwrap();
        assert_eq!(pl.file, "/etc/nginx/sites-available/example.com");
        assert_eq!(pl.enabled_link.as_deref(), Some("/etc/nginx/sites-enabled/example.com"));
        assert!(pl.root_exists);
        // A conf.d file for the same domain doesn't matter in the sites layout.
        assert!(pl.blockers.is_empty(), "{:?}", pl.blockers);

        let confd = plan(
            &input,
            &parse_probe("LAYOUT=confd\nEXISTS=/etc/nginx/conf.d/example.com.conf\n"),
            &ok,
        )
        .unwrap();
        assert_eq!(confd.file, "/etc/nginx/conf.d/example.com.conf");
        assert_eq!(confd.enabled_link, None);
        assert!(!confd.root_exists);
        assert_eq!(confd.blockers, ["/etc/nginx/conf.d/example.com.conf already exists"]);

        assert!(plan(&input, &parse_probe("LAYOUT=none\n"), &ok).is_err());

        let site = NginxSite {
            file: "/etc/nginx/sites-enabled/old".into(),
            line: 3,
            server_names: vec!["WWW.example.com".into()],
            listens: vec![],
            ssl: false,
            ssl_certificate: None,
            root: None,
            redirect: None,
            locations: vec![],
        };
        let busy = NginxConfig {
            test_ok: false,
            test_output: "nginx: [emerg] unexpected \"}\"".into(),
            sites: vec![site],
            ..ok
        };
        let b = plan(&input, &p, &busy).unwrap().blockers;
        assert_eq!(b.len(), 2);
        assert!(b[0].contains("already has errors"));
        assert_eq!(b[1], "WWW.example.com is already served by /etc/nginx/sites-enabled/old:3");
    }

    #[test]
    fn placeholder_names_the_domain() {
        assert!(placeholder_page("example.com").contains("<title>example.com</title>"));
    }
}
