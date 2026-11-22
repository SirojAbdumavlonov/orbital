//! HTTPS certificates from Let's Encrypt through certbot: list them with their
//! expiry, see whether automatic renewal is set up, and build the commands
//! that issue, renew and delete certificates.
//!
//! Domain names, e-mail addresses, certificate names and folders are validated
//! and quoted before they go into a command.

use crate::error::{Error, Result};
use crate::shell::{quote, validate_path};
use crate::ssh::SshSession;
use chrono::{DateTime, FixedOffset, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Certificate {
    /// certbot's name for it (usually the first domain), used to renew or delete it.
    pub name: String,
    pub domains: Vec<String>,
    /// RFC 3339 expiry time.
    pub expires: Option<String>,
    /// Whole days until expiry; negative once expired.
    pub days_left: Option<i64>,
    pub valid: bool,
    pub key_type: Option<String>,
    pub certificate_path: Option<String>,
    pub private_key_path: Option<String>,
    /// certbot's own verdict, e.g. `VALID: 56 days` or `INVALID: EXPIRED`.
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoRenew {
    /// `certbot.timer`, `snap.certbot.renew.timer` or `/etc/cron.d/certbot`.
    pub via: String,
    pub active: bool,
    /// When the timer fires next, as systemd prints it.
    pub next_run: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CertbotStatus {
    pub version: Option<String>,
    pub certificates: Vec<Certificate>,
    pub auto_renew: Option<AutoRenew>,
    /// Installed authenticator plugins: `nginx`, `apache`, `webroot`, `standalone`, …
    pub plugins: Vec<String>,
    /// What certbot said when it couldn't list certificates.
    pub error: Option<String>,
    /// Another certbot holds the lock, so the list is unavailable for now.
    #[serde(default)]
    pub busy: bool,
}

/// How certbot proves to Let's Encrypt that the domain points at this server.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CertMethod {
    /// Uses the running nginx and adds the certificate to the matching server blocks.
    Nginx,
    /// The same for Apache.
    Apache,
    /// Puts a file in a folder the web server already serves.
    Webroot,
    /// Starts its own web server on port 80 (nothing else may be using it).
    Standalone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueRequest {
    pub domains: Vec<String>,
    /// For expiry warnings from Let's Encrypt. Optional.
    pub email: Option<String>,
    pub method: CertMethod,
    /// The folder the site serves, for `Webroot`.
    pub webroot: Option<String>,
    /// With nginx or Apache: also redirect plain HTTP to HTTPS.
    pub redirect: bool,
    /// Use Let's Encrypt's test servers (untrusted certificate, no rate limits).
    pub staging: bool,
    /// The user accepted the Let's Encrypt Subscriber Agreement.
    pub agree_tos: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RenewMode {
    /// Rehearse against the test servers; nothing changes.
    DryRun,
    /// Renew certificates that are due (within 30 days of expiry).
    Renew,
    /// Renew now even if not due.
    Force,
}

pub(crate) const STATUS_SCRIPT: &str = r#"export LC_ALL=C
command -v certbot >/dev/null 2>&1 || { echo @@missing; exit 0; }
echo "@@version $(certbot --version 2>&1 | head -1)"
echo @@certificates
certbot certificates 2>&1
echo @@plugins
certbot plugins 2>/dev/null | sed -n 's/^\* //p'
echo @@renew
for t in certbot.timer snap.certbot.renew.timer; do
  st=$(systemctl show -p LoadState -p ActiveState -p NextElapseUSecRealtime "$t" 2>/dev/null | tr '\n' '|')
  case "$st" in *LoadState=loaded*) echo "timer=$t|$st"; break;; esac
done
[ -f /etc/cron.d/certbot ] && echo "cron=/etc/cron.d/certbot"
"#;

fn parse_expiry(s: &str) -> Option<DateTime<FixedOffset>> {
    // "2027-01-10 12:00:00+00:00"
    DateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S%:z").ok()
}

/// Parse the status script's output. `None` when certbot is not installed.
pub fn parse_status(out: &str, now: DateTime<Utc>) -> Option<CertbotStatus> {
    let mut st = CertbotStatus::default();
    let mut section = "";
    let mut cur: Option<Certificate> = None;
    let mut listing: Vec<&str> = Vec::new();
    for line in out.lines() {
        let l = line.trim();
        if l == "@@missing" {
            return None;
        }
        if let Some(v) = l.strip_prefix("@@version") {
            let v = v.trim().trim_start_matches("certbot").trim();
            st.version = (!v.is_empty()).then(|| v.to_string());
            continue;
        }
        if let Some(name) = l.strip_prefix("@@") {
            section = match name {
                "certificates" => "certificates",
                "plugins" => "plugins",
                "renew" => "renew",
                _ => "",
            };
            continue;
        }
        match section {
            "certificates" => {
                listing.push(l);
                if let Some(name) = l.strip_prefix("Certificate Name:") {
                    st.certificates.extend(cur.take());
                    cur = Some(Certificate {
                        name: name.trim().to_string(),
                        ..Default::default()
                    });
                    continue;
                }
                let Some(c) = cur.as_mut() else { continue };
                let Some((k, v)) = l.split_once(':') else { continue };
                let v = v.trim();
                match k {
                    "Domains" => c.domains = v.split_whitespace().map(String::from).collect(),
                    "Key Type" => c.key_type = Some(v.to_string()),
                    "Certificate Path" => c.certificate_path = Some(v.to_string()),
                    "Private Key Path" => c.private_key_path = Some(v.to_string()),
                    "Expiry Date" => {
                        // "2027-01-10 12:00:00+00:00 (VALID: 56 days)"
                        let (date, verdict) = match v.split_once(" (") {
                            Some((d, rest)) => (d, rest.trim_end_matches(')')),
                            None => (v, ""),
                        };
                        c.status = verdict.to_string();
                        c.valid = verdict.starts_with("VALID");
                        if let Some(exp) = parse_expiry(date) {
                            c.expires = Some(exp.to_rfc3339());
                            c.days_left = Some((exp.with_timezone(&Utc) - now).num_days());
                        }
                    }
                    _ => {}
                }
            }
            "plugins" if !l.is_empty() => st.plugins.push(l.to_string()),
            "renew" => {
                if let Some(v) = l.strip_prefix("timer=") {
                    let mut parts = v.split('|');
                    let unit = parts.next().unwrap_or("").to_string();
                    let mut ar = AutoRenew {
                        via: unit,
                        ..Default::default()
                    };
                    for p in parts {
                        if let Some(s) = p.strip_prefix("ActiveState=") {
                            ar.active = s == "active";
                        } else if let Some(n) = p.strip_prefix("NextElapseUSecRealtime=") {
                            ar.next_run = (!n.is_empty() && n != "n/a").then(|| n.to_string());
                        }
                    }
                    st.auto_renew = Some(ar);
                } else if let Some(v) = l.strip_prefix("cron=") {
                    st.auto_renew.get_or_insert(AutoRenew {
                        via: v.to_string(),
                        active: true,
                        next_run: None,
                    });
                }
            }
            _ => {}
        }
    }
    st.certificates.extend(cur.take());
    let text = listing.join("\n");
    if st.certificates.is_empty() && !text.contains("No certificates found") && !text.trim().is_empty() {
        // certbot printed something other than a listing: an error worth showing.
        let msg: Vec<&str> = listing
            .iter()
            .copied()
            .filter(|l| !l.is_empty() && !l.starts_with("Saving debug log") && !l.starts_with("- - -"))
            .collect();
        st.error = Some(msg.join("\n"));
    }
    Some(st)
}

static DOMAIN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,61}[a-z0-9]$").unwrap());
static EMAIL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9._%+-]{1,64}@[A-Za-z0-9.-]{1,253}\.[A-Za-z]{2,63}$").unwrap());
static CERT_NAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$").unwrap());

pub fn validate_domain(d: &str) -> Result<String> {
    let d = d.trim().trim_end_matches('.').to_ascii_lowercase();
    if d.starts_with("*.") {
        return Err(Error::InvalidInput(format!(
            "{d}: wildcard certificates need a DNS challenge, which isn't supported yet"
        )));
    }
    if d.len() > 253 || !DOMAIN_RE.is_match(&d) {
        return Err(Error::InvalidInput(format!("{d:?} is not a domain name")));
    }
    Ok(d)
}

pub fn validate_cert_name(n: &str) -> Result<&str> {
    if CERT_NAME_RE.is_match(n) {
        Ok(n)
    } else {
        Err(Error::InvalidInput(format!("invalid certificate name: {n:?}")))
    }
}

/// The certbot command for a new certificate.
pub fn issue_command(r: &IssueRequest) -> Result<String> {
    issue_command_with(r, true)
}

/// nginx's own files Orbital never lets certbot change: the stock `default`
/// site and the main nginx.conf. HTTPS for a name goes only into that name's
/// own site file.
pub fn is_nginx_default_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(name, "default" | "default.conf" | "nginx.conf")
}

