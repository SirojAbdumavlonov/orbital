//! Tauri shell: a thin, typed bridge from the UI to `outpost_core::Engine`.
//! No business logic lives here — everything is in the core crate, tested headless.

mod commands;

use outpost_core::db::Database;
use outpost_core::engine::Engine;
use outpost_core::secrets::KeychainStore;
use std::sync::Arc;
use tauri::Manager;

pub const KEYCHAIN_SERVICE: &str = "app.outpost.desktop";

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let db = Database::open(&dir.join("outpost.db")).map_err(|e| e.to_string())?;
            let engine = Engine::new(Arc::new(db), Arc::new(KeychainStore::new(KEYCHAIN_SERVICE)));
            app.manage(Arc::new(engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_profiles,
            commands::create_profile,
            commands::update_profile,
            commands::delete_profile,
            commands::set_secret,
            commands::has_secret,
            commands::known_hosts,
            commands::forget_host,
            commands::connect,
            commands::disconnect,
            commands::list_dir,
            commands::read_file,
            commands::write_file,
            commands::safe_write,
            commands::validator_for,
            commands::list_backups,
            commands::mkdir,
            commands::create_file,
            commands::rename,
            commands::remove,
            commands::chmod,
            commands::upload,
            commands::download,
            commands::metrics,
            commands::list_services,
            commands::service_detail,
            commands::service_action,
            commands::list_containers,
            commands::container_stats,
            commands::inspect_container,
            commands::container_logs,
            commands::container_action,
            commands::run_command,
            commands::open_terminal,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_close,
            commands::audit_query,
            commands::audit_verify,
            commands::audit_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Outpost");
}
