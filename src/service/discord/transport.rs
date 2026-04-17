//! The side-effectful surface that `zad discord <verb>` depends on.
//!
//! [`DiscordTransport`] is a thin trait over the same method set that
//! [`DiscordHttp`] already exposes. Its purpose is to let the CLI hold a
//! `Box<dyn DiscordTransport>` and stay oblivious to whether the
//! underlying implementation is the live Serenity-backed client or a
//! `--dry-run` preview that never touches the network.
//!
//! The pattern is reusable: every service under `src/service/<name>/`
//! that wants a dry-run mode should define its own `<Name>Transport`
//! trait over its runtime verbs, implement it for its live client, and
//! ship a matching `DryRun<Name>Transport` that emits [`DryRunOp`]
//! records into a shared [`DryRunSink`] instead of calling the remote
//! API.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::Result;
use crate::service::discord::client::DiscordHttp;
use crate::service::{
    ChannelId, ChannelInfo, DryRunOp, DryRunSink, GuildInfo, MemberInfo, Message, MessageId,
    Target, UserId,
};

/// Runtime surface of the Discord service. Each method corresponds one
/// to one with a verb reachable from `zad discord …`.
#[async_trait]
pub trait DiscordTransport: Send + Sync {
    async fn send(&self, target: Target, body: &str) -> Result<MessageId>;
    async fn history(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>>;
    async fn list_channels(&self, guild: u64) -> Result<Vec<ChannelInfo>>;
    async fn list_guilds(&self) -> Result<Vec<GuildInfo>>;
    async fn list_members(&self, guild: u64, limit: u64) -> Result<Vec<MemberInfo>>;
    async fn join_channel(&self, channel: ChannelId) -> Result<()>;
    async fn leave_channel(&self, channel: ChannelId) -> Result<()>;
    async fn create_channel(&self, guild: u64, name: &str) -> Result<()>;
    async fn delete_channel(&self, channel: ChannelId) -> Result<()>;
}

#[async_trait]
impl DiscordTransport for DiscordHttp {
    async fn send(&self, target: Target, body: &str) -> Result<MessageId> {
        DiscordHttp::send(self, target, body).await
    }
    async fn history(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>> {
        DiscordHttp::history(self, channel, limit).await
    }
    async fn list_channels(&self, guild: u64) -> Result<Vec<ChannelInfo>> {
        DiscordHttp::list_channels(self, guild).await
    }
    async fn list_guilds(&self) -> Result<Vec<GuildInfo>> {
        DiscordHttp::list_guilds(self).await
    }
    async fn list_members(&self, guild: u64, limit: u64) -> Result<Vec<MemberInfo>> {
        DiscordHttp::list_members(self, guild, limit).await
    }
    async fn join_channel(&self, channel: ChannelId) -> Result<()> {
        DiscordHttp::join_channel(self, channel).await
    }
    async fn leave_channel(&self, channel: ChannelId) -> Result<()> {
        DiscordHttp::leave_channel(self, channel).await
    }
    async fn create_channel(&self, guild: u64, name: &str) -> Result<()> {
        DiscordHttp::create_channel(self, guild, name).await
    }
    async fn delete_channel(&self, channel: ChannelId) -> Result<()> {
        DiscordHttp::delete_channel(self, channel).await
    }
}

/// Preview transport used when the caller passed `--dry-run`.
///
/// Intercepts every mutating verb (`send`, `join`, `leave`,
/// `create_channel`, `delete_channel`) by emitting a [`DryRunOp`] to
/// the sink and returning a stub success value — `MessageId(0)` for
/// `send` (never a real Discord snowflake), `Ok(())` for the rest.
///
/// Read verbs (`history`, `list_*`) return empty vectors rather than
/// delegating to a live client, because dry-run is intentionally
/// decoupled from credentials: no token is ever loaded in dry-run mode
/// (see `src/cli/discord.rs::discord_http_for`). This keeps
/// `--dry-run` usable before a bot is configured, and means read verbs
/// — which are not currently dry-run-eligible — have no well-defined
/// behaviour here and return the safe empty result.
pub struct DryRunDiscordTransport {
    sink: Arc<dyn DryRunSink>,
}

impl DryRunDiscordTransport {
    pub fn new(sink: Arc<dyn DryRunSink>) -> Self {
        Self { sink }
    }

    fn record(&self, verb: &'static str, summary: String, details: serde_json::Value) {
        self.sink.record(DryRunOp {
            service: "discord",
            verb,
            summary,
            details,
        });
    }
}

#[async_trait]
impl DiscordTransport for DryRunDiscordTransport {
    async fn send(&self, target: Target, body: &str) -> Result<MessageId> {
        let (kind, id) = match &target {
            Target::Channel(ChannelId(id)) => ("channel", *id),
            Target::Dm(UserId(id)) => ("dm", *id),
        };
        let len = body.chars().count();
        self.record(
            "send",
            format!("would send {len} chars to {kind} {id}"),
            json!({
                "command": "discord.send",
                "target": kind,
                "target_id": id.to_string(),
                "body": body,
                "body_chars": len,
            }),
        );
        Ok(MessageId(0))
    }

    async fn history(&self, _channel: ChannelId, _limit: usize) -> Result<Vec<Message>> {
        Ok(vec![])
    }

    async fn list_channels(&self, _guild: u64) -> Result<Vec<ChannelInfo>> {
        Ok(vec![])
    }

    async fn list_guilds(&self) -> Result<Vec<GuildInfo>> {
        Ok(vec![])
    }

    async fn list_members(&self, _guild: u64, _limit: u64) -> Result<Vec<MemberInfo>> {
        Ok(vec![])
    }

    async fn join_channel(&self, channel: ChannelId) -> Result<()> {
        self.record(
            "join",
            format!("would join thread channel {}", channel.0),
            json!({
                "command": "discord.join",
                "channel": channel.0.to_string(),
            }),
        );
        Ok(())
    }

    async fn leave_channel(&self, channel: ChannelId) -> Result<()> {
        self.record(
            "leave",
            format!("would leave thread channel {}", channel.0),
            json!({
                "command": "discord.leave",
                "channel": channel.0.to_string(),
            }),
        );
        Ok(())
    }

    async fn create_channel(&self, guild: u64, name: &str) -> Result<()> {
        self.record(
            "create_channel",
            format!("would create channel `{name}` in guild {guild}"),
            json!({
                "command": "discord.create_channel",
                "guild": guild.to_string(),
                "name": name,
            }),
        );
        Ok(())
    }

    async fn delete_channel(&self, channel: ChannelId) -> Result<()> {
        self.record(
            "delete_channel",
            format!("would delete channel {}", channel.0),
            json!({
                "command": "discord.delete_channel",
                "channel": channel.0.to_string(),
            }),
        );
        Ok(())
    }
}
