//! End-to-end tests against a real OpenSSH server.
//!
//! Skipped unless `ORBITAL_IT_HOST` is set. CI starts a disposable sshd
//! (see `.github/workflows/ci.yml` and `scripts/it-sshd.sh`).
//!
//!   ORBITAL_IT_HOST=127.0.0.1 ORBITAL_IT_PORT=2222 ORBITAL_IT_USER=orbital \
//!   ORBITAL_IT_PASS=... ORBITAL_IT_KEY=/path/id_ed25519 cargo test --test live_ssh
//!
//! Set `ORBITAL_IT_NO_PASSWORD=1` to skip the cases that need password login
//! (for example against a key-only sshd started without root).

use orbital_core::audit::{AuditFilter, Category, Outcome};
use orbital_core::config_edit::{ValidateStage, Validator};
use orbital_core::db::Database;
use orbital_core::engine::Engine;
use orbital_core::presence::UserPresence;
use orbital_core::profiles::{AuthMethod, ProfileInput, SudoMode};
use orbital_core::secrets::{MemoryStore, SecretKind};
use orbital_core::ssh::ConnectOptions;
use orbital_core::terminal::TermEvent;
use orbital_core::Error;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        host: std::env::var("ORBITAL_IT_HOST").ok()?,
        port: std::env::var("ORBITAL_IT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(22),
        user: std::env::var("ORBITAL_IT_USER").ok()?,
        pass: std::env::var("ORBITAL_IT_PASS").ok()?,
        key: std::env::var("ORBITAL_IT_KEY").ok(),
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
        jump_profile_id: None,
        protocol: Default::default(),
        region: None,
        path_style: false,
    }
}

/// Connect, going through the host-key trust prompt the way the UI does.
async fn connect(engine: &Engine, pid: &str, secret: Option<String>) -> orbital_core::engine::SessionInfo {
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
        eprintln!("skipped: ORBITAL_IT_HOST not set");
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
    let dir = format!("{}/orbital-it-{}", s.home, std::process::id());
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
    let local = std::env::temp_dir().join(format!("orbital-it-{}.bin", std::process::id()));
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
    let root_cfg = "/etc/orbital-it/app.conf";
    if engine.run_command(&sid, &format!("test -f {root_cfg}"), false).await.unwrap().ok() {
        let fc = engine.read_file(&sid, root_cfg).await.unwrap();
        assert!(!fc.writable);
        let denied = engine.write_file(&sid, root_cfg, "x\n", None).await;
        assert!(matches!(denied, Err(Error::PermissionDenied(_))), "{denied:?}");
        let rep = engine
            .safe_write(&sid, root_cfg, "mode=from-orbital\n", true, true, None)
            .await
            .unwrap();
        assert!(rep.privileged && rep.backup_path.is_some());
        let owner = engine.run_command(&sid, &format!("stat -c %U {root_cfg}"), false).await.unwrap();
        assert_eq!(owner.stdout.trim(), "root");
        assert_eq!(engine.read_file(&sid, root_cfg).await.unwrap().content, "mode=from-orbital\n");
    }

    // --- metrics ---
    let m1 = engine.metrics(&sid).await.unwrap();
    assert!(m1.snapshot.cores >= 1);
    assert!(m1.snapshot.mem_total > 0);
    let mem = &m1.snapshot.memory;
    assert_eq!(
        mem.used + mem.buff_cache + mem.free,
        m1.snapshot.mem_total,
        "the memory breakdown adds up"
    );
    assert!(mem.installed.is_none_or(|b| b >= m1.snapshot.mem_total));
    tokio::time::sleep(Duration::from_millis(500)).await;
    let m2 = engine.metrics(&sid).await.unwrap();
    assert!(m2.cpu_percent.is_some(), "second sample yields a CPU rate");

    // --- terminal in a given directory ---
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let b2 = buf.clone();
    let tid = engine
        .open_terminal(&sid, 120, 30, Some(dir.clone()), false, move |ev| {
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
    let Ok(key) = std::env::var("ORBITAL_IT_ENC_KEY") else { return };
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

// ---------- jump hosts (ProxyJump) ----------
//
// The test sshd doubles as its own bastion: the "target" is reached through a
// direct-tcpip channel of a first session to the same server.

fn via(e: &Env, auth: AuthMethod, key: Option<String>, jump: &str) -> ProfileInput {
    ProfileInput {
        name: "behind-bastion".into(),
        jump_profile_id: Some(jump.into()),
        ..profile(e, auth, key)
    }
}

#[tokio::test]
async fn jump_host_route_with_keys() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let bastion = engine.create_profile(profile(&e, AuthMethod::Key, Some(key.clone()))).unwrap();
    let web = engine.create_profile(via(&e, AuthMethod::Key, Some(key), &bastion.id)).unwrap();

    // First contact stops at the bastion's unknown host key.
    let fp = match engine.connect(&web.id, ConnectOptions::default()).await {
        Err(Error::UnknownHostKey {
            fingerprint, host, port, ..
        }) => {
            assert_eq!((host, port), (e.host.clone(), e.port));
            fingerprint
        }
        other => panic!("{:?}", other.map(|s| s.label)),
    };

    // A fingerprint confirmed for some other endpoint is not applied here ...
    let wrong_endpoint = ConnectOptions {
        accept_fingerprint: Some(fp.clone()),
        accept_host: Some(("elsewhere.example".into(), e.port)),
        ..Default::default()
    };
    let r = engine.connect(&web.id, wrong_endpoint).await;
    assert!(matches!(r, Err(Error::UnknownHostKey { .. })), "{:?}", r.map(|s| s.label));
    // ... and one confirmed without naming an endpoint never pins a jump host.
    let unnamed = ConnectOptions {
        accept_fingerprint: Some(fp.clone()),
        ..Default::default()
    };
    let r = engine.connect(&web.id, unnamed).await;
    assert!(matches!(r, Err(Error::UnknownHostKey { .. })), "{:?}", r.map(|s| s.label));
    assert!(engine.known_hosts().unwrap().is_empty(), "nothing pinned by refused attempts");

    // Confirmed for exactly this endpoint: pinned, and the route comes up.
    let s = engine
        .connect(
            &web.id,
            ConnectOptions {
                accept_fingerprint: Some(fp),
                accept_host: Some((e.host.clone(), e.port)),
                ..Default::default()
            },
        )
        .await
        .expect("connect through the jump host");
    assert_eq!(s.via, vec![format!("{}@{}:{}", e.user, e.host, e.port)]);

    // Commands and SFTP run on the far side of the tunnel.
    let o = engine.run_command(&s.id, "whoami", false).await.unwrap();
    assert_eq!(o.stdout.trim(), e.user);
    let o = engine.run_command(&s.id, "echo $SSH_CONNECTION", false).await.unwrap();
    assert!(o.stdout.contains("127.0.0.1") || o.stdout.contains("::1"), "{}", o.stdout);
    assert!(!engine.list_dir(&s.id, &s.home).await.unwrap().is_empty() || s.home.starts_with('/'));
    let file = format!("{}/orbital-jump-{}.txt", s.home, std::process::id());
    engine.write_file(&s.id, &file, "through the tunnel\n", None).await.unwrap();
    assert_eq!(engine.read_file(&s.id, &file).await.unwrap().content, "through the tunnel\n");
    engine.remove(&s.id, &file, false).await.unwrap();

    // Closing the session takes the bastion session down with it.
    let session = engine.session(&s.id).await.unwrap();
    engine.disconnect(&s.id).await.unwrap();
    for _ in 0..40 {
        if session.is_closed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(session.is_closed());

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    let open = log
        .iter()
        .find(|ev| ev.action == "session.open" && ev.outcome == Outcome::Success)
        .expect("session.open audited");
    assert_eq!(
        open.details.as_ref().unwrap()["via"][0],
        format!("{}@{}:{}", e.user, e.host, e.port)
    );
    assert!(log.iter().any(|ev| ev.action == "hostkey.trust"));
    assert!(engine.audit_verify().unwrap().ok);

    // A pinned bastion key that changes blocks the whole route.
    engine
        .db
        .conn()
        .execute("UPDATE known_hosts SET fingerprint = 'SHA256:attacker'", [])
        .unwrap();
    let r = engine.connect(&web.id, ConnectOptions::default()).await;
    assert!(matches!(r, Err(Error::HostKeyMismatch { .. })), "{:?}", r.map(|s| s.label));
}

#[tokio::test]
async fn jump_host_asks_for_each_hops_own_password() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let bastion = engine.create_profile(profile(&e, AuthMethod::Password, None)).unwrap();
    let web = engine.create_profile(via(&e, AuthMethod::Password, None, &bastion.id)).unwrap();
    let trust = |opts: ConnectOptions, fp: Option<String>| ConnectOptions {
        accept_fingerprint: fp,
        accept_host: Some((e.host.clone(), e.port)),
        ..opts
    };
    let fp = match engine.connect(&web.id, ConnectOptions::default()).await {
        Err(Error::UnknownHostKey { fingerprint, .. }) => fingerprint,
        other => panic!("{:?}", other.map(|s| s.label)),
    };

    // The bastion needs its password first — and says which profile it is for.
    let r = engine.connect(&web.id, trust(ConnectOptions::default(), Some(fp))).await;
    match r {
        Err(Error::CredentialsRequired { profile_id, .. }) => assert_eq!(profile_id, bastion.id),
        other => panic!("{:?}", other.map(|s| s.label)),
    }
    // The target's password alone is never tried on the bastion.
    let only_target = ConnectOptions {
        secret: Some(e.pass.clone()),
        ..Default::default()
    };
    match engine.connect(&web.id, only_target).await {
        Err(Error::CredentialsRequired { profile_id, .. }) => assert_eq!(profile_id, bastion.id),
        other => panic!("{:?}", other.map(|s| s.label)),
    }
    // Bastion password given, target's still missing.
    let hop = HashMap::from([(bastion.id.clone(), e.pass.clone())]);
    let bastion_only = ConnectOptions {
        hop_secrets: hop.clone(),
        ..Default::default()
    };
    match engine.connect(&web.id, bastion_only).await {
        Err(Error::CredentialsRequired { profile_id, .. }) => assert_eq!(profile_id, web.id),
        other => panic!("{:?}", other.map(|s| s.label)),
    }
    // A wrong bastion password is reported against the bastion.
    let wrong = ConnectOptions {
        hop_secrets: HashMap::from([(bastion.id.clone(), "nope".to_string())]),
        secret: Some(e.pass.clone()),
        ..Default::default()
    };
    match engine.connect(&web.id, wrong).await {
        Err(Error::AuthFailed { profile_id, .. }) => assert_eq!(profile_id, bastion.id),
        other => panic!("{:?}", other.map(|s| s.label)),
    }
    // Both given and remembered: each lands under its own profile.
    let both = ConnectOptions {
        hop_secrets: hop,
        secret: Some(e.pass.clone()),
        remember: true,
        ..Default::default()
    };
    let s = engine.connect(&web.id, both).await.expect("route with two passwords");
    assert!(engine.has_secret(&bastion.id, SecretKind::Password).unwrap());
    assert!(engine.has_secret(&web.id, SecretKind::Password).unwrap());
    // sudo on the target still works through the tunnel.
    assert_eq!(engine.run_command(&s.id, "id -u", true).await.unwrap().stdout.trim(), "0");
    engine.disconnect(&s.id).await.unwrap();
    let s = engine
        .connect(&web.id, ConnectOptions::default())
        .await
        .expect("from saved secrets");
    engine.disconnect(&s.id).await.unwrap();

    let dump = engine.audit_export("json").unwrap();
    assert!(!dump.contains(&e.pass), "secrets never logged");
}

// ---------- Touch ID gate ----------

#[derive(Default)]
struct FakePresence {
    approve: AtomicBool,
    asked: AtomicUsize,
}

impl UserPresence for FakePresence {
    fn available(&self) -> bool {
        true
    }
    fn verify(&self, _reason: &str) -> orbital_core::Result<()> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.approve.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::PresenceRequired("cancelled".into()))
        }
    }
}