/// Whether certbot's nginx installer may turn HTTPS on for `domains`: each
/// name must be served (an exact `server_name`) by a site in its own file,
/// and no stock file (`default`, nginx.conf) may name it, because certbot
/// writes into whichever block it picks and, when none matches, into the
/// default server. Ok: the files it will change. Err: why it may not.
pub fn nginx_install_targets(cfg: &crate::nginx::NginxConfig, domains: &[String]) -> std::result::Result<Vec<String>, String> {
    let mut files: Vec<String> = Vec::new();
    for d in domains {
        let d = d.trim().to_ascii_lowercase();
        let serving: Vec<&crate::nginx::NginxSite> = cfg
            .sites
            .iter()
            .filter(|site| site.server_names.iter().any(|n| n.eq_ignore_ascii_case(&d)))
            .collect();
        if let Some(stock) = serving.iter().find(|site| is_nginx_default_file(&site.file)) {
            return Err(format!(
                "{d} is named in {}, one of nginx's own files, which Orbital doesn't let certbot change",
                stock.file
            ));
        }
        let Some(site) = serving.first() else {
            return Err(format!(
                "no nginx site serves {d} yet, so certbot would put HTTPS into nginx's default site; create the site first (Nginx tab → New website)"
            ));
        };
        if !files.contains(&site.file) {
            files.push(site.file.clone());
        }
    }
    Ok(files)
}

