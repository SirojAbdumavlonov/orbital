//! Amazon S3 and S3-compatible storage (Backblaze B2, Cloudflare R2, Wasabi,
//! DigitalOcean Spaces, Google Cloud Storage with HMAC keys, MinIO, …).
//! Requests are signed with AWS Signature Version 4.
//!
//! The UI's `/` lists buckets; `/bucket/a/b.txt` is the object `a/b.txt`.
//! S3 has no real folders: a "folder" is a key prefix, and an empty one is
//! kept with a zero-byte `prefix/` object, as the AWS console does.

use super::entry;
use crate::error::{Error, Result};
use crate::profiles::Profile;
use crate::secrets::Secret;
use crate::sftp::FileEntry;
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::{Method, StatusCode};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');
const UNSIGNED: &str = "UNSIGNED-PAYLOAD";

fn enc(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

/// What a request needs to be signed.
pub(crate) struct SignInput<'a> {
    pub method: &'a str,
    pub host: &'a str,
    /// Already encoded, starting with `/`.
    pub path: &'a str,
    pub query: &'a [(String, String)],
    /// Lower-case names; `host`, `x-amz-date` and `x-amz-content-sha256` are added.
    pub headers: &'a [(String, String)],
    pub payload_hash: &'a str,
    pub region: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    /// `YYYYMMDDTHHMMSSZ`
    pub amz_date: &'a str,
}

pub(crate) fn canonical_query(q: &[(String, String)]) -> String {
    let mut v: Vec<(String, String)> = q.iter().map(|(k, v)| (enc(k), enc(v))).collect();
    v.sort();
    v.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

/// The headers to send: the given ones plus the signature's own.
pub(crate) fn sign(i: &SignInput) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = i.headers.to_vec();
    headers.push(("host".into(), i.host.into()));
    headers.push(("x-amz-content-sha256".into(), i.payload_hash.into()));
    headers.push(("x-amz-date".into(), i.amz_date.into()));
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed: String = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        i.method,
        i.path,
        canonical_query(i.query),
        canonical_headers,
        signed,
        i.payload_hash
    );
    let date = &i.amz_date[..8];
    let scope = format!("{date}/{}/s3/aws4_request", i.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        i.amz_date,
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let k = hmac(format!("AWS4{}", i.secret_key).as_bytes(), date);
    let k = hmac(&k, i.region);
    let k = hmac(&k, "s3");
    let k = hmac(&k, "aws4_request");
    let signature = hex(&hmac(&k, &to_sign));
    headers.retain(|(k, _)| k != "host");
    headers.push((
        "authorization".into(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
            i.access_key
        ),
    ));
    headers
}

/// Split a UI path into bucket and key.
pub(crate) fn split(path: &str) -> (Option<&str>, &str) {
    let p = path.trim_start_matches('/');
    if p.is_empty() {
        return (None, "");
    }
    match p.split_once('/') {
        Some((b, k)) => (Some(b), k),
        None => (Some(p), ""),
    }
}

fn is(n: &roxmltree::Node, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

fn child<'a>(n: &roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.children().find(|c| is(c, name)).and_then(|c| c.text())
}

fn iso_secs(t: Option<&str>) -> Option<u32> {
    t.and_then(|t| chrono::DateTime::parse_from_rfc3339(t.trim()).ok())
        .map(|d| d.timestamp().max(0) as u32)
}

fn doc(xml: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml).map_err(|e| Error::Other(format!("the storage server sent an unreadable answer: {e}")))
}

/// `(name, created)` for each bucket.
pub(crate) fn parse_buckets(xml: &str) -> Result<Vec<(String, Option<u32>)>> {
    let d = doc(xml)?;
    Ok(d.descendants()
        .filter(|n| is(n, "Bucket"))
        .filter_map(|b| Some((child(&b, "Name")?.to_string(), iso_secs(child(&b, "CreationDate")))))
        .collect())
}

#[derive(Debug, Default)]
pub(crate) struct Listing {
    /// `(key, size, mtime)`
    pub objects: Vec<(String, u64, Option<u32>)>,
    pub prefixes: Vec<String>,
    pub next: Option<String>,
}

