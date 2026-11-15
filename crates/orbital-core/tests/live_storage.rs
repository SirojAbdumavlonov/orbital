//! End-to-end tests for FTP, WebDAV and S3 connections against real servers.
//!
//! Skipped unless the ports are set. `scripts/it-storage.sh` starts throwaway
//! servers on 127.0.0.1 (pyftpdlib, WsgiDAV, moto) and prints:
//!
//!   ORBITAL_IT_FTP_PORT=2121 ORBITAL_IT_DAV_PORT=8022 ORBITAL_IT_S3_PORT=9022 \
//!   cargo test -p orbital-core --test live_storage -- --test-threads=1

use orbital_core::audit::{AuditFilter, Outcome};
use orbital_core::db::Database;
use orbital_core::engine::Engine;
use orbital_core::profiles::{AuthMethod, ProfileInput, Protocol};
use orbital_core::secrets::MemoryStore;
use orbital_core::sftp::EntryKind;
use orbital_core::ssh::ConnectOptions;
use orbital_core::Error;
use std::sync::Arc;

fn port(var: &str) -> Option<u16> {
    std::env::var(var).ok()?.parse().ok()
}

fn engine() -> Engine {
    Engine::new(Arc::new(Database::in_memory().unwrap()), Arc::new(MemoryStore::default()))
}

fn profile(protocol: Protocol, port: u16, path: Option<&str>) -> ProfileInput {
    ProfileInput {
        name: format!("it {protocol:?}"),
        host: "127.0.0.1".into(),
        port: Some(port),
        username: "orbital".into(),
        auth_method: AuthMethod::Password,
        key_path: None,
        sudo_mode: None,
        group_name: None,
        color: None,
        initial_path: path.map(String::from),
        jump_profile_id: None,
        protocol,
        region: None,
        path_style: true,
    }
}

fn opts(secret: &str) -> ConnectOptions {
    ConnectOptions {
        secret: Some(secret.into()),
        remember: true,
        ..Default::default()
    }
}

/// The same file workflow on any protocol, under `base`.
async fn workflow(engine: &Engine, sid: &str, base: &str) {
    let dir = format!("{base}/orbital-it");
    let _ = engine.remove(sid, &dir, true).await;
    engine.mkdir(sid, &dir).await.expect("mkdir");
    engine
        .write_file(sid, &format!("{dir}/a.txt"), "hello\n", None)
        .await
        .expect("write");
    assert_eq!(engine.read_file(sid, &format!("{dir}/a.txt")).await.unwrap().content, "hello\n");
    engine
        .rename(sid, &format!("{dir}/a.txt"), &format!("{dir}/b.txt"))
        .await
        .expect("rename");
    engine.create_file(sid, &format!("{dir}/empty.conf")).await.expect("create");

    // A local folder with a subfolder and a name that needs encoding.
    let local = tempfile::tempdir().unwrap();
    std::fs::create_dir(local.path().join("sub")).unwrap();
    std::fs::write(local.path().join("sub/x & y.txt"), b"xy").unwrap();
    std::fs::write(local.path().join("top.bin"), vec![7u8; 70_000]).unwrap();
    let sent = engine.upload(sid, local.path(), &format!("{dir}/up")).await.expect("upload folder");
    assert_eq!(sent, 70_002);

    let list = engine.list_dir(sid, &dir).await.unwrap();
    let names: Vec<_> = list.iter().map(|e| (e.name.as_str(), e.kind == EntryKind::Dir)).collect();
    assert_eq!(names, vec![("up", true), ("b.txt", false), ("empty.conf", false)], "{list:?}");
    let up = engine.list_dir(sid, &format!("{dir}/up")).await.unwrap();
    let top = up.iter().find(|e| e.name == "top.bin").expect("top.bin");
    assert_eq!(top.size, 70_000);
    let sub = engine.list_dir(sid, &format!("{dir}/up/sub")).await.unwrap();
    assert_eq!(sub[0].name, "x & y.txt");

    let out = local.path().join("down.txt");
    assert_eq!(engine.download(sid, &format!("{dir}/up/sub/x & y.txt"), &out).await.unwrap(), 2);
    assert_eq!(std::fs::read(&out).unwrap(), b"xy");

    // Server tools need SSH: refused clearly, not "not connected".
    assert!(matches!(engine.list_services(sid).await, Err(Error::InvalidInput(_))));
    assert!(engine.read_file(sid, &format!("{dir}/nope.txt")).await.is_err());

    engine.remove(sid, &dir, true).await.expect("remove folder");
    let top = if base.is_empty() { "/" } else { base };
    assert!(!engine.list_dir(sid, top).await.unwrap().iter().any(|e| e.name == "orbital-it"));

    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    for action in [
        "file.mkdir",
        "file.write",
        "file.rename",
        "file.upload",
        "file.download",
        "file.delete_recursive",
    ] {
        assert!(
            audit.iter().any(|ev| ev.action == action && ev.outcome == Outcome::Success),
            "{action} not audited"
        );
    }
    engine.disconnect(sid).await.unwrap();
    assert!(matches!(engine.list_dir(sid, top).await, Err(Error::NotConnected(_))));
}