#[tokio::test]
async fn saved_passphrase_is_used_only_after_user_verification() {
    let Some(e) = env() else { return };
    let Ok(key) = std::env::var("ORBITAL_IT_ENC_KEY") else { return };
    std::env::set_var("HOME", std::env::temp_dir());
    let presence = Arc::new(FakePresence::default());
    presence.approve.store(true, Ordering::SeqCst);
    let engine = Engine::with_presence(
        Arc::new(Database::in_memory().unwrap()),
        Arc::new(MemoryStore::default()),
        presence.clone(),
    );
    let mut input = profile(&e, AuthMethod::Key, Some(key));
    input.sudo_mode = Some(SudoMode::Never);
    let p = engine.create_profile(input).unwrap();
    // Typed passphrase, saved on success. Nothing saved yet, so nobody is asked.
    let s = connect(&engine, &p.id, Some("keypass".into())).await;
    engine.disconnect(&s.id).await.unwrap();
    assert!(engine.has_secret(&p.id, SecretKind::KeyPassphrase).unwrap());
    assert_eq!(presence.asked.load(Ordering::SeqCst), 0);

    engine.set_presence_gate(true).await.unwrap();
    let asked = presence.asked.load(Ordering::SeqCst);

    // Cancelled prompt: no session, and the passphrase was not used.
    presence.approve.store(false, Ordering::SeqCst);
    let r = engine.connect(&p.id, ConnectOptions::default()).await;
    assert!(matches!(r, Err(Error::PresenceRequired(_))), "{:?}", r.map(|s| s.label));

    // Approved: connects from the saved passphrase, asking exactly once.
    presence.approve.store(true, Ordering::SeqCst);
    let s = engine.connect(&p.id, ConnectOptions::default()).await.expect("after verification");
    assert_eq!(presence.asked.load(Ordering::SeqCst), asked + 2);
    engine.disconnect(&s.id).await.unwrap();

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(log.iter().any(|ev| ev.action == "session.open"
        && ev.outcome == Outcome::Success
        && ev.details.as_ref().is_some_and(|d| d["userVerified"] == true)));
    assert!(log
        .iter()
        .any(|ev| ev.action == "session.open" && ev.outcome == Outcome::Denied && ev.category == Category::Security));
}

// ---------- folder upload, server-to-server copy, software detection ----------

