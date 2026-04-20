//! GitHub's plug-in to the generic service lifecycle.
//!
//! Credential shape is a single Personal Access Token stored in the OS
//! keychain. Runtime verbs shell out to `gh` with `GH_TOKEN` set to
//! that PAT, so auth is consistent regardless of whether the user has
//! also run `gh auth login`. Non-secret metadata (`default_repo`,
//! `scopes`, `self_login`) lives in the flat TOML config next to the
//! service.
//!
//! See `docs/services.md#adding-a-new-service` for the full recipe.

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Input, Password, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef, resolve_scopes,
};
use crate::config::{GithubServiceCfg, ProjectConfig};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::github::client::GhCli;

const DEFAULT_SCOPES: &[&str] = &[
    "repo.read",
    "issues.read",
    "pulls.read",
    "checks.read",
    "search",
];
const ALL_SCOPES: &[&str] = &[
    "repo.read",
    "issues.read",
    "issues.write",
    "pulls.read",
    "pulls.write",
    "checks.read",
    "search",
];

const PAT_HELP_URL: &str = "https://github.com/settings/tokens";

// ---------------------------------------------------------------------------
// Credential shape
// ---------------------------------------------------------------------------

/// GitHub stores a single long-lived Personal Access Token. It's
/// wrapped in a named struct rather than `String` for parity with
/// services that have richer credential shapes.
pub struct GithubSecrets {
    pub pat: String,
}

// ---------------------------------------------------------------------------
// `zad service create github` args
// ---------------------------------------------------------------------------

/// GitHub-specific credential flags. We don't reuse `BotTokenArgs`
/// because the PAT nomenclature is distinct and we want the help text
/// to point at the PAT settings page.
#[derive(Debug, Args)]
pub struct PatArgs {
    /// Personal Access Token. Stored in the OS keychain, never in the
    /// TOML. Create one at https://github.com/settings/tokens.
    #[arg(long, conflicts_with = "pat_env")]
    pub pat: Option<String>,

    /// Read the Personal Access Token from the named environment
    /// variable. Useful in CI where the PAT is already an env secret.
    #[arg(long, conflicts_with = "pat")]
    pub pat_env: Option<String>,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub pat: PatArgs,
    #[command(flatten)]
    pub scopes: ScopesArg,

    /// Optional default repository for verbs that omit `--repo`.
    /// Format: `owner/name` (e.g. `octocat/hello-world`).
    #[arg(long)]
    pub default_repo: Option<String>,

    /// Optional default owner (user or org) for verbs like `code
    /// search` that scope to an org. Used when `--org` is omitted.
    #[arg(long)]
    pub default_owner: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// The trait impl
// ---------------------------------------------------------------------------

pub struct GithubLifecycle;

#[async_trait]
impl LifecycleService for GithubLifecycle {
    const NAME: &'static str = "github";
    const DISPLAY: &'static str = "GitHub";
    type Cfg = GithubServiceCfg;
    type Secrets = GithubSecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_github();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_github();
    }

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(GithubServiceCfg, GithubSecrets)> {
        let open_browser = !args.base.no_browser;
        let default_repo = resolve_default_repo(args.default_repo.as_deref(), non_interactive)?;
        let default_owner = resolve_default_owner(args.default_owner.as_deref(), non_interactive)?;
        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(),
            DEFAULT_SCOPES,
            ALL_SCOPES,
            non_interactive,
        )?;
        let pat = resolve_pat(
            args.pat.pat.as_deref(),
            args.pat.pat_env.as_deref(),
            open_browser,
            non_interactive,
        )?;
        Ok((
            GithubServiceCfg {
                scopes,
                default_repo,
                default_owner,
                self_login: None,
            },
            GithubSecrets { pat },
        ))
    }

    async fn validate(_cfg: &GithubServiceCfg, creds: &GithubSecrets) -> Result<String> {
        GhCli::validate_token(&creds.pat).await
    }

    fn store_secrets(creds: &GithubSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "pat", scope);
        secrets::store(&account, &creds.pat)?;
        Ok(vec![SecretRef {
            label: "pat",
            account,
            present: true,
        }])
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "pat", scope);
        secrets::delete(&account)?;
        Ok(vec![SecretRef {
            label: "pat",
            account,
            present: false,
        }])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "pat", scope);
        let present = secrets::load(&account)?.is_some();
        Ok(vec![SecretRef {
            label: "pat",
            account,
            present,
        }])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<GithubSecrets>> {
        let account = secrets::account(Self::NAME, "pat", scope);
        Ok(secrets::load(&account)?.map(|pat| GithubSecrets { pat }))
    }

    fn cfg_human(cfg: &GithubServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(r) = &cfg.default_repo {
            out.push(("repo", r.clone()));
        }
        if let Some(o) = &cfg.default_owner {
            out.push(("owner", o.clone()));
        }
        if let Some(u) = &cfg.self_login {
            out.push(("login", u.clone()));
        }
        out
    }

    fn cfg_json(cfg: &GithubServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "default_repo": cfg.default_repo,
            "default_owner": cfg.default_owner,
            "self_login": cfg.self_login,
        })
    }

    fn scopes_of(cfg: &GithubServiceCfg) -> &[String] {
        &cfg.scopes
    }

    fn post_create_hint(_cfg: &GithubServiceCfg) -> Option<String> {
        // Point users at the PAT settings page so they can narrow the
        // token's scopes if they overshot.
        Some(PAT_HELP_URL.to_string())
    }
}

