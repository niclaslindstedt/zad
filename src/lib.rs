//! zad — A Rust CLI that connects AI agents to external services (Discord,
//! GitHub, Slack, etc.) via scoped adapter configurations instead of MCP
//! servers.

pub mod adapter;
pub mod cli;
pub mod config;
pub mod error;
pub mod logging;
pub mod secrets;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
