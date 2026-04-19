//! 1Password's plug-in to the generic service lifecycle.
//!
//! Authentication is always a 1Password Service Account token —
//! agent-first, admin-rotated. The token is stored in the OS keychain
//! and exported as `OP_SERVICE_ACCOUNT_TOKEN` for every `op` child
//! process. See `docs/services.md#adding-a-new-service` for the full
//! recipe.

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Password, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef, resolve_scopes,
};
use crate::config::{OnePassServiceCfg, ProjectConfig};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::onepass::client::OnePassClient;

/// 1pass understands exactly two zad-level scopes. `read` admits the
/// read-oriented verbs (vaults/items/tags/get/read/inject); `write`
/// admits `create`. Both are off by default — operators enable
/// explicitly at `zad service create 1pass --scopes read,write` time.
const DEFAULT_SCOPES: &[&str] = &["read"];
const ALL_SCOPES: &[&str] = &["read", "write"];

/// Link to the 1Password Service Accounts dashboard. Printed during
/// interactive create so the operator can mint or rotate a token
/// without leaving the terminal.
const OP_SERVICE_ACCOUNTS_URL: &str =
    "https://my.1password.com/developer-tools/infrastructure-secrets/serviceaccount";

// ---------------------------------------------------------------------------
// credential shape
// ---------------------------------------------------------------------------

pub struct OnePassSecrets {
    pub service_account_token: String,
}

// ---------------------------------------------------------------------------
// `zad service create 1pass` args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub scopes: ScopesArg,

    /// 1Password sign-in address (e.g. `my.1password.com`,
    /// `team.1password.eu`).
    #[arg(long)]
    pub account: Option<String>,

    /// Service Account token. If omitted, zad reads `--token-env` or
    /// prompts interactively (password-style echo).
    #[arg(long, conflicts_with = "token_env")]
    pub token: Option<String>,

    /// Read the service-account token from this environment variable
    /// instead of a flag or prompt.
    #[arg(long, conflicts_with = "token")]
    pub token_env: Option<String>,

    /// Optional default vault for commands that omit `--vault`.
    #[arg(long)]
    pub default_vault: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// the trait impl
// ---------------------------------------------------------------------------

pub struct OnePassLifecycle;

#[async_trait]
impl LifecycleService for OnePassLifecycle {
    const NAME: &'static str = "1pass";
    const DISPLAY: &'static str = "1Password";
    type Cfg = OnePassServiceCfg;
    type Secrets = OnePassSecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_one_pass();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_one_pass();
    }

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(OnePassServiceCfg, OnePassSecrets)> {
        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(),
            DEFAULT_SCOPES,
            ALL_SCOPES,
            non_interactive,
        )?;

        let account = resolve_account(args.account.as_deref(), non_interactive)?;
        let token = resolve_token(
            args.token.as_deref(),
            args.token_env.as_deref(),
            non_interactive,
        )?;

        Ok((
            OnePassServiceCfg {
                account,
                scopes,
                default_vault: args.default_vault.clone(),
            },
            OnePassSecrets {
                service_account_token: token,
            },
        ))
    }

    async fn validate(cfg: &OnePassServiceCfg, creds: &OnePassSecrets) -> Result<String> {
        let client = OnePassClient::new(creds.service_account_token.clone(), cfg.account.clone());
        let me = client.whoami().await?;
        // Prefer the sign-in URL (which names the account domain) when
        // the service-account type is set — agents like to see which
        // account they're tied to. Fall back to UUID when the CLI
        // doesn't populate URL.
        let id = if !me.url.is_empty() {
            me.url
        } else if !me.user_uuid.is_empty() {
            me.user_uuid
        } else {
            "service-account".into()
        };
        Ok(id)
    }

    fn store_secrets(creds: &OnePassSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "service-account", scope);
        secrets::store(&account, &creds.service_account_token)?;
        Ok(vec![SecretRef {
            label: "token",
            account,
            present: true,
        }])
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "service-account", scope);
        secrets::delete(&account)?;
        Ok(vec![SecretRef {
            label: "token",
            account,
            present: false,
        }])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "service-account", scope);
        let present = secrets::load(&account)?.is_some();
        Ok(vec![SecretRef {
            label: "token",
            account,
            present,
        }])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<OnePassSecrets>> {
        let account = secrets::account(Self::NAME, "service-account", scope);
        let Some(token) = secrets::load(&account)? else {
            return Ok(None);
        };
        Ok(Some(OnePassSecrets {
            service_account_token: token,
        }))
    }

    fn cfg_human(cfg: &OnePassServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![("account", cfg.account.clone())];
        if let Some(v) = &cfg.default_vault {
            out.push(("vault", v.clone()));
        }
        out
    }

    fn cfg_json(cfg: &OnePassServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "account": cfg.account,
            "default_vault": cfg.default_vault,
        })
    }

    fn scopes_of(cfg: &OnePassServiceCfg) -> &[String] {
        &cfg.scopes
    }

    fn post_create_hint(_cfg: &OnePassServiceCfg) -> Option<String> {
        Some(OP_SERVICE_ACCOUNTS_URL.to_string())
    }
}

// ---------------------------------------------------------------------------
// prompt helpers
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn resolve_account(flag: Option<&str>, non_interactive: bool) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.trim().to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired("--account"));
    }
    println!();
    println!("1Password sign-in address (e.g. `my.1password.com`, `team.1password.eu`)");
    let v: String = dialoguer::Input::with_theme(&theme())
        .with_prompt("Sign-in address")
        .interact_text()?;
    Ok(v.trim().to_string())
}

fn resolve_token(
    flag: Option<&str>,
    env_flag: Option<&str>,
    non_interactive: bool,
) -> Result<String> {
    if let Some(env) = env_flag {
        return std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()));
    }
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired("--token or --token-env"));
    }
    println!();
    println!("Create a Service Account token:");
    println!("  {OP_SERVICE_ACCOUNTS_URL}");
    println!("Grant it the vaults / permissions this agent needs, then paste the");
    println!("`ops_…` token below.");
    let v = Password::with_theme(&theme())
        .with_prompt("Service Account token")
        .interact()?;
    Ok(v)
}