// ---------------------------------------------------------------------------
// Prompt helpers
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn resolve_default_repo(flag: Option<&str>, non_interactive: bool) -> Result<Option<String>> {
    if let Some(v) = flag {
        validate_repo(v)?;
        return Ok(Some(v.to_string()));
    }
    if non_interactive {
        return Ok(None);
    }
    println!();
    println!("Default repo accepts `owner/name` (e.g. `octocat/hello-world`).");
    println!("Leave blank to skip — every verb can still take `--repo` explicitly.");
    let v: String = Input::with_theme(&theme())
        .with_prompt("Default repo (leave blank for none)")
        .allow_empty(true)
        .interact_text()?;
    if v.trim().is_empty() {
        Ok(None)
    } else {
        validate_repo(&v).map(|_| Some(v))
    }
}

fn resolve_default_owner(flag: Option<&str>, non_interactive: bool) -> Result<Option<String>> {
    if let Some(v) = flag {
        validate_owner(v)?;
        return Ok(Some(v.to_string()));
    }
    if non_interactive {
        return Ok(None);
    }
    println!();
    println!("Default owner (user or org) is used by `code search --org`.");
    println!("Leave blank to skip.");
    let v: String = Input::with_theme(&theme())
        .with_prompt("Default owner (leave blank for none)")
        .allow_empty(true)
        .interact_text()?;
    if v.trim().is_empty() {
        Ok(None)
    } else {
        validate_owner(&v).map(|_| Some(v))
    }
}

fn resolve_pat(
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
        return Err(ZadError::MissingRequired("--pat or --pat-env"));
    }

    println!();
    println!("Create a GitHub Personal Access Token at:");
    println!("  {PAT_HELP_URL}");
    println!("Scope it to the repos and permissions this agent should have.");
    if open_browser {
        let _ = open::that(PAT_HELP_URL);
    }

    let v = Password::with_theme(&theme())
        .with_prompt("GitHub personal access token")
        .interact()?;
    Ok(v)
}

fn validate_repo(v: &str) -> Result<()> {
    let trimmed = v.trim();
    if trimmed.is_empty() {
        return Err(ZadError::Invalid("repo must not be empty".into()));
    }
    // Expect exactly one `/`, non-empty halves, no spaces.
    let mut parts = trimmed.split('/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(ZadError::Invalid(format!(
            "repo `{v}` must be in `owner/name` form"
        )));
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err(ZadError::Invalid(format!("repo `{v}` contains whitespace")));
    }
    Ok(())
}

fn validate_owner(v: &str) -> Result<()> {
    let trimmed = v.trim();
    if trimmed.is_empty() {
        return Err(ZadError::Invalid("owner must not be empty".into()));
    }
    if trimmed.contains('/') || trimmed.chars().any(char::is_whitespace) {
        return Err(ZadError::Invalid(format!(
            "owner `{v}` must be a bare user or org handle"
        )));
    }
    Ok(())
}
