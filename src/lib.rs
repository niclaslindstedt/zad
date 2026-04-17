//! zad — A Rust CLI that connects AI agents to external services (Discord,
//! GitHub, Slack, etc.) via scoped service configurations instead of MCP
//! servers.

// The `ZadError` variants aggregate third-party error types
// (`keyring::Error`, `dialoguer::Error`, `toml::de::Error`) that are
// individually over clippy's default 128-byte `result_large_err`
// threshold. Boxing every one for a CLI that returns Result a handful
// of times trades clarity for nothing measurable.
#![allow(clippy::result_large_err)]

pub mod cli;
pub mod config;
pub mod error;
pub mod logging;
pub mod output;
pub mod permissions;
pub mod secrets;
pub mod service;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
