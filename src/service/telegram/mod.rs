//! Telegram bot integration.
//!
//! This module mirrors the layout of `service::discord` so a Telegram
//! bot can be driven through the same generic [`Service`] trait and the
//! same credential/permissions plumbing every other zad service uses.
//!
//! ## Status
//!
//! Lifecycle only. This module currently exposes:
//!
//! - [`TelegramServiceCfg`] (re-exported from `crate::config`) for the
//!   on-disk credential file at `~/.zad/services/telegram/config.toml`.
//! - [`TelegramService`] — a thin type that holds the token and the
//!   declared scope set. Its [`Service`] trait methods are **not yet
//!   implemented**: every runtime verb returns
//!   [`ZadError::Unsupported`] so the CLI surface can be wired up (and
//!   exercised end-to-end for the `service telegram` lifecycle
//!   subcommands) before the Telegram Bot API client lands.
//!
//! ## TODO — next step: implement the runtime client
//!
//! The follow-up change should:
//!
//! 1. Add a `client.rs` (`TelegramHttp` wrapping a `reqwest::Client`
//!    pointed at `https://api.telegram.org/bot<TOKEN>/`) with one async
//!    method per Bot API verb the CLI needs: `send_message`,
//!    `get_updates`, `get_me`, `get_chat`, `leave_chat`, `ban_chat_member`.
//! 2. Add a `transport.rs` with a `TelegramTransport` trait + a
//!    `DryRunTelegramTransport` that emits `DryRunOp` records to the
//!    shared [`DryRunSink`] — same pattern as
//!    `service::discord::transport`.
//! 3. Add a `gateway.rs` with a long-polling listener built on
//!    `getUpdates` (Telegram does not use a persistent WebSocket; a
//!    webhook variant can come later).
//! 4. Add a `permissions.rs` specialised to Telegram — the per-function
//!    blocks will be `send`, `read`, `chats`, `discover`, `manage`,
//!    matching the verbs the CLI exposes. Re-use the generic
//!    `permissions::{pattern, content, time}` primitives.
//! 5. Fill in the [`Service`] impl below so `Service::send_message`,
//!    `Service::read_messages`, `Service::listen`, and `Service::manage`
//!    delegate to the new transport.
//!
//! Keep the public surface aligned with the Discord module so future
//! cross-service helpers (e.g. a shared DryRun sink, a shared message
//! render) keep working without per-service special cases.

use std::collections::BTreeSet;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::error::{Result, ZadError};
use crate::service::{ChannelId, Event, ManageCmd, Message, MessageId, Service, Target};

/// Thin placeholder for the Telegram service. Holds the bot token and
/// the declared scope set so the lifecycle commands can construct one,
/// print diagnostics, and (eventually) hand it to a real client.
pub struct TelegramService {
    #[allow(dead_code)]
    token: String,
    #[allow(dead_code)]
    scopes: BTreeSet<String>,
}

impl TelegramService {
    /// Construct a service from a bot token and its declared scope set.
    /// No network I/O happens here — the token is not validated until a
    /// future `TelegramHttp::validate_token` (via `getMe`) is wired up.
    pub fn new(token: impl Into<String>, scopes: BTreeSet<String>) -> Self {
        Self {
            token: token.into(),
            scopes,
        }
    }
}

#[async_trait]
impl Service for TelegramService {
    fn name(&self) -> &'static str {
        "telegram"
    }

    // TODO: POST /bot<TOKEN>/sendMessage with chat_id + text.
    // Honour Telegram's 4096-codepoint limit, support Target::Dm (user
    // chat_id) and Target::Channel (group/supergroup/channel chat_id,
    // usually a negative i64).
    async fn send_message(&self, _target: Target, _body: &str) -> Result<MessageId> {
        Err(ZadError::Unsupported(
            "telegram: send_message not implemented yet",
        ))
    }

    // TODO: there is no `read_messages` analogue in the Bot API — the
    // closest is `getUpdates` (long-poll). For parity with Discord we
    // can back this with a one-shot `getUpdates` filtered by chat_id,
    // or (when the bot has admin rights) `getChatHistory`-equivalents
    // via Bot API 7.x. Decide at implementation time.
    async fn read_messages(&self, _channel: ChannelId, _limit: usize) -> Result<Vec<Message>> {
        Err(ZadError::Unsupported(
            "telegram: read_messages not implemented yet",
        ))
    }

    // TODO: long-poll `getUpdates` with an incrementing offset, yield
    // one Event per update. Webhook mode is a later enhancement.
    async fn listen(&self) -> Result<BoxStream<'static, Event>> {
        Err(ZadError::Unsupported(
            "telegram: listen not implemented yet",
        ))
    }

    // TODO: map ManageCmd::{CreateChannel, DeleteChannel} onto Telegram
    // Bot API equivalents. Bots cannot create groups or channels (that
    // is a user-API-only operation), so CreateChannel should return a
    // clear `ZadError::Unsupported` explaining the limitation. Deletion
    // of a channel the bot administers is possible via `deleteChat` on
    // supergroups; that is the concrete plan for DeleteChannel.
    async fn manage(&self, _cmd: ManageCmd) -> Result<()> {
        Err(ZadError::Unsupported(
            "telegram: manage not implemented yet",
        ))
    }
}
