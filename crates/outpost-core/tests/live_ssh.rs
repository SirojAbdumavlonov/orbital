//! End-to-end tests against a real OpenSSH server.
//!
//! Skipped unless `OUTPOST_IT_HOST` is set. CI starts a disposable sshd
//! (see `.github/workflows/ci.yml` and `scripts/it-sshd.sh`).
//!
//!   OUTPOST_IT_HOST=127.0.0.1 OUTPOST_IT_PORT=2222 OUTPOST_IT_USER=outpost \
//!   OUTPOST_IT_PASS=... OUTPOST_IT_KEY=/path/id_ed25519 cargo test --test live_ssh

use outpost_core::audit::{AuditFilter, Category, Outcome};
use outpost_core::config_edit::{ValidateStage, Validator};
use outpost_core::db::Database;
use outpost_core::engine::Engine;
use outpost_core::profiles::{AuthMethod, ProfileInput, SudoMode};
use outpost_core::secrets::{MemoryStore, SecretKind};
use outpost_core::ssh::ConnectOptions;
use outpost_core::terminal::TermEvent;
use outpost_core::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Env {
    host: String,
    port: u16,
    user: String,
    pass: String,
    key: Option<String>,
}

fn env() -> Option<Env> {
    Some(Env {
        host: std::env::var("OUTPOST_IT_HOST").ok()?,
        port: std::env::var("OUTPOST_IT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(22),
        user: std::env::var("OUTPOST_IT_USER").ok()?,
        pass: std::env::var("OUTPOST_IT_PASS").ok()?,
        key: std::env::var("OUTPOST_IT_KEY").ok(),
    })
}

fn engine() -> Engine {
    std::env::set_var("HOME", std::env::temp_dir()); // isolate from real ~/.ssh/known_hosts
    Engine::new(Arc::new(Database::in_memory().unwrap()), Arc::new(MemoryStore::default()))
}

fn profile(e: &Env, auth: AuthMethod, key: Option<String>) -> ProfileInput {
    ProfileInput {
        name: "it".into(),
        host: e.host.clone(),
        port: Some(e.port),
        username: e.user.clone(),
        auth_method: auth,
        key_path: key,
        sudo_mode: Some(SudoMode::Password),
        group_name: None,
        color: None,
        initial_path: None,
    }
}

/// Connect, going through the host-key trust prompt the way the UI does.
async fn connect(engine: &Engine, pid: &str, secret: Option<String>) -> outpost_core::engine::SessionInfo {
    let opts = ConnectOptions {
        secret: secret.clone(),
        remember: true,
        ..Default::default()
    };
    let fp = match engine.connect(pid, opts).await {
        Err(Error::UnknownHostKey { fingerprint, .. }) => fingerprint,
        other => panic!("expected UnknownHostKey on first connect, got {:?}", other.map(|s| s.label)),
    };
    assert!(fp.starts_with("SHA256:"));
    engine
        .connect(
            pid,
            ConnectOptions {
                accept_fingerprint: Some(fp),
                secret,
                remember: true,
                ..Default::default()
            },
        )
        .await
        .expect("connect after trusting host key")
}

#[tokio::test]
async fn full_workflow_password_auth() {
    let Some(e) = env() else {
        eprintln!("skipped: OUTPOST_IT_HOST not set");
        return;
    };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Password, None)).unwrap();

    // No stored password -> UI is asked for credentials.
    let r = engine.connect(&p.id, ConnectOptions::default()).await;
    assert!(matches!(
        r,
        Err(Error::UnknownHostKey { .. }) | Err(Error::CredentialsRequired { .. })
    ));

    // Wrong password is rejected and audited.
    let fp = match engine
        .connect(
            &p.id,
            ConnectOptions {
                secret: Some("nope".into()),
                ..Default::default()
            },
        )
        .await
    {
        Err(Error::UnknownHostKey { fingerprint, .. }) => fingerprint,
        other => panic!("{:?}", other.map(|s| s.label)),
    };
    let bad = engine
        .connect(
            &p.id,
            ConnectOptions {
                accept_fingerprint: Some(fp),
                secret: Some("nope".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(bad, Err(Error::AuthFailed { .. })), "{:?}", bad.map(|s| s.label));

    // Host key is pinned now; correct password logs in and is remembered.
    let s = engine
        .connect(
            &p.id,
            ConnectOptions {
                secret: Some(e.pass.clone()),
                remember: true,
                ..Default::default()
            },
        )
        .await
        .expect("login");
    assert!(engine.has_secret(&p.id, SecretKind::Password).unwrap());
    assert!(s.home.starts_with('/'));
    engine.disconnect(&s.id).await.unwrap();

    // Reconnect purely from the stored secret.
    let s = engine
        .connect(&p.id, ConnectOptions::default())
        .await
        .expect("reconnect with keychain secret");
    let sid = s.id.clone();

    // --- SFTP ---
    let dir = format!("{}/outpost-it-{}", s.home, std::process::id());
    engine.mkdir(&sid, &dir).await.unwrap();
    let file = format!("{dir}/notes.txt");
    engine.create_file(&sid, &file).await.unwrap();
    let mtime = engine.write_file(&sid, &file, "first version, long line\n", None).await.unwrap();
    // Shorter rewrite must not leave trailing bytes (truncate on write).
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mtime2 = engine.write_file(&sid, &file, "v2\n", mtime).await.unwrap();
    assert_eq!(engine.read_file(&sid, &file).await.unwrap().content, "v2\n");
    // Stale mtime -> conflict instead of silently overwriting someone else's edit.
    let conflict = engine.write_file(&sid, &file, "v3\n", mtime).await;
    assert!(matches!(conflict, Err(Error::Conflict { .. })), "{conflict:?}");
    assert!(mtime2.is_some());
    let listing = engine.list_dir(&sid, &dir).await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "notes.txt");
    engine.rename(&sid, &file, &format!("{dir}/renamed.txt")).await.unwrap();
    engine.chmod(&sid, &format!("{dir}/renamed.txt"), 0o600).await.unwrap();
    assert_eq!(engine.list_dir(&sid, &dir).await.unwrap()[0].mode, "rw-------");

    // Upload / download round trip.
    let local = std::env::temp_dir().join(format!("outpost-it-{}.bin", std::process::id()));
    let payload: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&local, &payload).unwrap();
    assert_eq!(
        engine.upload(&sid, &local, &format!("{dir}/blob.bin")).await.unwrap(),
        payload.len() as u64
    );
    let back = local.with_extension("back");
    engine.download(&sid, &format!("{dir}/blob.bin"), &back).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), payload);

    // --- safe save: pre-install validator rejects, file untouched ---
    let cfg = format!("{dir}/app.conf");
    engine.write_file(&sid, &cfg, "mode=ok\n", None).await.unwrap();
    let session = engine.session(&sid).await.unwrap();
    let must_contain_ok = Validator {
        name: "grep".into(),
        command: "grep -q ^mode=ok {file}".into(),
        stage: ValidateStage::PreInstall,
    };
    let r = session
        .safe_write(&cfg, "mode=bad\n", Some(must_contain_ok.clone()), false, None)
        .await;
    assert!(matches!(r, Err(Error::ValidationFailed { .. })), "{r:?}");
    assert_eq!(engine.read_file(&sid, &cfg).await.unwrap().content, "mode=ok\n");
    // passes -> written, backup created
    let rep = session
        .safe_write(&cfg, "mode=ok-v2\n", Some(must_contain_ok), false, None)
        .await
        .unwrap();
    assert!(rep.backup_path.is_some());
    assert_eq!(engine.read_file(&sid, &cfg).await.unwrap().content, "mode=ok-v2\n");
    // post-install validator fails -> automatic rollback
    let always_fail = Validator {
        name: "false".into(),
        command: "false".into(),
        stage: ValidateStage::PostInstall,
    };
    let r = session.safe_write(&cfg, "mode=whatever\n", Some(always_fail), false, None).await;
    assert!(matches!(r, Err(Error::ValidationFailed { .. })));
    assert_eq!(engine.read_file(&sid, &cfg).await.unwrap().content, "mode=ok-v2\n", "rolled back");

    // --- sudo (password via stdin) ---
    let o = engine.run_command(&sid, "id -u", true).await.unwrap();
    assert_eq!(o.stdout.trim(), "0", "sudo should elevate: {o:?}");
    // privileged safe save of a root-owned file, preserving owner
    let root_cfg = "/etc/outpost-it/app.conf";
    if engine.run_command(&sid, &format!("test -f {root_cfg}"), false).await.unwrap().ok() {
        let fc = engine.read_file(&sid, root_cfg).await.unwrap();
        assert!(!fc.writable);
        let denied = engine.write_file(&sid, root_cfg, "x\n", None).await;
        assert!(matches!(denied, Err(Error::PermissionDenied(_))), "{denied:?}");
        let rep = engine
            .safe_write(&sid, root_cfg, "mode=from-outpost\n", true, true, None)
            .await
            .unwrap();
        assert!(rep.privileged && rep.backup_path.is_some());
        let owner = engine.run_command(&sid, &format!("stat -c %U {root_cfg}"), false).await.unwrap();
        assert_eq!(owner.stdout.trim(), "root");
        assert_eq!(engine.read_file(&sid, root_cfg).await.unwrap().content, "mode=from-outpost\n");
    }

    // --- metrics ---
    let m1 = engine.metrics(&sid).await.unwrap();
    assert!(m1.snapshot.cores >= 1);
    assert!(m1.snapshot.mem_total > 0);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let m2 = engine.metrics(&sid).await.unwrap();
    assert!(m2.cpu_percent.is_some(), "second sample yields a CPU rate");

    // --- terminal in a given directory ---
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let b2 = buf.clone();
    let tid = engine
        .open_terminal(&sid, 120, 30, Some(dir.clone()), move |ev| {
            if let TermEvent::Data(d) = ev {
                b2.lock().unwrap().extend(d)
            }
        })
        .await
        .unwrap();
    engine.terminal_resize(&tid, 100, 40).await.unwrap();
    engine
        .terminal_write(&tid, b"echo \"PWD=$(pwd)\"; echo COLS=$(tput cols 2>/dev/null || stty size)\n")
        .await
        .unwrap();
    let want = format!("PWD={dir}");
    let mut ok = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if String::from_utf8_lossy(&buf.lock().unwrap()).contains(&want) {
            ok = true;
            break;
        }
    }
    assert!(ok, "terminal output: {}", String::from_utf8_lossy(&buf.lock().unwrap()));
    engine.terminal_close(&tid).await.unwrap();

    // --- cleanup + audit ---
    engine.remove(&sid, &dir, true).await.unwrap();
    assert!(matches!(engine.remove(&sid, "/", true).await, Err(Error::InvalidInput(_))));
    engine.disconnect(&sid).await.unwrap();

    let log = engine
        .audit_query(&AuditFilter {
            limit: Some(500),
            ..Default::default()
        })
        .unwrap();
    let has = |a: &str, o: Outcome| log.iter().any(|ev| ev.action == a && ev.outcome == o);
    assert!(has("hostkey.trust", Outcome::Success));
    assert!(has("session.open", Outcome::Failure)); // wrong password
    assert!(has("file.write", Outcome::Failure)); // conflict
    assert!(has("file.write", Outcome::Denied)); // root-owned via sftp
    assert!(has("config.write", Outcome::Success));
    assert!(has("command.run", Outcome::Success));
    assert!(has("terminal.open", Outcome::Success));
    assert!(has("file.delete_recursive", Outcome::Success));
    assert!(
        log.iter().all(|ev| !ev.command.clone().unwrap_or_default().contains(&e.pass)),
        "secrets never logged"
    );
    assert!(log.iter().any(|ev| ev.category == Category::Security));
    let v = engine.audit_verify().unwrap();
    assert!(v.ok, "{v:?}");
    println!("audit events: {}", v.checked);
}