#[tokio::test]
async fn ftp_files() {
    let Some(port) = port("ORBITAL_IT_FTP_PORT") else {
        eprintln!("skipped: ORBITAL_IT_FTP_PORT not set");
        return;
    };
    let engine = engine();
    let p = engine.create_profile(profile(Protocol::Ftp, port, None)).unwrap();
    assert!(matches!(
        engine.connect(&p.id, ConnectOptions::default()).await,
        Err(Error::CredentialsRequired { .. })
    ));
    assert!(matches!(engine.connect(&p.id, opts("wrong")).await, Err(Error::AuthFailed { .. })));
    let s = engine.connect(&p.id, opts("secret")).await.expect("connect");
    assert_eq!(s.protocol, Protocol::Ftp);
    assert_eq!(s.home, "/");
    workflow(&engine, &s.id, "").await;
    // The password was remembered.
    let again = engine.connect(&p.id, ConnectOptions::default()).await.expect("saved password");
    engine.disconnect(&again.id).await.unwrap();
}

#[tokio::test]
async fn ftps_refuses_an_untrusted_certificate_or_plain_server() {
    let Some(port) = port("ORBITAL_IT_FTP_PORT") else {
        return;
    };
    let engine = engine();
    let p = engine.create_profile(profile(Protocol::Ftps, port, None)).unwrap();
    // The test server has no TLS: FTPS must fail rather than fall back to plain FTP.
    assert!(engine.connect(&p.id, opts("secret")).await.is_err());
}

#[tokio::test]
async fn webdav_files() {
    let Some(port) = port("ORBITAL_IT_DAV_PORT") else {
        eprintln!("skipped: ORBITAL_IT_DAV_PORT not set");
        return;
    };
    let engine = engine();
    let p = engine.create_profile(profile(Protocol::Webdav, port, None)).unwrap();
    assert!(matches!(engine.connect(&p.id, opts("wrong")).await, Err(Error::AuthFailed { .. })));
    let s = engine.connect(&p.id, opts("secret")).await.expect("connect");
    assert_eq!(s.label, "http://orbital@127.0.0.1");
    workflow(&engine, &s.id, "").await;
}

#[tokio::test]
async fn s3_files_and_buckets() {
    let Some(port) = port("ORBITAL_IT_S3_PORT") else {
        eprintln!("skipped: ORBITAL_IT_S3_PORT not set");
        return;
    };
    let engine = engine();
    let p = engine.create_profile(profile(Protocol::S3, port, None)).unwrap();
    assert!(matches!(
        engine.connect(&p.id, ConnectOptions::default()).await,
        Err(Error::CredentialsRequired { .. })
    ));
    let s = engine.connect(&p.id, opts("secret")).await.expect("connect");
    let _ = engine.remove(&s.id, "/orbital-it-bucket", false).await;
    engine.mkdir(&s.id, "/orbital-it-bucket").await.expect("create bucket");
    assert!(engine
        .list_dir(&s.id, "/")
        .await
        .unwrap()
        .iter()
        .any(|e| e.name == "orbital-it-bucket" && e.kind == EntryKind::Dir));
    // A folder rename moves every object under it.
    engine.mkdir(&s.id, "/orbital-it-bucket/f").await.unwrap();
    engine.write_file(&s.id, "/orbital-it-bucket/f/one.txt", "1", None).await.unwrap();
    engine
        .rename(&s.id, "/orbital-it-bucket/f", "/orbital-it-bucket/g")
        .await
        .expect("rename folder");
    assert_eq!(engine.read_file(&s.id, "/orbital-it-bucket/g/one.txt").await.unwrap().content, "1");
    assert!(engine
        .list_dir(&s.id, "/orbital-it-bucket")
        .await
        .unwrap()
        .iter()
        .all(|e| e.name != "f"));
    engine.remove(&s.id, "/orbital-it-bucket/g", true).await.unwrap();
    // The workflow disconnects at the end, so the bucket is removed through a new session.
    workflow(&engine, &s.id, "/orbital-it-bucket").await;
    let s2 = engine.connect(&p.id, ConnectOptions::default()).await.expect("saved key");
    engine
        .remove(&s2.id, "/orbital-it-bucket", false)
        .await
        .expect("delete empty bucket");
}
