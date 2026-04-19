//! Generic lifecycle driver for every service.
//!
//! ## Why this module exists
//!
//! `zad service {create,enable,disable,show,delete} <name>` does the
//! same five things for every service — check scope/paths, prompt for
//! missing flags, validate credentials against the provider, store
//! secrets in the keychain, and emit human/JSON output. The ~700
//! lines of that plumbing used to live inside `service_discord.rs`,
//! which meant adding Telegram (or Slack, or Reddit, or GitHub…)
//! would have been a 700-line copy-paste.
//!
//! This module hosts all of that plumbing exactly once. Each service
//! plugs in by implementing [`LifecycleService`] — roughly 80 lines
//! including its clap args, secret shape, token validator, and
//! rendering hooks. See `docs/services.md#adding-a-new-service`
//! for the full recipe.
//!
//! ## Design
//!
//! The trait separates what varies per service from what doesn't:
//!
//! | Varies per service              | Shared (this module) |
//! |---------------------------------|----------------------|
//! | non-secret config schema        | path resolution      |
//! | secret shape (1 token? 3 OAuth fields? a PEM?) | keychain I/O driven by the service's reports |
//! | create-time prompts             | base flag plumbing (`--local`, `--force`, `--json`, …) |
//! | token validator (hits provider) | JSON envelope + human banners |
//! | non-secret display fields       | scope-block rendering with dynamic column widths |
//!
//! Credential shape is intentionally *not* assumed to be one bot
//! token — services declare a [`LifecycleService::Secrets`] type and
//! report every keychain entry they wrote via [`SecretRef`], so a
//! Reddit service can store `client_secret` + `refresh_token` under
//! two accounts and Discord can store its single bot token under one,
//! without this module knowing the difference.

use async_trait::async_trait;
use clap::Args;
use serde::{Serialize, de::DeserializeOwned};

use crate::config::{self, ProjectConfig};
use crate::error::{Result, ZadError};
use crate::secrets::Scope;

// ---------------------------------------------------------------------------
// The trait every service implements
// ---------------------------------------------------------------------------