/// `install`: false gets the certificate only (`certonly`), with nginx or
/// Apache answering Let's Encrypt's check through a temporary config, and
/// changes no web-server file.
pub fn issue_command_with(r: &IssueRequest, install: bool) -> Result<String> {
    if !r.agree_tos {
        return Err(Error::InvalidInput(
            "accept the Let's Encrypt Subscriber Agreement to request a certificate".into(),
        ));
    }
    if r.domains.is_empty() || r.domains.len() > 100 {
        return Err(Error::InvalidInput("give between 1 and 100 domain names".into()));
    }
    let mut domains: Vec<String> = Vec::new();
    for d in &r.domains {
        let d = validate_domain(d)?;
        if !domains.contains(&d) {
            domains.push(d);
        }
    }
    let mut cmd = String::from("certbot");
    match r.method {
        CertMethod::Nginx if !install => cmd.push_str(" certonly --nginx"),
        CertMethod::Apache if !install => cmd.push_str(" certonly --apache"),
        CertMethod::Nginx => cmd.push_str(" --nginx"),
        CertMethod::Apache => cmd.push_str(" --apache"),
        CertMethod::Webroot => {
            let w = r
                .webroot
                .as_deref()
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .ok_or_else(|| Error::InvalidInput("choose the folder the site serves files from".into()))?;
            validate_path(w)?;
            cmd.push_str(&format!(" certonly --webroot -w {}", quote(w)));
        }
        CertMethod::Standalone => cmd.push_str(" certonly --standalone"),
    }
    for d in &domains {
        cmd.push_str(&format!(" -d {}", quote(d)));
    }
    cmd.push_str(" --non-interactive --agree-tos");
    match r.email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) if EMAIL_RE.is_match(e) => cmd.push_str(&format!(" -m {}", quote(e))),
        Some(e) => return Err(Error::InvalidInput(format!("{e:?} is not an e-mail address"))),
        None => cmd.push_str(" --register-unsafely-without-email"),
    }
    if install && matches!(r.method, CertMethod::Nginx | CertMethod::Apache) {
        cmd.push_str(if r.redirect { " --redirect" } else { " --no-redirect" });
    }
    if r.staging {
        cmd.push_str(" --staging");
    }
    Ok(cmd)
}

pub fn renew_command(name: Option<&str>, mode: RenewMode) -> Result<String> {
    // Without a terminal, certbot renew first sleeps a random 0-8 minutes (so
    // cron jobs everywhere don't hit Let's Encrypt at once). Someone clicked
    // this button and is waiting: start now.
    let mut cmd = String::from("certbot renew --non-interactive --no-random-sleep-on-renew");
    if let Some(n) = name {
        cmd.push_str(&format!(" --cert-name {}", quote(validate_cert_name(n)?)));
    }
    match mode {
        RenewMode::DryRun => cmd.push_str(" --dry-run"),
        RenewMode::Force => cmd.push_str(" --force-renewal"),
        RenewMode::Renew => {}
    }
    Ok(cmd)
}

