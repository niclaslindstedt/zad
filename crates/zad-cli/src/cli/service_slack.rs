//! Slack's plug-in to the generic service lifecycle.
//!
//! Slack bots use a single long-lived bot token (`xoxb-...`). An optional
//! App-Level Token (`xapp-...`) enables Socket Mode for real-time events
//! via `zad slack listen`. Without it, the service still works for all
//! send/read/channels/discover verbs.

use crate::cli::DialoguerExt;
use async_trait::async_trait;
use clap::Args;
use dialoguer::{Input, Password, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    BotTokenArgs, CliLifecycle, CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg,
    SecretRef, resolve_bot_token, resolve_scopes,
};
use zad::config::{ProjectConfig, SlackServiceCfg};
use zad::error::{Result, ZadError};
use zad::secrets::{self, Scope};
use zad::service::slack::client::SlackHttp;

const DEFAULT_SCOPES: &[&str] = &["chat:write", "channels:history", "channels:read"];
const ALL_SCOPES: &[&str] = &[
    "chat:write",
    "channels:history",
    "channels:read",
    "im:write",
    "im:history",
    "users:read",
    "channels:join",
    "reactions:write",
    "team:read",
];

// ---------------------------------------------------------------------------
// Slack's credential shape
// ---------------------------------------------------------------------------

pub struct SlackSecrets {
    pub bot_token: String,
    /// App-Level Token for Socket Mode (`xapp-...`). Optional.
    pub app_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Slack's `zad service create slack` args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub token: BotTokenArgs,
    #[command(flatten)]
    pub scopes: ScopesArg,
    /// Slack App ID (found on api.slack.com/apps → Basic Information).
    #[arg(long)]
    pub app_id: Option<String>,
    /// Optional default channel ID or name for verbs that omit `--channel`.
    #[arg(long)]
    pub default_channel: Option<String>,
    /// Your Slack user ID (`U...`). Resolves `@me` in send targets. Find it
    /// in Slack: click your name → View profile → More → Copy member ID.
    /// Leave unset to skip; set later via `zad slack self set <id>`.
    #[arg(long)]
    pub self_user: Option<String>,
    /// App-Level Token (`xapp-...`) for Socket Mode real-time events.
    /// Optional — the service works without it for send/read/channels.
    #[arg(long)]
    pub app_token: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// The trait impl
// ---------------------------------------------------------------------------

pub struct SlackLifecycle;

#[async_trait]
impl LifecycleService for SlackLifecycle {
    const NAME: &'static str = "slack";
    const DISPLAY: &'static str = "Slack";
    type Cfg = SlackServiceCfg;
    type Secrets = SlackSecrets;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_slack();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_slack();
    }

    async fn validate(cfg: &SlackServiceCfg, creds: &SlackSecrets) -> Result<String> {
        let info = SlackHttp::unscoped(&creds.bot_token)
            .auth_test()
            .await
            .map_err(|e| ZadError::Service {
                name: Self::NAME,
                message: format!("token validation failed: {e}"),
            })?;
        // Patch workspace into cfg. Since validate() takes `&SlackServiceCfg`
        // we can't mutate it here; the driver uses the return string for
        // display only. Workspace is set during create via a second save.
        let _ = cfg;
        Ok(format!(
            "@{} in {} ({})",
            info.user, info.team, info.team_id
        ))
    }