#[tokio::test]
async fn folder_upload_and_copy_between_servers() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    // Two independent sessions stand in for two servers.
    let a = connect(&engine, &p.id, None).await;
    let b = engine.connect(&p.id, ConnectOptions::default()).await.unwrap();
    assert_ne!(a.id, b.id);

    // A local tree with a nested folder, an executable, and a symlink to skip.
    let local = std::env::temp_dir().join(format!("orbital-it-tree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&local);
    std::fs::create_dir_all(local.join("conf.d")).unwrap();
    std::fs::write(local.join("readme.txt"), "hello\n").unwrap();
    std::fs::write(local.join("conf.d/site.conf"), "server { listen 80; }\n").unwrap();
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(local.join("blob.bin"), &big).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(local.join("run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(local.join("run.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", local.join("link")).unwrap();
    }

    let src = format!("{}/orbital-it-src-{}", a.home, std::process::id());
    let dst = format!("{}/orbital-it-dst-{}", b.home, std::process::id());
    let expected = 6 + 22 + big.len() as u64 + 10;
    assert_eq!(engine.upload(&a.id, &local, &src).await.unwrap(), expected);
    let names = |v: Vec<orbital_core::sftp::FileEntry>| v.into_iter().map(|f| f.name).collect::<Vec<_>>();
    assert_eq!(
        names(engine.list_dir(&a.id, &src).await.unwrap()),
        ["conf.d", "blob.bin", "readme.txt", "run.sh"],
        "symlink not uploaded"
    );

    // Server to server: whole folder, modes kept, nothing followed out of the tree.
    engine
        .run_command(&a.id, &format!("ln -s /etc/passwd {src}/escape"), false)
        .await
        .unwrap();
    let rep = engine.copy_between(&a.id, &src, &b.id, &dst, false).await.unwrap();
    assert_eq!((rep.files, rep.dirs, rep.skipped, rep.bytes), (4, 2, 1, expected));
    assert_eq!(
        engine.read_file(&b.id, &format!("{dst}/conf.d/site.conf")).await.unwrap().content,
        "server { listen 80; }\n"
    );
    let listing = engine.list_dir(&b.id, &dst).await.unwrap();
    assert_eq!(listing.iter().find(|f| f.name == "run.sh").unwrap().mode, "rwxr-x---");
    assert!(listing.iter().all(|f| f.name != "escape"));
    let back = local.join("blob.back");
    engine.download(&b.id, &format!("{dst}/blob.bin"), &back).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), big);

    // Existing destination is refused unless overwriting was asked for.
    let again = engine.copy_between(&a.id, &src, &b.id, &dst, false).await;
    assert!(matches!(again, Err(Error::InvalidInput(_))), "{again:?}");
    engine.copy_between(&a.id, &src, &b.id, &dst, true).await.unwrap();
    // Single file, and the same-server guard.
    let one = engine
        .copy_between(&a.id, &format!("{src}/readme.txt"), &b.id, &format!("{dst}/copy.txt"), false)
        .await
        .unwrap();
    assert_eq!((one.files, one.bytes), (1, 6));
    assert!(engine.copy_between(&a.id, &src, &a.id, &dst, true).await.is_err());

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    let copy = log
        .iter()
        .find(|ev| ev.action == "file.copy_from_server" && ev.outcome == Outcome::Success)
        .expect("copy audited");
    assert!(copy.details.as_ref().unwrap()["from"]
        .as_str()
        .unwrap()
        .ends_with(&format!(":{src}/readme.txt")));
    assert!(log
        .iter()
        .any(|ev| ev.action == "file.copy_from_server" && ev.outcome == Outcome::Failure));

    engine.remove(&a.id, &src, true).await.unwrap();
    engine.remove(&b.id, &dst, true).await.unwrap();
    let _ = std::fs::remove_dir_all(&local);
    engine.disconnect(&a.id).await.unwrap();
    engine.disconnect(&b.id).await.unwrap();
}

#[tokio::test]
async fn detects_installed_software() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let tools = engine.detect_tools(&s.id).await.unwrap();
    // We are talking to an sshd, so at the very least that one is running.
    let sshd = tools.iter().find(|t| t.id == "sshd").expect("sshd detected");
    assert!(sshd.running, "{sshd:?}");
    // Running tools are listed first.
    let first_stopped = tools.iter().position(|t| !t.running).unwrap_or(tools.len());
    assert!(tools[first_stopped..].iter().all(|t| !t.running));
    // nginx, when present, yields a readable config; when absent, a clean failure.
    let cfg = engine.nginx_config(&s.id).await.unwrap();
    if tools.iter().any(|t| t.id == "nginx" && t.binary.is_some()) && cfg.test_ok {
        assert!(!cfg.files.is_empty());
    } else {
        assert!(cfg.files.is_empty() || !cfg.test_ok);
    }
    engine.disconnect(&s.id).await.unwrap();
}

// ---------- open ports, sshd review, firewall, fail2ban, certificates ----------

fn key_profile(e: &Env) -> Option<ProfileInput> {
    Some(profile(e, AuthMethod::Key, Some(e.key.clone()?)))
}

/// Read-only views work on any server, and bad input to the changing calls
/// is refused before anything is sent, and audited.
#[tokio::test]
async fn security_views_read_and_bad_input_is_refused() {
    use orbital_core::certbot::{CertMethod, IssueRequest};
    use orbital_core::firewall::{UfwAction, UfwDirection, UfwProtocol, UfwRuleInput};
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;

    // The sshd we talk to is listening on its port.
    let ports = engine.listening_ports(&s.id).await.unwrap();
    let ours = ports
        .ports
        .iter()
        .find(|x| x.port == e.port && x.protocol == "tcp")
        .unwrap_or_else(|| panic!("port {} not listed: {ports:?}", e.port));
    if let Some(name) = &ours.process {
        assert!(name.contains("sshd"), "{ours:?}");
    }

    let r = engine.sshd_report(&s.id).await.unwrap();
    assert!(r.version.as_deref().unwrap_or("").contains("OpenSSH"), "{r:?}");
    if r.readable {
        assert!(r.checks.iter().any(|c| c.id == "root-login"));
    }

    // Each tool either answers or says cleanly that it is missing or needs root.
    let ok_or_missing = |r: Result<(), Error>| {
        assert!(
            matches!(r, Ok(()) | Err(Error::NotFound(_)) | Err(Error::PermissionDenied(_))),
            "{r:?}"
        )
    };
    ok_or_missing(engine.ufw_status(&s.id).await.map(|_| ()));
    ok_or_missing(engine.fail2ban_status(&s.id).await.map(|_| ()));
    ok_or_missing(engine.certbot_status(&s.id).await.map(|_| ()));

    let bad_rule = UfwRuleInput {
        action: UfwAction::Allow,
        direction: UfwDirection::In,
        port: Some("22; touch /tmp/pwned".into()),
        protocol: UfwProtocol::Tcp,
        address: None,
        app: None,
        comment: None,
    };
    assert!(matches!(engine.ufw_add(&s.id, bad_rule, false).await, Err(Error::InvalidInput(_))));
    assert!(matches!(
        engine.fail2ban_unban(&s.id, Some("sshd"), "1.2.3.4 && id").await,
        Err(Error::InvalidInput(_))
    ));
    let no_tos = IssueRequest {
        domains: vec!["example.org".into()],
        email: None,
        method: CertMethod::Standalone,
        webroot: None,
        redirect: false,
        staging: true,
        agree_tos: false,
    };
    assert!(matches!(engine.certbot_issue(&s.id, no_tos).await, Err(Error::InvalidInput(_))));
    assert!(matches!(
        engine.certbot_delete(&s.id, "../x", true).await,
        Err(Error::InvalidInput(_))
    ));

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    for action in ["firewall.rule_add", "fail2ban.unban", "certificate.issue", "certificate.delete"] {
        assert!(
            log.iter()
                .any(|ev| ev.action == action && ev.outcome != Outcome::Success && ev.category == Category::Security),
            "{action} refusal not audited"
        );
    }
    engine.disconnect(&s.id).await.unwrap();
}

