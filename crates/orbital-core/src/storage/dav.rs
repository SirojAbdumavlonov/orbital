//! WebDAV over HTTP or HTTPS (Nextcloud, ownCloud, Synology, Apache mod_dav, …).
//! The profile's path is the server's WebDAV folder, e.g.
//! `/remote.php/dav/files/USER` on Nextcloud; the UI's `/` is that folder.

use super::entry;
use crate::error::{Error, Result};
use crate::profiles::{Profile, Protocol};
use crate::secrets::Secret;
use crate::sftp::FileEntry;
use futures_util::StreamExt;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::{Method, StatusCode};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Characters left as they are in a path segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

pub struct DavClient {
    http: reqwest::Client,
    /// e.g. `https://cloud.example.com:443` (no trailing slash)
    origin: String,
    /// e.g. `/remote.php/dav/files/alice` (no trailing slash; may be empty)
    base: String,
    user: String,
    password: Option<Secret>,
    profile: Profile,
}

pub(crate) fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// One entry of a PROPFIND answer.
#[derive(Debug, PartialEq)]
pub(crate) struct DavItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<u32>,
}

fn href_path(href: &str) -> String {
    let path = if href.starts_with("http://") || href.starts_with("https://") {
        url::Url::parse(href).map(|u| u.path().to_string()).unwrap_or_default()
    } else {
        href.to_string()
    };
    let decoded = percent_decode_str(&path).decode_utf8_lossy().into_owned();
    decoded.trim_end_matches('/').to_string()
}

/// Parse a `207 Multi-Status` PROPFIND answer for the folder at `self_path`
/// (the server path that was asked for, decoded). The folder itself is left out.
pub(crate) fn parse_propfind(xml: &str, self_path: &str) -> Result<Vec<DavItem>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Other(format!("the WebDAV server sent an unreadable listing: {e}")))?;
    let me = self_path.trim_end_matches('/');
    let is = |n: &roxmltree::Node, name: &str| n.is_element() && n.tag_name().name() == name;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| is(n, "response")) {
        let Some(href) = resp.children().find(|n| is(n, "href")).and_then(|n| n.text()) else {
            continue;
        };
        let path = href_path(href.trim());
        if path == me {
            continue;
        }
        let Some(name) = path.rsplit('/').next().filter(|s| !s.is_empty()) else {
            continue;
        };
        let mut item = DavItem {
            name: name.to_string(),
            is_dir: false,
            size: 0,
            mtime: None,
        };
        for ps in resp.children().filter(|n| is(n, "propstat")) {
            let ok = ps
                .children()
                .find(|n| is(n, "status"))
                .and_then(|n| n.text())
                .is_none_or(|t| t.contains(" 200"));
            if !ok {
                continue;
            }
            for prop in ps.children().filter(|n| is(n, "prop")) {
                for p in prop.children().filter(|n| n.is_element()) {
                    match p.tag_name().name() {
                        "resourcetype" => item.is_dir |= p.children().any(|c| is(&c, "collection")),
                        "getcontentlength" => item.size = p.text().and_then(|t| t.trim().parse().ok()).unwrap_or(0),
                        "getlastmodified" => {
                            item.mtime = p
                                .text()
                                .and_then(|t| httpdate::parse_http_date(t.trim()).ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as u32)
                        }
                        _ => {}
                    }
                }
            }
        }
        out.push(item);
    }
    Ok(out)
}

impl DavClient {
    pub async fn connect(p: &Profile, secret: Option<Secret>) -> Result<Self> {
        let scheme = if p.protocol == Protocol::Webdavs { "https" } else { "http" };
        let default = if scheme == "https" { 443 } else { 80 };
        let origin = if p.port == default {
            format!("{scheme}://{}", p.host)
        } else {
            format!("{scheme}://{}:{}", p.host, p.port)
        };
        url::Url::parse(&origin).map_err(|_| Error::InvalidInput(format!("{:?} isn't a valid server address", p.host)))?;
        let base = p.initial_path.as_deref().unwrap_or("").trim().trim_end_matches('/').to_string();
        if !base.is_empty() && !base.starts_with('/') {
            return Err(Error::InvalidInput("the WebDAV path must start with /".into()));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("Orbital/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        let c = Self {
            http,
            origin,
            base,
            user: p.username.clone(),
            password: secret,
            profile: p.clone(),
        };
        // Check the address and the login.
        c.propfind("/", "0").await?;
        Ok(c)
    }

    fn server_path(&self, path: &str) -> String {
        format!("{}{}", self.base, if path == "/" { "/" } else { path })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin, encode_path(&self.server_path(path)))
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self.http.request(method, self.url(path));
        if !self.user.is_empty() || self.password.is_some() {
            r = r.basic_auth(&self.user, self.password.as_ref().map(|s| s.as_str()));
        }
        r
    }

