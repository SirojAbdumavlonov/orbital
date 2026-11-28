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
    // Go to Folder tells a folder from a file or nothing by listing: both of those are errors.
    assert!(engine.list_dir(&sid, "/etc").await.is_ok());
    assert!(engine.list_dir(&sid, "/etc/hostname").await.is_err(), "a file isn't a folder");
    assert!(engine.list_dir(&sid, "/no/such/folder").await.is_err());
    // A second window attaches to the same connection by its id.
    assert_eq!(engine.session_info(&sid).await.unwrap().profile_id, p.id);

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
        // A plain save of a root-owned file goes through sudo, in place: still root's.
        engine.write_file(&sid, root_cfg, "x\n", None).await.unwrap();
        let owner = engine.run_command(&sid, &format!("stat -c %U {root_cfg}"), false).await.unwrap();
        assert_eq!(owner.stdout.trim(), "root");
        let rep = engine
            .safe_write(&sid, root_cfg, "mode=from-orbital\n", true, true, None)
            .await
            .unwrap();
        assert!(rep.privileged && rep.backup_path.is_some());
        let owner = engine.run_command(&sid, &format!("stat -c %U {root_cfg}"), false).await.unwrap();
        assert_eq!(owner.stdout.trim(), "root");
        assert_eq!(engine.read_file(&sid, root_cfg).await.unwrap().content, "mode=from-orbital\n");
    }

    // --- the Files tab in a folder only root can write: every action falls back to sudo ---
    let rootdir = "/opt/orbital-it-root";
    engine
        .run_command(
            &sid,
            &format!(
                "rm -rf {rootdir}; mkdir -p {rootdir}/locked && echo secret > {rootdir}/locked/inside.txt && chown -R root:root {rootdir} && chmod 755 {rootdir} && chmod 700 {rootdir}/locked && chmod 644 {rootdir}/locked/inside.txt"
            ),
            true,
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(
        !engine.run_command(&sid, &format!("test -w {rootdir}"), false).await.unwrap().ok(),
        "the login user must not be able to write in {rootdir} for this test"
    );
    engine.create_file(&sid, &format!("{rootdir}/new.txt")).await.unwrap();
    assert!(
        engine.create_file(&sid, &format!("{rootdir}/new.txt")).await.is_err(),
        "no silent overwrite"
    );
    engine
        .write_file(&sid, &format!("{rootdir}/new.txt"), "hello\n", None)
        .await
        .unwrap();
    engine.mkdir(&sid, &format!("{rootdir}/sub")).await.unwrap();
    engine
        .rename(&sid, &format!("{rootdir}/new.txt"), &format!("{rootdir}/sub/renamed.txt"))
        .await
        .unwrap();
    engine.chmod(&sid, &format!("{rootdir}/sub/renamed.txt"), 0o640).await.unwrap();
    let listed = engine.list_dir(&sid, &format!("{rootdir}/locked")).await.unwrap();
    assert_eq!(
        listed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["inside.txt"],
        "unreadable folder listed as root"
    );
    assert_eq!(
        engine
            .read_file(&sid, &format!("{rootdir}/locked/inside.txt"))
            .await
            .unwrap()
            .content,
        "secret\n"
    );
    let local_dl = std::env::temp_dir().join("orbital-it-dl.txt");
    assert_eq!(
        engine
            .download(&sid, &format!("{rootdir}/locked/inside.txt"), &local_dl)
            .await
            .unwrap(),
        7
    );
    let local_up = std::env::temp_dir().join("orbital-it-up");
    std::fs::create_dir_all(local_up.join("a")).unwrap();
    std::fs::write(local_up.join("a/b.txt"), "up\n").unwrap();
    engine.upload(&sid, &local_up, &format!("{rootdir}/uploaded")).await.unwrap();
    assert_eq!(
        engine
            .read_file(&sid, &format!("{rootdir}/uploaded/a/b.txt"))
            .await
            .unwrap()
            .content,
        "up\n"
    );
    let ls = engine
        .run_command(
            &sid,
            // As root: the file is now 640 root:root, so the login user can't read it.
            &format!("stat -c '%U %a' {rootdir}/sub/renamed.txt; cat {rootdir}/sub/renamed.txt; ls /tmp | grep -c orbital-upload || true"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(ls.stdout, "root 640\nhello\n0\n", "{ls:?}");
    engine.remove(&sid, &format!("{rootdir}/sub/renamed.txt"), false).await.unwrap();
    engine.remove(&sid, &format!("{rootdir}/sub"), false).await.unwrap();
    engine.remove(&sid, rootdir, true).await.unwrap();
    assert!(!engine.run_command(&sid, &format!("test -e {rootdir}"), false).await.unwrap().ok());

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
    // Each sample also lands in the monitoring history (a point a minute).
    let now = chrono::Utc::now().timestamp();
    let (_, hist) = engine.metric_history(&s.profile_id, now - 3600, now + 60, 720).unwrap();
    assert!(!hist.is_empty() && hist.last().unwrap().uptime.unwrap_or(0) > 0, "{hist:?}");
    // And that minute's busiest programs (names only) and every disk.
    let moment = engine
        .metric_moment(&s.profile_id, hist.last().unwrap().t, 120)
        .unwrap()
        .expect("the minute was recorded");
    assert!(
        !moment.procs.is_empty() && moment.procs.iter().all(|p| !p.name.contains(' ')),
        "{moment:?}"
    );
    assert!(!moment.disks.is_empty(), "{moment:?}");

    // --- terminal in a given directory ---
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let b2 = buf.clone();
    let tid = engine
        .open_terminal(&sid, 120, 30, Some(dir.clone()), false, Default::default(), move |ev| {
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
                                                  // A root-owned file saved without sudo selected goes through sudo now, so it isn't refused.
    assert!(!has("file.write", Outcome::Denied));
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

    // Protect SSH: refused without keeping this computer out (lock-out check), then
    // applied, checked by fail2ban, read back, and in effect; bad input changes nothing.
    use orbital_core::fail2ban::SshProtection;
    let risky = SshProtection {
        ignore_self: false,
        ..Default::default()
    };
    assert!(matches!(
        engine.fail2ban_protect(&s.id, &risky, false).await,
        Err(Error::PreflightFailed { .. })
    ));
    let p = SshProtection {
        max_retry: 7,
        find_time_mins: 20,
        ban_time_mins: 120,
        ..Default::default()
    };
    let st = engine.fail2ban_protect(&s.id, &p, false).await.unwrap();
    let sshd = st.jails.iter().find(|j| j.name == "sshd").expect("sshd jail");
    assert_eq!(sshd.maxretry.as_deref(), Some("7"));
    assert_eq!(sshd.findtime.as_deref(), Some("1200"));
    assert!(st.jails.iter().any(|j| j.name == "recidive"), "{st:?}");
    let back = engine.fail2ban_protection(&s.id).await.unwrap().unwrap();
    assert_eq!((back.max_retry, back.find_time_mins, back.ban_time_mins), (7, 20, 120));
    assert!(engine
        .fail2ban_protect(
            &s.id,
            &SshProtection {
                max_retry: 1,
                ..Default::default()
            },
            false
        )
        .await
        .is_err());
    engine
        .run_command(
            &s.id,
            "rm -f /etc/fail2ban/jail.d/zz-orbital-sshd.local && fail2ban-client reload",
            true,
        )
        .await
        .unwrap();

    // More jails: nginx's auth jail (nginx is installed for these tests) turned on with
    // its limits, read back, then removed; a jail Orbital doesn't know is refused.
    use orbital_core::fail2ban::JailSetting;
    let opts = engine.fail2ban_jail_options(&s.id).await.unwrap();
    if opts.iter().any(|o| o.name == "nginx-http-auth" && o.available) {
        let j = JailSetting {
            name: "nginx-http-auth".into(),
            enabled: true,
            max_retry: 4,
            find_time_mins: 15,
            ban_time_mins: 30,
        };
        let st = engine.fail2ban_set_jails(&s.id, std::slice::from_ref(&j)).await.unwrap();
        let nj = st.jails.iter().find(|x| x.name == "nginx-http-auth").expect("jail running");
        assert_eq!(nj.maxretry.as_deref(), Some("4"));
        let back = engine.fail2ban_jail_options(&s.id).await.unwrap();
        assert_eq!(
            back.iter().find(|o| o.name == "nginx-http-auth").unwrap().setting.as_ref(),
            Some(&j)
        );
        engine
            .run_command(
                &s.id,
                "rm -f /etc/fail2ban/jail.d/zz-orbital-jails.local && fail2ban-client reload",
                true,
            )
            .await
            .unwrap();
    }
    let bad = JailSetting {
        name: "sshd".into(),
        enabled: false,
        max_retry: 5,
        find_time_mins: 10,
        ban_time_mins: 60,
    };
    assert!(engine.fail2ban_set_jails(&s.id, &[bad]).await.is_err());

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
    // A log time range that isn't a date never reaches the server.
    let r = engine.container_logs(&s.id, "web", 100, Some("$(id)"), None).await;
    assert!(matches!(r, Err(Error::InvalidInput(_))), "{r:?}");
    // Installing refuses anything outside the fixed list; moving a container needs another server.
    assert!(matches!(
        engine.install_software(&s.id, "x; reboot", false).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine.move_preview(&s.id, "anything", &s.id, &Default::default()).await,
        Err(Error::InvalidInput(_))
    ));
    // Network checks from the server: itself answers on its SSH port; bad targets never run.
    use orbital_core::netcheck::NetCheck;
    let r = engine.net_check(&s.id, NetCheck::Port, "127.0.0.1", Some(e.port)).await.unwrap();
    assert!(r.ok, "{r:?}");
    let r = engine.net_check(&s.id, NetCheck::Dns, "localhost", None).await.unwrap();
    assert!(!r.output.is_empty(), "{r:?}");
    assert!(matches!(
        engine.net_check(&s.id, NetCheck::Ping, "-x; reboot", None).await,
        Err(Error::InvalidInput(_))
    ));
    // A unit's journal over the last hour reads without error (empty or "no journalctl" is fine), and a bad unit never reaches the server.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    engine
        .unit_logs(&s.id, "ssh.service", 50, Some(now - 3600), Some(now))
        .await
        .unwrap();
    let r = engine.unit_logs(&s.id, "x;id", 50, None, None).await;
    assert!(matches!(r, Err(Error::InvalidInput(_))), "{r:?}");
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
        fine(engine.list_databases(&s.id, kind, None).await.map(|_| ()));
    }
    fine(engine.redis_info(&s.id, None).await.map(|_| ()));
    fine(engine.apache_status(&s.id, None).await.map(|_| ()));
    // A bad container name never reaches the server.
    let r = engine.list_databases(&s.id, DbKind::Postgres, Some("db; reboot")).await;
    assert!(matches!(r, Err(Error::InvalidInput(_))), "{r:?}");
    fine(engine.haproxy_status(&s.id).await.map(|_| ()));

    let local = std::env::temp_dir().join("orbital-it-never.dump");
    let r = engine
        .backup_database(&s.id, DbKind::Postgres, "x; reboot", BackupFormat::Native, &local)
        .await;
    assert!(matches!(r, Err(Error::InvalidInput(_))), "{r:?}");
    assert!(!local.exists());
    assert!(engine.redis_action(&s.id, RedisAction::FlushDb(99_999), None).await.is_err());

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
        .open_terminal(&s.id, 120, 30, Some("/tmp".into()), true, Default::default(), move |ev| {
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
    // Nothing listens there: refused when opened, in plain words, instead of failing later in a browser.
    match engine.open_tunnel(&s.id, "127.0.0.1", 1, None, None).await {
        Err(Error::InvalidInput(m)) => assert!(m.contains("nothing answers on 127.0.0.1:1"), "{m}"),
        other => panic!("{other:?}"),
    }
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
    // A certificate for this name may only change the site's own file (never nginx's default);
    // a name no site serves gets the certificate alone.
    let own = orbital_core::certbot::nginx_install_targets(&cfg, std::slice::from_ref(&domain)).unwrap();
    assert!(
        own.len() == 1 && own[0].ends_with(&domain) && !orbital_core::certbot::is_nginx_default_file(&own[0]),
        "{own:?}"
    );
    assert!(orbital_core::certbot::nginx_install_targets(&cfg, &["nobody-serves-this.test".to_string()]).is_err());
    // nginx applies a reload in the background: give it a few seconds to serve the new site.
    let page = engine
        .run_command(
            &s.id,
            &format!("for i in 1 2 3 4 5 6 7 8 9 10; do o=$(curl -s -H 'Host: {domain}' http://127.0.0.1/); case \"$o\" in *'is served by nginx'*) break;; esac; sleep 0.5; done; printf '%s' \"$o\""),
            false,
        )
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

    // Test: nginx -t passes and the server serves the site under its name (internet: not checked in CI).
    let t = engine.website_test(&s.id, &domain, false).await.unwrap();
    assert!(t.config_ok, "{}", t.config_output);
    assert!(t.server.status.is_some_and(|c| c == 200 || c == 301), "{:?}", t.server);

    // Routes: a redirect added to the site's server block is written to its real file
    // (not the sites-enabled link), passes nginx -t, is served after the reload, and
    // removing it gives the file back as it was.
    {
        use orbital_core::routes::{Route, RouteChange, RouteTarget};
        let cfg = engine.nginx_config(&s.id).await.unwrap();
        let site = cfg.sites.iter().find(|x| x.server_names.contains(&domain)).unwrap();
        let view = engine.nginx_routes(&s.id, &site.file, site.line).await.unwrap();
        assert!(
            view.file.ends_with(&domain) && !view.file.contains("sites-enabled"),
            "{}",
            view.file
        );
        assert_eq!(view.routes.len(), 1, "{:?}", view.routes);
        let original = engine.run_command(&s.id, &format!("cat {}", view.file), true).await.unwrap().stdout;
        let redirect = Route {
            path: "/old".into(),
            exact: true,
            target: RouteTarget::Redirect {
                to: "/new".into(),
                code: 301,
            },
        };
        let add = RouteChange::Add { route: redirect };
        let preview = engine.nginx_route_change(&s.id, &site.file, site.line, &add, false).await.unwrap();
        assert!(!preview.applied && preview.after.contains("location = /old"));
        let done = engine.nginx_route_change(&s.id, &site.file, site.line, &add, true).await.unwrap();
        assert!(done.applied && done.reloaded && done.backup_path.is_some(), "{done:?}");
        let head = engine
            .run_command(
                &s.id,
                &format!("for i in 1 2 3 4 5 6 7 8 9 10; do o=$(curl -s -o /dev/null -w '%{{http_code}} %{{redirect_url}}' -H 'Host: {domain}' http://127.0.0.1/old); case \"$o\" in 301*) break;; esac; sleep 0.5; done; printf '%s' \"$o\""),
                false,
            )
            .await
            .unwrap()
            .stdout;
        assert!(head.starts_with("301") && head.ends_with("/new"), "{head}");
        engine
            .nginx_route_change(&s.id, &site.file, site.line, &RouteChange::Remove { spec: "= /old".into() }, true)
            .await
            .unwrap();
        let back = engine.run_command(&s.id, &format!("cat {}", view.file), true).await.unwrap().stdout;
        assert_eq!(back, original, "adding and removing a route leaves the file as it was");
        // nginx's own files are refused.
        assert!(engine.nginx_routes(&s.id, "/etc/nginx/nginx.conf", 1).await.is_err());
        let log = engine
            .audit_query(&AuditFilter {
                limit: Some(500),
                ..Default::default()
            })
            .unwrap();
        assert!(log.iter().any(|ev| ev.action == "nginx.route" && ev.outcome == Outcome::Success));
    }

    // Remove it (and its folder): the file and its link are gone, a backup is kept, nginx still passes.
    let backup = engine
        .remove_website(&s.id, &rep.plan.file, None, Some(root.as_str()))
        .await
        .unwrap();
    assert!(backup.starts_with("/var/backups/orbital-nginx/"), "{backup}");
    assert!(!exists(rep.plan.file.clone()).await);
    if let Some(l) = &rep.plan.enabled_link {
        assert!(!exists(l.clone()).await);
    }
    assert!(!exists(root.clone()).await);
    assert!(engine.nginx_config(&s.id).await.unwrap().test_ok);
    assert!(engine
        .audit_query(&AuditFilter::default())
        .unwrap()
        .iter()
        .any(|ev| ev.action == "website.remove" && ev.outcome == Outcome::Success));
    // Only nginx site files can be removed.
    assert!(matches!(
        engine.remove_website(&s.id, "/etc/nginx/nginx.conf", None, None).await,
        Err(Error::InvalidInput(_))
    ));

    engine
        .run_command(&s.id, &format!("rm -f '{backup}'; nginx -s reload"), true)
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
    let runs = engine
        .run_snippet(&snip.id, std::slice::from_ref(&s.id), &values, false)
        .await
        .unwrap();
    let out = runs[0].output.as_ref().expect("ran");
    assert_eq!(out.stdout.trim_end(), tricky, "the value arrived as one argument");
    assert!(!runs[0].needs_sudo);
    let missing = engine
        .run_snippet(&snip.id, std::slice::from_ref(&s.id), &Default::default(), false)
        .await;
    assert!(matches!(missing, Err(Error::InvalidInput(_))));
    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(audit
        .iter()
        .any(|ev| ev.action == "snippet.run" && ev.command.as_deref().is_some_and(|c| c.contains("'a b; echo"))));

    // Refused as the login user: offered again with sudo, which then works (only if this login isn't root).
    let sudo_ok = engine.run_command(&s.id, "true", true).await.is_ok_and(|o| o.ok());
    if e.user != "root" && sudo_ok {
        let root_only = engine
            .save_snippet(
                None,
                &SnippetInput {
                    name: "root only".into(),
                    command: "touch /root/.orbital-it-snippet && rm -f /root/.orbital-it-snippet".into(),
                    description: None,
                    sudo: false,
                },
            )
            .unwrap();
        let runs = engine
            .run_snippet(&root_only.id, std::slice::from_ref(&s.id), &Default::default(), false)
            .await
            .unwrap();
        assert!(runs[0].needs_sudo, "{:?}", runs[0]);
        let again = engine
            .run_snippet(&root_only.id, std::slice::from_ref(&s.id), &Default::default(), true)
            .await
            .unwrap();
        assert!(
            again[0].output.as_ref().is_some_and(|o| o.ok()) && !again[0].needs_sudo,
            "{:?}",
            again[0]
        );
    }
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

/// A tmux terminal keeps running after it is closed, and is listed to reattach.
#[tokio::test]
async fn tmux_session_survives_closing_the_terminal() {
    use orbital_core::terminal::Launch;
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let has = engine.run_command(&a.id, "command -v tmux || true", false).await.unwrap();
    if has.stdout.trim().is_empty() {
        return; // no tmux on this machine
    }
    let name = format!("orbital-it-{}", std::process::id());
    let tid = engine
        .open_terminal(&a.id, 100, 30, None, false, Launch::Tmux { session: name.clone() }, |_| {})
        .await
        .unwrap();
    let mut listed = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if engine.tmux_sessions(&a.id, false).await.unwrap().iter().any(|s| s.name == name) {
            listed = true;
            break;
        }
    }
    assert!(listed, "the tmux session started");
    engine.terminal_close(&tid).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = engine.tmux_sessions(&a.id, false).await.unwrap();
    assert!(
        after.iter().any(|s| s.name == name && !s.attached),
        "still there, detached: {after:?}"
    );
    engine
        .run_command(&a.id, &format!("tmux kill-session -t {name}"), false)
        .await
        .unwrap();
}

/// Disk cleanup scans without changing anything; Stop process ends a process
/// and refuses PID 1.
#[tokio::test]
async fn cleanup_scan_and_stop_process() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() {
        return; // needs sudo with a password (CI)
    }
    let engine = engine();
    let a = sudo_session(&engine, &e).await;
    let items = engine.cleanup_scan(&a.id).await.unwrap();
    assert!(items.iter().any(|i| i.id == "oldlogs"), "{items:?}");

    let out = engine
        .run_command(&a.id, "nohup sleep 300 >/dev/null 2>&1 & echo $!", false)
        .await
        .unwrap();
    let pid: u32 = out.stdout.trim().parse().unwrap();
    engine.stop_process(&a.id, pid, false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let alive = engine
        .run_command(&a.id, &format!("kill -0 {pid} 2>/dev/null && echo yes || echo no"), false)
        .await
        .unwrap();
    assert_eq!(alive.stdout.trim(), "no");
    assert!(matches!(engine.stop_process(&a.id, 1, true).await, Err(Error::InvalidInput(_))));
}

/// The AI agent's server side, with a fake bot token (Telegram isn't
/// reachable with it, so the agent keeps retrying while running): Python and
/// the AI libraries install, the config is root-only, the service runs,
/// pairing codes are taken, status hides the keys, and remove leaves nothing.
#[tokio::test]
async fn ai_agent_installs_pairs_and_removes() {
    use base64::Engine as _;
    use orbital_core::agent as a;
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() {
        return; // needs root via sudo and systemd (CI)
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    assert!(!engine.agent_status(&s.id).await.unwrap().installed);

    let root = |cmd: String| {
        let engine = &engine;
        let sid = s.id.clone();
        async move {
            let o = engine.run_command(&sid, &cmd, true).await.unwrap();
            assert_eq!(o.exit_code, 0, "{cmd}\n{}\n{}", o.stdout, o.stderr);
            o
        }
    };
    let b64 = |t: &str| base64::engine::general_purpose::STANDARD.encode(t);
    root(a::PREPARE_SCRIPT.to_string()).await;
    root(format!(
        "echo {} | base64 -d | sh -c {}",
        b64(a::AGENT_PY),
        orbital_core::shell::quote(a::WRITE_AGENT)
    ))
    .await;
    let setup = a::AgentSetup {
        telegram_token: "123456789:AAH-fakefakefakefakefakefakefakefake".into(),
        provider: a::Provider::Claude,
        api_key: "sk-ant-api03-fakefakefakefakefakefakefake".into(),
        model: None,
        alerts: true,
    };
    let cfg = a::config_json(&setup, "orbital_it_bot", "it").unwrap();
    root(format!(
        "echo {} | base64 -d | sh -c {}",
        b64(&cfg),
        orbital_core::shell::quote(a::WRITE_CONFIG)
    ))
    .await;
    let mode = root("stat -c %a /etc/orbital-agent/config.json".into()).await;
    assert_eq!(mode.stdout.trim(), "600", "the keys are readable by root only");
    root(format!(
        "echo {} | base64 -d | sh -c {}",
        b64(a::UNIT),
        orbital_core::shell::quote(a::WRITE_UNIT)
    ))
    .await;
    root("/opt/orbital-agent/venv/bin/python -c 'import anthropic, openai'".into()).await;

    let code = engine.agent_pair_code(&s.id).await.unwrap();
    assert_eq!(code.code.len(), 8);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let st = engine.agent_status(&s.id).await.unwrap();
    assert!(st.installed && st.running, "{st:?}");
    assert_eq!(st.bot.as_deref(), Some("orbital_it_bot"));
    assert_eq!(st.version.as_deref(), Some(a::VERSION));
    assert!(st.pair_code_active && st.owner.is_none());
    assert!(!format!("{st:?}").contains("fakefake"), "status never shows the keys");

    engine.agent_set_alerts(&s.id, false).await.unwrap();
    assert!(!engine.agent_status(&s.id).await.unwrap().alerts);
    // Switching the model (the list itself needs a real key, so it's set directly).
    root(a::set_model_command("claude-sonnet-5-5").unwrap()).await;
    assert_eq!(
        engine.agent_status(&s.id).await.unwrap().model.as_deref(),
        Some("claude-sonnet-5-5")
    );
    // With a fake key the agent's model list reports the provider's refusal.
    assert!(engine.agent_models(&s.id).await.is_err());
    // Daily summary on and off; a test message needs a paired account first.
    engine.agent_set_daily(&s.id, Some("08:30")).await.unwrap();
    assert_eq!(engine.agent_status(&s.id).await.unwrap().daily_report.as_deref(), Some("08:30"));
    engine.agent_set_daily(&s.id, None).await.unwrap();
    assert!(engine.agent_status(&s.id).await.unwrap().daily_report.is_none());
    let e = engine.agent_test(&s.id).await.unwrap_err().to_string();
    assert!(e.contains("not paired"), "{e}");
    // Orbital's alerts through the agent ("agent.py say", the text on stdin): refused until paired.
    let ch = engine
        .save_notify_channel(
            None,
            &orbital_core::notify::ChannelInput {
                name: "Agent".into(),
                enabled: true,
                events: vec![orbital_core::notify::AlertKind::Health],
                config: orbital_core::notify::ChannelConfig::Agent {
                    profile_id: s.profile_id.clone(),
                    server: "test".into(),
                },
                secret: String::new(),
                quiet: None,
            },
        )
        .unwrap();
    let e = engine.test_notify_channel(&ch.id).await.unwrap_err().to_string();
    assert!(e.contains("isn't paired"), "{e}");
    engine.agent_remove(&s.id).await.unwrap();
    let gone = root("ls /opt/orbital-agent /etc/orbital-agent 2>&1 || true".into()).await;
    assert!(gone.stdout.contains("No such file"), "{}", gone.stdout);
    assert!(!engine.agent_status(&s.id).await.unwrap().installed);
}

/// Find files by name below a folder, and download a folder as one .tar.gz.
#[tokio::test]
async fn find_files_and_download_a_folder() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let dir = format!("{}/orbital-it-find-{}", a.home, std::process::id());
    engine
        .run_command(
            &a.id,
            &format!("mkdir -p {dir}/conf.d {dir}/node_modules/x {dir}/.git && echo a > {dir}/conf.d/site.conf && echo b > {dir}/nginx.conf && echo c > {dir}/node_modules/x/nginx.conf && echo d > {dir}/.git/nginx.conf"),
            false,
        )
        .await
        .unwrap();

    let r = engine.find_files(&a.id, &dir, "CONF").await.unwrap();
    let names: Vec<_> = r.files.iter().map(|f| f.path.trim_start_matches(&dir).to_string()).collect();
    assert_eq!(
        names,
        ["/conf.d", "/nginx.conf", "/conf.d/site.conf"],
        "case-insensitive, skips .git and node_modules"
    );
    assert!(r.files[0].dir && !r.files[1].dir && !r.truncated);
    assert!(engine.find_files(&a.id, &dir, "a/b").await.is_err());

    let local = std::env::temp_dir().join(format!("orbital-it-folder-{}.tar.gz", std::process::id()));
    let n = engine.download_folder(&a.id, &dir, &local).await.unwrap();
    assert!(n > 0 && std::fs::metadata(&local).unwrap().len() == n);
    let list = std::process::Command::new("tar").arg("tzf").arg(&local).output().unwrap();
    let listing = String::from_utf8_lossy(&list.stdout);
    let top = dir.rsplit('/').next().unwrap();
    assert!(
        listing.contains(&format!("{top}/conf.d/site.conf")) && listing.contains(&format!("{top}/nginx.conf")),
        "{listing}"
    );
    let _ = std::fs::remove_file(&local);
    assert!(engine.download_folder(&a.id, "/", &local).await.is_err());
    engine.run_command(&a.id, &format!("rm -rf {dir}"), false).await.unwrap();
}

/// Paste in Files on the same server: a file and a folder are copied with their
/// contents and modes, an existing name or a folder into itself is refused,
/// and the copy is in the activity log.
#[tokio::test]
async fn paste_copies_within_the_server() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let dir = format!("{}/orbital-it-paste-{}", a.home, std::process::id());
    engine
        .run_command(
            &a.id,
            &format!("mkdir -p '{dir}/app dir/sub' && echo hi > '{dir}/app dir/sub/a.txt' && printf x > '{dir}/run.sh' && chmod 750 '{dir}/run.sh'"),
            false,
        )
        .await
        .unwrap();

    engine
        .copy_within(&a.id, &format!("{dir}/run.sh"), &format!("{dir}/run copy.sh"))
        .await
        .unwrap();
    engine
        .copy_within(&a.id, &format!("{dir}/app dir"), &format!("{dir}/app dir copy"))
        .await
        .unwrap();
    let out = engine
        .run_command(
            &a.id,
            &format!("cat '{dir}/app dir copy/sub/a.txt'; stat -c %a '{dir}/run copy.sh' 2>/dev/null || stat -f %Lp '{dir}/run copy.sh'"),
            false,
        )
        .await
        .unwrap();
    let text = out.stdout;
    assert!(text.contains("hi") && text.contains("750"), "{text}");

    let exists = engine
        .copy_within(&a.id, &format!("{dir}/run.sh"), &format!("{dir}/run copy.sh"))
        .await;
    assert!(exists.unwrap_err().to_string().contains("already exists"));
    assert!(engine
        .copy_within(&a.id, &format!("{dir}/app dir"), &format!("{dir}/app dir/sub/x"))
        .await
        .is_err());

    let log = engine
        .audit_query(&AuditFilter {
            limit: Some(500),
            ..Default::default()
        })
        .unwrap();
    assert!(log.iter().any(|ev| ev.action == "file.copy" && ev.outcome == Outcome::Success));
    engine.run_command(&a.id, &format!("rm -rf '{dir}'"), false).await.unwrap();
}

/// nginx's stock default site: never removed as a website, and turned off only
/// by its sites-enabled link, with the file left byte for byte as it was.
/// Needs nginx with the Debian layout; opt in with ORBITAL_IT_NGINX=1 (CI does).
#[tokio::test]
async fn nginx_default_site_is_never_changed() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NGINX").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_NGINX=1 on a disposable machine with nginx");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let root = |c: &str| {
        let (engine, id, c) = (&engine, s.id.clone(), c.to_string());
        async move { engine.run_command(&id, &c, true).await.unwrap() }
    };
    if !root("test -L /etc/nginx/sites-enabled/default && echo yes || true")
        .await
        .stdout
        .contains("yes")
    {
        eprintln!("skipped: no sites-enabled/default link here");
        return;
    }
    // A healthy nginx has nothing to repair.
    assert!(engine.nginx_broken_sites(&s.id).await.unwrap().is_empty());
    let sum = "sha256sum /etc/nginx/sites-available/default";
    let before = root(sum).await.stdout;

    assert!(matches!(
        engine.remove_website(&s.id, "/etc/nginx/sites-available/default", None, None).await,
        Err(Error::InvalidInput(_))
    ));
    let target = engine.disable_default_site(&s.id).await.unwrap();
    assert!(target.ends_with("sites-available/default"), "{target}");
    let after = root("test -e /etc/nginx/sites-enabled/default && echo still || echo gone; nginx -t 2>&1 | tail -1")
        .await
        .stdout;
    assert!(after.contains("gone") && after.contains("successful"), "{after}");
    assert_eq!(root(sum).await.stdout, before, "the default file itself is untouched");
    // Already off: nothing to do.
    assert_eq!(engine.disable_default_site(&s.id).await.unwrap(), "");

    root("ln -s /etc/nginx/sites-available/default /etc/nginx/sites-enabled/default && systemctl reload nginx").await;
    engine.disconnect(&s.id).await.unwrap();
}

/// The health dot's failed-services check runs on a real server (empty or a list of units, never an error).
#[tokio::test]
async fn failed_units_are_listed() {
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let failed = engine.failed_units(&a.id).await.unwrap();
    assert!(failed.iter().all(|u| u.ends_with(".service")), "{failed:?}");
}

/// Scheduled backups: a folder job is written (script + cron line), runs,
/// shows up with its backup, restores to a copy, and is removed again.
/// Writes under /usr/local/lib and /etc/cron.d: ORBITAL_IT_USERS=1 (CI).
#[tokio::test]
async fn scheduled_folder_backup_runs_and_restores() {
    use orbital_core::backups::{BackupJob, BackupSource, Every, RestoreTo, Schedule};
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let dir = format!("/srv/orbital-it-backup-{}", std::process::id());
    let id = format!("it-{}", std::process::id());
    engine
        .run_command(&s.id, &format!("mkdir -p {dir}/sub && echo kept > {dir}/sub/a.txt"), true)
        .await
        .unwrap();
    let job = BackupJob {
        id: id.clone(),
        name: "IT folder".into(),
        source: BackupSource::Folder { path: dir.clone() },
        schedule: Schedule {
            every: Every::Week,
            weekday: 0,
            hour: 3,
            minute: 30,
        },
        keep: 2,
    };
    engine.save_backup_job(&s.id, &job).await.unwrap();
    let cron = engine
        .run_command(&s.id, &format!("cat /etc/cron.d/orbital-backup-{id}"), true)
        .await
        .unwrap()
        .stdout;
    assert!(
        cron.contains(&format!("30 3 * * 0 root /usr/local/lib/orbital/backup-{id}.sh")),
        "{cron}"
    );

    let file = engine.run_backup(&s.id, &id).await.unwrap();
    assert!(
        file.starts_with(&format!("/var/backups/orbital/{id}/{id}-")) && file.ends_with(".tar.gz"),
        "{file}"
    );
    let list = engine.backup_jobs(&s.id).await.unwrap();
    let st = list.iter().find(|x| x.job.id == id).expect("job listed");
    assert!(
        st.scheduled && st.last_error.is_none() && st.files.len() == 1 && st.files[0].bytes > 0,
        "{st:?}"
    );
    assert_eq!(st.job, job);

    let restored = engine.restore_scheduled_backup(&s.id, &id, &file, RestoreTo::Copy).await.unwrap();
    let got = engine
        .run_command(
            &s.id,
            &format!("cat '{restored}/{}/sub/a.txt'", dir.rsplit('/').next().unwrap()),
            true,
        )
        .await
        .unwrap()
        .stdout;
    assert_eq!(got.trim(), "kept");
    // Only the job's own backups can be restored.
    assert!(engine
        .restore_scheduled_backup(&s.id, &id, "/etc/shadow", RestoreTo::Copy)
        .await
        .is_err());

    engine.delete_backup_job(&s.id, &id, true).await.unwrap();
    assert!(engine.backup_jobs(&s.id).await.unwrap().iter().all(|x| x.job.id != id));
    let gone = engine
        .run_command(
            &s.id,
            &format!("ls /etc/cron.d/orbital-backup-{id} /var/backups/orbital/{id} 2>&1; rm -rf {dir} {restored}"),
            true,
        )
        .await
        .unwrap()
        .stdout;
    assert!(gone.matches("No such file").count() == 2, "{gone}");
}

/// Services → New service: a command becomes a running systemd service, its
/// secret stays in a root-only env file (not the unit), a name the system
/// already uses is refused, and the service is removed again.
/// Writes under /etc/systemd and /etc/orbital: ORBITAL_IT_USERS=1 (CI).
#[tokio::test]
async fn new_service_runs_keeps_secrets_out_and_is_removed() {
    use orbital_core::units::{Restart, ServiceSpec};
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine with systemd");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    if !s.has_systemd {
        eprintln!("skipped: no systemd here");
        return;
    }
    let name = format!("orbital-it-{}", std::process::id());
    let spec = ServiceSpec {
        name: name.clone(),
        description: "Orbital test service".into(),
        command: "while true; do sleep 30; done".into(),
        workdir: Some("/tmp".into()),
        user: None,
        restart: Restart::Always,
        env: vec![("API_TOKEN".into(), "not-a-real-secret-123".into())],
    };
    let (active, out) = engine.create_service(&s.id, &spec).await.unwrap();
    assert!(active, "{out}");
    let files = engine
        .run_command(
            &s.id,
            &format!("cat /etc/systemd/system/{name}.service; stat -c '%a' /etc/orbital/env/{name}.env"),
            true,
        )
        .await
        .unwrap()
        .stdout;
    assert!(
        files.contains("EnvironmentFile=/etc/orbital/env/") && !files.contains("not-a-real-secret-123"),
        "{files}"
    );
    assert!(files.trim_end().ends_with("600"), "{files}");

    // A unit the system already has is never shadowed.
    let taken = ServiceSpec {
        name: "cron".into(),
        ..spec.clone()
    };
    assert!(engine.create_service(&s.id, &taken).await.is_err());
    // Nor removed through Orbital.
    assert!(engine.remove_service(&s.id, "cron").await.is_err());

    engine.remove_service(&s.id, &name).await.unwrap();
    let gone = engine
        .run_command(
            &s.id,
            &format!("systemctl is-active {name}.service; ls /etc/systemd/system/{name}.service 2>&1"),
            true,
        )
        .await
        .unwrap()
        .stdout;
    assert!(
        gone.contains("No such file") && !gone.lines().next().unwrap_or("").starts_with("active"),
        "{gone}"
    );
    let log = engine
        .audit_query(&AuditFilter {
            limit: Some(500),
            ..Default::default()
        })
        .unwrap();
    assert!(
        log.iter()
            .all(|ev| !ev.command.clone().unwrap_or_default().contains("not-a-real-secret-123")),
        "secret never logged"
    );
}

/// Add swap: on a server that already has swap (CI's runner does) nothing is
/// changed and the reason is given; a size out of range is refused.
#[tokio::test]
async fn add_swap_checks_before_changing_anything() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    assert!(engine.add_swap(&s.id, 10).await.is_err(), "too small");
    let has_swap = !engine
        .run_command(&s.id, "swapon --show --noheadings", true)
        .await
        .unwrap()
        .stdout
        .trim()
        .is_empty();
    if has_swap {
        let before = engine.run_command(&s.id, "cat /etc/fstab", true).await.unwrap().stdout;
        let err = engine.add_swap(&s.id, 1024).await.unwrap_err().to_string();
        assert!(err.contains("already on"), "{err}");
        assert_eq!(
            engine.run_command(&s.id, "cat /etc/fstab", true).await.unwrap().stdout,
            before,
            "nothing changed"
        );
    }
}