/// Connect with the password, which is also the sudo password, for the cases
/// that change root-owned state.
async fn sudo_session(engine: &Engine, e: &Env) -> orbital_core::engine::SessionInfo {
    let p = engine.create_profile(profile(e, AuthMethod::Password, None)).unwrap();
    engine.set_secret(&p.id, SecretKind::SudoPassword, Some(&e.pass)).unwrap();
    connect(engine, &p.id, Some(e.pass.clone())).await
}

/// Needs ufw installed and on, with incoming allowed by default so the test
/// can't cut the machine off (CI sets this up). Opt in with ORBITAL_IT_UFW=1.
#[tokio::test]
async fn ufw_rules_add_change_delete_and_lockout_guard() {
    use orbital_core::firewall::{UfwAction, UfwDirection, UfwProtocol, UfwRuleInput, UfwRuleRef};
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_UFW").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_UFW=1 on a disposable machine with ufw");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let rule = |action, port: &str| UfwRuleInput {
        action,
        direction: UfwDirection::In,
        port: Some(port.into()),
        protocol: UfwProtocol::Tcp,
        address: None,
        app: None,
        comment: Some("orbital-it".into()),
    };
    let ours = |st: &orbital_core::firewall::UfwStatus| -> Vec<UfwRuleRef> {
        st.rules
            .iter()
            .filter(|r| r.comment.as_deref() == Some("orbital-it"))
            .map(|r| UfwRuleRef {
                number: r.number,
                raw: r.raw.clone(),
            })
            .collect()
    };

    let st = engine.ufw_status(&s.id).await.unwrap();
    assert!(st.active, "CI turns ufw on before this test");
    assert!(ours(&st).is_empty(), "leftover rules from an earlier run");

    // A deny for the SSH port in use is stopped before it is sent.
    let lock = engine.ufw_add(&s.id, rule(UfwAction::Deny, &e.port.to_string()), false).await;
    assert!(matches!(lock, Err(Error::PreflightFailed { .. })), "{lock:?}");
    assert!(ours(&engine.ufw_status(&s.id).await.unwrap()).is_empty());

    // Add, change (keeps its place, replaces the IPv6 copy too), delete.
    let st = engine.ufw_add(&s.id, rule(UfwAction::Allow, "18080"), false).await.unwrap();
    let added = ours(&st);
    assert!(
        !added.is_empty() && st.rules.iter().any(|r| r.to.starts_with("18080/tcp")),
        "{st:?}"
    );
    let first = added[0].number;
    let st = engine
        .ufw_update(&s.id, added, rule(UfwAction::Limit, "18081"), false)
        .await
        .unwrap();
    assert!(!st.rules.iter().any(|r| r.to.starts_with("18080")), "{st:?}");
    let changed = st.rules.iter().find(|r| r.to == "18081/tcp").expect("changed rule");
    assert_eq!((changed.number, changed.action.as_str()), (first, "LIMIT"));

    // A stale number is refused.
    let stale = vec![UfwRuleRef {
        number: changed.number,
        raw: "something else".into(),
    }];
    assert!(matches!(engine.ufw_delete(&s.id, stale, false).await, Err(Error::InvalidInput(_))));

    let st = engine.ufw_delete(&s.id, ours(&st), false).await.unwrap();
    assert!(ours(&st).is_empty(), "{st:?}");

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(log
        .iter()
        .any(|ev| ev.action == "firewall.rule_add" && ev.outcome == Outcome::Denied));
    assert!(log
        .iter()
        .any(|ev| ev.action == "firewall.rule_update" && ev.outcome == Outcome::Success));
    assert!(log
        .iter()
        .any(|ev| ev.action == "firewall.rule_delete" && ev.outcome == Outcome::Success));
    assert!(
        !engine.audit_export("json").unwrap().contains(&e.pass),
        "sudo password leaked into the log"
    );
    engine.disconnect(&s.id).await.unwrap();
}

/// Needs fail2ban running with its stock sshd jail (CI starts it). Opt in
/// with ORBITAL_IT_FAIL2BAN=1. Also checks certbot, which CI installs.
#[tokio::test]
async fn fail2ban_ban_and_unban() {
    use orbital_core::certbot::RenewMode;
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_FAIL2BAN").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_FAIL2BAN=1 on a disposable machine with fail2ban");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let st = engine.fail2ban_status(&s.id).await.unwrap();
    assert!(st.running, "{st:?}");
    let jail = st.jails.iter().find(|j| j.name == "sshd").expect("stock sshd jail");
    let test_ip = "203.0.113.77";
    assert!(!jail.banned.iter().any(|ip| ip == test_ip));

    // The address this connection comes from can't be banned.
    let me = st.client_ip.clone().expect("client address");
    let r = engine.fail2ban_ban(&s.id, "sshd", &me).await;
    assert!(matches!(r, Err(Error::PreflightFailed { .. })), "{r:?}");

    let st = engine.fail2ban_ban(&s.id, "sshd", test_ip).await.unwrap();
    assert!(st
        .jails
        .iter()
        .find(|j| j.name == "sshd")
        .unwrap()
        .banned
        .iter()
        .any(|ip| ip == test_ip));
    let st = engine.fail2ban_unban(&s.id, None, test_ip).await.unwrap();
    assert!(!st
        .jails
        .iter()
        .find(|j| j.name == "sshd")
        .unwrap()
        .banned
        .iter()
        .any(|ip| ip == test_ip));

    // certbot answers with root, and a dry run with nothing to renew succeeds.
    if let Ok(c) = engine.certbot_status(&s.id).await {
        assert!(c.error.is_none(), "{c:?}");
        let out = engine.certbot_renew(&s.id, None, RenewMode::DryRun).await.unwrap();
        assert_eq!(out.exit_code, 0, "{out:?}");
    }

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    for (action, outcome) in [
        ("fail2ban.ban", Outcome::Denied),
        ("fail2ban.ban", Outcome::Success),
        ("fail2ban.unban", Outcome::Success),
    ] {
        assert!(
            log.iter().any(|ev| ev.action == action && ev.outcome == outcome),
            "{action} {outcome:?}"
        );
    }
    engine.disconnect(&s.id).await.unwrap();
}

/// Server details read on any host; Compose refuses bad input before sending anything, and audits it.
#[tokio::test]
async fn server_details_and_compose_input() {
    use orbital_core::docker::{ComposeAction, ComposeTarget};
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;

    let i = engine.system_info(&s.id).await.unwrap();
    assert!(i.hostname.is_some() && i.kernel.is_some() && i.arch.is_some(), "{i:?}");
    assert!(i.processes.unwrap_or(0) > 0, "{i:?}");

    let bad = ComposeTarget {
        project: "shop".into(),
        files: vec!["../../etc/passwd".into()],
        working_dir: None,
        service: None,
    };
    let r = engine.compose_action(&s.id, bad, ComposeAction::Up).await;
    assert!(matches!(r, Err(Error::InvalidInput(_))), "{r:?}");
    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(log.iter().any(|ev| ev.action == "compose.up" && ev.outcome != Outcome::Success));
    engine.disconnect(&s.id).await.unwrap();
}

