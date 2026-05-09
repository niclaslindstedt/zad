//! Telegram HTTP client.
//!
//! Hand-rolled REST wrapper around Telegram's Bot API. Exposes the
//! narrow surface the CLI verbs need: `validate_token` for the
//! lifecycle `create` flow, `send_message` for `zad telegram send`,
//! and `get_updates` for `read` / `chats` / `discover`.
//!
//! ## Why not a higher-level crate?
//!
//! A dependency like `teloxide` or `frankenstein` would hand us typed
//! wrappers for every Bot API method, but they pull in a much larger
//! dep tree than we need for our handful of endpoints. `reqwest` is
//! already a direct dependency, so a hand-rolled REST client here has
//! near-zero binary-size cost and keeps full control over timeouts,
//! retries, and error mapping.
//!
//! ## Error mapping
//!
//! Every Bot API response envelope is `{ ok: bool, description: String,
//! result: T }`. A non-`ok` response or a non-2xx HTTP status surfaces
//! as `ZadError::Service { name: "telegram", message: ... }`. Callers
//! never need to know about `reqwest::Error`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Result, ZadError};

/// Telegram's hard cap on a single text message's character count. The
/// Bot API counts UTF-16 code units, but Telegram's documented limit is
/// 4096 *characters*; zad enforces the stricter codepoint count to stay
/// safe on the server side.
pub const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

/// Telegram's hard cap on a media caption (used by `sendDocument` and
/// `sendMediaGroup`). Narrower than the plain text limit, so when
/// attachments are present the CLI enforces this value instead.
pub const TELEGRAM_MAX_CAPTION_LEN: usize = 1024;

/// Upper bound on the number of files in a single `sendMediaGroup`
/// call. The lower bound is 2 (groups of one are rejected by the API);
/// a single file must use `sendDocument` instead.
pub const TELEGRAM_MAX_MEDIA_GROUP: usize = 10;

const API_BASE: &str = "https://api.telegram.org";

/// Thin REST wrapper around Telegram's Bot API.
///
/// Carries the declared scope set so every runtime method can gate
/// itself locally before touching the network. `config_path` is only
/// used to make scope-denied error messages actionable.
#[derive(Clone)]
pub struct TelegramHttp {
    token: String,
    scopes: BTreeSet<String>,
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

    fn require_scope(&self, scope: &'static str) -> Result<()> {
        if self.scopes.contains(scope) {
            return Ok(());
        }
        Err(ZadError::ScopeDenied {
            service: "telegram",
            scope,
            config_path: self.config_path.clone(),
        })
    }