/// Compose stacks: a file Compose rejects is never kept; a real stack is
/// created, deployed, listed as running, its logs read, and stopped.
/// Needs Docker with Compose v2 (CI's runner has it): ORBITAL_IT_USERS=1.
#[tokio::test]
async fn compose_stack_is_checked_deployed_and_stopped() {
    use orbital_core::stacks::{stack_file, StackAction};
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine with Docker");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    if !engine
        .run_command(&s.id, "docker compose version >/dev/null 2>&1 && echo yes", true)
        .await
        .unwrap()
        .stdout
        .contains("yes")
    {
        eprintln!("skipped: no docker compose here");
        return;
    }
    let name = format!("orbital-it-{}", std::process::id());
    let bad = engine
        .create_stack(&s.id, &format!("{name}-bad"), "services: [this is not compose", false)
        .await;
    assert!(matches!(bad, Err(Error::ValidationFailed { .. })), "{bad:?}");
    assert!(engine
        .run_command(&s.id, &format!("test -e /opt/stacks/{name}-bad && echo kept"), true)
        .await
        .unwrap()
        .stdout
        .trim()
        .is_empty());

    let compose = "services:\n  idle:\n    image: busybox:1.36\n    command: [\"sh\", \"-c\", \"echo started; sleep 300\"]\n";
    let out = engine.create_stack(&s.id, &name, compose, true).await.unwrap().expect("deployed");
    assert!(out.ok(), "{}", out.combined());
    let list = engine.stacks(&s.id).await.unwrap();
    let st = list.iter().find(|x| x.name == name).expect("listed");
    assert!(st.running && st.ours, "{st:?}");
    let file = stack_file(&name);
    // Logs show what the container printed (it may take a moment to start).
    let mut logs = String::new();
    for _ in 0..10 {
        logs = engine.stack_action(&s.id, &name, &file, StackAction::Logs).await.unwrap().stdout;
        if logs.contains("started") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(logs.contains("started"), "{logs}");
    // Creating it twice is refused.
    assert!(engine.create_stack(&s.id, &name, compose, false).await.is_err());
    let down = engine.stack_action(&s.id, &name, &file, StackAction::Down).await.unwrap();
    assert!(down.ok(), "{}", down.combined());
    assert!(!engine.stacks(&s.id).await.unwrap().iter().any(|x| x.name == name && x.running));
    engine
        .run_command(&s.id, &format!("rm -rf /opt/stacks/{name} /opt/stacks/{name}-bad"), true)
        .await
        .unwrap();
}

/// An app from the catalogue: a taken port, an unknown app and a bad name are
/// refused before anything is written; the install writes compose.yaml, the
/// generated token only to a 0600 .env (not the file, not the audit log), and
/// starts it on 127.0.0.1 only.
/// Needs Docker with Compose v2 (CI's runner has it): ORBITAL_IT_USERS=1.
#[tokio::test]
async fn app_installs_privately_with_secrets_only_in_env() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        eprintln!("skipped: set ORBITAL_IT_USERS=1 on a disposable machine with Docker");
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    if !engine
        .run_command(&s.id, "docker compose version >/dev/null 2>&1 && echo yes", true)
        .await
        .unwrap()
        .stdout
        .contains("yes")
    {
        eprintln!("skipped: no docker compose here");
        return;
    }
    let name = format!("orbital-app-{}", std::process::id());
    // The test's own sshd port is taken.
    let taken = engine.install_app(&s.id, "vaultwarden", &name, e.port.max(1024), false).await;
    if e.port >= 1024 {
        assert!(matches!(&taken, Err(Error::InvalidInput(m)) if m.contains("in use")), "{taken:?}");
    }
    assert!(engine.install_app(&s.id, "x; reboot", &name, 18083, false).await.is_err());
    assert!(engine.install_app(&s.id, "vaultwarden", "Bad Name", 18083, false).await.is_err());
    let none = engine
        .run_command(&s.id, &format!("test -e /opt/stacks/{name} && echo kept"), true)
        .await
        .unwrap();
    assert!(none.stdout.trim().is_empty(), "nothing written before the checks pass");

    // Vaultwarden: a generated admin token, shown once, only in .env.
    let r = engine.install_app(&s.id, "vaultwarden", &name, 18083, false).await.unwrap();
    assert!(r.started, "{}", r.output);
    assert_eq!(r.shown.len(), 1);
    let token = r.shown[0].value.clone();
    assert_eq!(token.len(), 32);
    let check = engine
        .run_command(
            &s.id,
            &format!("stat -c '%a' /opt/stacks/{name}/.env; cat /opt/stacks/{name}/.env; grep -c ADMIN_TOKEN /opt/stacks/{name}/compose.yaml; ss -Hltn | awk '{{print $4}}' | grep ':18083$'"),
            true,
        )
        .await
        .unwrap()
        .stdout;
    assert!(check.starts_with("600"), "{check}");
    assert!(check.contains(&format!("ADMIN_TOKEN={token}")), "the token is in .env");
    assert!(check.contains("127.0.0.1:18083") && !check.contains("0.0.0.0:18083"), "{check}");
    let compose = engine
        .run_command(&s.id, &format!("cat /opt/stacks/{name}/compose.yaml"), true)
        .await
        .unwrap()
        .stdout;
    assert!(compose.contains("${ADMIN_TOKEN}") && !compose.contains(&token));
    let log = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(
        log.iter().all(|a| !format!("{a:?}").contains(&token)),
        "the token isn't in the audit log"
    );
    let list = engine.stacks(&s.id).await.unwrap();
    assert!(list.iter().any(|x| x.name == name && x.running && x.ours));
    // Twice is refused.
    assert!(engine.install_app(&s.id, "vaultwarden", &name, 18084, false).await.is_err());
    let file = orbital_core::stacks::stack_file(&name);
    let down = engine
        .stack_action(&s.id, &name, &file, orbital_core::stacks::StackAction::Down)
        .await
        .unwrap();
    assert!(down.ok(), "{}", down.combined());
    engine
        .run_command(&s.id, &format!("rm -rf /opt/stacks/{name}"), true)
        .await
        .unwrap();
}

