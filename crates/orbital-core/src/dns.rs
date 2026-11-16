//! DNS: records at a DNS provider, and what the public resolvers answer.
//!
//! Provider accounts (Cloudflare for now) are rows in `dns_accounts`; their
//! API tokens live in the vault under `dns:<id>:token` and are read through
//! the gated store like server credentials. Checking a name doesn't need an
//! account: it asks public DNS-over-HTTPS resolvers, the same answer a
//! browser or Let's Encrypt would get.

use crate::error::{Error, Result};
use crate::secrets::Secret;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

pub const CLOUDFLARE_API: &str = "https://api.cloudflare.com/client/v4";

/// Public resolvers asked when checking a name: (label, JSON DoH endpoint).
/// By IP address, not name: the check must not depend on this Mac's own DNS
/// (which may cache the old answer, or not resolve `dns.google` at all, as on
/// some networks). Both certificates cover these IPs, so TLS still verifies.
pub const RESOLVERS: &[(&str, &str)] = &[
    ("Cloudflare 1.1.1.1", "https://1.1.1.1/dns-query"),
    ("Google 8.8.8.8", "https://8.8.8.8/resolve"),
];

/// reqwest's message plus its causes ("… (Connect) <- operation timed out"),
/// shortened to what a person can act on.
pub fn net_error(e: &reqwest::Error) -> String {
    let mut causes = Vec::new();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        causes.push(s.to_string());
        src = s.source();
    }
    let last = causes.last().cloned().unwrap_or_default().to_lowercase();
    if e.is_timeout() || last.contains("timed out") {
        "no answer (timed out); the network may block this resolver".into()
    } else if e.is_connect() {
        format!("couldn't connect: {}", causes.last().cloned().unwrap_or_else(|| e.to_string()))
    } else {
        causes.last().cloned().unwrap_or_else(|| e.to_string())
    }
}

pub fn token_key(account_id: &str) -> String {
    format!("dns:{account_id}:token")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Cloudflare,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Cloudflare => "cloudflare",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cloudflare" => Some(Self::Cloudflare),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Cloudflare => "Cloudflare",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsAccount {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsZone {
    pub id: String,
    pub name: String,
    /// `active`, `pending` (nameservers not switched yet), …
    pub status: String,
    /// Where the domain's registrar must point for the zone to be active.
    pub name_servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsRecord {
    pub id: String,
    /// Full name, e.g. `www.example.com`.
    pub name: String,
    /// As the provider names it; may be a type Orbital can't create (SRV, NS, …).
    #[serde(rename = "type")]
    pub rtype: String,
    pub content: String,
    /// Seconds; 1 means automatic.
    pub ttl: u32,
    pub proxied: bool,
    pub proxiable: bool,
    pub priority: Option<u16>,
    pub comment: Option<String>,
}

/// The record types Orbital creates and checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecordType {
    A,
    AAAA,
    CNAME,
    TXT,
    MX,
}