    /// Check that **at least one** of the candidate scopes is declared.
    /// Used by endpoints that legitimately back multiple verbs
    /// (`getUpdates` underlies `read`, `chats`, and `discover`), so a
    /// single fixed scope would be a lie. The CLI layer still enforces
    /// the verb-specific scope up front; this is the library-level
    /// defense in depth.
    fn require_any_scope(&self, scopes: &[&'static str]) -> Result<()> {
        if scopes.iter().any(|s| self.scopes.contains(*s)) {
            return Ok(());
        }
        Err(ZadError::ScopeDenied {
            service: "telegram",
            scope: scopes[0],
            config_path: self.config_path.clone(),
        })
    }

    /// Call `getMe`, returning the bot's full identity (numeric ID,
    /// `@username`, first name). Used by the lifecycle driver during
    /// `zad service create telegram` to confirm the token is live, and
    /// by the self-capture flow to learn the bot's username (to prompt
    /// `"send /start to @{botname}"`) and its numeric ID (to ignore
    /// bot-sourced messages when picking the user's private chat).
    ///
    /// No scope check: this runs before scopes are persisted.
    pub async fn get_me(&self) -> Result<BotIdentity> {
        let envelope: ApiEnvelope<GetMeResult> = self.get("getMe", &[]).await?;
        let me = envelope.into_result()?;
        if !me.is_bot {
            return Err(ZadError::Service {
                name: "telegram",
                message: "Telegram reports this token as a non-bot account".into(),
            });
        }
        Ok(BotIdentity {
            id: me.id,
            username: me.username,
            first_name: me.first_name,
        })
    }

    /// Call `getMe` and return only the display name. Preserved as a
    /// thin wrapper because the lifecycle's `validate()` hook wants a
    /// single string; structured callers use [`Self::get_me`] directly.
    pub async fn validate_token(&self) -> Result<String> {
        let me = self.get_me().await?;
        Ok(me.username.unwrap_or(me.first_name))
    }

    /// POST `/sendMessage`. Returns the Bot API's `message_id` on
    /// success. Scope: `messages.send`.
    pub async fn send_message(&self, chat: i64, body: &str) -> Result<i64> {
        self.require_scope("messages.send")?;
        let len = body.chars().count();
        if len > TELEGRAM_MAX_MESSAGE_LEN {
            return Err(ZadError::Invalid(format!(
                "message body is {len} characters; Telegram's hard limit is {TELEGRAM_MAX_MESSAGE_LEN}"
            )));
        }
        let payload = serde_json::json!({ "chat_id": chat, "text": body });
        let envelope: ApiEnvelope<SentMessage> = self.post("sendMessage", &payload).await?;
        let msg = envelope.into_result()?;
        Ok(msg.message_id)
    }

    /// POST `/sendDocument` with a single file. When `body` is
    /// non-empty it is sent as the file's `caption` (Telegram's caption
    /// cap is [`TELEGRAM_MAX_CAPTION_LEN`], enforced at the CLI layer
    /// before reaching here). Scope: `messages.send`.
    pub async fn send_document(&self, chat: i64, body: &str, file: &Path) -> Result<i64> {
        self.require_scope("messages.send")?;
        let part = file_part(file).await?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat.to_string())
            .part("document", part);
        if !body.is_empty() {
            form = form.text("caption", body.to_string());
        }
        let envelope: ApiEnvelope<SentMessage> = self.post_multipart("sendDocument", form).await?;
        let msg = envelope.into_result()?;
        Ok(msg.message_id)
    }

    /// POST `/sendMediaGroup` with 2-10 files. Each file is attached as
    /// a multipart part named `f0`, `f1`, … and referenced from the
    /// `media` JSON via `attach://f<i>`. The caption, if non-empty, is
    /// placed on the first item only — subsequent items carry no
    /// caption. Returns the `message_id` of the first item in the
    /// group (the Bot API returns a `Vec`, and the CLI surface only
    /// prints one id). Scope: `messages.send`.
    pub async fn send_media_group(&self, chat: i64, body: &str, files: &[PathBuf]) -> Result<i64> {
        self.require_scope("messages.send")?;
        if files.len() < 2 || files.len() > TELEGRAM_MAX_MEDIA_GROUP {
            return Err(ZadError::Invalid(format!(
                "sendMediaGroup requires 2..={TELEGRAM_MAX_MEDIA_GROUP} files, got {}",
                files.len()
            )));
        }
        let mut form = reqwest::multipart::Form::new().text("chat_id", chat.to_string());
        let mut media_entries: Vec<serde_json::Value> = Vec::with_capacity(files.len());
        for (i, path) in files.iter().enumerate() {
            let name = format!("f{i}");
            let part = file_part(path).await?;
            form = form.part(name.clone(), part);
            let mut entry = serde_json::json!({
                "type": "document",
                "media": format!("attach://{name}"),
            });
            if i == 0 && !body.is_empty() {
                entry["caption"] = serde_json::Value::String(body.to_string());
            }
            media_entries.push(entry);
        }
        form = form.text(
            "media",
            serde_json::to_string(&serde_json::Value::Array(media_entries))
                .expect("serde_json::Value serialization cannot fail"),
        );
        let envelope: ApiEnvelope<Vec<SentMessage>> =
            self.post_multipart("sendMediaGroup", form).await?;
        let msgs = envelope.into_result()?;
        msgs.into_iter()
            .next()
            .map(|m| m.message_id)
            .ok_or_else(|| ZadError::Service {
                name: "telegram",
                message: "sendMediaGroup returned an empty message list".into(),
            })
    }

