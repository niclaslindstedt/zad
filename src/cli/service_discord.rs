//! Discord's plug-in to the generic service lifecycle.
//!
//! Everything in this file is Discord-specific — scope names, prompts
//! for application ID and default guild, the shape of the Discord
//! credential (`DiscordSecrets` = one bot token), and the call that
//! validates the token against the Discord API. The generic plumbing
//! (flag parsing, path resolution, JSON envelopes, human banners,
//! keychain I/O sequencing) lives in `src/cli/lifecycle.rs` and is
//! shared with every other service.
//!
//! See `docs/services.md#adding-a-new-service` for the recipe a new
//! service would follow. This file is the first — and, until
//! Telegram/Slack/etc. land, only — implementation of that recipe.

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Input, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    BotTokenArgs, CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef,
    resolve_bot_token, resolve_scopes,
};
use crate::config::{DiscordServiceCfg, ProjectConfig};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::discord::DiscordHttp;

const DEFAULT_SCOPES: &[&str] = &["guilds", "messages.read", "messages.send"];
const ALL_SCOPES: &[&str] = &[
    "guilds",
    "messages.read",
    "messages.send",
    "channels.manage",
    "gateway.listen",
];

// ---------------------------------------------------------------------------
// Discord's credential shape
// ---------------------------------------------------------------------------

/// Discord only uses one secret — the long-lived bot token — so
/// `Secrets` wraps it in a named struct rather than `String` for
/// parity with services that need richer shapes.
pub struct DiscordSecrets {
    pub bot_token: String,
}

// ---------------------------------------------------------------------------
// Discord's `zad service create discord` args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub token: BotTokenArgs,
    #[command(flatten)]
    pub scopes: ScopesArg,
    /// Discord application (bot) ID.
    #[arg(long)]
    pub application_id: Option<String>,
    /// Optional default guild (server) ID.
    #[arg(long)]
    pub default_guild: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// The trait impl — this is the entire Discord-specific surface
// ---------------------------------------------------------------------------

pub struct DiscordLifecycle;

#[async_trait]
impl LifecycleService for DiscordLifecycle {
    const NAME: &'static str = "discord";
    const DISPLAY: &'static str = "Discord";
    type Cfg = DiscordServiceCfg;
    type Secrets = DiscordSecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_discord();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_discord();
    }

    fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(DiscordServiceCfg, DiscordSecrets)> {
        let application_id =
            resolve_application_id(args.application_id.as_deref(), non_interactive)?;
        let default_guild = resolve_default_guild(args.default_guild.as_deref(), non_interactive)?;
        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(),
            DEFAULT_SCOPES,
            ALL_SCOPES,
            non_interactive,
        )?;
        let bot_token = resolve_bot_token(
            args.token.bot_token.as_deref(),
            args.token.bot_token_env.as_deref(),
            non_interactive,
            Self::DISPLAY,
        )?;
        Ok((
            DiscordServiceCfg {
                application_id,
                scopes,
                default_guild,
            },
            DiscordSecrets { bot_token },
        ))
    }

    async fn validate(_cfg: &DiscordServiceCfg, creds: &DiscordSecrets) -> Result<String> {
        DiscordHttp::unscoped(&creds.bot_token)
            .validate_token()
            .await
            .map_err(|e| ZadError::Service {
                name: Self::NAME,
                message: format!("token validation failed: {e}"),
            })
    }

    fn store_secrets(creds: &DiscordSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
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

    fn cfg_human(cfg: &DiscordServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![("app id", cfg.application_id.clone())];
        if let Some(g) = &cfg.default_guild {
            out.push(("guild", g.clone()));
        }
        out
    }

    fn cfg_json(cfg: &DiscordServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "application_id": cfg.application_id,
            "default_guild": cfg.default_guild,
        })
    }

    fn scopes_of(cfg: &DiscordServiceCfg) -> &[String] {
        &cfg.scopes
    }
}

// ---------------------------------------------------------------------------
// Discord-specific prompt helpers
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn resolve_application_id(flag: Option<&str>, non_interactive: bool) -> Result<String> {
    if let Some(v) = flag {
        return validate_numeric(v, "application-id").map(|_| v.to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired("--application-id"));
    }
    let v: String = Input::with_theme(&theme())
        .with_prompt("Discord application ID")
        .validate_with(|s: &String| validate_numeric(s, "application-id").map(|_| ()))
        .interact_text()?;
    Ok(v)
}

fn resolve_default_guild(flag: Option<&str>, non_interactive: bool) -> Result<Option<String>> {
    if let Some(v) = flag {
        validate_numeric(v, "default-guild")?;
        return Ok(Some(v.to_string()));
    }
    if non_interactive {
        return Ok(None);
    }
    let v: String = Input::with_theme(&theme())
        .with_prompt("Default guild ID (leave blank for none)")
        .allow_empty(true)
        .interact_text()?;
    if v.trim().is_empty() {
        Ok(None)
    } else {
        validate_numeric(&v, "default-guild").map(|_| Some(v))
    }
}

fn validate_numeric(v: &str, field: &'static str) -> Result<()> {
    if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
        Ok(())
    } else {
        Err(ZadError::Invalid(format!(
            "{field} must be a numeric Discord snowflake, got `{v}`"
        )))
    }
}