/// Logins and log search read a real server: the test's own SSH sign-in is
/// among the recent ones, and a search finds a line written to the journal.
#[tokio::test]
async fn logins_and_log_search_read_the_server() {
    use orbital_core::logsearch::LogSource;
    let Some(e) = env() else { return };
    let Some(key) = e.key.clone() else { return };
    let engine = engine();
    let p = engine.create_profile(profile(&e, AuthMethod::Key, Some(key))).unwrap();
    let a = connect(&engine, &p.id, None).await;
    let r = engine.logins(&a.id).await.unwrap();
    assert!(r.top_addresses.len() <= 10 && r.top_users.len() <= 10);
    let marker = format!("orbital-it-search-{}", std::process::id());
    engine
        .run_command(&a.id, &format!("logger -t orbital-it {marker} || true"), false)
        .await
        .unwrap();
    let has_journal = engine
        .run_command(&a.id, "command -v journalctl >/dev/null && echo yes", false)
        .await
        .unwrap()
        .stdout
        .contains("yes");
    if has_journal {
        let mut found = false;
        for _ in 0..10 {
            let hits = engine.search_logs(&a.id, &marker, 1, LogSource::Journal).await.unwrap();
            if hits.hits.iter().any(|h| h.line.contains(&marker)) {
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        assert!(found, "the journal line is found");
    }
    assert!(engine.search_logs(&a.id, "x", 1, LogSource::Files).await.is_err(), "too short");
    let files = engine
        .search_logs(&a.id, "orbital-unlikely-term-zz", 1, LogSource::Files)
        .await
        .unwrap();
    assert!(files.hits.is_empty() && !files.truncated);
}

/// Kernel tuning: a value is written to Orbital's sysctl file and loaded,
/// an unknown key is refused before anything is written, and removing every
/// value removes the file. Needs password sudo (the CI sshd has it).
#[tokio::test]
async fn kernel_tuning_writes_loads_and_removes_its_file() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let before = engine.kernel_report(&s.id).await.unwrap();
    let cur = before
        .settings
        .iter()
        .find(|x| x.key == "vm.swappiness")
        .and_then(|x| x.current.clone());
    let Some(cur) = cur else {
        eprintln!("skipped: no vm.swappiness here");
        return;
    };
    // Write the value it already has: nothing about the machine changes.
    let v = [("vm.swappiness".to_string(), cur.clone())].into_iter().collect();
    let r = engine.kernel_apply(&s.id, &v).await.unwrap();
    assert!(r.file_exists);
    assert_eq!(
        r.settings.iter().find(|x| x.key == "vm.swappiness").unwrap().ours.as_deref(),
        Some(cur.as_str())
    );
    let bad = [("vm.no_such_knob".to_string(), "1".to_string())].into_iter().collect();
    assert!(matches!(engine.kernel_apply(&s.id, &bad).await, Err(Error::InvalidInput(_))));
    let still = engine.kernel_report(&s.id).await.unwrap();
    assert!(still.file_exists, "the refused key left the file as it was");
    let gone = engine.kernel_apply(&s.id, &Default::default()).await.unwrap();
    assert!(!gone.file_exists);
    engine.disconnect(&s.id).await.unwrap();
}

/// SSH hardening: turning password login off from a password connection is
/// refused; a harmless setting is written, tested and removed again.
#[tokio::test]
async fn ssh_hardening_refuses_lockout_and_writes_its_dropin() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let state = engine.harden_state(&s.id).await.unwrap();
    let pw_off = [("PasswordAuthentication".to_string(), "no".to_string())].into_iter().collect();
    let r = engine.harden_ssh(&s.id, &pw_off, false).await;
    assert!(matches!(r, Err(Error::PreflightFailed { .. })), "{r:?}");
    let odd = [("PasswordAuthentication".to_string(), "yes".to_string())].into_iter().collect();
    assert!(matches!(engine.harden_ssh(&s.id, &odd, true).await, Err(Error::InvalidInput(_))));
    if !state.includes_dropins {
        eprintln!("skipped: this sshd_config has no Include of sshd_config.d");
        return;
    }
    let x11 = [("X11Forwarding".to_string(), "no".to_string())].into_iter().collect();
    let done = engine.harden_ssh(&s.id, &x11, false).await.unwrap();
    assert_eq!(done.state.current.get("X11Forwarding").map(String::as_str), Some("no"));
    let removed = engine.harden_ssh(&s.id, &Default::default(), false).await.unwrap();
    assert!(removed.state.current.is_empty());
    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(audit.iter().any(|ev| ev.action == "sshd.harden" && ev.outcome == Outcome::Denied));
    engine.disconnect(&s.id).await.unwrap();
}

