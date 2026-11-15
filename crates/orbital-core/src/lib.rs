//! # orbital-core
//!
//! The engine behind Orbital. Kept free of any UI/Tauri dependency so it can be
//! unit- and integration-tested headless (see `tests/`).

pub mod audit;
pub mod certbot;
pub mod config_edit;
pub mod cron;
pub mod databases;
pub mod db;
pub mod deploy;
pub mod docker;
pub mod engine;
pub mod error;
pub mod fail2ban;
pub mod firewall;
pub mod known_hosts;
pub mod logs;
pub mod metrics;
pub mod nginx;
pub mod ports;
pub mod presence;
pub mod profiles;
pub mod secrets;
pub mod services;
pub mod sftp;
pub mod shell;
pub mod ssh;
pub mod sshd;
pub mod storage;
pub mod streams;
pub mod sysinfo;
pub mod terminal;
pub mod tools;
pub mod tunnels;
pub mod usage;
pub mod users;
pub mod vault;
pub mod webservers;

pub use error::{Error, Result};
