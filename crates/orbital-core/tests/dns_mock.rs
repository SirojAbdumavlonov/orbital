//! DNS accounts, records and checks end to end, against a local stand-in for
//! Cloudflare's API and a DNS-over-HTTPS resolver that answers from the same
//! records (so a new record "propagates" at once). No network needed.

use orbital_core::audit::{AuditFilter, Outcome};
use orbital_core::db::Database;
use orbital_core::dns::{ProviderKind, RecordInput, RecordType};
use orbital_core::engine::Engine;
use orbital_core::secrets::{MemoryStore, SecretStore};
use orbital_core::Error;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

type Records = Arc<Mutex<Vec<Value>>>;

/// (method, path with query, headers, body) → (status, JSON body)
type Handler = Arc<dyn Fn(&str, &str, &str, &str) -> (u16, Value) + Send + Sync>;

async fn serve(handler: Handler) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let head_end = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while buf.len() < head_end + 4 + len {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let body = String::from_utf8_lossy(&buf[head_end + 4..]).to_string();
                let mut first = head.lines().next().unwrap_or_default().split_whitespace();
                let (method, path) = (
                    first.next().unwrap_or_default().to_string(),
                    first.next().unwrap_or_default().to_string(),
                );
                let (code, reply) = handler(&method, &path, &head, &body);
                let text = reply.to_string();
                let resp = format!(
                    "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

fn ok(result: Value) -> (u16, Value) {
    (
        200,
        json!({ "success": true, "errors": [], "result": result, "result_info": { "page": 1, "total_pages": 1 } }),
    )
}

fn cloudflare(records: Records) -> Handler {
    Arc::new(move |method, path, head, body| {
        if !head.to_ascii_lowercase().contains("authorization: bearer good-token") {
            return (
                403,
                json!({ "success": false, "errors": [{ "code": 9109, "message": "Invalid access token" }] }),
            );
        }
        let path = path.split('?').next().unwrap_or_default();
        let mut recs = records.lock().unwrap();
        match (method, path) {
            ("GET", "/zones") => {
                ok(json!([{ "id": "zone1", "name": "example.com", "status": "active", "name_servers": ["ada.ns.cloudflare.com"] }]))
            }
            ("GET", "/zones/zone1/dns_records") => ok(Value::Array(recs.clone())),
            ("POST", "/zones/zone1/dns_records") => {
                let mut r: Value = serde_json::from_str(body).unwrap();
                r["id"] = json!(format!("rec{}", recs.len() + 1));
                r["proxiable"] = json!(true);
                recs.push(r.clone());
                ok(r)
            }
            (m, p) if p.starts_with("/zones/zone1/dns_records/") => {
                let id = p.rsplit('/').next().unwrap();
                let Some(i) = recs.iter().position(|r| r["id"] == id) else {
                    return (
                        404,
                        json!({ "success": false, "errors": [{ "code": 81044, "message": "Record does not exist." }] }),
                    );
                };
                if m == "DELETE" {
                    recs.remove(i);
                    return ok(json!({ "id": id }));
                }
                let mut r: Value = serde_json::from_str(body).unwrap();
                r["id"] = json!(id);
                recs[i] = r.clone();
                ok(r)
            }
            _ => (
                404,
                json!({ "success": false, "errors": [{ "code": 7003, "message": "No route" }] }),
            ),
        }
    })
}

/// A JSON DoH resolver answering from the mock's records.
fn resolver(records: Records) -> Handler {
    Arc::new(move |_, path, _, _| {
        let q = path.split_once('?').map(|x| x.1).unwrap_or_default();
        let get = |k: &str| {
            q.split('&')
                .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
                .unwrap_or_default()
                .to_string()
        };
        let (name, t) = (get("name"), get("type"));
        let code = match t.as_str() {
            "A" => 1,
            "AAAA" => 28,
            "TXT" => 16,
            _ => 5,
        };
        let recs = records.lock().unwrap();
        let answers: Vec<Value> = recs
            .iter()
            .filter(|r| r["name"] == name.as_str() && r["type"] == t.as_str())
            .map(|r| {
                let data = if t == "TXT" {
                    format!("\"{}\"", r["content"].as_str().unwrap())
                } else {
                    r["content"].as_str().unwrap().to_string()
                };
                json!({ "name": format!("{name}."), "type": code, "TTL": 300, "data": data })
            })
            .collect();
        let exists = recs.iter().any(|r| r["name"] == name.as_str());
        (200, json!({ "Status": if exists { 0 } else { 3 }, "Answer": answers }))
    })
}

fn record(t: RecordType, name: &str, content: &str) -> RecordInput {
    RecordInput {
        rtype: t,
        name: name.into(),
        content: content.into(),
        ttl: 1,
        proxied: false,
        priority: None,
    }
}

#[tokio::test]
async fn accounts_records_and_checks() {
    let records: Records = Arc::default();
    let cf = serve(cloudflare(records.clone())).await;
    let doh = serve(resolver(records.clone())).await;
    let store = Arc::new(MemoryStore::default());
    let engine = Engine::new(Arc::new(Database::in_memory().unwrap()), store.clone());
    engine.set_dns_endpoints(
        &cf,
        vec![
            ("Mock A".into(), format!("{doh}/dns-query")),
            ("Mock B".into(), format!("{doh}/resolve")),
        ],
    );

    // A token that doesn't work is refused and nothing is saved.
    let bad = engine.add_dns_account("Personal", ProviderKind::Cloudflare, "wrong").await;
    assert!(matches!(bad, Err(Error::PermissionDenied(_))), "{bad:?}");
    assert!(engine.dns_accounts().unwrap().is_empty());

    let acc = engine
        .add_dns_account("Personal", ProviderKind::Cloudflare, " good-token ")
        .await
        .unwrap();
    assert_eq!(store.get(&format!("dns:{}:token", acc.id)).unwrap().unwrap().as_str(), "good-token");
    let zones = engine.dns_zones(&acc.id).await.unwrap();
    assert_eq!((zones[0].id.as_str(), zones[0].name.as_str()), ("zone1", "example.com"));

    // Nothing there yet.
    let before = engine
        .dns_check("www.example.com", RecordType::A, &["203.0.113.10".into()])
        .await
        .unwrap();
    assert!(!before.verified);
    assert_eq!(before.answers[0].status, "NXDOMAIN");

    // Bad input never reaches the provider.
    assert!(matches!(
        engine
            .create_dns_record(&acc.id, "zone1", "example.com", &record(RecordType::A, "www", "not-an-ip"))
            .await,
        Err(Error::InvalidInput(_))
    ));
    let created = engine
        .create_dns_record(&acc.id, "zone1", "example.com", &record(RecordType::A, "www", "203.0.113.10"))
        .await
        .unwrap();
    assert_eq!(created.name, "www.example.com");
    let check = engine
        .dns_check("www.example.com", RecordType::A, &["203.0.113.10".into()])
        .await
        .unwrap();
    assert!(check.verified, "{check:?}");
    assert_eq!(check.answers.len(), 2);

    let txt = engine
        .create_dns_record(&acc.id, "zone1", "example.com", &record(RecordType::TXT, "@", "v=spf1 -all"))
        .await
        .unwrap();
    assert!(
        engine
            .dns_check("example.com", RecordType::TXT, &["v=spf1 -all".into()])
            .await
            .unwrap()
            .verified
    );

    // Change the address: the old one no longer verifies, the new one does.
    engine
        .update_dns_record(
            &acc.id,
            "zone1",
            "example.com",
            &created.id,
            &record(RecordType::A, "www.example.com", "203.0.113.20"),
        )
        .await
        .unwrap();
    assert!(
        !engine
            .dns_check("www.example.com", RecordType::A, &["203.0.113.10".into()])
            .await
            .unwrap()
            .verified
    );
    assert!(
        engine
            .dns_check("www.example.com", RecordType::A, &["203.0.113.20".into()])
            .await
            .unwrap()
            .verified
    );
    let listed = engine.dns_records(&acc.id, "zone1").await.unwrap();
    assert_eq!(listed.len(), 2);

    engine
        .delete_dns_record(&acc.id, "zone1", &txt.id, "TXT example.com")
        .await
        .unwrap();
    assert!(matches!(
        engine.delete_dns_record(&acc.id, "zone1", "../../zones", "x").await,
        Err(Error::InvalidInput(_))
    ));

    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    for action in ["dns.account_add", "dns.record_create", "dns.record_update", "dns.record_delete"] {
        assert!(
            audit.iter().any(|e| e.action == action && e.outcome == Outcome::Success),
            "{action} not audited"
        );
    }
    assert!(audit.iter().any(|e| e.action == "dns.account_add" && e.outcome == Outcome::Failure));
    // The token never reaches the activity log.
    assert!(!serde_json::to_string(&audit).unwrap().contains("good-token"));

    // With the Touch ID gate on, a fresh start can't read the token without the user.
    store.set("app:presence-gate", "1").unwrap();
    let locked = Engine::new(engine.db.clone(), store.clone());
    locked.set_dns_endpoints(&cf, vec![]);
    let refused = locked.dns_zones(&acc.id).await;
    assert!(refused.is_err() && !matches!(refused, Err(Error::NotFound(_))), "{refused:?}");

    engine.remove_dns_account(&acc.id).await.unwrap();
    assert!(store.get(&format!("dns:{}:token", acc.id)).unwrap().is_none());
    assert!(engine.dns_accounts().unwrap().is_empty());
}