/// The security audit runs on a real system and reports at least the account checks.
#[tokio::test]
async fn security_audit_runs() {
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let r = engine.security_audit(&s.id).await.unwrap();
    assert!(r.findings.iter().any(|f| f.id == "uid0"), "{r:?}");
    assert!(r.findings.iter().any(|f| f.id == "suid"));
    engine.disconnect(&s.id).await.unwrap();
}

/// SQL console: bad input is refused before anything runs; where psql is
/// installed, a query returns rows and read-only mode refuses a write.
#[tokio::test]
async fn sql_console_refuses_bad_input_and_runs_queries() {
    use orbital_core::databases::DbKind;
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    assert!(matches!(
        engine.sql_query(&s.id, DbKind::Postgres, None, "x; id", "select 1", true).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine.sql_query(&s.id, DbKind::Postgres, None, "postgres", "  ", true).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        engine
            .sql_query(&s.id, DbKind::Postgres, Some("bad name!"), "postgres", "select 1", true)
            .await,
        Err(Error::InvalidInput(_))
    ));
    match engine
        .sql_query(&s.id, DbKind::Postgres, None, "postgres", "select 1 as one, 'a,b' as two", true)
        .await
    {
        Err(Error::NotFound(_)) => eprintln!("psql isn't installed: only the refusals ran"),
        Ok(r) if r.error.is_none() => {
            assert_eq!(r.columns, ["one", "two"]);
            assert_eq!(r.rows[0], [Some("1".to_string()), Some("a,b".to_string())]);
            let w = engine
                .sql_query(
                    &s.id,
                    DbKind::Postgres,
                    None,
                    "postgres",
                    "create table orbital_it_ro (x int)",
                    true,
                )
                .await
                .unwrap();
            assert!(w.error.unwrap_or_default().contains("read-only"));
        }
        other => eprintln!("postgres answered something else (not running?): {other:?}"),
    }
    let audit = engine.audit_query(&AuditFilter::default()).unwrap();
    assert!(
        !serde_json::to_string(&audit).unwrap().contains("'a,b'"),
        "the query text isn't logged"
    );
    engine.disconnect(&s.id).await.unwrap();
}

