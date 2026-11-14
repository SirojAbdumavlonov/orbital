//! # outpost-core
//!
//! The engine behind Outpost. Kept free of any UI/Tauri dependency so it can be
//! unit- and integration-tested headless (see `tests/`).

pub mod audit;
pub mod config_edit;
pub mod db;
pub mod docker;
pub mod engine;
pub mod error;
pub mod known_hosts;
pub mod metrics;
pub mod profiles;
pub mod secrets;
pub mod services;
pub mod sftp;
pub mod shell;
pub mod ssh;
pub mod terminal;

pub use error::{Error, Result};