#[tokio::test]
async fn key_auth_and_host_key_change() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let o = engine.run_command(&s.id, "whoami", false).await.unwrap();
    assert_eq!(o.stdout.trim(), e.user);
    engine.disconnect(&s.id).await.unwrap();

    // Tamper with the pinned key -> connection must be refused (MITM protection).
    engine
        .db
        .conn()
        .execute("UPDATE known_hosts SET fingerprint = 'SHA256:attacker'", [])
        .unwrap();
    let r = engine.connect(&p.id, ConnectOptions::default()).await;
    assert!(matches!(r, Err(Error::HostKeyMismatch { .. })), "{:?}", r.map(|s| s.label));
    // ... and accepting a fingerprint does NOT override a mismatch.
    let r = engine
        .connect(
            &p.id,
            ConnectOptions {
                accept_fingerprint: Some("SHA256:attacker".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(r, Err(Error::HostKeyMismatch { .. })));
}

#[tokio::test]
async fn encrypted_key_needs_passphrase() {
    let Some(e) = env() else { return };
    let Ok(key) = std::env::var("OUTPOST_IT_ENC_KEY") else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let r = engine.connect(&p.id, ConnectOptions::default()).await;
    assert!(matches!(
        r,
        Err(Error::CredentialsRequired { .. }) | Err(Error::UnknownHostKey { .. })
    ));
    let s = connect(&engine, &p.id, Some("keypass".into())).await;
    assert!(engine.has_secret(&p.id, SecretKind::KeyPassphrase).unwrap());
    engine.disconnect(&s.id).await.unwrap();
}