/// A recorded terminal writes a .cast file with the output, listed and readable.
#[tokio::test]
async fn terminal_recording_writes_a_cast_file() {
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let dir = tempfile::tempdir().unwrap();
    orbital_core::recording::set_dir(dir.path().to_path_buf());
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let tid = engine
        .open_recorded_terminal(&s.id, 100, 30, Some("/tmp".into()), false, Default::default(), |_| {})
        .await
        .unwrap();
    engine.terminal_write(&tid, b"echo ORBITAL_$((40+2))\n").await.unwrap();
    engine.terminal_resize(&tid, 120, 40).await.unwrap();
    let mut found = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let list = engine.recordings().unwrap();
        if let Some(r) = list.first() {
            if engine.read_recording(&r.name).unwrap().contains("ORBITAL_42") {
                found = true;
                break;
            }
        }
    }
    engine.terminal_close(&tid).await.unwrap();
    assert!(found, "the output reached the recording");
    let r = &engine.recordings().unwrap()[0];
    assert!(engine.read_recording(&r.name).unwrap().contains("\"120x40\""));
    engine.delete_recording(&r.name).unwrap();
    assert!(engine.recordings().unwrap().is_empty());
    engine.disconnect(&s.id).await.unwrap();
}

/// Processes: the list has sshd and a process we start; its priority can be
/// lowered and it can be stopped; PID 1 is refused.
#[tokio::test]
async fn processes_list_renice_and_signal() {
    use orbital_core::procs::Signal;
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    let started = engine
        .run_command(&s.id, "nohup sleep 300 >/dev/null 2>&1 & echo $!", false)
        .await
        .unwrap();
    let pid: u32 = started.stdout.trim().parse().unwrap();
    let list = engine.processes(&s.id).await.unwrap();
    assert!(list.iter().any(|x| x.name == "sshd" || x.name.starts_with("sshd")), "sshd listed");
    assert!(list.iter().any(|x| x.pid == pid), "our sleep is listed");
    let d = engine.process_detail(&s.id, pid).await.unwrap();
    assert_eq!(d.pid, pid);
    if cfg!(target_os = "linux") {
        engine.renice_process(&s.id, pid, 10).await.unwrap();
        assert_eq!(
            engine.processes(&s.id).await.unwrap().iter().find(|x| x.pid == pid).unwrap().nice,
            10
        );
    }
    assert!(matches!(
        engine.signal_process(&s.id, 1, Signal::Term).await,
        Err(Error::InvalidInput(_))
    ));
    engine.signal_process(&s.id, pid, Signal::Term).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!engine.processes(&s.id).await.unwrap().iter().any(|x| x.pid == pid), "stopped");
    engine.disconnect(&s.id).await.unwrap();
}