    async fn check(&self, resp: reqwest::Response, path: &str) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() || status == StatusCode::MULTI_STATUS {
            return Ok(resp);
        }
        Err(match status {
            StatusCode::UNAUTHORIZED => Error::AuthFailed {
                user: self.user.clone(),
                host: self.profile.host.clone(),
                profile_id: self.profile.id.clone(),
            },
            StatusCode::FORBIDDEN => Error::PermissionDenied(path.to_string()),
            StatusCode::NOT_FOUND => Error::NotFound(path.to_string()),
            StatusCode::METHOD_NOT_ALLOWED if path == "/" => {
                Error::Other("this address doesn't speak WebDAV; check the path (Nextcloud: /remote.php/dav/files/USER)".into())
            }
            StatusCode::METHOD_NOT_ALLOWED => Error::Other(format!("{path} already exists")),
            StatusCode::CONFLICT => Error::NotFound(format!("the folder for {path}")),
            StatusCode::INSUFFICIENT_STORAGE => Error::Other("the server is out of space".into()),
            s => Error::Other(format!("WebDAV {s} for {path}")),
        })
    }

    async fn send(&self, req: reqwest::RequestBuilder, path: &str) -> Result<reqwest::Response> {
        let resp = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                Error::Ssh(format!("couldn't reach {}: {e}", self.origin))
            } else {
                Error::Other(format!("WebDAV: {e}"))
            }
        })?;
        self.check(resp, path).await
    }

    async fn propfind(&self, path: &str, depth: &str) -> Result<String> {
        let req = self
            .request(Method::from_bytes(b"PROPFIND").expect("method"), path)
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(PROPFIND_BODY);
        let resp = self.send(req, path).await?;
        resp.text().await.map_err(|e| Error::Other(e.to_string()))
    }

    pub async fn list(&self, path: &str) -> Result<Vec<FileEntry>> {
        let xml = self.propfind(path, "1").await?;
        Ok(parse_propfind(&xml, &self.server_path(path))?
            .into_iter()
            .map(|i| entry(path, &i.name, i.is_dir, i.size, i.mtime))
            .collect())
    }

    pub async fn read(&self, path: &str, limit: u64) -> Result<Vec<u8>> {
        let resp = self.send(self.request(Method::GET, path), path).await?;
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

    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<()> {
        self.send(self.request(Method::PUT, path).body(data), path).await.map(|_| ())
    }

    pub async fn mkdir(&self, path: &str) -> Result<()> {
        self.send(self.request(Method::from_bytes(b"MKCOL").expect("method"), path), path)
            .await
            .map(|_| ())
    }

    /// DELETE removes folders with everything in them (that's how WebDAV works).
    pub async fn remove(&self, path: &str) -> Result<()> {
        self.send(self.request(Method::DELETE, path), path).await.map(|_| ())
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let req = self
            .request(Method::from_bytes(b"MOVE").expect("method"), from)
            .header("Destination", self.url(to))
            .header("Overwrite", "F");
        self.send(req, from).await.map(|_| ())
    }

    pub async fn download(&self, remote: &str, local: &Path) -> Result<u64> {
        let resp = self.send(self.request(Method::GET, remote), remote).await?;
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
        let file = tokio::fs::File::open(local).await?;
        let len = file.metadata().await?.len();
        let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));
        let req = self.request(Method::PUT, remote).header("Content-Length", len).body(body);
        self.send(req, remote).await?;
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEXTCLOUD: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
 <d:response><d:href>/remote.php/dav/files/alice/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getlastmodified>Sun, 15 Nov 2026 10:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/files/alice/Photos%20%26%20Video/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getlastmodified>Sun, 15 Nov 2026 10:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response><d:href>https://cloud.example.com/remote.php/dav/files/alice/notes.txt</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>1234</d:getcontentlength><d:getlastmodified>Thu, 01 Jan 1970 00:01:40 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"#;

    #[test]
    fn parses_a_nextcloud_listing() {
        let items = parse_propfind(NEXTCLOUD, "/remote.php/dav/files/alice/").unwrap();
        assert_eq!(
            items,
            vec![
                DavItem {
                    name: "Photos & Video".into(),
                    is_dir: true,
                    size: 0,
                    mtime: Some(1_794_736_800)
                },
                DavItem {
                    name: "notes.txt".into(),
                    is_dir: false,
                    size: 1234,
                    mtime: Some(100)
                },
            ]
        );
    }

    #[test]
    fn parses_other_namespace_prefixes() {
        let xml = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/a.conf</D:href><D:propstat><D:prop><D:getcontentlength>5</D:getcontentlength><D:resourcetype/></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        let items = parse_propfind(xml, "/dav").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "a.conf");
        assert_eq!(items[0].size, 5);
    }

    #[test]
    fn encodes_each_segment() {
        assert_eq!(encode_path("/a b/c&d/é"), "/a%20b/c%26d/%C3%A9");
        assert!(parse_propfind("not xml", "/").is_err());
    }
}
