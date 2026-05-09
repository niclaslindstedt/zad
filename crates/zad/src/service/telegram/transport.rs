//! The side-effectful surface that `zad telegram <verb>` depends on.
//!
//! [`TelegramTransport`] is a thin trait over the verb set exposed by
//! [`TelegramHttp`]. Its purpose is to let the CLI hold a
//! `Box<dyn TelegramTransport>` and stay oblivious to whether the
//! underlying implementation is the live Bot-API-backed client or a
//! `--dry-run` preview that never touches the network.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::{Result, ZadError};
use crate::service::telegram::client::{TELEGRAM_MAX_MEDIA_GROUP, TelegramHttp};
use crate::service::{DryRunOp, DryRunSink};

/// Signed chat identifier. Telegram chat IDs are negative for
/// (super)groups and positive for private chats and most channels.
pub type ChatId = i64;

/// A single message fetched from the Bot API. Shaped to match the
/// fields the CLI needs for `zad telegram read` — message id, author
/// username (or first-name fallback), and the text body. Further
/// fields (reply_to, entities, media) can be added alongside new verbs
/// that need them.
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
    async fn send(&self, chat: ChatId, body: &str, attachments: &[PathBuf]) -> Result<i64>;
    async fn history(&self, chat: ChatId, limit: usize) -> Result<Vec<ChatMessage>>;
    async fn list_chats(&self) -> Result<Vec<ChatInfo>>;
}

#[async_trait]
impl TelegramTransport for TelegramHttp {
    async fn send(&self, chat: ChatId, body: &str, attachments: &[PathBuf]) -> Result<i64> {
        match attachments.len() {
            0 => TelegramHttp::send_message(self, chat, body).await,
            1 => TelegramHttp::send_document(self, chat, body, &attachments[0]).await,
            n if n <= TELEGRAM_MAX_MEDIA_GROUP => {
                TelegramHttp::send_media_group(self, chat, body, attachments).await
            }
            n => Err(ZadError::Invalid(format!(
                "{n} attachments is above Telegram's sendMediaGroup cap of {TELEGRAM_MAX_MEDIA_GROUP}"
            ))),
        }
    }

    async fn history(&self, chat: ChatId, limit: usize) -> Result<Vec<ChatMessage>> {
        // Bot API's `getUpdates` is forward-only: it returns whatever
        // the bot has buffered since its previous `getUpdates` call.
        // Filter client-side to messages for `chat`, most recent first,
        // capped at `limit`. The manpage documents the "new messages
        // only" caveat.
        let updates = TelegramHttp::get_updates(self, None).await?;
        let mut out: Vec<ChatMessage> = Vec::new();
        for u in &updates {
            for m in u.messages() {
                if m.chat.id != chat {
                    continue;
                }
                out.push(ChatMessage {
                    id: m.message_id,
                    chat: m.chat.id,
                    author: m.author(),
                    body: m.body(),
                });
            }
        }
        // Most recent first, matching how the CLI surface documents the
        // ordering before rendering oldest-first for humans.
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out.truncate(limit);
        Ok(out)
    }

    async fn list_chats(&self) -> Result<Vec<ChatInfo>> {
        let updates = TelegramHttp::get_updates(self, None).await?;
        let mut seen: std::collections::BTreeMap<i64, ChatInfo> = std::collections::BTreeMap::new();
        for u in &updates {
            for c in u.chats() {
                seen.entry(c.id).or_insert_with(|| ChatInfo {
                    id: c.id,
                    title: c.display_title(),
                    kind: c.kind.clone(),
                    username: c.username.clone(),
                });
            }
        }
        Ok(seen.into_values().collect())
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
    async fn send(&self, chat: ChatId, body: &str, attachments: &[PathBuf]) -> Result<i64> {
        let len = body.chars().count();
        let atts: Vec<serde_json::Value> = attachments
            .iter()
            .map(|p| {
                let basename = p
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string());
                let bytes = std::fs::metadata(p).map(|m| m.len()).ok();
                json!({
                    "path": p.display().to_string(),
                    "basename": basename,
                    "bytes": bytes,
                })
            })
            .collect();
        let method = match attachments.len() {
            0 => "sendMessage",
            1 => "sendDocument",
            _ => "sendMediaGroup",
        };
        let summary = if atts.is_empty() {
            format!("would send {len} chars to chat {chat}")
        } else {
            format!(
                "would send {len} chars + {} file(s) to chat {chat} (via {method})",
                atts.len()
            )
        };
        self.record(
            "send",
            summary,
            json!({
                "command": "telegram.send",
                "method": method,
                "chat_id": chat.to_string(),
                "body": body,
                "body_chars": len,
                "attachments": atts,
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