/// Disks, packages: read on a real system; removing sshd's package is refused.
#[tokio::test]
async fn disks_and_packages_read_and_guard() {
    use orbital_core::packages::PkgAction;
    let Some(e) = env() else { return };
    let Some(input) = key_profile(&e) else { return };
    let engine = engine();
    let p = engine.create_profile(input).unwrap();
    let s = connect(&engine, &p.id, None).await;
    if cfg!(target_os = "linux") {
        let d = engine.disk_report(&s.id).await.unwrap();
        assert!(d.filesystems.iter().any(|f| f.mountpoint == "/"), "{d:?}");
    }
    match engine.installed_packages(&s.id).await {
        Err(Error::NotFound(_)) => eprintln!("no apt/dnf here: package checks skipped"),
        Ok(list) => {
            assert!(list.iter().any(|x| x.name.contains("ssh")), "an ssh package is installed");
            assert!(matches!(engine.search_packages(&s.id, "x; id").await, Err(Error::InvalidInput(_))));
            let ssh = if list.iter().any(|x| x.name == "openssh-server") {
                "openssh-server"
            } else {
                "openssh"
            };
            let r = engine.package_change(&s.id, ssh, PkgAction::Remove, false).await;
            assert!(matches!(r, Err(Error::PreflightFailed { .. })), "{r:?}");
        }
        Err(other) => panic!("{other:?}"),
    }
    engine.disconnect(&s.id).await.unwrap();
}