impl RecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            RecordType::A => "A",
            RecordType::AAAA => "AAAA",
            RecordType::CNAME => "CNAME",
            RecordType::TXT => "TXT",
            RecordType::MX => "MX",
        }
    }
    /// The number DNS uses for the type.
    fn code(self) -> u64 {
        match self {
            RecordType::A => 1,
            RecordType::CNAME => 5,
            RecordType::MX => 15,
            RecordType::TXT => 16,
            RecordType::AAAA => 28,
        }
    }
    fn proxiable(self) -> bool {
        matches!(self, RecordType::A | RecordType::AAAA | RecordType::CNAME)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordInput {
    #[serde(rename = "type")]
    pub rtype: RecordType,
    /// `@` for the zone itself, `www`, or a full name in the zone.
    pub name: String,
    pub content: String,
    /// 1 = automatic, else 60…86400 seconds.
    pub ttl: u32,
    #[serde(default)]
    pub proxied: bool,
    #[serde(default)]
    pub priority: Option<u16>,
}

// ---------- names and values ----------

fn valid_hostname(name: &str, wildcard: bool) -> bool {
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    name.split('.').enumerate().all(|(i, label)| {
        (wildcard && i == 0 && label == "*")
            || (!label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    })
}

/// A record name as typed (`@`, `www`, `www.example.com`) → the full name in `zone`.
pub fn fqdn(name: &str, zone: &str) -> Result<String> {
    let zone = zone.trim().trim_end_matches('.').to_ascii_lowercase();
    let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
    let full = if name.is_empty() || name == "@" || name == zone {
        zone.clone()
    } else if name.ends_with(&format!(".{zone}")) {
        name
    } else {
        format!("{name}.{zone}")
    };
    if !valid_hostname(&full, true) {
        return Err(Error::InvalidInput(format!("{full:?} isn't a valid DNS name")));
    }
    Ok(full)
}

/// Check a record before it is sent, and put it in the provider's form.
pub fn validate(input: &RecordInput, zone: &str) -> Result<RecordInput> {
    let name = fqdn(&input.name, zone)?;
    let raw = input.content.trim();
    let bad = |what: &str| Error::InvalidInput(format!("{what} for a {} record: {raw:?}", input.rtype.as_str()));
    let content = match input.rtype {
        RecordType::A => raw.parse::<Ipv4Addr>().map_err(|_| bad("not an IPv4 address"))?.to_string(),
        RecordType::AAAA => raw.parse::<Ipv6Addr>().map_err(|_| bad("not an IPv6 address"))?.to_string(),
        RecordType::CNAME | RecordType::MX => {
            let host = raw.trim_end_matches('.').to_ascii_lowercase();
            if !valid_hostname(&host, false) {
                return Err(bad("not a host name"));
            }
            if input.rtype == RecordType::CNAME && host == name {
                return Err(Error::InvalidInput("a CNAME can't point at itself".into()));
            }
            host
        }
        RecordType::TXT => {
            if raw.is_empty() || raw.len() > 2048 || raw.chars().any(|c| c.is_control()) {
                return Err(bad("empty, too long or with line breaks"));
            }
            raw.to_string()
        }
    };
    if input.ttl != 1 && !(60..=86400).contains(&input.ttl) {
        return Err(Error::InvalidInput("TTL must be automatic or between 60 and 86400 seconds".into()));
    }
    let priority = match input.rtype {
        RecordType::MX => Some(input.priority.unwrap_or(10)),
        _ => None,
    };
    Ok(RecordInput {
        rtype: input.rtype,
        name,
        content,
        ttl: input.ttl,
        proxied: input.proxied && input.rtype.proxiable(),
        priority,
    })
}

/// One value in comparable form: IPs canonical, names lowercase without the
/// final dot, TXT strings unquoted and joined.
pub fn normalize(rtype: RecordType, value: &str) -> String {
    let v = value.trim();
    match rtype {
        RecordType::A | RecordType::AAAA => v.parse::<IpAddr>().map(|ip| ip.to_string()).unwrap_or_else(|_| v.into()),
        RecordType::CNAME => v.trim_end_matches('.').to_ascii_lowercase(),
        RecordType::MX => {
            let mut parts = v.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(p), Some(h)) => format!("{p} {}", h.trim_end_matches('.').to_ascii_lowercase()),
                _ => v.trim_end_matches('.').to_ascii_lowercase(),
            }
        }
        RecordType::TXT => unquote_txt(v),
    }
}

/// `"abc" "def"` → `abcdef`; an unquoted value stays as it is.
fn unquote_txt(v: &str) -> String {
    if !v.starts_with('"') {
        return v.to_string();
    }
    let mut out = String::new();
    let mut inside = false;
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => inside = !inside,
            '\\' if inside => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            c if inside => out.push(c),
            _ => {}
        }
    }
    out
}