pub(crate) fn parse_listing(xml: &str) -> Result<Listing> {
    let d = doc(xml)?;
    let root = d.root_element();
    let mut l = Listing::default();
    for n in root.children() {
        if is(&n, "Contents") {
            if let Some(k) = child(&n, "Key") {
                let size = child(&n, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
                l.objects.push((k.to_string(), size, iso_secs(child(&n, "LastModified"))));
            }
        } else if is(&n, "CommonPrefixes") {
            if let Some(p) = child(&n, "Prefix") {
                l.prefixes.push(p.to_string());
            }
        }
    }
    if child(&root, "IsTruncated") == Some("true") {
        l.next = child(&root, "NextContinuationToken").map(str::to_string);
    }
    Ok(l)
}

/// `(code, message)` from an S3 error answer.
pub(crate) fn parse_error(xml: &str) -> Option<(String, String)> {
    let d = roxmltree::Document::parse(xml).ok()?;
    let e = d.descendants().find(|n| is(n, "Error"))?;
    Some((child(&e, "Code")?.to_string(), child(&e, "Message").unwrap_or("").to_string()))
}

pub struct S3Client {
    http: reqwest::Client,
    https: bool,
    host: String,
    port: u16,
    region: String,
    path_style: bool,
    access_key: String,
    secret_key: Secret,
    /// Buckets found to live in another region than the profile's.
    regions: Mutex<HashMap<String, String>>,
    profile: Profile,
}

struct Req<'a> {
    method: Method,
    bucket: Option<&'a str>,
    key: &'a str,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Option<reqwest::Body>,
    /// Retry once with the bucket's region if the server names another one.
    retry: bool,
}

impl<'a> Req<'a> {
    fn new(method: Method, bucket: Option<&'a str>, key: &'a str) -> Self {
        Self {
            method,
            bucket,
            key,
            query: vec![],
            headers: vec![],
            body: None,
            retry: true,
        }
    }
    fn q(mut self, k: &str, v: &str) -> Self {
        self.query.push((k.into(), v.into()));
        self
    }
    fn h(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}

impl S3Client {
    pub async fn connect(p: &Profile, secret: Option<Secret>) -> Result<Self> {
        let secret_key = secret.ok_or_else(|| Error::CredentialsRequired {
            what: "Secret access key".into(),
            user: p.username.clone(),
            host: p.host.clone(),
            profile_id: p.id.clone(),
        })?;
        let host = p
            .host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string();
        let local = host == "localhost" || host.starts_with("127.") || host.ends_with(".local");
        let https = p.port == 443 || !(p.port == 80 || local);
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("Orbital/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        let c = Self {
            http,
            https,
            port: p.port,
            region: p
                .region
                .clone()
                .filter(|r| !r.trim().is_empty())
                .unwrap_or_else(|| "us-east-1".into()),
            path_style: p.path_style || local,
            access_key: p.username.clone(),
            secret_key,
            regions: Mutex::new(HashMap::new()),
            host,
            profile: p.clone(),
        };
        // Check the keys: list the chosen bucket, or the buckets.
        let start = p
            .initial_path
            .as_deref()
            .map(|s| format!("/{}", s.trim().trim_matches('/')))
            .unwrap_or_else(|| "/".into());
        c.list(if start == "/" { "/" } else { &start }).await.map_err(|e| match e {
            Error::PermissionDenied(_) if start == "/" => {
                Error::Other("these keys may not list buckets; put the bucket name in the profile's Bucket field".into())
            }
            e => e,
        })?;
        Ok(c)
    }

    fn region_for(&self, bucket: Option<&str>) -> String {
        bucket
            .and_then(|b| self.regions.lock().ok()?.get(b).cloned())
            .unwrap_or_else(|| self.region.clone())
    }

    /// AWS's own endpoint follows the region; other services keep theirs.
    fn endpoint(&self, region: &str) -> String {
        let aws = self.host == "s3.amazonaws.com" || (self.host.starts_with("s3.") && self.host.ends_with(".amazonaws.com"));
        if aws && region != "us-east-1" {
            format!("s3.{region}.amazonaws.com")
        } else {
            self.host.clone()
        }
    }

