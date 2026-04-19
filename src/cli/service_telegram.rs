//! Telegram's plug-in to the generic service lifecycle.
//!
//! Everything in this file is Telegram-specific — scope names, the
//! prompt for a default chat, the shape of the Telegram credential
//! (`TelegramSecrets` = one bot token), and the call that validates
//! the token against the Bot API. The generic plumbing (flag parsing,
//! path resolution, JSON envelopes, human banners, keychain I/O
//! sequencing) lives in `src/cli/lifecycle.rs` and is shared with
//! every other service.
//!
//! See `docs/services.md#adding-a-new-service` for the full recipe.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Confirm, Input, Password, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    BotTokenArgs, CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef,
    resolve_scopes,
};
use crate::config::{ProjectConfig, TelegramServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::telegram::TelegramHttp;
use crate::service::telegram::client::BotIdentity;

/// How long `capture_self_chat` will wait for the user to send a
/// message to the bot before giving up. Picked to be long enough for a
/// context-switch to the Telegram client, short enough to fail fast if
/// the user never sends anything.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(1500);

const DEFAULT_SCOPES: &[&str] = &["chats", "messages.read", "messages.send"];
const ALL_SCOPES: &[&str] = &["chats", "messages.read", "messages.send", "gateway.listen"];

// ---------------------------------------------------------------------------
// Telegram's credential shape
// ---------------------------------------------------------------------------

/// Telegram uses one long-lived bot token, issued by @BotFather. It
/// carries the bot's identity on its own — there is no separate
/// application ID.
pub struct TelegramSecrets {
    pub bot_token: String,
}

// ---------------------------------------------------------------------------
// Telegram's `zad service create telegram` args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub token: BotTokenArgs,
    #[command(flatten)]
    pub scopes: ScopesArg,
    /// Optional default chat for verbs that omit `--chat`. Accepts a
    /// numeric chat ID (negative for groups/supergroups), a
    /// `@username` (channels, public supergroups), or a directory
    /// alias.
    #[arg(long)]
    pub default_chat: Option<String>,
    /// Private-chat ID for the human user this bot belongs to.
    /// Resolved from the literal `@me` in later send targets. If
    /// omitted in interactive mode, `create` offers to capture it by
    /// polling for your first message to the bot; if omitted in
    /// non-interactive mode, the field is left unset (you can fill
    /// it later via `zad telegram self capture|set`).
    #[arg(long)]
    pub self_chat: Option<i64>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// The trait impl — this is the entire Telegram-specific surface
// ---------------------------------------------------------------------------

pub struct TelegramLifecycle;

#[async_trait]
impl LifecycleService for TelegramLifecycle {
    const NAME: &'static str = "telegram";
    const DISPLAY: &'static str = "Telegram";
    type Cfg = TelegramServiceCfg;
    type Secrets = TelegramSecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_telegram();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_telegram();
    }

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(TelegramServiceCfg, TelegramSecrets)> {
        let open_browser = !args.base.no_browser;
        let default_chat = resolve_default_chat(args.default_chat.as_deref(), non_interactive)?;
        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(),
            DEFAULT_SCOPES,
            ALL_SCOPES,
            non_interactive,
        )?;
        let bot_token = resolve_telegram_bot_token(
            args.token.bot_token.as_deref(),
            args.token.bot_token_env.as_deref(),
            open_browser,
            non_interactive,
        )?;
        let self_chat_id =
            resolve_self_chat_id(args.self_chat, &bot_token, open_browser, non_interactive).await?;
        Ok((
            TelegramServiceCfg {
                scopes,
                default_chat,
                self_chat_id,
            },
            TelegramSecrets { bot_token },
        ))
    }

    async fn validate(_cfg: &TelegramServiceCfg, creds: &TelegramSecrets) -> Result<String> {
        TelegramHttp::unscoped(&creds.bot_token)
            .validate_token()
            .await
            .map_err(|e| ZadError::Service {
                name: Self::NAME,
                message: format!("token validation failed: {e}"),
            })
    }

    fn store_secrets(creds: &TelegramSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        secrets::store(&account, &creds.bot_token)?;
        Ok(vec![SecretRef {
            label: "token",
            account,
            present: true,
        }])
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        secrets::delete(&account)?;
        Ok(vec![SecretRef {
            label: "token",
            account,
            present: false,
        }])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        let present = secrets::load(&account)?.is_some();
        Ok(vec![SecretRef {
            label: "token",
            account,
            present,
        }])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<TelegramSecrets>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        Ok(secrets::load(&account)?.map(|bot_token| TelegramSecrets { bot_token }))
    }

    fn cfg_human(cfg: &TelegramServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(c) = &cfg.default_chat {
            out.push(("chat", c.clone()));
        }
        if let Some(id) = cfg.self_chat_id {
            out.push(("self", id.to_string()));
        }
        out
    }

    fn cfg_json(cfg: &TelegramServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "default_chat": cfg.default_chat,
            "self_chat_id": cfg.self_chat_id,
        })
    }

    fn scopes_of(cfg: &TelegramServiceCfg) -> &[String] {
        &cfg.scopes
    }

    // No `post_create_hint`: Telegram bots are added to chats by an
    // admin pasting `@botname` into the chat, not by visiting a URL.
    // Offering a link would be misleading.
}

