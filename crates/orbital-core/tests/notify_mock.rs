//! Alerts to Telegram and email end to end, against a local stand-in for
//! Telegram's Bot API and a tiny SMTP server. No network needed.

use orbital_core::audit::{AuditFilter, Category, Outcome};
use orbital_core::db::Database;
use orbital_core::engine::Engine;
use orbital_core::notify::{Alert, AlertKind, ChannelConfig, ChannelInput, Level, Security};
use orbital_core::secrets::{MemoryStore, SecretStore};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// (method, path with query, headers, body) → (status, JSON body)
type Handler = Arc<dyn Fn(&str, &str, &str, &str) -> (u16, Value) + Send + Sync>;

const TOKEN: &str = "123456789:AAH0abcdefghijklmnopqrstuvwxyz012345";

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

/// A Telegram that knows one bot and records what it was asked to send.
fn telegram(sent: Arc<Mutex<Vec<Value>>>) -> Handler {
    Arc::new(move |_method, path, _head, body| {
        let Some(rest) = path.strip_prefix(&format!("/bot{TOKEN}/")) else {
            return (401, json!({ "ok": false, "error_code": 401, "description": "Unauthorized" }));
        };
        match rest {
            "getMe" => (
                200,
                json!({ "ok": true, "result": { "id": 123456789, "is_bot": true, "username": "my_servers_bot" } }),
            ),
            "getUpdates" => (
                200,
                json!({ "ok": true, "result": [
                    { "update_id": 1, "message": { "chat": { "id": 100200300, "type": "private", "first_name": "Alex", "username": "alex" } } }
                ] }),
            ),
            "sendMessage" => {
                let v: Value = serde_json::from_str(body).unwrap();
                if v["chat_id"] != json!(100200300) {
                    return (400, json!({ "ok": false, "description": "Bad Request: chat not found" }));
                }
                sent.lock().unwrap().push(v);
                (200, json!({ "ok": true, "result": { "message_id": 1 } }))
            }
            _ => (404, json!({ "ok": false, "description": "Not Found" })),
        }
    })
}

/// A plain SMTP server (no TLS, no sign-in) that keeps each message it receives.
async fn smtp(mails: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let mails = mails.clone();
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut r = BufReader::new(r);
                let _ = w.write_all(b"220 test ESMTP\r\n").await;
                let mut line = String::new();
                let mut data = false;
                let mut mail = String::new();
                loop {
                    line.clear();
                    if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    if data {
                        if line == ".\r\n" {
                            data = false;
                            mails.lock().unwrap().push(std::mem::take(&mut mail));
                            let _ = w.write_all(b"250 queued\r\n").await;
                        } else {
                            mail.push_str(&line);
                        }
                        continue;
                    }
                    let cmd = line.to_ascii_uppercase();
                    let reply: &[u8] = if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                        b"250-test\r\n250 8BITMIME\r\n"
                    } else if cmd.starts_with("DATA") {
                        data = true;
                        b"354 go on\r\n"
                    } else if cmd.starts_with("QUIT") {
                        let _ = w.write_all(b"221 bye\r\n").await;
                        return;
                    } else {
                        b"250 ok\r\n"
                    };
                    let _ = w.write_all(reply).await;
                }
            });
        }
    });
    port
}

fn input(name: &str, events: Vec<AlertKind>, config: ChannelConfig, secret: &str) -> ChannelInput {
    ChannelInput {
        name: name.into(),
        enabled: true,
        events,
        config,
        secret: secret.into(),
    }
}