pub fn delete_command(name: &str) -> Result<String> {
    Ok(format!(
        "certbot delete --non-interactive --cert-name {}",
        quote(validate_cert_name(name)?)
    ))
}

/// Lists the web-server config files that still point at a certificate, so it
/// isn't deleted out from under a running site. nginx is asked for every file
/// it really loads (`nginx -T`, which also covers includes outside /etc/nginx
/// and folders reached through links); Apache's folders are searched,
/// following links (`grep -R`).
pub fn usage_check_command(name: &str) -> Result<String> {
    let dir = quote(&format!("/etc/letsencrypt/live/{}/", validate_cert_name(name)?));
    Ok(format!(
        "{{ if command -v nginx >/dev/null 2>&1; then nginx -T 2>/dev/null | awk -v d={dir} '/^# configuration file /{{f=$4; sub(/:$/, \"\", f)}} index($0, d) && f != \"\" {{print f}}'; fi; \
grep -RlF -- {dir} /etc/nginx /etc/apache2 /etc/httpd 2>/dev/null; }} | sort -u; true"
    ))
}

/// What the certificate list says while another certbot (a renewal, a test or
/// the timer's own run) holds certbot's lock.
pub const BUSY: &str = "certbot is busy: another certbot run (a renewal or a renewal test) is in progress on this server. The list comes back when it finishes.";

/// certbot refused because another instance holds its lock.
pub fn is_busy(err: &str) -> bool {
    err.contains("Another instance of Certbot is already running")
}

