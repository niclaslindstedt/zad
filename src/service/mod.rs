pub mod discord;
pub mod gcal;
pub mod onepass;
pub mod registry;
pub mod telegram;

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(pub u64);

#[derive(Debug, Clone)]
pub enum Target {
    Channel(ChannelId),
    Dm(UserId),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub channel: ChannelId,
    pub author: UserId,
    pub body: String,
}

/// Lightweight descriptor for a channel returned by a service's listing
/// endpoint. `kind` is a free-form, service-specific label (e.g. `"text"`,
/// `"voice"`, `"public_thread"` for Discord).
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: ChannelId,
    pub name: String,
    pub kind: String,
    pub parent: Option<ChannelId>,
    pub position: u16,
}

/// Minimal guild (server) descriptor used by the discovery surface.
#[derive(Debug, Clone)]
pub struct GuildInfo {
    pub id: u64,
    pub name: String,
}

/// Minimal member descriptor used by the discovery surface. `display_name`
/// is the name the directory should index under (server nickname if set,
/// else the user's global display name, else the raw username).
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    MessageCreated(Message),
    MessageDeleted { channel: ChannelId, id: MessageId },
    Ready,
}

#[derive(Debug, Clone)]
pub enum ManageCmd {
    CreateChannel { guild: u64, name: String },
    DeleteChannel { channel: ChannelId },
}

#[async_trait]
pub trait Service: Send + Sync {
    fn name(&self) -> &'static str;
    async fn send_message(&self, target: Target, body: &str) -> Result<MessageId>;
    async fn read_messages(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>>;
    async fn listen(&self) -> Result<BoxStream<'static, Event>>;
    async fn manage(&self, cmd: ManageCmd) -> Result<()>;
}

/// A single side-effect that would have been sent to a remote service.
/// Services emit one of these to a [`DryRunSink`] when `--dry-run` is
/// active, in place of the underlying network call. Service-agnostic by
/// design so any future service wrapper (Slack, GitHub, …) can reuse it.
#[derive(Debug, Clone)]
pub struct DryRunOp {
    /// Service name, matching [`Service::name`] (e.g. `"discord"`).
    pub service: &'static str,
    /// Verb being previewed (e.g. `"send"`, `"join"`, `"leave"`).
    pub verb: &'static str,
    /// One-line human-readable summary, e.g. `"would send 42 chars to channel 12345"`.
    pub summary: String,
    /// Structured payload the caller would have sent. Rendered verbatim
    /// as JSON by [`StderrTracingSink`]; other sinks may reformat.
    pub details: serde_json::Value,
}

/// Where [`DryRunOp`] records go when intercepted. Implementations decide
/// whether to print, log, buffer for tests, etc.
pub trait DryRunSink: Send + Sync {
    fn record(&self, op: DryRunOp);
}

/// Default sink: logs a one-line `tracing::info!` at the `dry_run` target
/// and prints the structured payload as pretty JSON on stdout. The
/// summary goes through `tracing` so it lands in the debug file log
/// alongside every other service event; the JSON lands on stdout so
/// piped consumers (`| jq`) get a machine-readable preview.
pub struct StderrTracingSink;

impl DryRunSink for StderrTracingSink {
    fn record(&self, op: DryRunOp) {
        tracing::info!(
            target: "dry_run",
            service = op.service,
            verb = op.verb,
            "DRY RUN: {}",
            op.summary,
        );
        match serde_json::to_string_pretty(&op.details) {
            Ok(rendered) => println!("{rendered}"),
            Err(e) => eprintln!("dry-run: failed to render payload as JSON: {e}"),
        }
    }
}

/// Construct the default stderr+stdout sink wrapped in an [`Arc`] ready
/// to hand to a dry-run transport. Exists so the call-sites in
/// `src/cli/` don't need to import `Arc` + `StderrTracingSink` directly.
pub fn default_dry_run_sink() -> Arc<dyn DryRunSink> {
    Arc::new(StderrTracingSink)
}
