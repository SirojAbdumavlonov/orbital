//! Uptime checks against a local mock HTTP server and real TCP ports: results,
//! the expected text, alerts on up → down → up, the activity log, and that a
//! restart picks up the stored state instead of alerting again.
//!
//! `cargo test -p orbital-core --test uptime_mock`

use orbital_core::audit::{AuditFilter, Category, Outcome};
use orbital_core::db::Database;
use orbital_core::engine::Engine;
use orbital_core::secrets::MemoryStore;
use orbital_core::uptime::{Change, CheckInput, CheckKind, CheckState, UptimeEvent};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the mock server answers with; changed while the test runs.
#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
}

/// A tiny HTTP/1.1 server on 127.0.0.1 that answers every request with `reply`.
async fn mock_http(reply: Arc<Mutex<Reply>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                let r = reply.lock().unwrap().clone();
                let head = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    r.status,
                    r.body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(r.body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

fn engine(db: Arc<Database>) -> (Arc<Engine>, Arc<Mutex<Vec<UptimeEvent>>>) {
    let e = Arc::new(Engine::new(db, Arc::new(MemoryStore::default())));
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    e.set_uptime_listener(move |ev| sink.lock().unwrap().push(ev));
    (e, events)
}

fn changes(events: &Mutex<Vec<UptimeEvent>>) -> Vec<Change> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            UptimeEvent::Changed { change, .. } => Some(*change),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn http_check_alerts_on_down_and_recovery() {
    let reply = Arc::new(Mutex::new(Reply {
        status: 200,
        body: "<h1>Shop</h1><button>Add to cart</button>".into(),
    }));
    let port = mock_http(reply.clone()).await;
    let db = Arc::new(Database::in_memory().unwrap());
    let (e, events) = engine(db.clone());

    let c = e
        .save_uptime_check(
            None,
            CheckInput {
                name: "Shop".into(),
                kind: CheckKind::Http,
                target: format!("http://127.0.0.1:{port}/health"),
                expected_status: Some(200),
                contains: Some("Add to cart".into()),
                interval_minutes: 1,
                paused: false,
            },
        )
        .unwrap();

    let s = e.run_uptime_check(&c.id).await.unwrap();
    assert_eq!(s.state, CheckState::Up);
    let last = s.last.unwrap();
    assert_eq!(last.status_code, Some(200));
    assert!(last.latency_ms.is_some());
    assert!(last.cert_expires.is_none(), "no certificate over plain HTTP");
    assert!(changes(&events).is_empty(), "up on the first run is not an alert");

    // The page still answers 200 but the shop is gone: down.
    reply.lock().unwrap().body = "<h1>Maintenance</h1>".into();
    let s = e.run_uptime_check(&c.id).await.unwrap();
    assert_eq!(s.state, CheckState::Down);
    assert_eq!(s.last.unwrap().error.as_deref(), Some("the page doesn't contain \"Add to cart\""));
    assert_eq!(changes(&events), [Change::WentDown]);

    // Still down, now with an error status: no second alert.
    reply.lock().unwrap().status = 503;
    let s = e.run_uptime_check(&c.id).await.unwrap();
    assert_eq!(
        s.last.unwrap().error.as_deref(),
        Some("got 503 Service Unavailable, expected 200 OK")
    );
    assert_eq!(changes(&events).len(), 1);

    *reply.lock().unwrap() = Reply {
        status: 200,
        body: "Add to cart".into(),
    };
    let s = e.run_uptime_check(&c.id).await.unwrap();
    assert_eq!(s.state, CheckState::Up);
    assert_eq!(changes(&events), [Change::WentDown, Change::Recovered]);
    assert_eq!(s.recent.len(), 4);
    assert_eq!(s.uptime_24h, Some(50.0));
    assert_eq!(s.changes.len(), 2);
    assert!(s.last_error.is_some(), "the last failure stays visible after recovery");

    // Both changes are in the activity log.
    let log = e
        .audit_query(&AuditFilter {
            category: Some(Category::Uptime),
            ..Default::default()
        })
        .unwrap();
    let down = log.iter().find(|ev| ev.action == "uptime.down").expect("down recorded");
    assert_eq!(down.outcome, Outcome::Failure);
    assert!(down.target.as_deref().unwrap().starts_with("Shop ("));
    assert!(log.iter().any(|ev| ev.action == "uptime.up" && ev.outcome == Outcome::Success));
    assert!(log.iter().any(|ev| ev.action == "uptime.check_create"));
    assert!(e.audit_verify().unwrap().ok);

    // Goes down, then the app restarts: the new engine alerts only on the change back.
    reply.lock().unwrap().status = 500;
    e.run_uptime_check(&c.id).await.unwrap();
    let (e2, events2) = engine(db.clone());
    e2.run_uptime_check(&c.id).await.unwrap();
    assert!(changes(&events2).is_empty(), "still down after a restart is not a new alert");
    reply.lock().unwrap().status = 200;
    e2.run_uptime_check(&c.id).await.unwrap();
    assert_eq!(changes(&events2), [Change::Recovered]);

    // Paused checks don't run on the schedule and show no state.
    e2.set_uptime_paused(&c.id, true).unwrap();
    assert_eq!(e2.list_uptime().unwrap()[0].state, CheckState::Unknown);
    e2.delete_uptime_check(&c.id).unwrap();
    assert!(e2.list_uptime().unwrap().is_empty());
}

#[tokio::test]
async fn tcp_check_and_the_scheduler() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let open = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            drop(s);
        }
    });
    // A port nothing listens on.
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };

    let (e, events) = engine(Arc::new(Database::in_memory().unwrap()));
    let tcp = |port: u16| CheckInput {
        name: String::new(),
        kind: CheckKind::Tcp,
        target: format!("127.0.0.1:{port}"),
        expected_status: None,
        contains: None,
        interval_minutes: 5,
        paused: false,
    };
    let up = e.save_uptime_check(None, tcp(open)).unwrap();
    let down = e.save_uptime_check(None, tcp(closed)).unwrap();
    assert_eq!(up.name, up.target);

    // The scheduler runs both straight away (a failure is retried once, ~3 s).
    tokio::spawn(e.clone().run_uptime());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let list = e.list_uptime().unwrap();
        if list.iter().all(|s| s.last.is_some()) {
            let by = |id: &str| list.iter().find(|s| s.check.id == id).unwrap().clone();
            assert_eq!(by(&up.id).state, CheckState::Up);
            let d = by(&down.id);
            assert_eq!(d.state, CheckState::Down);
            assert!(d.last.unwrap().error.unwrap().starts_with("couldn't connect"));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the scheduler didn't run the checks");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(changes(&events), [Change::WentDown], "down on first sight is an alert");
    let ev = events.lock().unwrap();
    assert!(ev
        .iter()
        .any(|e| matches!(e, UptimeEvent::Result { check_id, .. } if *check_id == up.id)));
}

/// Needs the internet: reads a real site's certificate expiry over HTTPS.
/// `cargo test -p orbital-core --test uptime_mock -- --ignored`
#[tokio::test]
#[ignore]
async fn https_check_reads_the_certificate_expiry() {
    let (e, _) = engine(Arc::new(Database::in_memory().unwrap()));
    let c = e
        .save_uptime_check(
            None,
            CheckInput {
                name: String::new(),
                kind: CheckKind::Http,
                target: "https://example.com/".into(),
                expected_status: Some(200),
                contains: None,
                interval_minutes: 15,
                paused: false,
            },
        )
        .unwrap();
    let s = e.run_uptime_check(&c.id).await.unwrap();
    let last = s.last.unwrap();
    assert!(last.up, "{:?}", last.error);
    let exp = chrono::DateTime::parse_from_rfc3339(&last.cert_expires.expect("certificate expiry")).unwrap();
    assert!(exp > chrono::Utc::now());
}