/// Databases, Apache, HAProxy and users read cleanly on any host (or say they
/// are missing), and bad or self-locking changes are refused and audited.
#[tokio::test]
async fn databases_web_servers_and_users_read_and_refuse() {
    use orbital_core::databases::{BackupFormat, DbKind, RedisAction};
    use orbital_core::users::{NewUser, UserChange};
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;

    let fine = |r: Result<(), Error>| {
        assert!(
            matches!(r, Ok(()) | Err(Error::NotFound(_)) | Err(Error::PermissionDenied(_))),
            "{r:?}"
        )
    };
    for kind in [DbKind::Postgres, DbKind::Mysql, DbKind::Mongo] {
        fine(engine.list_databases(&s.id, kind).await.map(|_| ()));
    }
    fine(engine.redis_info(&s.id).await.map(|_| ()));
    fine(engine.apache_status(&s.id).await.map(|_| ()));
    fine(engine.haproxy_status(&s.id).await.map(|_| ()));

    let local = std::env::temp_dir().join("orbital-it-never.dump");
    let r = engine
        .backup_database(&s.id, DbKind::Postgres, "x; reboot", BackupFormat::Native, &local)
        .await;
    assert!(matches!(r, Err(Error::InvalidInput(_))), "{r:?}");
    assert!(!local.exists());
    assert!(engine.redis_action(&s.id, RedisAction::FlushDb(99_999)).await.is_err());

    let users = engine.list_users(&s.id).await.unwrap();
    assert!(users.users.iter().any(|u| u.uid == 0), "root is listed");
    // macOS keeps people in Directory Services, not /etc/passwd; Linux lists the login user.
    let listed = users.users.iter().any(|u| u.name == e.user);
    let bad = NewUser {
        name: "Bad Name".into(),
        create_home: true,
        ..Default::default()
    };
    assert!(matches!(engine.create_user(&s.id, bad, None).await, Err(Error::InvalidInput(_))));
    let me = engine
        .change_user(&s.id, &e.user, UserChange::Delete { remove_home: false }, None, true)
        .await;
    if listed {
        assert!(
            matches!(me, Err(Error::PreflightFailed { .. })),
            "deleting yourself is refused even with force: {me:?}"
        );
    } else {
        assert!(matches!(me, Err(Error::NotFound(_))), "{me:?}");
    }

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    for action in ["database.backup", "user.create", "user.delete"] {
        assert!(
            log.iter().any(|ev| ev.action == action && ev.outcome != Outcome::Success),
            "{action} refusal audited"
        );
    }
    engine.disconnect(&s.id).await.unwrap();
}

/// Creates, changes and deletes a real account. Needs root through sudo on a
/// disposable machine: opt in with ORBITAL_IT_USERS=1 (CI does).
#[tokio::test]
async fn user_lifecycle() {
    use orbital_core::users::{NewUser, UserChange};
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let name = format!("orbit{}", std::process::id() % 100_000);
    let secret = "Tr1cky:pass word!";
    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl it@orbital";
    let u = NewUser {
        name: name.clone(),
        full_name: Some("Orbital Test".into()),
        shell: Some("/bin/sh".into()),
        create_home: true,
        groups: vec!["adm".into()],
        ssh_keys: vec![key.into()],
        must_change_password: true,
        ..Default::default()
    };
    let r = engine.create_user(&s.id, u, Some(secret.into())).await.unwrap();
    let acct = r.users.iter().find(|x| x.name == name).expect("created");
    assert_eq!((acct.full_name.as_str(), acct.shell.as_str()), ("Orbital Test", "/bin/sh"));
    assert_eq!(acct.groups, ["adm"]);
    assert_eq!((acct.password_status.as_deref(), acct.ssh_keys), (Some("P"), Some(1)));
    let keys = engine
        .run_command(
            &s.id,
            &format!("stat -c '%a %U' /home/{name}/.ssh /home/{name}/.ssh/authorized_keys"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        keys.stdout.split_whitespace().collect::<Vec<_>>(),
        ["700", name.as_str(), "600", name.as_str()]
    );

    let r = engine
        .change_user(&s.id, &name, UserChange::Groups { groups: vec![] }, None, false)
        .await
        .unwrap();
    assert!(r.users.iter().find(|x| x.name == name).unwrap().groups.is_empty());
    let r = engine.change_user(&s.id, &name, UserChange::Lock, None, false).await.unwrap();
    assert_eq!(
        r.users.iter().find(|x| x.name == name).unwrap().password_status.as_deref(),
        Some("L")
    );
    let r = engine
        .change_user(&s.id, &name, UserChange::Delete { remove_home: true }, None, false)
        .await
        .unwrap();
    assert!(!r.users.iter().any(|x| x.name == name));

    let dump = engine.audit_export("json").unwrap();
    assert!(
        !dump.contains(secret) && !dump.contains("pass word"),
        "the new password reached the log"
    );
    engine.disconnect(&s.id).await.unwrap();
}

/// With sudo set up, a terminal opens as root and the saved sudo password is
/// typed for the user without ever appearing on screen. Needs password login.
#[tokio::test]
async fn root_terminal_uses_saved_sudo_password_invisibly() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let b2 = buf.clone();
    let tid = engine
        .open_terminal(&s.id, 120, 30, Some("/tmp".into()), true, move |ev| {
            if let TermEvent::Data(d) = ev {
                b2.lock().unwrap().extend(d)
            }
        })
        .await
        .unwrap();
    engine.terminal_write(&tid, b"echo ROOT=$(id -u) DIR=$(pwd)\n").await.unwrap();
    let mut out = String::new();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        out = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
        if out.contains("ROOT=0 DIR=/tmp") {
            break;
        }
    }
    assert!(out.contains("ROOT=0 DIR=/tmp"), "{out}");
    assert!(!out.contains(&e.pass), "the sudo password was shown in the terminal");
    assert!(!out.contains("[orbital:sudo]"), "the prompt marker was shown");
    engine.terminal_close(&tid).await.unwrap();
    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(log
        .iter()
        .any(|ev| ev.action == "terminal.open" && ev.target.as_deref() == Some("/tmp (as root)")));
    engine.disconnect(&s.id).await.unwrap();
}

/// Scheduled jobs, disk usage and CPU/memory by program read on any host;
/// a job that isn't in a crontab can't be run through "Run now".
#[tokio::test]
async fn cron_and_resource_usage_read() {
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;

    let cron = engine.cron_report(&s.id).await.unwrap();
    assert!(cron.server_now.is_some(), "{cron:?}");
    let r = engine.cron_run(&s.id, &e.user, "echo not-a-scheduled-job").await;
    assert!(matches!(r, Err(Error::NotFound(_))), "{r:?}");

    let u = engine.disk_usage(&s.id, "/tmp").await.unwrap();
    assert!(u.fs_size.is_some(), "{u:?}");
    assert!(matches!(engine.disk_usage(&s.id, "relative").await, Err(Error::InvalidInput(_))));
    assert!(matches!(engine.disk_usage(&s.id, "/no/such/dir").await, Err(Error::NotFound(_))));

    let procs = engine.process_usage(&s.id).await.unwrap();
    assert!(
        procs.cores > 0 && procs.processes.iter().any(|p| p.command.contains("sshd")),
        "{:?}",
        procs.by_program
    );
    engine.disconnect(&s.id).await.unwrap();
}