    async fn send(&self, mut r: Req<'_>) -> Result<reqwest::Response> {
        let region = self.region_for(r.bucket);
        let endpoint = self.endpoint(&region);
        let key_path = format!("/{}", r.key.split('/').map(enc).collect::<Vec<_>>().join("/"));
        // Virtual-hosted addressing (bucket.endpoint) unless asked otherwise, or
        // when the bucket name has dots (it wouldn't match the certificate).
        let (host, path) = match r.bucket {
            Some(b) if !self.path_style && !b.contains('.') => (format!("{b}.{endpoint}"), key_path),
            Some(b) => (
                endpoint.clone(),
                if r.key.is_empty() {
                    format!("/{}", enc(b))
                } else {
                    format!("/{}{key_path}", enc(b))
                },
            ),
            None => (endpoint.clone(), "/".to_string()),
        };
        let default_port = if self.https { 443 } else { 80 };
        let host_header = if self.port == default_port {
            host.clone()
        } else {
            format!("{host}:{}", self.port)
        };
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let mut extra: Vec<(String, String)> = r.headers.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect();
        extra.sort();
        let signed = sign(&SignInput {
            method: r.method.as_str(),
            host: &host_header,
            path: &path,
            query: &r.query,
            headers: &extra,
            payload_hash: UNSIGNED,
            region: &region,
            access_key: &self.access_key,
            secret_key: self.secret_key.as_str(),
            amz_date: &amz_date,
        });
        let qs = canonical_query(&r.query);
        let url = format!(
            "{}://{host_header}{path}{}{qs}",
            if self.https { "https" } else { "http" },
            if qs.is_empty() { "" } else { "?" }
        );
        let mut req = self.http.request(r.method.clone(), url);
        for (k, v) in &signed {
            req = req.header(k, v);
        }
        if let Some(b) = r.body.take() {
            req = req.body(b);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                Error::Ssh(format!("couldn't reach {host}: {e}"))
            } else {
                Error::Other(format!("S3: {e}"))
            }
        })?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let moved = resp
            .headers()
            .get("x-amz-bucket-region")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = resp.text().await.unwrap_or_default();
        if let (Some(b), Some(reg), true) = (r.bucket, &moved, r.retry) {
            if *reg != region {
                if let Ok(mut m) = self.regions.lock() {
                    m.insert(b.to_string(), reg.clone());
                }
                // Bodies are only sent by PUT, which is never retried (see `put`).
                return Box::pin(self.send(Req { retry: false, ..r })).await;
            }
        }
        let (code, msg) = parse_error(&text).unwrap_or_else(|| (status.as_str().into(), String::new()));
        let what = match (r.bucket, r.key) {
            (Some(b), "") => b.to_string(),
            (Some(b), k) => format!("{b}/{k}"),
            (None, _) => self.host.clone(),
        };
        Err(match (status, code.as_str()) {
            (_, "InvalidAccessKeyId" | "SignatureDoesNotMatch" | "InvalidToken" | "ExpiredToken") => Error::AuthFailed {
                user: self.access_key.clone(),
                host: self.host.clone(),
                profile_id: self.profile.id.clone(),
            },
            (StatusCode::FORBIDDEN, _) => {
                Error::PermissionDenied(format!("{what}{}", if msg.is_empty() { String::new() } else { format!(": {msg}") }))
            }
            (StatusCode::NOT_FOUND, _) => Error::NotFound(what),
            (_, "BucketNotEmpty") => Error::InvalidInput(format!("the bucket {what} isn't empty; delete what's in it first")),
            _ => Error::Other(format!("{code}: {}", if msg.is_empty() { &what } else { &msg })),
        })
    }

    async fn list_page(&self, bucket: &str, prefix: &str, delimiter: bool, token: Option<&str>) -> Result<Listing> {
        let mut r = Req::new(Method::GET, Some(bucket), "")
            .q("list-type", "2")
            .q("prefix", prefix)
            .q("max-keys", "1000");
        if delimiter {
            r = r.q("delimiter", "/");
        }
        if let Some(t) = token {
            r = r.q("continuation-token", t);
        }
        let text = self.send(r).await?.text().await.map_err(|e| Error::Other(e.to_string()))?;
        parse_listing(&text)
    }

    /// Every key under `prefix` (no delimiter), for folder deletes and renames.
    async fn all_keys(&self, bucket: &str, prefix: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut token = None;
        loop {
            let page = self.list_page(bucket, prefix, false, token.as_deref()).await?;
            out.extend(page.objects.into_iter().map(|o| o.0));
            match page.next {
                Some(t) => token = Some(t),
                None => return Ok(out),
            }
        }
    }

    pub async fn list(&self, path: &str) -> Result<Vec<FileEntry>> {
        let (bucket, key) = split(path);
        let Some(bucket) = bucket else {
            let text = self
                .send(Req::new(Method::GET, None, ""))
                .await?
                .text()
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
            return Ok(parse_buckets(&text)?.into_iter().map(|(n, t)| entry("/", &n, true, 0, t)).collect());
        };
        let prefix = if key.is_empty() { String::new() } else { format!("{key}/") };
        let mut out = Vec::new();
        let mut token = None;
        loop {
            let page = self.list_page(bucket, &prefix, true, token.as_deref()).await?;
            for p in &page.prefixes {
                let name = p[prefix.len()..].trim_end_matches('/');
                if !name.is_empty() {
                    out.push(entry(path, name, true, 0, None));
                }
            }
            for (k, size, t) in &page.objects {
                let name = &k[prefix.len().min(k.len())..];
                // The folder's own marker object.
                if !name.is_empty() && !name.contains('/') {
                    out.push(entry(path, name, false, *size, *t));
                }
            }
            if out.len() > 20_000 {
                break;
            }
            match page.next {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        Ok(out)
    }

    fn object(path: &str) -> Result<(&str, &str)> {
        match split(path) {
            (Some(b), k) if !k.is_empty() => Ok((b, k)),
            _ => Err(Error::InvalidInput("pick a file inside a bucket".into())),
        }
    }

    pub async fn read(&self, path: &str, limit: u64) -> Result<Vec<u8>> {
        let (b, k) = Self::object(path)?;
        let resp = self.send(Req::new(Method::GET, Some(b), k)).await?;
        if resp.content_length().is_some_and(|n| n > limit) {
            return Err(Error::InvalidInput("the file is too large to open in the editor".into()));
        }
        let mut out = Vec::new();
        let mut body = resp.bytes_stream();
        while let Some(chunk) = body.next().await {
            out.extend_from_slice(&chunk.map_err(|e| Error::Other(e.to_string()))?);
            if out.len() as u64 > limit {
                return Err(Error::InvalidInput("the file is too large to open in the editor".into()));
            }
        }
        Ok(out)
    }

    /// PUT is sent to the bucket's known region (a listing found it first), so it isn't retried.
    async fn put(&self, b: &str, k: &str, body: reqwest::Body, len: u64) -> Result<()> {
        let mut r = Req::new(Method::PUT, Some(b), k).h("content-length", len.to_string());
        r.body = Some(body);
        r.retry = false;
        self.send(r).await.map(|_| ())
    }

    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<()> {
        let (b, k) = Self::object(path)?;
        let len = data.len() as u64;
        self.put(b, k, data.into(), len).await
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        match split(path) {
            (Some(b), "") => {
                let mut r = Req::new(Method::PUT, Some(b), "");
                if self.region != "us-east-1" {
                    let body = format!(
                        r#"<CreateBucketConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>"#,
                        self.region
                    );
                    r = r.h("content-length", body.len().to_string());
                    r.body = Some(body.into());
                }
                r.retry = false;
                self.send(r).await.map(|_| ())
            }
            (Some(b), k) => self.put(b, &format!("{k}/"), Vec::new().into(), 0).await,
            (None, _) => Err(Error::InvalidInput("invalid folder".into())),
        }
    }

    pub async fn remove(&self, path: &str, recursive: bool) -> Result<()> {
        match split(path) {
            (Some(b), "") => self.send(Req::new(Method::DELETE, Some(b), "")).await.map(|_| ()),
            (Some(b), k) => {
                if recursive {
                    for key in self.all_keys(b, &format!("{k}/")).await? {
                        self.send(Req::new(Method::DELETE, Some(b), &key)).await?;
                    }
                }
                // The object itself, or the folder's marker (deleting a missing key is fine in S3).
                let key = if recursive { format!("{k}/") } else { k.to_string() };
                self.send(Req::new(Method::DELETE, Some(b), &key)).await.map(|_| ())
            }
            (None, _) => Err(Error::InvalidInput("invalid path".into())),
        }
    }

    async fn copy(&self, from_b: &str, from_k: &str, to_b: &str, to_k: &str) -> Result<()> {
        let source = format!("/{from_b}/{}", from_k.split('/').map(enc).collect::<Vec<_>>().join("/"));
        let r = Req::new(Method::PUT, Some(to_b), to_k)
            .h("x-amz-copy-source", source)
            .h("content-length", "0");
        let resp = self.send(r).await?;
        // A copy can fail after "200 OK"; the error is then in the body.
        let text = resp.text().await.unwrap_or_default();
        if let Some((code, msg)) = parse_error(&text) {
            return Err(Error::Other(format!("{code}: {msg}")));
        }
        Ok(())
    }

    /// S3 can't rename: copy, then delete the original.
    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let ((Some(fb), fk), (Some(tb), tk)) = (split(from), split(to)) else {
            return Err(Error::InvalidInput("buckets can't be renamed".into()));
        };
        if fk.is_empty() || tk.is_empty() {
            return Err(Error::InvalidInput("buckets can't be renamed".into()));
        }
        let folder = self.all_keys(fb, &format!("{fk}/")).await?;
        if folder.is_empty() {
            self.copy(fb, fk, tb, tk).await?;
            return self.send(Req::new(Method::DELETE, Some(fb), fk)).await.map(|_| ());
        }
        for key in &folder {
            let rest = &key[fk.len()..];
            self.copy(fb, key, tb, &format!("{tk}{rest}")).await?;
        }
        for key in &folder {
            self.send(Req::new(Method::DELETE, Some(fb), key)).await?;
        }
        Ok(())
    }

    pub async fn download(&self, remote: &str, local: &Path) -> Result<u64> {
        let (b, k) = Self::object(remote)?;
        let resp = self.send(Req::new(Method::GET, Some(b), k)).await?;
        let mut file = tokio::fs::File::create(local).await?;
        let mut n = 0u64;
        let mut body = resp.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| Error::Other(e.to_string()))?;
            file.write_all(&chunk).await?;
            n += chunk.len() as u64;
        }
        file.flush().await?;
        Ok(n)
    }

    pub async fn upload(&self, local: &Path, remote: &str) -> Result<u64> {
        let (b, k) = Self::object(remote)?;
        let file = tokio::fs::File::open(local).await?;
        let len = file.metadata().await?.len();
        if len > 5 * 1024 * 1024 * 1024 {
            return Err(Error::InvalidInput(
                "files over 5 GB can't be uploaded to S3 in one piece yet".into(),
            ));
        }
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        self.put(b, k, body, len).await?;
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "GET Object" example from the AWS Signature Version 4 documentation.
    #[test]
    fn signs_the_aws_example() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let h = sign(&SignInput {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            query: &[],
            headers: &[("range".into(), "bytes=0-9".into())],
            payload_hash: empty,
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            amz_date: "20130524T000000Z",
        });
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert!(!h.iter().any(|(k, _)| k == "host"));
    }

    /// The "GET Bucket (List Objects)" example: query parameters are sorted and encoded.
    #[test]
    fn signs_a_query() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let h = sign(&SignInput {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/",
            query: &[("prefix".into(), "J".into()), ("max-keys".into(), "2".into())],
            headers: &[],
            payload_hash: empty,
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            amz_date: "20130524T000000Z",
        });
        let auth = &h.iter().find(|(k, _)| k == "authorization").unwrap().1;
        assert!(
            auth.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"),
            "{auth}"
        );
        assert_eq!(
            canonical_query(&[("b".into(), "a b/c".into()), ("a".into(), "".into())]),
            "a=&b=a%20b%2Fc"
        );
    }

    #[test]
    fn splits_paths() {
        assert_eq!(split("/"), (None, ""));
        assert_eq!(split("/b"), (Some("b"), ""));
        assert_eq!(split("/b/a/c.txt"), (Some("b"), "a/c.txt"));
    }

    #[test]
    fn parses_listings_and_errors() {
        let buckets = r#"<ListAllMyBucketsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Owner><ID>x</ID></Owner><Buckets><Bucket><Name>logs</Name><CreationDate>2026-01-02T03:04:05.000Z</CreationDate></Bucket><Bucket><Name>site.example.com</Name><CreationDate>2026-01-02T03:04:05.000Z</CreationDate></Bucket></Buckets></ListAllMyBucketsResult>"#;
        let b = parse_buckets(buckets).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].0, "site.example.com");
        assert_eq!(b[0].1, Some(1_767_323_045));

        let list = r#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>logs</Name><Prefix>app/</Prefix><KeyCount>3</KeyCount><IsTruncated>true</IsTruncated><NextContinuationToken>abc=</NextContinuationToken>
<Contents><Key>app/</Key><Size>0</Size></Contents>
<Contents><Key>app/a.log</Key><LastModified>2026-01-02T03:04:05.000Z</LastModified><Size>42</Size></Contents>
<CommonPrefixes><Prefix>app/old/</Prefix></CommonPrefixes></ListBucketResult>"#;
        let l = parse_listing(list).unwrap();
        assert_eq!(l.objects.len(), 2);
        assert_eq!(l.objects[1], ("app/a.log".into(), 42, Some(1_767_323_045)));
        assert_eq!(l.prefixes, vec!["app/old/"]);
        assert_eq!(l.next.as_deref(), Some("abc="));

        let err = r#"<?xml version="1.0" encoding="UTF-8"?><Error><Code>SignatureDoesNotMatch</Code><Message>The request signature we calculated does not match</Message></Error>"#;
        assert_eq!(parse_error(err).unwrap().0, "SignatureDoesNotMatch");
        assert!(parse_error("<ok/>").is_none());
    }
}
