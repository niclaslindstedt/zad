//! Telegram HTTP client.
//!
//! Currently exposes just enough surface to validate a bot token during
//! `zad service create telegram` (`validate_token` → the Bot API's
//! `getMe` endpoint). The rest of the runtime surface — `send`,
//! `history`, `list_chats`, `list_updates` — is stubbed; filling those
//! in is tracked alongside `src/cli/telegram.rs`.
//!
//! ## Why not a higher-level crate?
//!
//! A dependency like `teloxide` or `frankenstein` would hand us typed
//! wrappers for every Bot API method, but they pull in a much larger
//! dep tree than we need for a single `getMe` call today. `reqwest` is
//! already transitively present via `serenity`, so a hand-rolled REST
//! client here has near-zero binary-size cost. When the runtime verbs
//! land we can revisit: staying on `reqwest` keeps full control over
//! timeouts, retries, and error mapping; moving to a typed crate
//! trades that control for less glue code.
//!
//! ## Error mapping
//!
//! Every Bot API response envelope is `{ ok: bool, description: String,
//! result: T }`. A non-`ok` response or a non-2xx HTTP status surfaces
//! as `ZadError::Service { name: "telegram", message: ... }`. Callers
//! never need to know about `reqwest::Error`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{Result, ZadError};

/// Telegram's hard cap on a single text message's character count. The
/// Bot API counts UTF-16 code units, but Telegram's documented limit is
/// 4096 *characters*; zad enforces the stricter codepoint count to stay
/// safe on the server side.
pub const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

const API_BASE: &str = "https://api.telegram.org";

/// Thin REST wrapper around Telegram's Bot API.
///
/// Carries the declared scope set so future runtime methods can gate
/// themselves locally before touching the network. `config_path` is
/// only used to make scope-denied error messages actionable.
#[derive(Clone)]
pub struct TelegramHttp {
    #[allow(dead_code)] // used by validate_token; future verbs will consume it too
    token: String,
    #[allow(dead_code)] // consumed by runtime verbs once they land
    scopes: BTreeSet<String>,
    #[allow(dead_code)] // embedded in ScopeDenied errors
    config_path: PathBuf,
}

impl TelegramHttp {
    /// Construct a client with a declared scope set. Every subsequent
    /// network method will check its required scope against this set
    /// before hitting the API. `config_path` is only used to make the
    /// scope-denied error actionable — no I/O happens against it here.
    pub fn new(token: &str, scopes: BTreeSet<String>, config_path: PathBuf) -> Self {
        Self {
            token: token.to_string(),
            scopes,
            config_path,
        }
    }

    /// Construct a client with no declared scopes. Only safe for code
    /// paths that call [`Self::validate_token`] and nothing else —
    /// `service create telegram` validates a token *before* scopes are
    /// persisted.
    pub fn unscoped(token: &str) -> Self {
        Self::new(token, BTreeSet::new(), PathBuf::new())
    }

    /// Call `getMe`, returning the bot's username on success. Used by
    /// the lifecycle driver during `zad service create telegram` to
    /// confirm the token is live before writing anything to disk.
    ///
    /// No scope check: this runs before scopes are persisted.
    pub async fn validate_token(&self) -> Result<String> {
        let url = format!("{API_BASE}/bot{}/getMe", self.token);
        let resp =
            reqwest::Client::new()
                .get(&url)
                .send()
                .await
                .map_err(|e| ZadError::Service {
                    name: "telegram",
                    message: format!("network error talking to Telegram: {e}"),
                })?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ZadError::Service {
                name: "telegram",
                message: format!("getMe returned HTTP {status}: {body}"),
            });
        }
        let envelope: ApiEnvelope<GetMeResult> =
            resp.json().await.map_err(|e| ZadError::Service {
                name: "telegram",
                message: format!("failed to decode Telegram response: {e}"),
            })?;
        if !envelope.ok {
            return Err(ZadError::Service {
                name: "telegram",
                message: envelope
                    .description
                    .unwrap_or_else(|| "Telegram rejected the token".into()),
            });
        }
        let me = envelope.result.ok_or(ZadError::Service {
            name: "telegram",
            message: "Telegram returned ok=true without a result payload".into(),
        })?;
        if !me.is_bot {
            return Err(ZadError::Service {
                name: "telegram",
                message: "Telegram reports this token as a non-bot account".into(),
            });
        }
        Ok(me.username.unwrap_or(me.first_name))
    }

    // TODO: send_message(chat_id, body) -> MessageId
    //   POST /bot<token>/sendMessage { chat_id, text }
    //   Scope: messages.send

    // TODO: get_updates(offset, limit) -> Vec<Update>
    //   GET /bot<token>/getUpdates?offset=...&limit=...&timeout=0
    //   Scope: messages.read
    //   Note: `getUpdates` is long-poll and incompatible with a webhook
    //   setup; documenting both paths in `permissions.toml` is out of
    //   scope for the stub.

    // TODO: get_chat(chat_id) -> Chat
    //   GET /bot<token>/getChat?chat_id=...
    //   Scope: chats

    // TODO: discover(): stream getUpdates briefly and cache
    //   chat_id → title/username in the telegram Directory.
    //   Scope: chats
}

// ---------------------------------------------------------------------------
// Response envelope types. Kept private to the module — callers only see
// domain types (once they exist) or the already-mapped ZadError.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    ok: bool,
    description: Option<String>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct GetMeResult {
    #[allow(dead_code)]
    id: i64,
    is_bot: bool,
    first_name: String,
    username: Option<String>,
}