/// Adds, pauses, resumes and deletes a job in the login user's real crontab,
/// so it runs only on a disposable machine (ORBITAL_IT_USERS=1, set in CI).
#[tokio::test]
async fn cron_job_lifecycle() {
    use orbital_core::cron::CronChange;
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine");
        return;
    }
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let cmd = format!("echo orbital-it-{} > /dev/null", std::process::id());
    let mine = |r: &orbital_core::cron::CronReport| r.jobs.iter().find(|j| j.command == cmd).cloned();

    let r = engine
        .cron_change(
            &s.id,
            &e.user,
            CronChange::Add {
                schedule: "17 3 * * 1-5".into(),
                command: cmd.clone(),
                note: Some("orbital test".into()),
            },
        )
        .await
        .unwrap();
    let job = mine(&r).expect("added");
    assert_eq!(
        (job.description.as_str(), job.note.as_deref()),
        ("Every weekday at 03:17", Some("orbital test"))
    );
    assert!(!job.next_runs.is_empty());

    // Bad syntax never reaches the crontab.
    let bad = engine
        .cron_change(
            &s.id,
            &e.user,
            CronChange::Add {
                schedule: "99 * * * *".into(),
                command: "true".into(),
                note: None,
            },
        )
        .await;
    assert!(matches!(bad, Err(Error::InvalidInput(_))), "{bad:?}");

    let out = engine.cron_run(&s.id, &e.user, &cmd).await.unwrap();
    assert_eq!(out.exit_code, 0, "{out:?}");

    let r = engine
        .cron_change(&s.id, &e.user, CronChange::Pause { raw: job.raw.clone() })
        .await
        .unwrap();
    let paused = mine(&r).unwrap();
    assert!(paused.paused && paused.next_runs.is_empty());
    let r = engine
        .cron_change(&s.id, &e.user, CronChange::Resume { raw: paused.raw.clone() })
        .await
        .unwrap();
    let job = mine(&r).unwrap();
    assert!(!job.paused);
    // A stale line is refused, not guessed at.
    let stale = engine
        .cron_change(
            &s.id,
            &e.user,
            CronChange::Delete {
                raw: "0 0 * * * not there".into(),
            },
        )
        .await;
    assert!(matches!(stale, Err(Error::InvalidInput(_))));
    let r = engine
        .cron_change(&s.id, &e.user, CronChange::Delete { raw: job.raw.clone() })
        .await
        .unwrap();
    assert!(mine(&r).is_none());
    engine.disconnect(&s.id).await.unwrap();
}