// ---------- checking what resolvers answer ----------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolverAnswer {
    pub resolver: String,
    /// `NOERROR`, `NXDOMAIN` (the name doesn't exist), `SERVFAIL`, …
    pub status: String,
    pub values: Vec<String>,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsCheck {
    pub name: String,
    #[serde(rename = "type")]
    pub rtype: RecordType,
    /// What the answer should contain; empty means "any answer".
    pub expected: Vec<String>,
    pub answers: Vec<ResolverAnswer>,
    /// Every resolver gave the expected answer.
    pub verified: bool,
    pub checked_at: String,
}

fn status_name(code: u64) -> String {
    match code {
        0 => "NOERROR".into(),
        1 => "FORMERR".into(),
        2 => "SERVFAIL".into(),
        3 => "NXDOMAIN".into(),
        5 => "REFUSED".into(),
        n => format!("RCODE {n}"),
    }
}

/// A JSON DoH answer → (status, values of `rtype`). CNAMEs on the way to an
/// A answer are skipped unless CNAME was asked for.
pub fn parse_doh(body: &[u8], rtype: RecordType) -> Result<(String, Vec<String>)> {
    let v: Value = serde_json::from_slice(body).map_err(|e| Error::Other(format!("unexpected DNS answer: {e}")))?;
    let status = status_name(v["Status"].as_u64().unwrap_or(2));
    let values = v["Answer"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|r| r["type"].as_u64() == Some(rtype.code()))
                .filter_map(|r| r["data"].as_str())
                .map(|d| normalize(rtype, d))
                .collect()
        })
        .unwrap_or_default();
    Ok((status, values))
}

/// `all`: every expected value must be answered (checking a record).
/// Otherwise any one is enough (checking a name points at a server with several addresses).
pub fn matches(rtype: RecordType, expected: &[String], values: &[String], all: bool) -> bool {
    if expected.is_empty() {
        return !values.is_empty();
    }
    let want: Vec<String> = expected.iter().map(|e| normalize(rtype, e)).collect();
    if all {
        want.iter().all(|w| values.contains(w))
    } else {
        want.iter().any(|w| values.contains(w))
    }
}

pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("Orbital/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Other(e.to_string()))
}

/// Ask each resolver for `name` and compare with `expected`.
pub async fn check(
    http: &reqwest::Client,
    resolvers: &[(String, String)],
    name: &str,
    rtype: RecordType,
    expected: &[String],
    all: bool,
) -> Result<DnsCheck> {
    let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
    if !valid_hostname(&name, false) {
        return Err(Error::InvalidInput(format!("{name:?} isn't a valid DNS name")));
    }
    let asks = resolvers.iter().map(|(label, url)| {
        let req = http
            .get(url)
            .query(&[("name", name.as_str()), ("type", rtype.as_str())])
            .header("accept", "application/dns-json");
        async move {
            let r = async {
                let resp = req.send().await.map_err(|e| Error::Other(net_error(&e)))?;
                if !resp.status().is_success() {
                    return Err(Error::Other(format!("HTTP {}", resp.status())));
                }
                let body = resp.bytes().await.map_err(|e| Error::Other(net_error(&e)))?;
                parse_doh(&body, rtype)
            }
            .await;
            match r {
                Ok((status, values)) => ResolverAnswer {
                    resolver: label.clone(),
                    ok: status == "NOERROR" && matches(rtype, expected, &values, all),
                    status,
                    values,
                    error: None,
                },
                Err(e) => ResolverAnswer {
                    resolver: label.clone(),
                    status: "ERROR".into(),
                    values: vec![],
                    ok: false,
                    error: Some(e.to_string()),
                },
            }
        }
    });
    // All resolvers at once; results keep the resolvers' order.
    let answers = futures_util::future::join_all(asks).await;
    Ok(DnsCheck {
        verified: !answers.is_empty() && answers.iter().all(|a| a.ok),
        name,
        rtype,
        expected: expected.iter().map(|e| normalize(rtype, e)).collect(),
        answers,
        checked_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Addresses that can be reached from the internet (no private, loopback,
/// link-local or shared carrier ranges).
pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || (o[0] == 100 && (64..128).contains(&o[1])))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            !(v.is_loopback() || v.is_unspecified() || v.is_multicast() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80)
        }
    }
}