/// Timers: a schedule is checked by systemd, a timer is created, started and removed.
#[tokio::test]
async fn systemd_timer_create_and_remove() {
    use orbital_core::timers::TimerSpec;
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    if engine.check_schedule(&s.id, "daily").await.is_err() {
        eprintln!("skipped: no systemd here");
        return;
    }
    assert!(matches!(
        engine.check_schedule(&s.id, "Fooday 25:99").await,
        Err(Error::InvalidInput(_))
    ));
    let spec = TimerSpec {
        name: "orbital-it-timer".into(),
        description: "Orbital test".into(),
        command: "echo orbital-timer".into(),
        user: None,
        schedule: "*-*-* 03:17:00".into(),
        persistent: false,
    };
    engine.create_timer(&s.id, &spec).await.unwrap();
    let listed = engine
        .run_command(&s.id, "systemctl list-timers --all --no-pager | grep -c orbital-it-timer", false)
        .await
        .unwrap();
    assert_eq!(listed.stdout.trim(), "1");
    engine.remove_timer(&s.id, "orbital-it-timer").await.unwrap();
    assert!(engine.remove_timer(&s.id, "ssh").await.is_err(), "only Orbital's timers");
    engine.disconnect(&s.id).await.unwrap();
}

/// Docker without its plugins (as when only docker-ce and docker-ce-cli were
/// installed): the check says Compose is missing, and installing the plugins
/// picks the packages for how Docker came and makes `docker compose` work.
/// Needs Docker from the system's packages (the Ubuntu test server): ORBITAL_IT_USERS=1.
#[tokio::test]
async fn docker_plugins_are_found_missing_and_installed() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let p = engine.docker_plugins(&s.id).await.unwrap();
    if !p.docker || p.source != "docker.io" {
        eprintln!("skipped: needs Docker from the docker.io package ({p:?})");
        return;
    }
    engine
        .run_command(
            &s.id,
            "DEBIAN_FRONTEND=noninteractive apt-get remove -y -q docker-compose-v2 docker-buildx >/dev/null 2>&1; true",
            true,
        )
        .await
        .unwrap();
    let before = engine.docker_plugins(&s.id).await.unwrap();
    assert!(!before.compose, "{before:?}");
    let after = engine.install_docker_plugins(&s.id).await.unwrap();
    assert!(after.compose && after.buildx, "{after:?}");
    let o = engine
        .run_command(&s.id, "dpkg -s docker-compose-v2 docker-buildx >/dev/null && echo both", true)
        .await
        .unwrap();
    assert!(o.stdout.contains("both"), "{o:?}");
}