/// A tunnel reaches the server's own SSH port; a followed log shows new lines;
/// a deploy fast-forwards, builds, restarts and can roll back.
#[tokio::test]
async fn tunnels_logs_and_deploys() {
    use orbital_core::deploy::{AppInput, RestartKind};
    use orbital_core::logs::LogSource;
    use orbital_core::streams::StreamEvent;
    use tokio::io::AsyncReadExt;
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;

    // --- tunnel: the SSH server greets through it ---
    let t = engine
        .open_tunnel(&s.id, "127.0.0.1", e.port, None, Some("ssh".into()))
        .await
        .unwrap();
    let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", t.local_port)).await.unwrap();
    let mut buf = [0u8; 8];
    tokio::time::timeout(Duration::from_secs(10), sock.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"SSH-2.0-");
    assert_eq!(engine.list_tunnels().await.len(), 1);
    assert!(engine.open_tunnel(&s.id, "x;id", 22, None, None).await.is_err());
    engine.close_tunnel(&t.id).await.unwrap();
    assert!(engine.list_tunnels().await.is_empty());

    // --- logs: new lines arrive ---
    let dir = format!("{}/orbital-it-{}", s.home, std::process::id());
    engine
        .run_command(&s.id, &format!("rm -rf {dir}; mkdir -p {dir} && echo first > {dir}/app.log"), false)
        .await
        .unwrap();
    let got = Arc::new(Mutex::new(String::new()));
    let g2 = got.clone();
    let lid = engine
        .follow_logs(
            &s.id,
            LogSource::File {
                path: format!("{dir}/app.log"),
            },
            10,
            move |ev| {
                if let StreamEvent::Data(d) = ev {
                    g2.lock().unwrap().push_str(&String::from_utf8_lossy(&d));
                }
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    engine
        .run_command(&s.id, &format!("echo second-line >> {dir}/app.log"), false)
        .await
        .unwrap();
    for _ in 0..30 {
        if got.lock().unwrap().contains("second-line") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let text = got.lock().unwrap().clone();
    assert!(text.contains("first") && text.contains("second-line"), "{text}");
    engine.stop_logs(&lid).await.unwrap();

    // --- deploy: origin repo + checkout on the "server" ---
    let git = |args: &str| format!("git -c user.email=it@orbital -c user.name=it {args}");
    let setup = format!(
        "cd {dir} && git init -q --bare origin.git && git clone -q origin.git app 2>/dev/null && cd app && git checkout -q -b main && echo v1 > VERSION && {} && {} && git push -q origin main",
        git("add VERSION"),
        git("commit -q -m v1")
    );
    let o = engine.run_command(&s.id, &setup, false).await.unwrap();
    assert!(o.ok(), "{o:?}");
    let app = engine
        .save_app(
            &p.id,
            None,
            AppInput {
                name: "IT app".into(),
                path: format!("{dir}/app"),
                branch: "main".into(),
                build: "cat VERSION > built.txt".into(),
                restart_kind: RestartKind::Command,
                restart_target: Some("touch restarted".into()),
            },
        )
        .unwrap();
    let v1 = engine.app_checkout(&s.id, &app.id).await.unwrap().head.unwrap();
    // A new commit arrives in origin from elsewhere.
    let push = format!(
        "cd {dir} && git clone -q -b main origin.git other && cd other && echo v2 > VERSION && {} && git push -q origin main",
        git("commit -qam v2")
    );
    assert!(engine.run_command(&s.id, &push, false).await.unwrap().ok());

    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let ev2 = events.clone();
    let rec = engine
        .run_deploy(&s.id, &app.id, None, move |ev| ev2.lock().unwrap().push(format!("{ev:?}")))
        .await
        .unwrap();
    assert_eq!(rec.outcome, "success", "{:?}", events.lock().unwrap());
    assert_eq!(rec.summary.as_deref(), Some("1 new commit"));
    let built = engine.read_file(&s.id, &format!("{dir}/app/built.txt")).await.unwrap().content;
    assert_eq!(built.trim(), "v2");
    assert!(engine
        .list_dir(&s.id, &format!("{dir}/app"))
        .await
        .unwrap()
        .iter()
        .any(|f| f.name == "restarted"));

    // Roll back to v1.
    let back = engine.run_deploy(&s.id, &app.id, Some(v1.clone()), |_| {}).await.unwrap();
    assert_eq!(back.outcome, "success");
    assert_eq!(
        engine.app_checkout(&s.id, &app.id).await.unwrap().head.as_deref(),
        Some(v1.as_str())
    );
    assert_eq!(
        engine
            .read_file(&s.id, &format!("{dir}/app/built.txt"))
            .await
            .unwrap()
            .content
            .trim(),
        "v1"
    );

    // Local changes on the server block a deploy instead of being overwritten.
    engine
        .run_command(&s.id, &format!("echo hacked > {dir}/app/VERSION"), false)
        .await
        .unwrap();
    let blocked = engine.run_deploy(&s.id, &app.id, None, |_| {}).await;
    assert!(matches!(blocked, Err(Error::PreflightFailed { .. })), "{blocked:?}");
    assert_eq!(engine.deploy_history(&app.id).unwrap().len(), 2);

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    for a in ["tunnel.open", "tunnel.close", "deploy.run", "deploy.rollback"] {
        assert!(log.iter().any(|ev| ev.action == a), "{a} audited");
    }
    engine.run_command(&s.id, &format!("rm -rf {dir}"), false).await.unwrap();
    engine.disconnect(&s.id).await.unwrap();
}

/// "New website": the server block is written through safe save, enabled,
/// tested with `nginx -t` and served; a block nginx rejects leaves nothing
/// behind. Needs nginx installed and running and sudo; it writes under
/// /etc/nginx and /var/www, so opt in with ORBITAL_IT_NGINX=1 (CI does).
#[tokio::test]
async fn new_website_server_block_is_written_validated_and_rolled_back() {
    use orbital_core::website::{SiteKind, WebsiteInput};
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NGINX").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_NGINX=1 on a disposable machine with nginx");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let domain = format!("orbital-it-{}.test", std::process::id());
    let root = format!("/var/www/{domain}");
    let input = WebsiteInput {
        domain: domain.clone(),
        include_www: true,
        site: SiteKind::Static { root: root.clone() },
    };

    // Bad input is refused before anything is sent.
    let bad = WebsiteInput {
        site: SiteKind::Proxy { port: 80 },
        ..input.clone()
    };
    assert!(matches!(engine.create_website(&s.id, &bad).await, Err(Error::InvalidInput(_))));
    let bad = WebsiteInput {
        site: SiteKind::Static { root: "/etc".into() },
        ..input.clone()
    };
    assert!(matches!(engine.website_plan(&s.id, &bad).await, Err(Error::InvalidInput(_))));

    let plan = engine.website_plan(&s.id, &input).await.unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert!(!plan.root_exists);
    assert_eq!(plan.names, [domain.clone(), format!("www.{domain}")]);

    let rep = engine.create_website(&s.id, &input).await.unwrap();
    assert!(rep.root_created);
    assert_eq!(rep.write.validator.as_deref(), Some("nginx"));
    assert!(rep.reloaded, "{}", rep.reload_output);
    let exists = |p: String| {
        let engine = &engine;
        let sid = s.id.clone();
        async move {
            engine
                .run_command(&sid, &format!("test -e '{p}' || test -L '{p}'"), true)
                .await
                .unwrap()
                .ok()
        }
    };
    assert!(exists(rep.plan.file.clone()).await);
    if let Some(link) = &rep.plan.enabled_link {
        assert!(exists(link.clone()).await);
    }

    // nginx lists it and serves the placeholder page under that name.
    let cfg = engine.nginx_config(&s.id).await.unwrap();
    assert!(cfg.test_ok, "{}", cfg.test_output);
    let site = cfg
        .sites
        .iter()
        .find(|x| x.server_names.contains(&domain))
        .expect("new site in nginx -T");
    assert_eq!(site.root.as_deref(), Some(root.as_str()));
    let page = engine
        .run_command(&s.id, &format!("curl -s -H 'Host: {domain}' http://127.0.0.1/"), false)
        .await
        .unwrap();
    assert!(page.stdout.contains(&format!("{domain} is served by nginx")), "{page:?}");

    // A second time it's refused: the file exists and the name is served.
    let again = engine.website_plan(&s.id, &input).await.unwrap();
    assert!(again.blockers.len() >= 2, "{:?}", again.blockers);
    assert!(matches!(engine.create_website(&s.id, &input).await, Err(Error::InvalidInput(_))));

    // A block nginx rejects is taken back: neither file nor link remains.
    let (file2, link2) = (
        format!("{}-broken", rep.plan.file),
        rep.plan.enabled_link.as_ref().map(|l| format!("{l}-broken")),
    );
    let session = engine.session(&s.id).await.unwrap();
    let r = session
        .install_site(&file2, link2.as_deref(), "server { this is not nginx }\n")
        .await;
    assert!(matches!(r, Err(Error::ValidationFailed { .. })), "{r:?}");
    assert!(!exists(file2.clone()).await);
    if let Some(l) = link2 {
        assert!(!exists(l).await);
    }
    assert!(engine.nginx_config(&s.id).await.unwrap().test_ok);

    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(log
        .iter()
        .any(|ev| ev.action == "website.create" && ev.outcome == Outcome::Success && ev.target.as_deref() == Some(domain.as_str())));
    assert!(log.iter().any(|ev| ev.action == "website.create" && ev.outcome == Outcome::Failure));

    // Clean up.
    let link = rep.plan.enabled_link.clone().unwrap_or_default();
    engine
        .run_command(
            &s.id,
            &format!("rm -rf -- '{}' '{link}' '{root}'; nginx -s reload", rep.plan.file),
            true,
        )
        .await
        .unwrap();
    engine.disconnect(&s.id).await.unwrap();
}

/// authorized_keys: a new key is added and removed again, a bad key never
/// reaches the server, and the key this connection signs in with can't be
/// removed without `force`.
#[tokio::test]
async fn authorized_keys_add_remove_and_lockout_guard() {
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;

    let before = engine.authorized_keys(&s.id).await.unwrap();
    assert!(!before.keys.is_empty(), "{before:?}");
    // The key this test connects with has its .pub beside it, so it counts as this computer's.
    let own = before.keys.iter().find(|k| k.on_this_computer).expect("the login key is marked");

    let dir = tempfile::tempdir().unwrap();
    let new = orbital_core::sshkeys::generate(dir.path(), "it_extra", "orbital-it", None).unwrap();
    assert!(matches!(
        engine.add_authorized_key(&s.id, "ssh-ed25519 AAAA nope").await,
        Err(Error::InvalidInput(_))
    ));
    let added = engine.add_authorized_key(&s.id, &new.public_line).await.unwrap();
    assert!(added
        .keys
        .iter()
        .any(|k| k.fingerprint.as_deref() == Some(new.fingerprint.as_str())));
    assert!(
        matches!(
            engine.add_authorized_key(&s.id, &new.public_line).await,
            Err(Error::InvalidInput(_))
        ),
        "twice"
    );
    let line = added.keys.iter().find(|k| k.comment == "orbital-it").unwrap().line.clone();
    let after = engine.remove_authorized_key(&s.id, &line, false).await.unwrap();
    assert_eq!(after.keys.len(), before.keys.len());

    // Removing our own login key would lock this connection's way in: refused.
    let r = engine.remove_authorized_key(&s.id, &own.line, false).await;
    assert!(matches!(r, Err(Error::PreflightFailed { .. })), "{r:?}");
    assert_eq!(engine.authorized_keys(&s.id).await.unwrap().keys.len(), before.keys.len());

    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    for action in ["sshkey.authorize", "sshkey.revoke"] {
        assert!(
            audit.iter().any(|ev| ev.action == action && ev.outcome == Outcome::Success),
            "{action}"
        );
    }
    assert!(audit.iter().any(|ev| ev.action == "sshkey.revoke" && ev.outcome == Outcome::Denied));
    engine.disconnect(&s.id).await.unwrap();
}

/// A snippet runs on the server with its parameter as one quoted word, and
/// each run is audited with the command that was sent.
#[tokio::test]
async fn snippet_runs_with_quoted_parameters() {
    use orbital_core::snippets::SnippetInput;
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let snip = engine
        .save_snippet(
            None,
            &SnippetInput {
                name: "echo".into(),
                command: "printf '%s\\n' {{text}}".into(),
                description: None,
                sudo: false,
            },
        )
        .unwrap();
    let tricky = "a b; echo INJECTED $(id) 'q'";
    let values = [("text".to_string(), tricky.to_string())].into_iter().collect();
    let runs = engine.run_snippet(&snip.id, std::slice::from_ref(&s.id), &values).await.unwrap();
    let out = runs[0].output.as_ref().expect("ran");
    assert_eq!(out.stdout.trim_end(), tricky, "the value arrived as one argument");
    let missing = engine.run_snippet(&snip.id, std::slice::from_ref(&s.id), &Default::default()).await;
    assert!(matches!(missing, Err(Error::InvalidInput(_))));
    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(audit
        .iter()
        .any(|ev| ev.action == "snippet.run" && ev.command.as_deref().is_some_and(|c| c.contains("'a b; echo"))));
    engine.disconnect(&s.id).await.unwrap();
}

/// Each safe save keeps the previous version; it can be read back and put
/// back, and the restore keeps a backup of what it replaced.
#[tokio::test]
async fn config_backups_are_listed_read_and_restored() {
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let dir = format!("/tmp/orbital-it-bak-{}", std::process::id());
    engine.mkdir(&s.id, &dir).await.unwrap();
    let cfg = format!("{dir}/app.conf");
    engine.write_file(&s.id, &cfg, "v1\n", None).await.unwrap();
    engine.safe_write(&s.id, &cfg, "v2\n", false, false, None).await.unwrap();
    // Backup names have one-second resolution: make the next one distinct.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    engine.safe_write(&s.id, &cfg, "v3\n", false, false, None).await.unwrap();

    let backups = engine.list_backups(&s.id, &cfg, false).await.unwrap();
    assert_eq!(backups.len(), 2, "{backups:?}");
    let oldest = backups.last().unwrap();
    assert_eq!(engine.read_backup(&s.id, &cfg, oldest, false).await.unwrap(), "v1\n");
    assert!(matches!(
        engine.read_backup(&s.id, &cfg, "/etc/passwd", false).await,
        Err(Error::InvalidInput(_))
    ));

    tokio::time::sleep(Duration::from_millis(1100)).await;
    engine.restore_backup(&s.id, &cfg, oldest, false).await.unwrap();
    assert_eq!(engine.read_file(&s.id, &cfg).await.unwrap().content, "v1\n");
    let after = engine.list_backups(&s.id, &cfg, false).await.unwrap();
    assert_eq!(
        engine.read_backup(&s.id, &cfg, &after[0], false).await.unwrap(),
        "v3\n",
        "the restore kept v3"
    );
    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(audit
        .iter()
        .any(|ev| ev.action == "config.restore" && ev.outcome == Outcome::Success));
    engine.remove(&s.id, &dir, true).await.unwrap();
    engine.disconnect(&s.id).await.unwrap();
}

/// A job sees its progress: the lines a command prints (inside
/// `with_output`), nothing from commands outside it, and a transfer's files
/// and bytes.
#[tokio::test]
async fn job_progress_streams_lines_and_transfers() {
    use orbital_core::progress::{self, ProgressEvent};
    use std::sync::{Arc, Mutex};
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let b = engine.connect(&p.id, ConnectOptions::default()).await.unwrap();

    let seen = Arc::new(Mutex::new(Vec::<ProgressEvent>::new()));
    let sink: progress::Sink = {
        let seen = seen.clone();
        Arc::new(move |ev| seen.lock().unwrap().push(ev))
    };
    let lines = |seen: &Mutex<Vec<ProgressEvent>>| {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                ProgressEvent::Line { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    // Lines arrive as printed, from stdout and stderr, with escapes removed.
    progress::scope(Some(sink.clone()), async {
        engine.run_command(&a.id, "echo quiet", false).await.unwrap();
        progress::with_output(engine.run_command(&a.id, "printf 'one\\n'; printf '\\033[1mtwo\\033[0m\\n' >&2; printf 'three'", false))
            .await
            .unwrap();
    })
    .await;
    assert_eq!(lines(&seen), ["one", "two", "three"], "only the command inside with_output streams");

    // A server-to-server copy reports a step and its files and bytes.
    seen.lock().unwrap().clear();
    let src = format!("{}/orbital-it-job-{}", a.home, std::process::id());
    let dst = format!("{}/orbital-it-job-copy-{}", b.home, std::process::id());
    engine
        .run_command(
            &a.id,
            &format!("mkdir -p {src} && head -c 700000 /dev/zero > {src}/a.bin && echo hi > {src}/b.txt"),
            false,
        )
        .await
        .unwrap();
    let rep = progress::scope(Some(sink), engine.copy_between(&a.id, &src, &b.id, &dst, false))
        .await
        .unwrap();
    assert_eq!((rep.files, rep.bytes), (2, 700_003));
    let ev = seen.lock().unwrap().clone();
    assert!(matches!(ev.first(), Some(ProgressEvent::Step { text }) if text.starts_with("Copying ")));
    match ev.iter().rev().find(|e| matches!(e, ProgressEvent::Transfer { .. })) {
        Some(ProgressEvent::Transfer { files, bytes, .. }) => assert_eq!((*files, *bytes), (2, 700_003), "the last report is the total"),
        _ => panic!("no transfer progress: {ev:?}"),
    }
    engine.run_command(&a.id, &format!("rm -rf {src} {dst}"), false).await.unwrap();
}

/// The terminal's suggestions read the server's shell history: newest first,
/// once each, secrets left out. Writes history only for CI's own test user.
#[tokio::test]
async fn shell_history_for_suggestions() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    // Whatever is there parses (possibly nothing).
    engine.shell_history(&a.id, false).await.unwrap();
    if e.user != "orbital" {
        return; // don't touch a real user's history
    }
    engine
        .run_command(
            &a.id,
            "cp -p ~/.bash_history /tmp/orbital-it-hist 2>/dev/null; printf 'ls -la\\nexport APP_PASSWORD=x\\nsystemctl status nginx\\nls -la\\n' >> ~/.bash_history",
            false,
        )
        .await
        .unwrap();
    let h = engine.shell_history(&a.id, false).await.unwrap();
    assert_eq!(&h[..2], ["ls -la", "systemctl status nginx"]);
    assert!(!h.iter().any(|c| c.contains("PASSWORD")));
    engine
        .run_command(
            &a.id,
            "if [ -f /tmp/orbital-it-hist ]; then mv /tmp/orbital-it-hist ~/.bash_history; else rm -f ~/.bash_history; fi",
            false,
        )
        .await
        .unwrap();
}

/// Stopping a job ends its command on the server, not just the wait for it.
#[tokio::test]
async fn stopping_a_job_ends_its_command_on_the_server() {
    use orbital_core::progress;
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = std::sync::Arc::new(engine());
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let marker = format!("sleep {}", 4000 + std::process::id() % 1000);

    let started = std::time::Instant::now();
    let job = {
        let (engine, sid, cmd) = (engine.clone(), a.id.clone(), marker.clone());
        tokio::spawn(async move { progress::run_job(4242, None, engine.run_command(&sid, &cmd, false)).await })
    };
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(progress::cancel(4242));
    let r = job.await.unwrap();
    assert!(matches!(r, Err(orbital_core::error::Error::Cancelled)), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(8), "stopped promptly");

    tokio::time::sleep(Duration::from_millis(500)).await;
    let left = engine
        .run_command(&a.id, &format!("pgrep -f '^{marker}$' || true"), false)
        .await
        .unwrap();
    assert_eq!(left.stdout.trim(), "", "the command was ended on the server");
}

/// Updates: list what can be upgraded; in CI (root via sudo) also refresh the
/// package list and see the refresh recorded. Installing isn't run here.
#[tokio::test]
async fn updates_list_and_refresh() {
    let Some(e) = env() else { return };
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let r = engine.updates(&s.id).await.unwrap();
    if r.manager.is_none() {
        return; // not apt/dnf/yum (macOS)
    }
    if std::env::var("ORBITAL_IT_USERS").is_err() {
        return;
    }
    let after = engine.refresh_updates(&s.id).await.unwrap();
    assert_eq!(after.manager, r.manager);
    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(
        log.iter().any(|ev| ev.action == "updates.refresh"),
        "the refresh is in the activity log"
    );
}