// ---------------------------------------------------------------------------
// Telegram-specific prompt helpers
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn resolve_default_chat(flag: Option<&str>, non_interactive: bool) -> Result<Option<String>> {
    if let Some(v) = flag {
        validate_chat(v)?;
        return Ok(Some(v.to_string()));
    }
    if non_interactive {
        return Ok(None);
    }

    println!();
    println!("Default chat accepts any of:");
    println!("  • @username           (public channel or supergroup)");
    println!("  • numeric chat ID     (e.g. -1001234567890 for a group)");
    println!("  • alias               (resolved later via the directory)");
    println!("For private chats, message @userinfobot to get your chat ID.");
    println!("Leave blank to skip — you can set a default chat later.");

    let v: String = Input::with_theme(&theme())
        .with_prompt("Default chat ID, @username, or alias (leave blank for none)")
        .allow_empty(true)
        .interact_text()?;
    if v.trim().is_empty() {
        Ok(None)
    } else {
        validate_chat(&v).map(|_| Some(v))
    }
}

/// Telegram-specific bot-token prompt: same flag/env contract as the
/// generic `resolve_bot_token`, but the interactive path surfaces (and
/// optionally opens) the @BotFather chat, since that's the only source
/// of a Telegram bot token — there's no developer portal.
fn resolve_telegram_bot_token(
    flag: Option<&str>,
    env_flag: Option<&str>,
    open_browser: bool,
    non_interactive: bool,
) -> Result<String> {
    if let Some(env) = env_flag {
        return std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()));
    }
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired("--bot-token or --bot-token-env"));
    }

    let url = BOTFATHER_URL;
    println!();
    println!("Telegram bot tokens are issued by @BotFather:");
    println!("  {url}");
    println!("Send /newbot to create a bot, or /mybots → pick a bot → \"API Token\"");
    println!("for an existing one. Copy the token and paste it below.");
    if open_browser {
        let _ = open::that(url);
    }

    let v = Password::with_theme(&theme())
        .with_prompt("Telegram bot token")
        .interact()?;
    Ok(v)
}

const BOTFATHER_URL: &str = "https://t.me/BotFather";