/// The same with Docker CE from Docker's own repository (only docker-ce and
/// docker-ce-cli installed): its plugin packages are the ones installed.
/// Runs only where Docker CE is set up that way (ORBITAL_IT_DOCKER_CE=1).
#[tokio::test]
async fn docker_ce_plugins_come_from_dockers_repository() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_DOCKER_CE").is_err() {
        return;
    }
    let engine = engine();
    let s = sudo_session(&engine, &e).await;
    let before = engine.docker_plugins(&s.id).await.unwrap();
    assert_eq!(before.source, "docker-ce", "{before:?}");
    assert!(!before.compose, "{before:?}");
    let after = engine.install_docker_plugins(&s.id).await.unwrap();
    assert!(after.compose && after.buildx, "{after:?}");
    let o = engine
        .run_command(
            &s.id,
            "dpkg -s docker-compose-plugin docker-buildx-plugin >/dev/null && dpkg -s docker-ce >/dev/null && echo ok",
            true,
        )
        .await
        .unwrap();
    assert!(o.stdout.contains("ok"), "Docker CE kept, its plugins installed: {o:?}");
}

/// Admins copying what only root can read or write: a root-only file and a
/// root-only folder go from one server to a root-only place on another (tar
/// through this Mac with sudo on both sides), and a root-only file downloads.
/// Two sessions stand in for two servers. Needs sudo: ORBITAL_IT_USERS=1.
#[tokio::test]
async fn root_only_files_copy_between_servers_and_download() {
    let Some(e) = env() else { return };
    if std::env::var("ORBITAL_IT_USERS").is_err() || std::env::var("ORBITAL_IT_NO_PASSWORD").is_ok() {
        return;
    }
    let engine = engine();
    let a = sudo_session(&engine, &e).await;
    // A second connection to the same saved server (its key is trusted and its password saved now).
    let b = engine.connect(&a.profile_id, ConnectOptions::default()).await.unwrap();
    let tag = std::process::id();
    let src_dir = format!("/root/orbital-src-{tag}");
    let dst_dir = format!("/etc/orbital-dst-{tag}");
    engine
        .run_command(
            &a.id,
            &format!("mkdir -p {src_dir}/sub && echo top-secret > {src_dir}/sub/key.txt && chmod 700 {src_dir} && chmod 600 {src_dir}/sub/key.txt && mkdir -p {dst_dir} && chmod 755 {dst_dir}"),
            true,
        )
        .await
        .unwrap();
    // A plain read is refused for the login user, so this goes through the sudo path.
    let file = engine
        .copy_between(
            &a.id,
            &format!("{src_dir}/sub/key.txt"),
            &b.id,
            &format!("{dst_dir}/key.txt"),
            false,
        )
        .await
        .unwrap();
    assert_eq!(file.files, 1);
    let folder = engine
        .copy_between(&a.id, &src_dir, &b.id, &format!("{dst_dir}/tree"), false)
        .await
        .unwrap();
    assert!(folder.files == 1 && folder.bytes > 0, "{folder:?}");
    let check = engine
        .run_command(
            &b.id,
            &format!("cat {dst_dir}/key.txt {dst_dir}/tree/sub/key.txt; stat -c '%a' {dst_dir}/tree/sub/key.txt"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(check.stdout, "top-secret\ntop-secret\n600\n", "contents and mode kept");
    // Not over an existing one unless asked.
    assert!(engine
        .copy_between(
            &a.id,
            &format!("{src_dir}/sub/key.txt"),
            &b.id,
            &format!("{dst_dir}/key.txt"),
            false
        )
        .await
        .is_err());
    // Downloading a root-only file to this computer.
    let local = std::env::temp_dir().join(format!("orbital-root-dl-{tag}.txt"));
    let n = engine.download(&a.id, &format!("{src_dir}/sub/key.txt"), &local).await.unwrap();
    assert_eq!(n, 11);
    assert_eq!(std::fs::read_to_string(&local).unwrap(), "top-secret\n");
    std::fs::remove_file(&local).ok();
    engine
        .run_command(&a.id, &format!("rm -rf {src_dir} {dst_dir}"), true)
        .await
        .unwrap();
}
