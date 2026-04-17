//! zad — A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped adapter configurations instead of MCP servers.

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}