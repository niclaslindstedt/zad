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

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Input, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    BotTokenArgs, CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef,
    resolve_bot_token, resolve_scopes,
};
use crate::config::{ProjectConfig, TelegramServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::telegram::TelegramHttp;

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

    fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(TelegramServiceCfg, TelegramSecrets)> {
        let default_chat = resolve_default_chat(args.default_chat.as_deref(), non_interactive)?;
        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(),
            DEFAULT_SCOPES,
            ALL_SCOPES,
            non_interactive,
        )?;
        // Bot tokens come from a chat with @BotFather on Telegram, not
        // a web page — so we take the vanilla password-prompt path
        // without any developer-portal deep link.
        let bot_token = resolve_bot_token(
            args.token.bot_token.as_deref(),
            args.token.bot_token_env.as_deref(),
            non_interactive,
            Self::DISPLAY,
        )?;
        Ok((
            TelegramServiceCfg {
                scopes,
                default_chat,
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
        out
    }

    fn cfg_json(cfg: &TelegramServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "default_chat": cfg.default_chat,
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