// ---------- Cloudflare ----------

/// Zone and record ids go into URL paths: only the characters Cloudflare uses.
fn check_id(id: &str) -> Result<&str> {
    if !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(id)
    } else {
        Err(Error::InvalidInput(format!("invalid id {id:?}")))
    }
}

pub struct Cloudflare {
    http: reqwest::Client,
    base: String,
    token: Secret,
}

/// Cloudflare's error text from an API reply.
pub fn cloudflare_error(v: &Value) -> Option<String> {
    if v["success"].as_bool() == Some(true) {
        return None;
    }
    let msgs: Vec<String> = v["errors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    let m = e["message"].as_str().unwrap_or("error");
                    match e["code"].as_u64() {
                        Some(c) => format!("{m} ({c})"),
                        None => m.to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(if msgs.is_empty() {
        "Cloudflare refused the request".into()
    } else {
        msgs.join("; ")
    })
}

pub fn parse_zone(v: &Value) -> Option<DnsZone> {
    Some(DnsZone {
        id: v["id"].as_str()?.to_string(),
        name: v["name"].as_str()?.to_string(),
        status: v["status"].as_str().unwrap_or("unknown").to_string(),
        name_servers: v["name_servers"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    })
}

pub fn parse_record(v: &Value) -> Option<DnsRecord> {
    Some(DnsRecord {
        id: v["id"].as_str()?.to_string(),
        name: v["name"].as_str()?.to_string(),
        rtype: v["type"].as_str()?.to_string(),
        content: v["content"].as_str().unwrap_or_default().to_string(),
        ttl: v["ttl"].as_u64().unwrap_or(1) as u32,
        proxied: v["proxied"].as_bool().unwrap_or(false),
        proxiable: v["proxiable"].as_bool().unwrap_or(false),
        priority: v["priority"].as_u64().map(|p| p as u16),
        comment: v["comment"].as_str().filter(|s| !s.is_empty()).map(String::from),
    })
}

fn record_body(r: &RecordInput) -> Value {
    let mut b = json!({ "type": r.rtype.as_str(), "name": r.name, "content": r.content, "ttl": r.ttl });
    if r.rtype.proxiable() {
        b["proxied"] = json!(r.proxied);
    }
    if let Some(p) = r.priority {
        b["priority"] = json!(p);
    }
    b
}

impl Cloudflare {
    pub fn new(base: &str, token: Secret) -> Result<Self> {
        Ok(Self {
            http: http_client()?,
            base: base.trim_end_matches('/').to_string(),
            token,
        })
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.token.as_str());
        if let Some(b) = body {
            req = req.header("content-type", "application/json").body(b.to_string());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Other(format!("couldn't reach Cloudflare: {}", net_error(&e))))?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| Error::Other(e.to_string()))?;
        let v: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Other(format!("Cloudflare answered HTTP {status} without the usual reply")))?;
        if let Some(msg) = cloudflare_error(&v) {
            return Err(if status == 401 || status == 403 {
                Error::PermissionDenied(format!("Cloudflare refused the token: {msg}"))
            } else {
                Error::Other(format!("Cloudflare: {msg}"))
            });
        }
        Ok(v)
    }

    /// Every page of a list.
    async fn all(&self, path: &str, per_page: u32) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for page in 1..=50 {
            let sep = if path.contains('?') { '&' } else { '?' };
            let v = self
                .call(reqwest::Method::GET, &format!("{path}{sep}page={page}&per_page={per_page}"), None)
                .await?;
            out.extend(v["result"].as_array().cloned().unwrap_or_default());
            let pages = v["result_info"]["total_pages"].as_u64().unwrap_or(1);
            if page as u64 >= pages {
                break;
            }
        }
        Ok(out)
    }

    pub async fn zones(&self) -> Result<Vec<DnsZone>> {
        let mut z: Vec<_> = self.all("/zones", 50).await?.iter().filter_map(parse_zone).collect();
        z.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(z)
    }

    pub async fn records(&self, zone_id: &str) -> Result<Vec<DnsRecord>> {
        let mut r: Vec<_> = self
            .all(&format!("/zones/{}/dns_records", check_id(zone_id)?), 100)
            .await?
            .iter()
            .filter_map(parse_record)
            .collect();
        r.sort_by(|a, b| (a.name.len(), &a.name, &a.rtype).cmp(&(b.name.len(), &b.name, &b.rtype)));
        Ok(r)
    }

    pub async fn create(&self, zone_id: &str, r: &RecordInput) -> Result<DnsRecord> {
        let v = self
            .call(
                reqwest::Method::POST,
                &format!("/zones/{}/dns_records", check_id(zone_id)?),
                Some(record_body(r)),
            )
            .await?;
        parse_record(&v["result"]).ok_or_else(|| Error::Other("Cloudflare didn't return the new record".into()))
    }

    pub async fn update(&self, zone_id: &str, record_id: &str, r: &RecordInput) -> Result<DnsRecord> {
        let v = self
            .call(
                reqwest::Method::PUT,
                &format!("/zones/{}/dns_records/{}", check_id(zone_id)?, check_id(record_id)?),
                Some(record_body(r)),
            )
            .await?;
        parse_record(&v["result"]).ok_or_else(|| Error::Other("Cloudflare didn't return the record".into()))
    }

    pub async fn delete(&self, zone_id: &str, record_id: &str) -> Result<()> {
        self.call(
            reqwest::Method::DELETE,
            &format!("/zones/{}/dns_records/{}", check_id(zone_id)?, check_id(record_id)?),
            None,
        )
        .await?;
        Ok(())
    }
}