    /// GET `/getUpdates`. Returns whatever updates Telegram has buffered
    /// since the last call with `timeout=0` so we never long-poll.
    ///
    /// The Bot API's update stream is **forward-only** — callers can't
    /// fetch historical messages through this endpoint. The `offset`
    /// parameter is accepted for completeness, but every zad verb
    /// today passes `None` (= "whatever's currently buffered").
    ///
    /// Scope: at least one of `messages.read`, `chats`. The CLI enforces
    /// the verb-specific scope before reaching this point; the check
    /// here is the library layer's defense in depth.
    pub async fn get_updates(&self, offset: Option<i64>) -> Result<Vec<Update>> {
        self.require_any_scope(&["messages.read", "chats"])?;
        self.get_updates_inner(offset).await
    }

    /// Same as [`Self::get_updates`] but without the scope check.
    /// Intended for lifecycle flows (e.g. self-chat capture during
    /// `service create`) that run on an [`Self::unscoped`] client.
    pub async fn get_updates_unscoped(&self, offset: Option<i64>) -> Result<Vec<Update>> {
        self.get_updates_inner(offset).await
    }

    async fn get_updates_inner(&self, offset: Option<i64>) -> Result<Vec<Update>> {
        let mut query: Vec<(&str, String)> = vec![("timeout", "0".into())];
        if let Some(o) = offset {
            query.push(("offset", o.to_string()));
        }
        let envelope: ApiEnvelope<Vec<Update>> = self.get("getUpdates", &query).await?;
        envelope.into_result()
    }

    // -----------------------------------------------------------------
    // low-level HTTP glue
    // -----------------------------------------------------------------

    fn url(&self, method: &str) -> String {
        format!("{API_BASE}/bot{}/{method}", self.token)
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        query: &[(&str, String)],
    ) -> Result<ApiEnvelope<T>> {
        let resp = reqwest::Client::new()
            .get(self.url(method))
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        decode_envelope(resp).await
    }

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<ApiEnvelope<T>> {
        let resp = reqwest::Client::new()
            .post(self.url(method))
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        decode_envelope(resp).await
    }

    async fn post_multipart<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        form: reqwest::multipart::Form,
    ) -> Result<ApiEnvelope<T>> {
        let resp = reqwest::Client::new()
            .post(self.url(method))
            .multipart(form)
            .send()
            .await
            .map_err(network_err)?;
        decode_envelope(resp).await
    }
}

/// Load a file from disk into a multipart `Part`, carrying the basename
/// along so Telegram stores the upload under the author-chosen name
/// rather than `blob`. Errors map to `ZadError::Invalid` with the path
/// embedded so the operator can fix a typo without a stack trace.
///
/// Buffers the whole file in memory. Fine for the practical sizes we
/// care about here (Telegram's `sendDocument` cap is 50 MiB via the
/// Bot API); large-file streaming would need reqwest's `stream`
/// feature, which is not currently enabled.
async fn file_part(path: &Path) -> Result<reqwest::multipart::Part> {
    let bytes = tokio::fs::read(path).await.map_err(|e| {
        ZadError::Invalid(format!("attachment `{}` not readable: {e}", path.display()))
    })?;
    let basename = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Ok(reqwest::multipart::Part::bytes(bytes).file_name(basename))
}

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "telegram",
        message: format!("network error talking to Telegram: {e}"),
    }
}

async fn decode_envelope<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
) -> Result<ApiEnvelope<T>> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ZadError::Service {
            name: "telegram",
            message: format!("HTTP {status}: {body}"),
        });
    }
    resp.json::<ApiEnvelope<T>>()
        .await
        .map_err(|e| ZadError::Service {
            name: "telegram",
            message: format!("failed to decode Telegram response: {e}"),
        })
}

// ---------------------------------------------------------------------------
// Bot API response envelope + the fragments our verbs consume. Every type
// here is `pub(crate)` or below — the CLI / transport layers work against
// the projected domain types in `transport.rs`, not these raw shapes.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ApiEnvelope<T> {
    ok: bool,
    description: Option<String>,
    result: Option<T>,
}