impl SshSession {
    pub async fn certbot_status(&self) -> Result<CertbotStatus> {
        let (o, _) = self.exec_inspect(STATUS_SCRIPT).await?;
        let mut st =
            parse_status(&o.stdout, Utc::now()).ok_or_else(|| Error::NotFound("certbot is not installed on this server".into()))?;
        if st.error.as_deref().is_some_and(is_busy) {
            st.busy = true;
            st.error = Some(BUSY.into());
        }
        if st
            .error
            .as_deref()
            .is_some_and(|e| e.contains("root") || e.contains("Permission denied"))
        {
            return Err(Error::PermissionDenied(
                "reading certificates needs root: turn on sudo in this server's settings (with the sudo password if it asks for one), then reconnect".into(),
            ));
        }
        Ok(st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "@@version certbot 2.9.0
@@certificates
Saving debug log to /var/log/letsencrypt/letsencrypt.log

- - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -
Found the following certs:
  Certificate Name: shop.example.com
    Serial Number: 4a1f
    Key Type: ECDSA
    Domains: shop.example.com www.shop.example.com
    Expiry Date: 2027-01-10 12:00:00+00:00 (VALID: 56 days)
    Certificate Path: /etc/letsencrypt/live/shop.example.com/fullchain.pem
    Private Key Path: /etc/letsencrypt/live/shop.example.com/privkey.pem
  Certificate Name: old.example.com
    Domains: old.example.com
    Expiry Date: 2026-11-01 08:00:00+00:00 (INVALID: EXPIRED)
    Certificate Path: /etc/letsencrypt/live/old.example.com/fullchain.pem
    Private Key Path: /etc/letsencrypt/live/old.example.com/privkey.pem
- - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -
@@plugins
nginx
standalone
webroot
@@renew
timer=certbot.timer|ActiveState=active|LoadState=loaded|NextElapseUSecRealtime=Sun 2026-11-15 22:41:00 UTC|
";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_certificates_and_renewal() {
        let st = parse_status(OUT, now()).unwrap();
        assert_eq!(st.version.as_deref(), Some("2.9.0"));
        assert!(st.error.is_none());
        assert_eq!(st.certificates.len(), 2);
        let c = &st.certificates[0];
        assert_eq!(c.name, "shop.example.com");
        assert_eq!(c.domains, ["shop.example.com", "www.shop.example.com"]);
        assert!(c.valid);
        assert_eq!(c.days_left, Some(56));
        assert_eq!(c.status, "VALID: 56 days");
        assert_eq!(c.key_type.as_deref(), Some("ECDSA"));
        assert_eq!(
            c.private_key_path.as_deref(),
            Some("/etc/letsencrypt/live/shop.example.com/privkey.pem")
        );
        let old = &st.certificates[1];
        assert!(!old.valid);
        assert_eq!(old.days_left, Some(-14));
        assert_eq!(st.plugins, ["nginx", "standalone", "webroot"]);
        let ar = st.auto_renew.unwrap();
        assert_eq!(ar.via, "certbot.timer");
        assert!(ar.active);
        assert_eq!(ar.next_run.as_deref(), Some("Sun 2026-11-15 22:41:00 UTC"));
    }

    #[test]
    fn empty_missing_and_error() {
        let st = parse_status("@@version certbot 2.1.0\n@@certificates\nSaving debug log to /x\n\nNo certificates found.\n@@plugins\n@@renew\ncron=/etc/cron.d/certbot\n", now()).unwrap();
        assert!(st.certificates.is_empty() && st.error.is_none());
        assert_eq!(st.auto_renew.unwrap().via, "/etc/cron.d/certbot");
        assert!(parse_status("@@missing\n", now()).is_none());
        let st = parse_status("@@certificates\nThe following error was encountered:\n[Errno 13] Permission denied: '/var/log/letsencrypt/.certbot.lock'\nEither run as root, or set --config-dir\n", now()).unwrap();
        assert!(st.error.unwrap().contains("run as root"));
    }

    fn req(domains: &[&str], method: CertMethod) -> IssueRequest {
        IssueRequest {
            domains: domains.iter().map(|d| d.to_string()).collect(),
            email: Some("ops@example.com".into()),
            method,
            webroot: None,
            redirect: true,
            staging: false,
            agree_tos: true,
        }
    }

    #[test]
    fn builds_issue_commands() {
        assert_eq!(
            issue_command(&req(
                &["Shop.Example.com", "www.shop.example.com", "shop.example.com"],
                CertMethod::Nginx
            ))
            .unwrap(),
            "certbot --nginx -d shop.example.com -d www.shop.example.com --non-interactive --agree-tos -m ops@example.com --redirect"
        );
        let web = IssueRequest {
            webroot: Some("/srv/my site".into()),
            email: None,
            staging: true,
            ..req(&["example.org"], CertMethod::Webroot)
        };
        assert_eq!(
            issue_command(&web).unwrap(),
            "certbot certonly --webroot -w '/srv/my site' -d example.org --non-interactive --agree-tos --register-unsafely-without-email --staging"
        );
        assert!(
            issue_command(&req(&["example.org"], CertMethod::Webroot)).is_err(),
            "webroot needs a folder"
        );
        assert!(issue_command(&IssueRequest {
            agree_tos: false,
            ..req(&["example.org"], CertMethod::Nginx)
        })
        .is_err());
        assert!(issue_command(&req(&[], CertMethod::Nginx)).is_err());
        for bad in [
            "*.example.org",
            "localhost",
            "exa mple.org",
            "a.org; reboot",
            "-d.org",
            "$(id).org",
            "1.2.3.4",
        ] {
            assert!(issue_command(&req(&[bad], CertMethod::Standalone)).is_err(), "{bad}");
        }
        let bad_mail = IssueRequest {
            email: Some("x' -d evil.org".into()),
            ..req(&["example.org"], CertMethod::Nginx)
        };
        assert!(issue_command(&bad_mail).is_err());
    }

    #[test]
    fn a_running_certbot_is_reported_as_busy() {
        assert!(is_busy("Another instance of Certbot is already running."));
        assert!(!is_busy("The requested nginx plugin does not appear to be installed"));
    }

    #[test]
    fn builds_renew_and_delete_commands() {
        assert_eq!(
            renew_command(None, RenewMode::DryRun).unwrap(),
            "certbot renew --non-interactive --no-random-sleep-on-renew --dry-run"
        );
        assert_eq!(
            renew_command(Some("shop.example.com"), RenewMode::Force).unwrap(),
            "certbot renew --non-interactive --no-random-sleep-on-renew --cert-name shop.example.com --force-renewal"
        );
        assert_eq!(
            delete_command("old.example.com-0001").unwrap(),
            "certbot delete --non-interactive --cert-name old.example.com-0001"
        );
        assert!(delete_command("--all").is_err());
        assert!(delete_command("a b").is_err());
        assert!(usage_check_command("x/../../etc").is_err());
        assert!(usage_check_command("shop.example.com")
            .unwrap()
            .contains("grep -RlF -- /etc/letsencrypt/live/shop.example.com/ /etc/nginx"));
    }

    fn site(file: &str, names: &[&str]) -> crate::nginx::NginxSite {
        crate::nginx::NginxSite {
            file: file.into(),
            line: 1,
            server_names: names.iter().map(|n| n.to_string()).collect(),
            listens: vec!["80".into()],
            ssl: false,
            ssl_certificate: None,
            root: None,
            redirect: None,
            locations: vec![],
        }
    }

    #[test]
    fn certbot_may_only_change_the_names_own_site_file() {
        let cfg = |sites: Vec<crate::nginx::NginxSite>| crate::nginx::NginxConfig {
            test_ok: true,
            test_output: String::new(),
            files: vec![],
            sites,
            upstreams: vec![],
        };
        let d = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let ok = cfg(vec![
            site("/etc/nginx/sites-enabled/default", &["_"]),
            site(
                "/etc/nginx/sites-enabled/stats.example.com",
                &["stats.example.com", "www.stats.example.com"],
            ),
        ]);
        assert_eq!(
            nginx_install_targets(&ok, &d(&["Stats.example.com", "www.stats.example.com"])).unwrap(),
            ["/etc/nginx/sites-enabled/stats.example.com"]
        );
        // Not served by any site: certbot would pick the default server.
        let e = nginx_install_targets(&ok, &d(&["new.example.com"])).unwrap_err();
        assert!(e.contains("no nginx site serves new.example.com"), "{e}");
        // Named in the stock default file: never edited.
        let in_default = cfg(vec![site("/etc/nginx/sites-enabled/default", &["_", "shop.example.com"])]);
        assert!(nginx_install_targets(&in_default, &d(&["shop.example.com"]))
            .unwrap_err()
            .contains("nginx's own files"));
        let conf_d = cfg(vec![site("/etc/nginx/conf.d/default.conf", &["a.example.com"])]);
        assert!(nginx_install_targets(&conf_d, &d(&["a.example.com"])).is_err());
        assert!(
            is_nginx_default_file("/etc/nginx/nginx.conf") && !is_nginx_default_file("/etc/nginx/sites-available/defaults.example.com")
        );
    }

    #[test]
    fn certificate_only_changes_no_web_server_file() {
        let mut r = req(&["stats.example.com"], CertMethod::Nginx);
        r.redirect = true;
        let only = issue_command_with(&r, false).unwrap();
        assert!(only.starts_with("certbot certonly --nginx -d stats.example.com"), "{only}");
        assert!(!only.contains("redirect"), "{only}");
        assert!(issue_command(&r).unwrap().starts_with("certbot --nginx") && issue_command(&r).unwrap().contains("--redirect"));
    }

    /// The in-use check finds a site nginx loads from outside /etc/nginx (here a
    /// fake `nginx -T`), lists each file once, and ignores other certificates.
    #[cfg(unix)]
    #[test]
    fn usage_check_asks_nginx_for_every_loaded_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("nginx");
        std::fs::write(
            &fake,
            "#!/bin/sh\ncat <<'EOF'\n# configuration file /etc/nginx/nginx.conf:\nhttp { include /opt/sites/*.conf; }\n\n# configuration file /opt/sites/stats.conf:\nserver {\n    ssl_certificate /etc/letsencrypt/live/stats.example.com/fullchain.pem;\n    ssl_certificate_key /etc/letsencrypt/live/stats.example.com/privkey.pem;\n}\n# configuration file /opt/sites/other.conf:\n    ssl_certificate /etc/letsencrypt/live/stats.example.com-0001/fullchain.pem;\nEOF\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = |name: &str| {
            let o = std::process::Command::new("sh")
                .arg("-c")
                .arg(usage_check_command(name).unwrap())
                .env("PATH", format!("{}:/usr/bin:/bin", dir.path().display()))
                .output()
                .unwrap();
            assert!(o.status.success());
            String::from_utf8(o.stdout).unwrap()
        };
        assert_eq!(run("stats.example.com"), "/opt/sites/stats.conf\n");
        assert_eq!(run("stats.example.com-0001"), "/opt/sites/other.conf\n");
        assert_eq!(run("unused.example.com"), "");
    }

    // Runs the script meant for Linux servers through a local sh; Windows lacks the tools it reads.
    #[cfg(unix)]
    #[test]
    fn script_runs_under_sh() {
        let out = std::process::Command::new("sh").arg("-c").arg(STATUS_SCRIPT).output().unwrap();
        assert!(out.status.success());
    }
}