// ---------- saved accounts ----------

use crate::db::Database;

fn row_account(r: &rusqlite::Row) -> rusqlite::Result<DnsAccount> {
    let kind: String = r.get("kind")?;
    Ok(DnsAccount {
        id: r.get("id")?,
        name: r.get("name")?,
        kind: ProviderKind::parse(&kind).unwrap_or(ProviderKind::Cloudflare),
        created_at: r.get("created_at")?,
    })
}

pub fn list_accounts(db: &Database) -> Result<Vec<DnsAccount>> {
    let conn = db.conn();
    let mut st = conn.prepare("SELECT * FROM dns_accounts ORDER BY name COLLATE NOCASE")?;
    let rows = st.query_map([], row_account)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_account(db: &Database, id: &str) -> Result<DnsAccount> {
    use rusqlite::OptionalExtension;
    db.conn()
        .query_row("SELECT * FROM dns_accounts WHERE id = ?1", [id], row_account)
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("DNS account {id}")))
}

pub fn insert_account(db: &Database, name: &str, kind: ProviderKind) -> Result<DnsAccount> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(Error::InvalidInput("give the account a name".into()));
    }
    let a = DnsAccount {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        kind,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    db.conn().execute(
        "INSERT INTO dns_accounts (id, name, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![a.id, a.name, a.kind.as_str(), a.created_at],
    )?;
    Ok(a)
}