    fn store_secrets(creds: &SlackSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let bot_account = secrets::account(Self::NAME, "bot", scope.clone());
        secrets::store(&bot_account, &creds.bot_token)?;
        let mut refs = vec![SecretRef {
            label: "token",
            account: bot_account,
            present: true,
        }];
        if let Some(app_tok) = &creds.app_token {
            let app_account = secrets::account(Self::NAME, "app", scope);
            secrets::store(&app_account, app_tok)?;
            refs.push(SecretRef {
                label: "app-token",
                account: app_account,
                present: true,
            });
        }
        Ok(refs)
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let bot_account = secrets::account(Self::NAME, "bot", scope.clone());
        secrets::delete(&bot_account)?;
        let app_account = secrets::account(Self::NAME, "app", scope);
        // App token may not be present; swallow missing-entry errors.
        let _ = secrets::delete(&app_account);
        Ok(vec![
            SecretRef {
                label: "token",
                account: bot_account,
                present: false,
            },
            SecretRef {
                label: "app-token",
                account: app_account,
                present: false,
            },
        ])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let bot_account = secrets::account(Self::NAME, "bot", scope.clone());
        let bot_present = secrets::load(&bot_account)?.is_some();
        let app_account = secrets::account(Self::NAME, "app", scope);
        let app_present = secrets::load(&app_account)?.is_some();
        Ok(vec![
            SecretRef {
                label: "token",
                account: bot_account,
                present: bot_present,
            },
            SecretRef {
                label: "app-token",
                account: app_account,
                present: app_present,
            },
        ])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<SlackSecrets>> {
        let bot_account = secrets::account(Self::NAME, "bot", scope.clone());
        let Some(bot_token) = secrets::load(&bot_account)? else {
            return Ok(None);
        };
        let app_account = secrets::account(Self::NAME, "app", scope);
        let app_token = secrets::load(&app_account)?;
        Ok(Some(SlackSecrets {
            bot_token,
            app_token,
        }))
    }

    fn cfg_human(cfg: &SlackServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![
            ("app id", cfg.app_id.clone()),
            ("workspace", cfg.workspace.clone()),
        ];
        if let Some(c) = &cfg.default_channel {
            out.push(("channel", c.clone()));
        }
        if let Some(u) = &cfg.self_user_id {
            out.push(("self", u.clone()));
        }
        out
    }

    fn cfg_json(cfg: &SlackServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "app_id": cfg.app_id,
            "workspace": cfg.workspace,
            "default_channel": cfg.default_channel,
            "self_user_id": cfg.self_user_id,
        })
    }

    fn scopes_of(cfg: &SlackServiceCfg) -> &[String] {
        &cfg.scopes
    }

    fn post_create_hint(cfg: &SlackServiceCfg) -> Option<String> {
        Some(format!(
            "https://api.slack.com/apps/{}/install-on-team",
            cfg.app_id
        ))
    }
}

#[async_trait]
impl CliLifecycle for SlackLifecycle {
    type CreateArgs = CreateArgs;

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(SlackServiceCfg, SlackSecrets)> {
        let open_browser = !args.base.no_browser;
        let app_id = resolve_app_id(args.app_id.as_deref(), open_browser, non_interactive)?;
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
        let app_token = resolve_app_token(args.app_token.as_deref(), non_interactive)?;
        let default_channel = args.default_channel.clone();
        let self_user_id = args.self_user.clone();

        // We'll fill workspace from auth.test during validate; store a
        // placeholder here so the Cfg round-trips through serde correctly.
        // The lifecycle driver calls validate() after resolve(), so by the
        // time the file is written `workspace` will be the real value.
        let workspace = String::new();

        Ok((
            SlackServiceCfg {
                app_id,
                workspace,
                scopes,
                default_channel,
                self_user_id,
            },
            SlackSecrets {
                bot_token,
                app_token,
            },
        ))
    }
}

// ---------------------------------------------------------------------------
// prompt helpers
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn resolve_app_id(flag: Option<&str>, open_browser: bool, non_interactive: bool) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired("--app-id"));
    }
    let url = "https://api.slack.com/apps";
    println!();
    println!("Your Slack apps live at:");
    println!("  {url}");
    println!("Create one (or open an existing app) and copy its App ID from Basic Information.");
    if open_browser {
        let _ = open::that(url);
    }
    let v: String = Input::with_theme(&theme())
        .with_prompt("Slack App ID")
        .interact_text()
        .into_zad()?;
    Ok(v.trim().to_string())
}

fn resolve_app_token(flag: Option<&str>, non_interactive: bool) -> Result<Option<String>> {
    if let Some(v) = flag {
        return Ok(Some(v.to_string()));
    }
    if non_interactive {
        return Ok(None);
    }
    println!();
    println!("Optional: an App-Level Token (`xapp-...`) enables Socket Mode so `listen` works.");
    println!("Find it in your Slack app dashboard → Basic Information → App-Level Tokens.");
    println!("Leave blank to skip — the service works without it for send/read/channels.");
    let v = Password::with_theme(&theme())
        .with_prompt("App-Level Token (leave blank to skip)")
        .allow_empty_password(true)
        .interact()
        .into_zad()?;
    let trimmed = v.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}