/// Everything the generic lifecycle driver needs to know about one
/// service. Implementors are usually unit structs (e.g.
/// `pub struct DiscordLifecycle;`) that just route to the service's
/// real types; the trait is stateless on purpose so it can be used
/// purely as a type-level handle in the driver functions below.
#[async_trait]
pub trait LifecycleService: Send + Sync + 'static {
    /// Lowercase identifier used in paths, commands, and keychain
    /// account names (`"discord"`, `"telegram"`, `"reddit"`, …).
    /// Must match the entry in [`crate::service::registry::SERVICES`]
    /// and the directory name under `src/service/`.
    const NAME: &'static str;

    /// Capitalized name for human-facing output (`"Discord"`).
    const DISPLAY: &'static str;

    /// Non-secret fields persisted to
    /// `~/.zad/services/<NAME>/config.toml` (global) or
    /// `~/.zad/projects/<slug>/services/<NAME>/config.toml` (local).
    /// Anything that isn't a credential belongs here: app/bot IDs,
    /// declared scopes, default targets.
    type Cfg: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;

    /// Secret material held in the OS keychain. Shape is up to the
    /// service: one token, three OAuth fields, a PEM blob — whatever
    /// the provider needs. The driver never inspects this type; it
    /// flows from [`Self::resolve`] into [`Self::validate`] and
    /// [`Self::store_secrets`] and is then dropped.
    type Secrets: Send + Sync;

    /// Per-service `zad service create <NAME>` flag struct. Must
    /// embed [`CreateArgsBase`] via `#[command(flatten)]` and expose
    /// it through [`CreateArgsLike::base`].
    type CreateArgs: Args + CreateArgsLike + Send + Sync;

    /// Mark the current project as using this service by writing a
    /// `[service.<NAME>]` entry into its project config.
    fn enable_in_project(cfg: &mut ProjectConfig);

    /// Remove this service's `[service.<NAME>]` entry from the
    /// current project config. Idempotent at the caller level — the
    /// driver checks `has_service` first.
    fn disable_in_project(cfg: &mut ProjectConfig);

    /// Build `(Cfg, Secrets)` from CLI args. Interactive mode
    /// prompts the user for any `Option<_>` fields that arrived
    /// empty; non-interactive mode returns
    /// [`ZadError::MissingRequired`] for anything still missing.
    async fn resolve(
        args: &Self::CreateArgs,
        non_interactive: bool,
    ) -> Result<(Self::Cfg, Self::Secrets)>;

    /// Confirm the credentials work by talking to the provider.
    /// Returns a short human identifier (bot username, GitHub App
    /// slug, Reddit username) on success. Called only when
    /// `--no-validate` is absent. Implementations should emit
    /// `ZadError::Service { name: NAME, … }` on provider errors so
    /// the message stream stays uniform across services.
    async fn validate(cfg: &Self::Cfg, secrets: &Self::Secrets) -> Result<String>;

    /// Write every piece of secret material to the OS keychain under
    /// `scope`. Returns the `SecretRef` for each account that was
    /// written so the driver can report them in the create output.
    /// Use [`crate::secrets::account`] to name each entry — do not
    /// invent account-string formats.
    fn store_secrets(secrets: &Self::Secrets, scope: Scope<'_>) -> Result<Vec<SecretRef>>;

    /// Remove every keychain entry for this service at `scope`.
    /// Idempotent — `keyring::Error::NoEntry` is a success. Returns
    /// the `SecretRef`s that were targeted (with `present = false`
    /// after deletion) so `delete`'s output lists them.
    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>>;

    /// Report keychain presence for `zad service show`. Returns one
    /// entry per account this service expects, with `present`
    /// reflecting the current state. Must list the same accounts
    /// that `store_secrets` writes.
    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>>;

    /// Load the full secret material from the keychain at `scope`.
    /// Returns `Ok(None)` if any required account is missing — lets
    /// `zad service status` report "credentials_present: false"
    /// without escalating to an error. `Ok(Some(_))` means every
    /// account this service stores via [`Self::store_secrets`] was
    /// present and read successfully.
    fn load_secrets(scope: Scope<'_>) -> Result<Option<Self::Secrets>>;

    /// Human-readable non-secret fields rendered in `create` and
    /// `show` output, as `(label, value)` pairs. Keep labels short
    /// (≤ 6 chars recommended) for consistent column alignment with
    /// the generic `scopes` / `token` lines; longer labels work but
    /// widen the column for every service.
    fn cfg_human(cfg: &Self::Cfg) -> Vec<(&'static str, String)>;

    /// Non-secret fields rendered for `--json`. Must return a
    /// `serde_json::Value::Object`; the driver splices its keys
    /// into the top-level envelope via `#[serde(flatten)]`-style
    /// merging, so keys here become siblings of `scope`,
    /// `config_path`, etc.
    fn cfg_json(cfg: &Self::Cfg) -> serde_json::Value;

    /// Declared scopes — stored verbatim in the TOML config's
    /// `scopes` array. Convenience accessor so the driver can
    /// surface them in `show` without reading `Cfg`.
    fn scopes_of(cfg: &Self::Cfg) -> &[String];

    /// Optional URL to surface immediately after `create` succeeds.
    /// Typical use is a deep link the user needs to visit next —
    /// e.g. Discord's OAuth bot-install page so the bot can be added
    /// to a guild. Returned URL is printed under the human banner
    /// (and included in the JSON envelope as `hint`); when the user
    /// did not pass `--no-browser`, the driver also tries to open it
    /// in the system browser. Default: no hint.
    fn post_create_hint(_cfg: &Self::Cfg) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// Shared arg types
// ---------------------------------------------------------------------------

/// Flags every `zad service create <name>` accepts. Services flatten
/// this into their own `CreateArgs` via `#[command(flatten)]`; the
/// driver reaches them through [`CreateArgsLike::base`].
#[derive(Debug, Args)]
pub struct CreateArgsBase {
    /// Write credentials to this project's private service directory
    /// (`~/.zad/projects/<slug>/services/<name>/config.toml`) instead
    /// of the shared global location. Local credentials take
    /// precedence over global ones for this project.
    #[arg(long)]
    pub local: bool,

    /// Overwrite any existing configuration at the chosen scope.
    #[arg(long)]
    pub force: bool,

    /// Fail instead of prompting for any missing value.
    #[arg(long)]
    pub non_interactive: bool,

    /// Skip the provider-side token validation step.
    #[arg(long)]
    pub no_validate: bool,

    /// Don't open URLs in the system browser. By default, services
    /// that have somewhere useful to send the user (e.g. Discord's
    /// developer-portal token page) will try to open it
    /// automatically; this disables that. The URL is still printed
    /// either way.
    #[arg(long)]
    pub no_browser: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

/// Lets the driver read [`CreateArgsBase`] out of a service-specific
/// wrapper. Implementations are one line: `fn base(&self) ->
/// &CreateArgsBase { &self.base }`.
pub trait CreateArgsLike {
    fn base(&self) -> &CreateArgsBase;
}

/// Drop-in clap flags for any service whose credential is a single
/// long-lived bot token (Discord, Telegram, Slack bot, …). Services
/// with richer credential shapes declare their own args instead.
#[derive(Debug, Args)]
pub struct BotTokenArgs {
    /// Bot token. Stored in the OS keychain, never in the TOML.
    #[arg(long, conflicts_with = "bot_token_env")]
    pub bot_token: Option<String>,

    /// Read the bot token from the named environment variable.
    #[arg(long, conflicts_with = "bot_token")]
    pub bot_token_env: Option<String>,
}

/// Drop-in clap flag for the common "comma-separated scopes" pattern.
/// Services that don't use scopes (or use a non-comma shape) skip it.
#[derive(Debug, Args)]
pub struct ScopesArg {
    /// Capabilities to enable (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub scopes: Option<Vec<String>>,
}

/// Shared shape for `zad service enable <name>`. No service needs
/// service-specific flags here today, so all services reuse it.
#[derive(Debug, Args)]
pub struct EnableArgs {
    /// Overwrite an existing `[service.<name>]` entry in the project
    /// config.
    #[arg(long)]
    pub force: bool,

    /// Fail instead of prompting (reserved — `enable` has no prompts).
    #[arg(long)]
    pub non_interactive: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DisableArgs {
    /// Succeed silently even if the service is not currently enabled
    /// in this project.
    #[arg(long)]
    pub force: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    /// Recommended when an agent is consuming the output — the JSON
    /// envelope is stable and includes the authenticated identity on
    /// success and the provider error string on failure.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// Delete the project-scoped credentials instead of the global
    /// ones.
    #[arg(long)]
    pub local: bool,

    /// Succeed silently even if no config file exists at the chosen
    /// scope.
    #[arg(long)]
    pub force: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

/// One OS-keychain entry belonging to a service, as reported by
/// [`LifecycleService::store_secrets`] / `delete_secrets` /
/// `inspect_secrets`. Surfaced both in `--json` output and the
/// human-readable scope blocks.
#[derive(Debug, Clone, Serialize)]
pub struct SecretRef {
    /// Human label, e.g. `"token"`, `"bot token"`, `"client secret"`.
    /// Rendered verbatim in `show`'s scope block, so keep it short
    /// and lowercase.
    pub label: &'static str,
    /// Full keychain account string — what was passed to
    /// [`crate::secrets::store`]. User-visible identifier.
    pub account: String,
    /// Whether the OS keychain currently has this entry.
    pub present: bool,
}

// ---------------------------------------------------------------------------
// JSON envelope structs (shared across every service)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CreateOutput {
    command: String,
    scope: &'static str,
    config_path: String,
    #[serde(flatten)]
    service: serde_json::Value,
    scopes: Vec<String>,
    secrets: Vec<SecretRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authenticated_as: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

#[derive(Debug, Serialize)]
struct EnableOutput {
    command: String,
    project_config: String,
    credentials_path: String,
    credentials_scope: &'static str,
}

#[derive(Debug, Serialize)]
struct DisableOutput {
    command: String,
    project_config: String,
    was_enabled: bool,
}

#[derive(Debug, Serialize)]
struct ShowOutput {
    command: String,
    service: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective: Option<&'static str>,
    global: ScopeBlock,
    local: ScopeBlock,
    project: ProjectBlock,
}

#[derive(Debug, Serialize)]
struct ScopeBlock {
    path: String,
    configured: bool,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    service: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    secrets: Vec<SecretRef>,
}

/// Per-service envelope for `zad service status --service <svc>`.
/// Also reused verbatim inside the aggregate `zad service status`
/// output so every service row carries the same shape whether the
/// caller asked about one service or all of them.
#[derive(Debug, Serialize)]
pub struct ServiceStatusOutput {
    /// `"service.status.<name>"` for the per-service command; left
    /// unset when the value is embedded in the aggregate output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub service: &'static str,
    /// Which scope would be used at runtime (local wins over global).
    /// `None` when neither scope is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<&'static str>,
    /// Overall health: `true` iff the effective scope pinged OK.
    /// `false` when no scope is configured, credentials are missing,
    /// or the provider rejected the token.
    pub ok: bool,
    pub global: StatusBlock,
    pub local: StatusBlock,
    pub project: ProjectBlock,
}

/// One scope's view for status. `check` is populated only for the
/// effective scope; non-effective scopes report presence but aren't
/// pinged (avoids doubling the provider's rate-limit burden when both
/// scopes happen to be configured).
#[derive(Debug, Serialize)]
pub struct StatusBlock {
    pub path: String,
    pub configured: bool,
    pub credentials_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<StatusCheck>,
}

#[derive(Debug, Serialize)]
pub struct StatusCheck {
    pub ok: bool,
    /// Identity the provider reported back (bot username, etc.) on a
    /// successful ping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticated_as: Option<String>,
    /// Provider-side or local error message on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectBlock {
    pub config: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
struct DeleteOutput {
    command: String,
    scope: &'static str,
    config_path: String,
    config_removed: bool,
    secrets: Vec<SecretRef>,
    project_still_references: bool,
}

// ---------------------------------------------------------------------------
// Generic driver: create
// ---------------------------------------------------------------------------

pub async fn run_create<T: LifecycleService>(args: T::CreateArgs) -> Result<()> {
    let base = args.base();
    let (path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) = if base.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_service_config_path_for(&slug, T::NAME)?;
        (
            p,
            "local (project-scoped)".to_string(),
            "local",
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_service_config_path(T::NAME)?,
            "global".to_string(),
            "global",
            Scope::Global,
        )
    };

    let existing: Option<T::Cfg> = config::load_flat(&path)?;
    if existing.is_some() && !base.force {
        return Err(ZadError::ServiceAlreadyConfigured {
            name: format!("{} ({scope_label})", T::NAME),
        });
    }

    let (cfg, creds) = T::resolve(&args, base.non_interactive).await?;

    let mut authenticated_as: Option<String> = None;
    if !base.no_validate {
        tracing::info!(service = T::NAME, "validating credentials");
        match T::validate(&cfg, &creds).await {
            Ok(name) => {
                if !base.json {
                    println!("  ✓ authenticated as `{name}`");
                }
                authenticated_as = Some(name);
            }
            Err(e) => return Err(e),
        }
    }

    let secrets_refs = T::store_secrets(&creds, keychain_scope)?;
    config::save_flat(&path, &cfg)?;

    let hint = T::post_create_hint(&cfg);

    if base.json {
        let out = CreateOutput {
            command: format!("service.create.{}", T::NAME),
            scope: scope_machine,
            config_path: path.display().to_string(),
            service: T::cfg_json(&cfg),
            scopes: T::scopes_of(&cfg).to_vec(),
            secrets: secrets_refs,
            authenticated_as,
            hint: hint.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        let lines = T::cfg_human(&cfg);
        let width = label_width(&lines, T::scopes_of(&cfg), &secrets_refs);
        println!();
        println!("{} credentials created ({scope_label}).", T::DISPLAY);
        let config_label = "config";
        let config_value = path.display().to_string();
        println!("  {config_label:width$} : {config_value}");
        for (label, value) in &lines {
            println!("  {label:width$} : {value}");
        }
        let scopes = T::scopes_of(&cfg);
        let scopes_label = "scopes";
        let scopes_value = if scopes.is_empty() {
            "(none)".to_string()
        } else {
            scopes.join(", ")
        };
        println!("  {scopes_label:width$} : {scopes_value}");
        for s in &secrets_refs {
            let label = s.label;
            let account = &s.account;
            println!("  {label:width$} : OS keychain (service=\"zad\", account=\"{account}\")");
        }
        println!();
        println!(
            "Next: run `zad service enable {}` in each project that should use {}.",
            T::NAME,
            T::DISPLAY
        );
        if let Some(url) = hint.as_deref() {
            println!();
            println!("  open: {url}");
        }
    }

    if let Some(url) = hint.as_deref()
        && !base.no_browser
        && !base.non_interactive
    {
        let _ = open::that(url);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Generic driver: enable
// ---------------------------------------------------------------------------

pub fn run_enable<T: LifecycleService>(args: EnableArgs) -> Result<()> {
    let slug = config::path::project_slug()?;
    let local_creds = config::path::project_service_config_path_for(&slug, T::NAME)?;
    let global_creds = config::path::global_service_config_path(T::NAME)?;

    let (creds_path, scope_label) = if local_creds.exists() {
        (local_creds.clone(), "local")
    } else if global_creds.exists() {
        (global_creds.clone(), "global")
    } else {
        return Err(ZadError::Invalid(format!(
            "no {} credentials found. Run `zad service create {}` \
             (or with `--local`) to register credentials first.\n\
             looked in:\n  {}\n  {}",
            T::DISPLAY,
            T::NAME,
            local_creds.display(),
            global_creds.display()
        )));
    };

    let project_path = config::path::project_config_path()?;
    let mut project_cfg = config::load_from(&project_path)?;
    if project_cfg.has_service(T::NAME) && !args.force {
        return Err(ZadError::ServiceAlreadyConfigured {
            name: T::NAME.to_string(),
        });
    }

    T::enable_in_project(&mut project_cfg);
    config::save_to(&project_path, &project_cfg)?;

    if args.json {
        let out = EnableOutput {
            command: format!("service.enable.{}", T::NAME),
            project_config: project_path.display().to_string(),
            credentials_path: creds_path.display().to_string(),
            credentials_scope: scope_label,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{} service enabled for this project.", T::DISPLAY);
        println!("  project config : {}", project_path.display());
        println!(
            "  credentials    : {} ({scope_label})",
            creds_path.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Generic driver: disable
// ---------------------------------------------------------------------------

pub fn run_disable<T: LifecycleService>(args: DisableArgs) -> Result<()> {
    let project_path = config::path::project_config_path()?;
    let mut project_cfg = config::load_from(&project_path)?;
    let was_enabled = project_cfg.has_service(T::NAME);

    if !was_enabled && !args.force {
        return Err(ZadError::Invalid(format!(
            "{} service is not enabled for this project ({}). \
             Pass --force to ignore.",
            T::NAME,
            project_path.display()
        )));
    }

    if was_enabled {
        T::disable_in_project(&mut project_cfg);
        config::save_to(&project_path, &project_cfg)?;
    }

    if args.json {
        let out = DisableOutput {
            command: format!("service.disable.{}", T::NAME),
            project_config: project_path.display().to_string(),
            was_enabled,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if was_enabled {
        println!("{} service disabled for this project.", T::DISPLAY);
        println!("  project config : {}", project_path.display());
    } else {
        println!(
            "{} service was not enabled for this project (nothing to do).",
            T::DISPLAY
        );
        println!("  project config : {}", project_path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Generic driver: show
// ---------------------------------------------------------------------------

pub fn run_show<T: LifecycleService>(args: ShowArgs) -> Result<()> {
    let slug = config::path::project_slug()?;
    let global_path = config::path::global_service_config_path(T::NAME)?;
    let local_path = config::path::project_service_config_path_for(&slug, T::NAME)?;

    let global_cfg: Option<T::Cfg> = config::load_flat(&global_path)?;
    let local_cfg: Option<T::Cfg> = config::load_flat(&local_path)?;

    let effective = if local_cfg.is_some() {
        Some("local")
    } else if global_cfg.is_some() {
        Some("global")
    } else {
        None
    };

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_enabled = project_cfg.has_service(T::NAME);

    if args.json {
        let out = ShowOutput {
            command: format!("service.show.{}", T::NAME),
            service: T::NAME,
            effective,
            global: scope_block::<T>(&global_path, global_cfg.as_ref(), Scope::Global)?,
            local: scope_block::<T>(&local_path, local_cfg.as_ref(), Scope::Project(&slug))?,
            project: ProjectBlock {
                config: project_path.display().to_string(),
                enabled: project_enabled,
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    println!("Service: {}", T::NAME);
    println!();
    println!("## Credentials");
    if let Some(label) = effective {
        println!("  effective : {label}");
    } else {
        println!(
            "  effective : (none — run `zad service create {}`)",
            T::NAME
        );
    }

    print_scope_block::<T>("global", &global_path, global_cfg.as_ref(), Scope::Global)?;
    print_scope_block::<T>(
        "local",
        &local_path,
        local_cfg.as_ref(),
        Scope::Project(&slug),
    )?;

    println!();
    println!("## Project");
    if project_enabled {
        println!("  enabled : yes");
    } else {
        println!("  enabled : no");
    }
    println!("  config  : {}", project_path.display());

    Ok(())
}

fn scope_block<T: LifecycleService>(
    path: &std::path::Path,
    cfg: Option<&T::Cfg>,
    scope: Scope<'_>,
) -> Result<ScopeBlock> {
    let mut block = ScopeBlock {
        path: path.display().to_string(),
        configured: cfg.is_some(),
        service: None,
        scopes: None,
        secrets: Vec::new(),
    };
    if let Some(c) = cfg {
        block.service = Some(T::cfg_json(c));
        block.scopes = Some(T::scopes_of(c).to_vec());
        block.secrets = T::inspect_secrets(scope)?;
    }
    Ok(block)
}

fn print_scope_block<T: LifecycleService>(
    label: &str,
    path: &std::path::Path,
    cfg: Option<&T::Cfg>,
    scope: Scope<'_>,
) -> Result<()> {
    println!();
    println!("  [{label}] {}", path.display());
    match cfg {
        None => println!("    status : not configured"),
        Some(c) => {
            let lines = T::cfg_human(c);
            let scopes = T::scopes_of(c);
            let secrets_refs = T::inspect_secrets(scope)?;
            let width = label_width(&lines, scopes, &secrets_refs);
            for (label, value) in &lines {
                println!("    {label:width$} : {value}");
            }
            let scopes_label = "scopes";
            let scopes_value = if scopes.is_empty() {
                "(none)".to_string()
            } else {
                scopes.join(", ")
            };
            println!("    {scopes_label:width$} : {scopes_value}");
            for s in &secrets_refs {
                let label = s.label;
                let state = if s.present { "stored" } else { "missing" };
                let account = &s.account;
                println!("    {label:width$} : {state} (service=\"zad\", account=\"{account}\")");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Generic driver: status
// ---------------------------------------------------------------------------

/// Run `zad service status --service <svc>` for service `T`. Emits
/// JSON or human output, then exits the process with code 1 if the
/// effective scope failed its live ping (or no scope is configured).
/// Agents can branch on `$?` without parsing the output.
pub async fn run_status<T: LifecycleService>(args: StatusArgs) -> Result<()> {
    let mut out = status_for::<T>().await?;
    out.command = Some(format!("service.status.{}", T::NAME));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        print_status_human(&out);
    }
    if !out.ok {
        std::process::exit(1);
    }
    Ok(())
}

/// Collect the status envelope for service `T` without emitting
/// anything. Shared between the single-service form
/// (`zad service status --service <svc>`) and the aggregate
/// (`zad service status`), which calls this once per service in
/// parallel.
pub async fn status_for<T: LifecycleService>() -> Result<ServiceStatusOutput> {
    let slug = config::path::project_slug()?;
    let global_path = config::path::global_service_config_path(T::NAME)?;
    let local_path = config::path::project_service_config_path_for(&slug, T::NAME)?;

    let global_cfg: Option<T::Cfg> = config::load_flat(&global_path)?;
    let local_cfg: Option<T::Cfg> = config::load_flat(&local_path)?;

    let effective = if local_cfg.is_some() {
        Some("local")
    } else if global_cfg.is_some() {
        Some("global")
    } else {
        None
    };

    // Only ping the effective scope. A non-effective scope that's
    // also configured reports credential presence but is not pinged:
    // it wouldn't be used at runtime anyway, and pinging it would
    // double the provider rate-limit cost for every `zad service status`.
    let global_block = build_status_block::<T>(
        &global_path,
        global_cfg.as_ref(),
        Scope::Global,
        effective == Some("global"),
    )
    .await;
    let local_block = build_status_block::<T>(
        &local_path,
        local_cfg.as_ref(),
        Scope::Project(&slug),
        effective == Some("local"),
    )
    .await;

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_enabled = project_cfg.has_service(T::NAME);

    let ok = match effective {
        Some("global") => global_block.check.as_ref().map(|c| c.ok).unwrap_or(false),
        Some("local") => local_block.check.as_ref().map(|c| c.ok).unwrap_or(false),
        _ => false,
    };

    Ok(ServiceStatusOutput {
        command: None,
        service: T::NAME,
        effective,
        ok,
        global: global_block,
        local: local_block,
        project: ProjectBlock {
            config: project_path.display().to_string(),
            enabled: project_enabled,
        },
    })
}

async fn build_status_block<T: LifecycleService>(
    path: &std::path::Path,
    cfg: Option<&T::Cfg>,
    scope: Scope<'_>,
    do_ping: bool,
) -> StatusBlock {
    let mut block = StatusBlock {
        path: path.display().to_string(),
        configured: cfg.is_some(),
        credentials_present: false,
        check: None,
    };
    let Some(cfg) = cfg else {
        return block;
    };

    // A keychain-read failure (backend unavailable, locked keyring)
    // is reported as "credentials missing" rather than aborting the
    // whole command — status is diagnostic output, not a place to
    // bubble keychain errors up to `main`.
    let secrets = match T::load_secrets(scope) {
        Ok(s) => s,
        Err(e) => {
            block.check = Some(StatusCheck {
                ok: false,
                authenticated_as: None,
                error: Some(format!("keychain error: {e}")),
            });
            return block;
        }
    };
    block.credentials_present = secrets.is_some();

    if !do_ping {
        return block;
    }

    block.check = Some(match secrets {
        None => StatusCheck {
            ok: false,
            authenticated_as: None,
            error: Some("credentials missing from keychain".into()),
        },
        Some(s) => match T::validate(cfg, &s).await {
            Ok(name) => StatusCheck {
                ok: true,
                authenticated_as: Some(name),
                error: None,
            },
            Err(e) => StatusCheck {
                ok: false,
                authenticated_as: None,
                error: Some(e.to_string()),
            },
        },
    });
    block
}

pub(crate) fn print_status_human(out: &ServiceStatusOutput) {
    println!("Service: {}", out.service);
    println!();
    println!("## Credentials");
    match out.effective {
        Some(label) => println!("  effective : {label}"),
        None => println!(
            "  effective : (none — run `zad service create {}`)",
            out.service
        ),
    }
    println!("  overall   : {}", if out.ok { "ok" } else { "FAILED" });

    print_status_scope("global", &out.global);
    print_status_scope("local", &out.local);

    println!();
    println!("## Project");
    println!(
        "  enabled : {}",
        if out.project.enabled { "yes" } else { "no" }
    );
    println!("  config  : {}", out.project.config);
}

fn print_status_scope(label: &str, block: &StatusBlock) {
    println!();
    println!("  [{label}] {}", block.path);
    if !block.configured {
        println!("    status : not configured");
        return;
    }
    println!(
        "    credentials : {}",
        if block.credentials_present {
            "present"
        } else {
            "missing"
        }
    );
    match &block.check {
        None => println!("    check       : (not the effective scope)"),
        Some(c) if c.ok => {
            let name = c.authenticated_as.as_deref().unwrap_or("(unknown)");
            println!("    check       : ok (authenticated as `{name}`)");
        }
        Some(c) => {
            let err = c.error.as_deref().unwrap_or("(no error message)");
            println!("    check       : FAILED ({err})");
        }
    }
}

// ---------------------------------------------------------------------------
// Generic driver: delete
// ---------------------------------------------------------------------------

pub fn run_delete<T: LifecycleService>(args: DeleteArgs) -> Result<()> {
    let (path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) = if args.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_service_config_path_for(&slug, T::NAME)?;
        (
            p,
            "local (project-scoped)".to_string(),
            "local",
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_service_config_path(T::NAME)?,
            "global".to_string(),
            "global",
            Scope::Global,
        )
    };

    let existed = path.exists();
    if !existed && !args.force {
        return Err(ZadError::Invalid(format!(
            "no {} credentials at {scope_label} scope ({}). \
             Pass --force to ignore.",
            T::NAME,
            path.display()
        )));
    }

    if existed {
        std::fs::remove_file(&path).map_err(|e| ZadError::Io {
            path: path.clone(),
            source: e,
        })?;
        // Attempt to tidy up the per-service dir if it's now empty;
        // ignore NotFound / DirectoryNotEmpty so leftover siblings
        // (e.g. a permissions.toml for the same service) don't cause
        // a confusing error from an unrelated concern.
        if let Some(parent) = path.parent() {
            match std::fs::remove_dir(parent) {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                    ) => {}
                Err(e) => {
                    return Err(ZadError::Io {
                        path: parent.to_owned(),
                        source: e,
                    });
                }
            }
        }
    }

    let secrets_refs = T::delete_secrets(keychain_scope)?;

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_still_references = project_cfg.has_service(T::NAME);

    if args.json {
        let out = DeleteOutput {
            command: format!("service.delete.{}", T::NAME),
            scope: scope_machine,
            config_path: path.display().to_string(),
            config_removed: existed,
            secrets: secrets_refs,
            project_still_references,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    println!("{} credentials deleted ({scope_label}).", T::DISPLAY);
    println!(
        "  config : {} ({})",
        path.display(),
        if existed { "removed" } else { "not present" }
    );
    for s in &secrets_refs {
        println!("  {} : OS keychain entry `{}` cleared", s.label, s.account);
    }

    if project_still_references {
        println!();
        println!(
            "warning: this project still references the {} service ({}).",
            T::NAME,
            project_path.display()
        );
        println!(
            "         Run `zad service disable {}` to remove the `[service.{}]` entry.",
            T::NAME,
            T::NAME
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Shared prompt helpers
// ---------------------------------------------------------------------------

/// Resolve a bot-style token from `--bot-token`, `--bot-token-env`, or
/// an interactive password prompt. Used by services that flatten
/// [`BotTokenArgs`] into their `CreateArgs`; services with exotic
/// credential shapes (OAuth client-secret, PEM file) roll their own.
pub fn resolve_bot_token(
    flag: Option<&str>,
    env_flag: Option<&str>,
    non_interactive: bool,
    display: &str,
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
    let v = dialoguer::Password::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt(format!("{display} bot token"))
        .interact()?;
    Ok(v)
}

/// Resolve a comma-separated scope list against the service's declared
/// allow-list, or fall back to its defaults in non-interactive mode,
/// or open a multi-select prompt interactively.
pub fn resolve_scopes(
    flag: Option<&[String]>,
    default_scopes: &[&'static str],
    all_scopes: &[&'static str],
    non_interactive: bool,
) -> Result<Vec<String>> {
    if let Some(list) = flag {
        let cleaned: Vec<String> = list
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for s in &cleaned {
            if !all_scopes.contains(&s.as_str()) {
                return Err(ZadError::Invalid(format!("unknown scope: {s}")));
            }
        }
        return Ok(cleaned);
    }
    if non_interactive {
        return Ok(default_scopes.iter().map(|s| s.to_string()).collect());
    }
    let defaults: Vec<bool> = all_scopes
        .iter()
        .map(|s| default_scopes.contains(s))
        .collect();
    let picks = dialoguer::MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Scopes (space to toggle, enter to confirm)")
        .items(all_scopes)
        .defaults(&defaults)
        .interact()?;
    Ok(picks
        .into_iter()
        .map(|i| all_scopes[i].to_string())
        .collect())
}

/// Leak an owned `String` to satisfy `Scope::Project(&'a str)`. Safe
/// in a fire-and-forget CLI: the binary runs one command and exits,
/// so the "leak" ends with the process. Used by the generic driver
/// and by any service-specific code that needs the same lifetime
/// escape hatch.
pub fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Minimum column width: `"scopes"` is 6, and historically Discord
/// aligned every label to 6 chars. Widen if a service uses longer
/// labels (e.g. `"client id"` at 9 chars) so secrets lines stay
/// aligned with cfg lines.
fn label_width(
    cfg_lines: &[(&'static str, String)],
    scopes: &[String],
    secrets: &[SecretRef],
) -> usize {
    let mut w = "scopes".len();
    for (l, _) in cfg_lines {
        w = w.max(l.len());
    }
    for s in secrets {
        w = w.max(s.label.len());
    }
    let _ = scopes;
    w
}