#[tokio::test]
async fn alerts_reach_telegram_and_email_and_secrets_stay_in_the_vault() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let base = serve(telegram(sent.clone())).await;
    let mails = Arc::new(Mutex::new(Vec::new()));
    let port = smtp(mails.clone()).await;

    let store = Arc::new(MemoryStore::default());
    let engine = Engine::new(Arc::new(Database::in_memory().unwrap()), store.clone());
    engine.set_telegram_base(&base);

    // Finding the chat: a wrong token is refused, the right one lists who wrote.
    assert!(engine
        .telegram_lookup("123456789:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
        .await
        .is_err());
    let bot = engine.telegram_lookup(TOKEN).await.unwrap();
    assert_eq!(bot.username, "my_servers_bot");
    assert_eq!(bot.chats[0].id, "100200300");

    let tg = engine
        .save_notify_channel(
            None,
            &input(
                "Phone",
                vec![AlertKind::Health, AlertKind::Uptime],
                ChannelConfig::Telegram {
                    chat_id: "100200300".into(),
                    chat_name: "Alex".into(),
                },
                TOKEN,
            ),
        )
        .unwrap();
    assert_eq!(store.get(&format!("notify:{}:secret", tg.id)).unwrap().unwrap().as_str(), TOKEN);
    let mail = engine
        .save_notify_channel(
            None,
            &input(
                "Ops mail",
                vec![AlertKind::Uptime],
                ChannelConfig::Email {
                    host: "127.0.0.1".into(),
                    port,
                    security: Security::None,
                    username: String::new(),
                    from: "orbital@example.com".into(),
                    to: vec!["ops@example.com".into()],
                },
                "",
            ),
        )
        .unwrap();

    engine.test_notify_channel(&tg.id).await.unwrap();
    assert!(sent.lock().unwrap()[0]["text"].as_str().unwrap().contains("Test from Orbital"));

    // A health alert goes only where health alerts are wanted.
    let health = Alert {
        kind: AlertKind::Health,
        level: Level::Bad,
        title: "web-1: nginx failed".into(),
        body: "nginx".into(),
    };
    let r = engine.send_alert(&health).await;
    assert_eq!(r.len(), 1);
    assert!(r[0].ok, "{r:?}");
    assert_eq!(sent.lock().unwrap().len(), 2);
    assert!(mails.lock().unwrap().is_empty());

    // An uptime alert goes to both; the email has the subject and the text.
    let down = Alert {
        kind: AlertKind::Uptime,
        level: Level::Bad,
        title: "shop is down".into(),
        body: "https://shop.example.com/\ntimed out".into(),
    };
    let r = engine.send_alert(&down).await;
    assert!(r.iter().all(|x| x.ok), "{r:?}");
    let m = mails.lock().unwrap()[0].clone();
    assert!(
        m.contains("Subject:") && m.contains("shop is down") && m.contains("timed out"),
        "{m}"
    );
    assert!(m.contains("To: ops@example.com"), "{m}");

    // Off means off.
    let mut off = input(
        "Phone",
        vec![AlertKind::Health],
        ChannelConfig::Telegram {
            chat_id: "100200300".into(),
            chat_name: "Alex".into(),
        },
        "",
    );
    off.enabled = false;
    engine.save_notify_channel(Some(&tg.id), &off).unwrap();
    assert_eq!(engine.send_alert(&health).await.len(), 0);
    assert_eq!(
        store.get(&format!("notify:{}:secret", tg.id)).unwrap().unwrap().as_str(),
        TOKEN,
        "an empty token keeps the saved one"
    );

    // An agent channel for a server that isn't connected fails, recorded, without breaking the others.
    let agent = engine
        .save_notify_channel(
            None,
            &input(
                "Agent",
                vec![AlertKind::Uptime],
                ChannelConfig::Agent {
                    profile_id: "p1".into(),
                    server: "web-1".into(),
                },
                "",
            ),
        )
        .unwrap();
    let r = engine.send_alert(&down).await;
    let a = r.iter().find(|x| x.channel_id == agent.id).unwrap();
    assert!(!a.ok && a.error.as_deref().unwrap().contains("isn't connected"), "{a:?}");
    assert!(r.iter().any(|x| x.channel_id == mail.id && x.ok));

    // The audit log knows what went where, never the token.
    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(log
        .iter()
        .any(|e| e.category == Category::Notify && e.action == "notify.send" && e.outcome == Outcome::Success));
    assert!(log.iter().any(|e| e.action == "notify.send" && e.outcome == Outcome::Failure));
    assert!(log.iter().all(|e| !format!("{e:?}").contains(TOKEN)));
    let raw: String = engine
        .db
        .conn()
        .query_row("SELECT group_concat(config || events, '') FROM notify_channels", [], |r| r.get(0))
        .unwrap();
    assert!(!raw.contains(TOKEN));

    // STARTTLS is required when chosen: a server that can't do it is refused, never used in the clear.
    let strict = engine
        .save_notify_channel(
            None,
            &input(
                "Strict",
                vec![AlertKind::Uptime],
                ChannelConfig::Email {
                    host: "127.0.0.1".into(),
                    port,
                    security: Security::StartTls,
                    username: String::new(),
                    from: "orbital@example.com".into(),
                    to: vec!["ops@example.com".into()],
                },
                "",
            ),
        )
        .unwrap();
    let before = mails.lock().unwrap().len();
    assert!(engine.test_notify_channel(&strict.id).await.is_err());
    assert_eq!(mails.lock().unwrap().len(), before);

    // Removing one deletes its secret too.
    engine.delete_notify_channel(&tg.id).unwrap();
    assert!(store.get(&format!("notify:{}:secret", tg.id)).unwrap().is_none());
}

#[tokio::test]
async fn a_flood_is_held_back() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let base = serve(telegram(sent.clone())).await;
    let engine = Engine::new(Arc::new(Database::in_memory().unwrap()), Arc::new(MemoryStore::default()));
    engine.set_telegram_base(&base);
    engine
        .save_notify_channel(
            None,
            &input(
                "Phone",
                vec![AlertKind::Health],
                ChannelConfig::Telegram {
                    chat_id: "100200300".into(),
                    chat_name: "Alex".into(),
                },
                TOKEN,
            ),
        )
        .unwrap();
    let a = Alert {
        kind: AlertKind::Health,
        level: Level::Warn,
        title: "x".into(),
        body: String::new(),
    };
    for _ in 0..orbital_core::notify::MAX_PER_HOUR {
        assert!(engine.send_alert(&a).await[0].ok);
    }
    let held = engine.send_alert(&a).await;
    assert!(!held[0].ok && held[0].error.as_deref().unwrap().contains("held back"));
    assert_eq!(sent.lock().unwrap().len(), orbital_core::notify::MAX_PER_HOUR);
}