impl<T> ApiEnvelope<T> {
    fn into_result(self) -> Result<T> {
        if !self.ok {
            return Err(ZadError::Service {
                name: "telegram",
                message: self
                    .description
                    .unwrap_or_else(|| "Telegram returned ok=false without a description".into()),
            });
        }
        self.result.ok_or(ZadError::Service {
            name: "telegram",
            message: "Telegram returned ok=true without a result payload".into(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct GetMeResult {
    id: i64,
    is_bot: bool,
    first_name: String,
    username: Option<String>,
}

/// Subset of `getMe` the caller gets back from [`TelegramHttp::get_me`].
/// `username` is optional because Telegram bots can, in theory, ship
/// without one (though BotFather enforces it today); `first_name` is
/// always present and is used as a display fallback.
#[derive(Debug, Clone)]
pub struct BotIdentity {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
}

#[derive(Debug, Deserialize)]
struct SentMessage {
    message_id: i64,
}

/// Projected shape of a Bot API `Update` envelope. We keep only the
/// fields zad actually consumes: the surrounding update id (for offset
/// bookkeeping if we ever want it) and whichever of the optional
/// message slots carries a payload. `message`, `channel_post`,
/// `edited_message`, and `edited_channel_post` all expose the same
/// `Message` shape; treating them uniformly means a bot in a channel
/// sees channel posts through the same `read` / `chats` surfaces.
#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    #[allow(dead_code)]
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<UpdateMessage>,
    #[serde(default)]
    pub edited_message: Option<UpdateMessage>,
    #[serde(default)]
    pub channel_post: Option<UpdateMessage>,
    #[serde(default)]
    pub edited_channel_post: Option<UpdateMessage>,
    #[serde(default)]
    pub my_chat_member: Option<ChatMemberUpdate>,
}

impl Update {
    /// Every message-bearing slot in the envelope. Used by `read` to
    /// filter by chat and by `chats` / `discover` to harvest
    /// descriptors.
    pub fn messages(&self) -> impl Iterator<Item = &UpdateMessage> {
        [
            self.message.as_ref(),
            self.edited_message.as_ref(),
            self.channel_post.as_ref(),
            self.edited_channel_post.as_ref(),
        ]
        .into_iter()
        .flatten()
    }

    /// Every chat referenced by this update, regardless of which slot
    /// carries it.
    pub fn chats(&self) -> impl Iterator<Item = &UpdateChat> {
        self.messages()
            .map(|m| &m.chat)
            .chain(self.my_chat_member.iter().map(|m| &m.chat))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMessage {
    pub message_id: i64,
    pub chat: UpdateChat,
    #[serde(default)]
    pub from: Option<UpdateUser>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

impl UpdateMessage {
    /// Body used for `read` output. Prefers `text`; falls back to
    /// `caption` for media with a caption; otherwise an empty string
    /// so the output format stays stable.
    pub fn body(&self) -> String {
        self.text
            .clone()
            .or_else(|| self.caption.clone())
            .unwrap_or_default()
    }

    /// Author display used for `read` output. Username if set, else
    /// the first name, else the numeric user id, else `unknown`.
    pub fn author(&self) -> String {
        let Some(from) = &self.from else {
            return "unknown".into();
        };
        if let Some(u) = &from.username {
            return u.clone();
        }
        if !from.first_name.is_empty() {
            return from.first_name.clone();
        }
        from.id.to_string()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChat {
    pub id: i64,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
}

impl UpdateChat {
    /// Best-effort human-readable title. Groups/channels carry
    /// `title`; private chats only have `first_name` + `username`.
    /// Falls back to the stringified id so downstream code always has
    /// something to print.
    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title
            && !t.is_empty()
        {
            return t.clone();
        }
        if let Some(u) = &self.username
            && !u.is_empty()
        {
            return u.clone();
        }
        if let Some(f) = &self.first_name
            && !f.is_empty()
        {
            return f.clone();
        }
        self.id.to_string()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUser {
    pub id: i64,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMemberUpdate {
    pub chat: UpdateChat,
}
