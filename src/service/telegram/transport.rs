//! The side-effectful surface that `zad telegram <verb>` will depend
//! on once the runtime verbs are wired up.
//!
//! [`TelegramTransport`] is a thin trait over the verb set exposed by
//! [`TelegramHttp`]. Its purpose is to let the CLI hold a
//! `Box<dyn TelegramTransport>` and stay oblivious to whether the
//! underlying implementation is the live Bot-API-backed client or a
//! `--dry-run` preview that never touches the network.
//!
//! The current trait body only reserves the shape — every method is a
//! `todo!()` until the corresponding verb is implemented. This lets
//! the CLI factory (`src/cli/telegram.rs::telegram_http_for`) compile
//! against a stable trait object while the underlying methods are
//! still being written.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::Result;
use crate::service::telegram::client::TelegramHttp;
use crate::service::{DryRunOp, DryRunSink};

/// Signed chat identifier. Telegram chat IDs are negative for
/// (super)groups and positive for private chats and most channels.
pub type ChatId = i64;

/// A single message fetched from the Bot API. Shaped to match the
/// fields the CLI needs for `zad telegram read` — message_id, author
/// username or first-name fallback, and the text body. Further fields
/// (reply_to, entities, media) will be added when the verbs that need
/// them land.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: i64,
    pub chat: ChatId,
    pub author: String,
    pub body: String,
}

/// Descriptor for a chat the bot has seen. `kind` is the Bot API's
/// `Chat::type` — `"private"`, `"group"`, `"supergroup"`, `"channel"`.
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub id: ChatId,
    pub title: String,
    pub kind: String,
    /// Public `@username` when the chat has one (channels and
    /// supergroups; private chats expose the user's username).
    pub username: Option<String>,
}

/// Runtime surface of the Telegram service. Each method corresponds
/// one-to-one with a verb reachable from `zad telegram …`.
#[async_trait]
pub trait TelegramTransport: Send + Sync {
    async fn send(&self, chat: ChatId, body: &str) -> Result<i64>;
    async fn history(&self, chat: ChatId, limit: usize) -> Result<Vec<ChatMessage>>;
    async fn list_chats(&self) -> Result<Vec<ChatInfo>>;
}

#[async_trait]
impl TelegramTransport for TelegramHttp {
    async fn send(&self, _chat: ChatId, _body: &str) -> Result<i64> {
        // TODO: POST /bot<token>/sendMessage { chat_id, text }.
        // Return the message_id from the response envelope. Scope:
        // `messages.send`.
        todo!("telegram send: not yet implemented")
    }

    async fn history(&self, _chat: ChatId, _limit: usize) -> Result<Vec<ChatMessage>> {
        // TODO: getUpdates is long-poll and forward-only — it only
        // returns *new* updates, never historical backfill. Options:
        //   (a) require a webhook setup and serve it locally; or
        //   (b) accept that `read` only works for updates that have
        //       accumulated since the last `getUpdates` call.
        // Pick (b) for the first cut and document the limitation in
        // the manpage. Scope: `messages.read`.
        todo!("telegram history: not yet implemented")
    }

    async fn list_chats(&self) -> Result<Vec<ChatInfo>> {
        // TODO: no direct "list all chats the bot is in" endpoint
        // exists. Source chats from the local directory cache plus
        // any updates seen in a recent `getUpdates` window.
        // Scope: `chats`.
        todo!("telegram list_chats: not yet implemented")
    }
}

/// Preview transport used when the caller passed `--dry-run`.
///
/// Intercepts every mutating verb (`send`) by emitting a [`DryRunOp`]
/// to the sink and returning a stub success value (`0` for message
/// IDs). Read verbs (`history`, `list_chats`) return empty vectors
/// rather than delegating to a live client, because dry-run is
/// intentionally decoupled from credentials: no token is ever loaded
/// in dry-run mode, which keeps `--dry-run` usable before a bot is
/// configured.
pub struct DryRunTelegramTransport {
    sink: Arc<dyn DryRunSink>,
}

impl DryRunTelegramTransport {
    pub fn new(sink: Arc<dyn DryRunSink>) -> Self {
        Self { sink }
    }

    fn record(&self, verb: &'static str, summary: String, details: serde_json::Value) {
        self.sink.record(DryRunOp {
            service: "telegram",
            verb,
            summary,
            details,
        });
    }
}

#[async_trait]
impl TelegramTransport for DryRunTelegramTransport {
    async fn send(&self, chat: ChatId, body: &str) -> Result<i64> {
        let len = body.chars().count();
        self.record(
            "send",
            format!("would send {len} chars to chat {chat}"),
            json!({
                "command": "telegram.send",
                "chat_id": chat.to_string(),
                "body": body,
                "body_chars": len,
            }),
        );
        Ok(0)
    }

    async fn history(&self, _chat: ChatId, _limit: usize) -> Result<Vec<ChatMessage>> {
        Ok(vec![])
    }

    async fn list_chats(&self) -> Result<Vec<ChatInfo>> {
        Ok(vec![])
    }
}