/// Lightweight sanity-check on a chat reference. Accepts:
///
/// - a signed decimal integer (`12345`, `-1001234567890`),
/// - a `@username` (letters/digits/underscores, at least 5 chars per
///   Telegram's rule),
/// - a bare alias (anything non-empty that isn't obviously neither of
///   the above).
///
/// The real membership / reachability check happens at the Bot API,
/// when the first runtime verb fires.
fn validate_chat(v: &str) -> Result<()> {
    let trimmed = v.trim();
    if trimmed.is_empty() {
        return Err(ZadError::Invalid("default-chat must not be empty".into()));
    }
    if trimmed.parse::<i64>().is_ok() {
        return Ok(());
    }
    if let Some(name) = trimmed.strip_prefix('@') {
        if name.len() >= 5 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Ok(());
        }
        return Err(ZadError::Invalid(format!(
            "default-chat `{v}` looks like a @username but isn't valid (5+ chars, [A-Za-z0-9_])"
        )));
    }
    // Bare alias — accept anything non-whitespace for the directory to
    // resolve later.
    if trimmed.chars().any(char::is_whitespace) {
        return Err(ZadError::Invalid(format!(
            "default-chat `{v}` contains whitespace"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Self-chat capture
// ---------------------------------------------------------------------------

/// Snapshot of the first private-chat message we saw during capture.
/// `chat_id` is the value persisted to config; the remaining fields are
/// used only to render the confirmation prompt.
#[derive(Debug, Clone)]
pub struct CapturedChat {
    pub chat_id: i64,
    pub first_name: String,
    pub username: Option<String>,
}

/// Resolve `self_chat_id` for `zad service create telegram`. If
/// `--self-chat` was passed, use it verbatim. In non-interactive mode
/// we leave it unset (the user can run `zad telegram self set` later).
/// Interactively, we show the bot's `@username` (from `getMe`), ask if
/// the user wants to capture now, and on `yes` run the polling loop.
async fn resolve_self_chat_id(
    flag: Option<i64>,
    bot_token: &str,
    open_browser: bool,
    non_interactive: bool,
) -> Result<Option<i64>> {
    if let Some(id) = flag {
        return Ok(Some(id));
    }
    if non_interactive {
        return Ok(None);
    }

    let client = TelegramHttp::unscoped(bot_token);
    let identity = client.get_me().await.map_err(|e| ZadError::Service {
        name: "telegram",
        message: format!("getMe failed while preparing self-chat capture: {e}"),
    })?;

    println!();
    println!("Optional: configure `@me` so commands like");
    println!("  zad telegram send --chat @me \"hello\"");
    println!("resolve to your own private chat with the bot.");

    let want = Confirm::with_theme(&theme())
        .with_prompt("Capture your self-chat now?")
        .default(true)
        .interact()?;
    if !want {
        println!(
            "Skipping. Run `zad telegram self capture` or `zad telegram self set <id>` later."
        );
        return Ok(None);
    }

    match capture_self_chat(&client, &identity, open_browser).await? {
        Some(c) => Ok(Some(c.chat_id)),
        None => Ok(None),
    }
}

/// Poll `getUpdates` for up to [`CAPTURE_TIMEOUT`] seconds, looking for
/// the first private-chat message whose `from.id` differs from the
/// bot's own ID. Returns `Some(CapturedChat)` on success (and after the
/// user confirms the detected identity), `None` if the user skipped or
/// the timeout elapsed.
///
/// Shared between the create-time path and `zad telegram self capture`
/// so both use the exact same prompts and filtering.
pub async fn capture_self_chat(
    client: &TelegramHttp,
    identity: &BotIdentity,
    open_browser: bool,
) -> Result<Option<CapturedChat>> {
    let handle = identity
        .username
        .as_deref()
        .map(|u| format!("@{u}"))
        .unwrap_or_else(|| identity.first_name.clone());
    let bot_url = identity
        .username
        .as_deref()
        .map(|u| format!("https://t.me/{u}"));

    println!();
    println!("Open Telegram and send {handle} any message (e.g. /start).");
    if let Some(url) = &bot_url {
        println!("  {url}");
        if open_browser {
            let _ = open::that(url);
        }
    }
    println!(
        "Waiting up to {}s for your message…",
        CAPTURE_TIMEOUT.as_secs()
    );

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let bot_id = identity.id;
    while Instant::now() < deadline {
        let updates = client
            .get_updates_unscoped(None)
            .await
            .map_err(|e| ZadError::Service {
                name: "telegram",
                message: format!("getUpdates failed during capture: {e}"),
            })?;
        for update in &updates {
            if let Some(msg) = update.message.as_ref()
                && msg.chat.kind == "private"
                && let Some(from) = msg.from.as_ref()
                && from.id != bot_id
            {
                let captured = CapturedChat {
                    chat_id: msg.chat.id,
                    first_name: from.first_name.clone(),
                    username: from.username.clone(),
                };
                return confirm_captured(captured);
            }
        }
        tokio::time::sleep(CAPTURE_POLL_INTERVAL).await;
    }

    println!(
        "No message received within {}s. Skipping — run `zad telegram self capture` when you're ready.",
        CAPTURE_TIMEOUT.as_secs()
    );
    Ok(None)
}

fn confirm_captured(c: CapturedChat) -> Result<Option<CapturedChat>> {
    let handle = c
        .username
        .as_deref()
        .map(|u| format!(" (@{u})"))
        .unwrap_or_default();
    let label = format!(
        "Identified you as {}{handle}, chat id {}.",
        c.first_name, c.chat_id
    );
    println!("  ✓ {label}");
    let ok = Confirm::with_theme(&theme())
        .with_prompt("Save as self-chat?")
        .default(true)
        .interact()?;
    if ok { Ok(Some(c)) } else { Ok(None) }
}