pub fn delete_account(db: &Database, id: &str) -> Result<()> {
    db.conn().execute("DELETE FROM dns_accounts WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(t: RecordType, name: &str, content: &str) -> RecordInput {
        RecordInput {
            rtype: t,
            name: name.into(),
            content: content.into(),
            ttl: 1,
            proxied: false,
            priority: None,
        }
    }

    #[test]
    fn names_in_a_zone() {
        assert_eq!(fqdn("@", "example.com").unwrap(), "example.com");
        assert_eq!(fqdn("", "example.com.").unwrap(), "example.com");
        assert_eq!(fqdn("WWW", "example.com").unwrap(), "www.example.com");
        assert_eq!(fqdn("api.example.com.", "example.com").unwrap(), "api.example.com");
        assert_eq!(fqdn("*", "example.com").unwrap(), "*.example.com");
        assert_eq!(fqdn("_dmarc", "example.com").unwrap(), "_dmarc.example.com");
        assert!(fqdn("a b", "example.com").is_err());
        assert!(fqdn("-x", "example.com").is_err());
        assert!(fqdn("a.*", "example.com").is_err());
        assert!(fqdn("x/y", "example.com").is_err());
    }

    #[test]
    fn validates_records() {
        let a = validate(&input(RecordType::A, "www", " 203.0.113.10 "), "example.com").unwrap();
        assert_eq!((a.name.as_str(), a.content.as_str()), ("www.example.com", "203.0.113.10"));
        assert!(validate(&input(RecordType::A, "www", "2001:db8::1"), "example.com").is_err());
        let v6 = validate(&input(RecordType::AAAA, "@", "2001:DB8:0::1"), "example.com").unwrap();
        assert_eq!(v6.content, "2001:db8::1");
        let c = validate(&input(RecordType::CNAME, "blog", "Hosting.Example.net."), "example.com").unwrap();
        assert_eq!(c.content, "hosting.example.net");
        assert!(validate(&input(RecordType::CNAME, "blog", "blog.example.com"), "example.com").is_err());
        let mx = validate(&input(RecordType::MX, "@", "mail.example.com"), "example.com").unwrap();
        assert_eq!(mx.priority, Some(10));
        assert!(validate(&input(RecordType::TXT, "@", "a\nb"), "example.com").is_err());
        let mut t = input(RecordType::TXT, "@", "v=spf1 -all");
        t.proxied = true;
        t.ttl = 30;
        assert!(validate(&t, "example.com").is_err());
        t.ttl = 300;
        assert!(!validate(&t, "example.com").unwrap().proxied, "TXT can't be proxied");
    }

    #[test]
    fn normalizes_answers() {
        assert_eq!(normalize(RecordType::CNAME, "Host.Example.NET."), "host.example.net");
        assert_eq!(normalize(RecordType::MX, "10 Mail.example.com."), "10 mail.example.com");
        assert_eq!(normalize(RecordType::TXT, "\"v=spf1 \" \"-all\""), "v=spf1 -all");
        assert_eq!(normalize(RecordType::TXT, "\"say \\\"hi\\\"\""), "say \"hi\"");
        assert_eq!(normalize(RecordType::AAAA, "2001:0db8::0001"), "2001:db8::1");
    }

    #[test]
    fn parses_doh_answers() {
        let body = br#"{"Status":0,"Answer":[
            {"name":"www.example.com.","type":5,"TTL":300,"data":"example.com."},
            {"name":"example.com.","type":1,"TTL":300,"data":"203.0.113.10"},
            {"name":"example.com.","type":1,"TTL":300,"data":"203.0.113.11"}]}"#;
        let (status, v) = parse_doh(body, RecordType::A).unwrap();
        assert_eq!(status, "NOERROR");
        assert_eq!(v, vec!["203.0.113.10", "203.0.113.11"]);
        let (_, c) = parse_doh(body, RecordType::CNAME).unwrap();
        assert_eq!(c, vec!["example.com"]);
        let (status, v) = parse_doh(br#"{"Status":3}"#, RecordType::A).unwrap();
        assert_eq!((status.as_str(), v.len()), ("NXDOMAIN", 0));
        assert!(parse_doh(b"<html>", RecordType::A).is_err());
    }

    #[test]
    fn matching() {
        let got = vec!["203.0.113.10".to_string(), "203.0.113.11".to_string()];
        assert!(matches(RecordType::A, &["203.0.113.10".into()], &got, true));
        assert!(!matches(RecordType::A, &["203.0.113.10".into(), "198.51.100.1".into()], &got, true));
        assert!(matches(RecordType::A, &["198.51.100.1".into(), "203.0.113.11".into()], &got, false));
        assert!(matches(RecordType::A, &[], &got, true));
        assert!(!matches(RecordType::A, &[], &[], true));
    }

    #[test]
    fn public_addresses() {
        for ip in [
            "10.0.0.1",
            "192.168.1.5",
            "172.16.0.1",
            "127.0.0.1",
            "100.64.0.1",
            "169.254.1.1",
            "fd00::1",
            "fe80::1",
            "::1",
        ] {
            assert!(!is_public(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["203.0.113.10", "8.8.8.8", "2a01:4f8::1"] {
            assert!(is_public(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn parses_cloudflare_replies() {
        let ok = json!({"success":true,"errors":[],"result":{"id":"abc123","name":"www.example.com","type":"A","content":"203.0.113.10","ttl":1,"proxied":true,"proxiable":true}});
        assert_eq!(cloudflare_error(&ok), None);
        let r = parse_record(&ok["result"]).unwrap();
        assert_eq!((r.rtype.as_str(), r.proxied, r.priority), ("A", true, None));
        let bad = json!({"success":false,"errors":[{"code":81057,"message":"Record already exists."}]});
        assert_eq!(cloudflare_error(&bad).unwrap(), "Record already exists. (81057)");
        let z = parse_zone(&json!({"id":"z1","name":"example.com","status":"pending","name_servers":["ada.ns.cloudflare.com"]})).unwrap();
        assert_eq!(z.name_servers, vec!["ada.ns.cloudflare.com"]);
        assert!(check_id("../zones").is_err());
        assert!(check_id("023e105f4ecef8ad9ca31a8372d0c353").is_ok());
    }

    #[test]
    fn saved_accounts() {
        let db = Database::in_memory().unwrap();
        assert!(insert_account(&db, "  ", ProviderKind::Cloudflare).is_err());
        let a = insert_account(&db, "Personal", ProviderKind::Cloudflare).unwrap();
        assert_eq!(list_accounts(&db).unwrap(), vec![a.clone()]);
        assert_eq!(get_account(&db, &a.id).unwrap().name, "Personal");
        delete_account(&db, &a.id).unwrap();
        assert!(matches!(get_account(&db, &a.id), Err(Error::NotFound(_))));
    }

    #[test]
    fn record_bodies() {
        let mut r = validate(&input(RecordType::MX, "@", "mail.example.com"), "example.com").unwrap();
        r.priority = Some(5);
        let b = record_body(&r);
        assert_eq!(b["priority"], 5);
        assert!(b.get("proxied").is_none());
        let a = validate(&input(RecordType::A, "x", "203.0.113.1"), "example.com").unwrap();
        assert_eq!(record_body(&a)["proxied"], false);
    }
}

#[cfg(test)]
mod real {
    use super::*;

    /// Against the real public resolvers: `cargo test -p orbital-core real_resolvers -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_resolvers() {
        let http = http_client().unwrap();
        for (label, url) in RESOLVERS {
            let r = http
                .get(*url)
                .query(&[("name", "example.com"), ("type", "A")])
                .header("accept", "application/dns-json")
                .send()
                .await;
            match r {
                Ok(resp) => println!(
                    "{label}: HTTP {} {:?}",
                    resp.status(),
                    resp.bytes()
                        .await
                        .map(|b| String::from_utf8_lossy(&b).chars().take(120).collect::<String>())
                ),
                Err(e) => {
                    let mut msg = e.to_string();
                    let mut src = std::error::Error::source(&e);
                    while let Some(s) = src {
                        msg.push_str(&format!(" <- {s}"));
                        src = s.source();
                    }
                    println!("{label}: ERROR {msg}");
                }
            }
        }
        let resolvers: Vec<(String, String)> = RESOLVERS.iter().map(|(l, u)| (l.to_string(), u.to_string())).collect();
        for name in ["example.com", "devlabs.life"] {
            let c = check(&http, &resolvers, name, RecordType::A, &[], true).await.unwrap();
            println!(
                "check {name}: verified={} {:?}",
                c.verified,
                c.answers.iter().map(|a| (&a.status, &a.values, &a.error)).collect::<Vec<_>>()
            );
        }
    }
}
