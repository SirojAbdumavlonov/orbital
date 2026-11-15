//! Tauri shell: a thin, typed bridge from the UI to `orbital_core::Engine`.
//! No business logic lives here — everything is in the core crate, tested headless.

mod commands;

use orbital_core::db::Database;
use orbital_core::engine::Engine;
use orbital_core::vault::VaultStore;
use std::path::Path;
use std::sync::Arc;
use tauri::Manager;

/// Orbital keeps its data in a hidden folder under Application Support
/// (Finder doesn't show it): the database and the encrypted credentials vault.
const DATA_DIR: &str = ".orbital";

/// Earlier homes of the database, newest first: Orbital's first data folder,
/// and the folder from when the app was called Outpost.
const OLD_HOMES: &[(&str, &str)] = &[("app.orbital.desktop", "orbital.db"), ("app.outpost.desktop", "outpost.db")];

/// Move the database (servers, trusted host keys, activity log) from an
/// earlier folder into `dir` if `dir` has none yet. Nothing is left behind.
fn move_old_data(dir: &Path) -> std::io::Result<()> {
    let db = dir.join("orbital.db");
    let Some(base) = dir.parent() else { return Ok(()) };
    if db.exists() {
        return Ok(());
    }
    for (folder, name) in OLD_HOMES {
        let old = base.join(folder);
        if !old.join(name).exists() {
            continue;
        }
        std::fs::create_dir_all(dir)?;
        // SQLite in WAL mode keeps recent writes in -wal and -shm; they move with it.
        for suffix in ["", "-wal", "-shm"] {
            let from = old.join(format!("{name}{suffix}"));
            if from.exists() {
                let to = dir.join(format!("orbital.db{suffix}"));
                if std::fs::rename(&from, &to).is_err() {
                    std::fs::copy(&from, &to)?;
                    std::fs::remove_file(&from)?;
                }
            }
        }
        // Remove the old folder if nothing else is in it.
        let _ = std::fs::remove_dir(&old);
        tracing::info!("moved data from {}", old.display());
        return Ok(());
    }
    Ok(())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Updates: signed with Orbital's updater key (public half in tauri.conf.json)
        // and fetched from the release's latest.json; the UI asks before installing.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let dir = app.path().data_dir()?.join(DATA_DIR);
            if let Err(e) = move_old_data(&dir) {
                tracing::error!("couldn't move data from the old folder: {e}");
            }
            // Saved credentials live in Orbital's own encrypted vault, not the
            // Keychain, so the app never asks for Keychain access.
            let secrets = Arc::new(VaultStore::open(&dir).map_err(|e| e.to_string())?);
            let db = Database::open(&dir.join("orbital.db")).map_err(|e| e.to_string())?;
            #[cfg(target_os = "macos")]
            let engine = Engine::with_presence(Arc::new(db), secrets, Arc::new(orbital_core::presence::LocalAuth));
            #[cfg(not(target_os = "macos"))]
            let engine = Engine::new(Arc::new(db), secrets);
            app.manage(Arc::new(engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_profiles,
            commands::create_profile,
            commands::update_profile,
            commands::delete_profile,
            commands::ssh_config_preview,
            commands::import_ssh_config,
            commands::set_secret,
            commands::has_secret,
            commands::known_hosts,
            commands::forget_host,
            commands::presence_status,
            commands::set_presence_gate,
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
            commands::copy_between,
            commands::detect_tools,
            commands::nginx_config,
            commands::listening_ports,
            commands::list_databases,
            commands::cron_report,
            commands::cron_change,
            commands::cron_run,
            commands::cron_preview,
            commands::list_users,
            commands::create_user,
            commands::change_user,
            commands::apache_status,
            commands::apache_site,
            commands::haproxy_status,
            commands::haproxy_server_state,
            commands::backup_database,
            commands::redis_info,
            commands::redis_action,
            commands::system_info,
            commands::disk_usage,
            commands::process_usage,
            commands::compose_action,
            commands::sshd_report,
            commands::ufw_status,
            commands::ufw_add,
            commands::ufw_update,
            commands::ufw_delete,
            commands::ufw_set_enabled,
            commands::fail2ban_status,
            commands::fail2ban_unban,
            commands::fail2ban_ban,
            commands::certbot_status,
            commands::certbot_issue,
            commands::certbot_renew,
            commands::certbot_delete,
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
            commands::open_tunnel,
            commands::close_tunnel,
            commands::list_tunnels,
            commands::log_files,
            commands::follow_logs,
            commands::stop_logs,
            commands::list_apps,
            commands::save_app,
            commands::delete_app,
            commands::deploy_history,
            commands::deploy_log,
            commands::app_checkout,
            commands::run_deploy,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_close,
            commands::audit_query,
            commands::audit_verify,
            commands::audit_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Orbital");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_the_newest_old_database_and_leaves_nothing_behind() {
        let root = std::env::temp_dir().join(format!("orbital-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let first = root.join("app.orbital.desktop");
        let outpost = root.join("app.outpost.desktop");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&outpost).unwrap();
        std::fs::write(first.join("orbital.db"), "newer").unwrap();
        std::fs::write(first.join("orbital.db-wal"), "wal").unwrap();
        std::fs::write(outpost.join("outpost.db"), "older").unwrap();
        let new = root.join(DATA_DIR);

        move_old_data(&new).unwrap();
        assert_eq!(std::fs::read_to_string(new.join("orbital.db")).unwrap(), "newer");
        assert_eq!(std::fs::read_to_string(new.join("orbital.db-wal")).unwrap(), "wal");
        assert!(!first.exists(), "the old folder is gone");

        // Once there is a database, nothing is moved over it.
        move_old_data(&new).unwrap();
        assert_eq!(std::fs::read_to_string(new.join("orbital.db")).unwrap(), "newer");
        assert!(outpost.join("outpost.db").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
