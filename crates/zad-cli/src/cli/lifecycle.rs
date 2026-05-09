//! CLI shell over the library's typed lifecycle drivers.
//!
//! The typed core — the `LifecycleService` trait, driver functions
//! (`create`, `enable`, `disable`, `show`, `delete`, `status_for`),
//! and the `*Outcome` types — lives in
//! [`zad::service::lifecycle`]. This module adds the CLI-only layer:
//!
//! 1. **`CliLifecycle`** — extends `LifecycleService` with a
//!    `clap::Args`-derived `CreateArgs` associated type and an
//!    interactive `resolve` step that turns CLI flags into
//!    `(Cfg, Secrets)`.
//! 2. **clap arg structs** — `CreateArgsBase`, `BotTokenArgs`,
//!    `ScopesArg`, `EnableArgs`, `DisableArgs`, `ShowArgs`,
//!    `StatusArgs`, `DeleteArgs`. Per-service `CreateArgs` flatten
//!    these in.
//! 3. **`run_*` driver wrappers** that prompt where needed, call
//!    into the library drivers, and render the typed `*Outcome`
//!    values to stdout in human or JSON form.
//! 4. **`resolve_bot_token` / `resolve_scopes`** — the only places
//!    `dialoguer` is invoked from inside the lifecycle layer.

use async_trait::async_trait;
use clap::Args;

use zad::error::{Result, ZadError};
use zad::secrets::Scope;
use zad::service::lifecycle::{
    self, CreateOpts, CreateOutcome, DeleteOpts, DeleteOutcome, DisableOpts, DisableOutcome,
    EnableOpts, EnableOutcome, ShowOpts, ShowOutcome,
};

use crate::cli::DialoguerExt;

// Re-exports so existing per-service adapters and tests keep their
// `use crate::cli::lifecycle::{SecretRef, ServiceStatusOutput, …}`
// paths working unchanged. The library module is the source of truth;
// these are thin facades.
pub use lifecycle::{
    LifecycleService, ProjectBlock, ScopeBlock, SecretRef, ServiceStatusOutput, StatusBlock,
    StatusCheck, leak, status_for,
};

// ---------------------------------------------------------------------------
// CLI extension trait
// ---------------------------------------------------------------------------

/// CLI-side extension of the library's `LifecycleService`. Adds the
/// clap-derived `CreateArgs` shape and the interactive `resolve` step.
/// Implementors live in `cli/service_<name>.rs` alongside their
/// `LifecycleService` impl.
#[async_trait]
pub trait CliLifecycle: LifecycleService {
    /// Per-service `zad service create <name>` flag struct. Must
    /// embed [`CreateArgsBase`] via `#[command(flatten)]` and expose
    /// it through [`CreateArgsLike::base`].
    type CreateArgs: Args + CreateArgsLike + Send + Sync;

    /// Build `(Cfg, Secrets)` from CLI args. Interactive mode prompts
    /// for any `Option<_>` fields that arrived empty; non-interactive
    /// mode returns [`ZadError::MissingRequired`] for anything still
    /// missing.
    async fn resolve(
        args: &Self::CreateArgs,
        non_interactive: bool,
    ) -> Result<(Self::Cfg, Self::Secrets)>;
}

// ---------------------------------------------------------------------------
// Shared clap arg structs
// ---------------------------------------------------------------------------

/// Flags every `zad service create <name>` accepts.
#[derive(Debug, Args)]
pub struct CreateArgsBase {
    /// Write credentials to this project's private service directory
    /// instead of the shared global location.
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

    /// Don't open URLs in the system browser.
    #[arg(long)]
    pub no_browser: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

/// Lets the driver read [`CreateArgsBase`] out of a service-specific
/// wrapper.
pub trait CreateArgsLike {
    fn base(&self) -> &CreateArgsBase;
}

/// Drop-in clap flags for a single bot-token credential.
#[derive(Debug, Args)]
pub struct BotTokenArgs {
    #[arg(long, conflicts_with = "bot_token_env")]
    pub bot_token: Option<String>,
    #[arg(long, conflicts_with = "bot_token")]
    pub bot_token_env: Option<String>,
}

/// Drop-in clap flag for the common "comma-separated scopes" pattern.
#[derive(Debug, Args)]
pub struct ScopesArg {
    /// Capabilities to enable (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Args)]
pub struct EnableArgs {
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub non_interactive: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DisableArgs {
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// JSON envelopes — wrap the library's typed outcomes in a `command`
// field for the existing CLI contract (e.g. `service.create.discord`).
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct CreateEnvelope<'a> {
    command: String,
    scope: &'static str,
    config_path: String,
    #[serde(flatten)]
    service: serde_json::Value,
    scopes: Vec<String>,
    secrets: &'a [SecretRef],
    #[serde(skip_serializing_if = "Option::is_none")]
    authenticated_as: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'a str>,
}

#[derive(Debug, serde::Serialize)]
struct EnableEnvelope {
    command: String,
    project_config: String,
    credentials_path: String,
    credentials_scope: &'static str,
}

#[derive(Debug, serde::Serialize)]
struct DisableEnvelope {
    command: String,
    project_config: String,
    was_enabled: bool,
}

#[derive(Debug, serde::Serialize)]
struct ShowEnvelope<'a> {
    command: String,
    service: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective: Option<&'static str>,
    global: &'a ScopeBlock,
    local: &'a ScopeBlock,
    project: &'a ProjectBlock,
}

#[derive(Debug, serde::Serialize)]
struct DeleteEnvelope<'a> {
    command: String,
    scope: &'static str,
    config_path: String,
    config_removed: bool,
    secrets: &'a [SecretRef],
    project_still_references: bool,
}

// ---------------------------------------------------------------------------
// CLI driver: create
// ---------------------------------------------------------------------------

pub async fn run_create<T: CliLifecycle>(args: T::CreateArgs) -> Result<()> {
    let base = args.base();
    let (config_path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) =
        if base.local {
            let slug = zad::config::path::project_slug()?;
            let p = zad::config::path::project_service_config_path_for(&slug, T::NAME)?;
            (
                p,
                "local (project-scoped)".to_string(),
                "local",
                Scope::Project(leak(slug)),
            )
        } else {
            (
                zad::config::path::global_service_config_path(T::NAME)?,
                "global".to_string(),
                "global",
                Scope::Global,
            )
        };

    let (cfg, mut creds) = T::resolve(&args, base.non_interactive).await?;

    let validate = !base.no_validate;
    let opts = CreateOpts {
        scope_label: scope_machine,
        scope: keychain_scope,
        config_path: config_path.clone(),
        force: base.force,
        validate,
    };
    let outcome: CreateOutcome = lifecycle::create::<T>(&cfg, &mut creds, opts).await?;

    if validate
        && let Some(name) = outcome.authenticated_as.as_deref()
        && !base.json
    {
        println!("  ✓ authenticated as `{name}`");
    }

    if base.json {
        let env = CreateEnvelope {
            command: format!("service.create.{}", T::NAME),
            scope: scope_machine,
            config_path: outcome.config_path.display().to_string(),
            service: T::cfg_json(&cfg),
            scopes: T::scopes_of(&cfg).to_vec(),
            secrets: &outcome.secrets,
            authenticated_as: outcome.authenticated_as.as_deref(),
            hint: outcome.hint.as_deref(),
        };
        println!("{}", serde_json::to_string_pretty(&env).unwrap());
    } else {
        let lines = T::cfg_human(&cfg);
        let scopes = T::scopes_of(&cfg);
        let width = label_width(&lines, scopes, &outcome.secrets);
        println!();
        println!("{} credentials created ({scope_label}).", T::DISPLAY);
        let config_label = "config";
        let config_value = outcome.config_path.display().to_string();
        println!("  {config_label:width$} : {config_value}");
        for (label, value) in &lines {
            println!("  {label:width$} : {value}");
        }
        let scopes_label = "scopes";
        let scopes_value = if scopes.is_empty() {
            "(none)".to_string()
        } else {
            scopes.join(", ")
        };
        println!("  {scopes_label:width$} : {scopes_value}");
        for s in &outcome.secrets {
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
        if let Some(url) = outcome.hint.as_deref() {
            println!();
            println!("  open: {url}");
        }
    }

    if let Some(url) = outcome.hint.as_deref()
        && !base.no_browser
        && !base.non_interactive
    {
        let _ = open::that(url);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// CLI driver: enable
// ---------------------------------------------------------------------------

pub fn run_enable<T: LifecycleService>(args: EnableArgs) -> Result<()> {
    let outcome: EnableOutcome = lifecycle::enable::<T>(EnableOpts { force: args.force })?;
    if args.json {
        let env = EnableEnvelope {
            command: format!("service.enable.{}", T::NAME),
            project_config: outcome.project_config.display().to_string(),
            credentials_path: outcome.credentials_path.display().to_string(),
            credentials_scope: outcome.credentials_scope,
        };
        println!("{}", serde_json::to_string_pretty(&env).unwrap());
    } else {
        println!("{} service enabled for this project.", T::DISPLAY);
        println!("  project config : {}", outcome.project_config.display());
        println!(
            "  credentials    : {} ({})",
            outcome.credentials_path.display(),
            outcome.credentials_scope
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI driver: disable
// ---------------------------------------------------------------------------

pub fn run_disable<T: LifecycleService>(args: DisableArgs) -> Result<()> {
    let outcome: DisableOutcome = lifecycle::disable::<T>(DisableOpts { force: args.force })?;
    if args.json {
        let env = DisableEnvelope {
            command: format!("service.disable.{}", T::NAME),
            project_config: outcome.project_config.display().to_string(),
            was_enabled: outcome.was_enabled,
        };
        println!("{}", serde_json::to_string_pretty(&env).unwrap());
    } else if outcome.was_enabled {
        println!("{} service disabled for this project.", T::DISPLAY);
        println!("  project config : {}", outcome.project_config.display());
    } else {
        println!(
            "{} service was not enabled for this project (nothing to do).",
            T::DISPLAY
        );
        println!("  project config : {}", outcome.project_config.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI driver: show
// ---------------------------------------------------------------------------

pub fn run_show<T: LifecycleService>(args: ShowArgs) -> Result<()> {
    let outcome: ShowOutcome = lifecycle::show::<T>(ShowOpts)?;

    if args.json {
        let env = ShowEnvelope {
            command: format!("service.show.{}", T::NAME),
            service: T::NAME,
            effective: outcome.effective,
            global: &outcome.global,
            local: &outcome.local,
            project: &outcome.project,
        };
        println!("{}", serde_json::to_string_pretty(&env).unwrap());
        return Ok(());
    }

    println!("Service: {}", T::NAME);
    println!();
    println!("## Credentials");
    if let Some(label) = outcome.effective {
        println!("  effective : {label}");
    } else {
        println!(
            "  effective : (none — run `zad service create {}`)",
            T::NAME
        );
    }

    print_scope_block::<T>("global", &outcome.global);
    print_scope_block::<T>("local", &outcome.local);

    println!();
    println!("## Project");
    if outcome.project.enabled {
        println!("  enabled : yes");
    } else {
        println!("  enabled : no");
    }
    println!("  config  : {}", outcome.project.config);
    Ok(())
}

fn print_scope_block<T: LifecycleService>(label: &str, block: &ScopeBlock) {
    println!();
    println!("  [{label}] {}", block.path);
    if !block.configured {
        println!("    status : not configured");
        return;
    }
    let scopes_owned: Vec<String> = block.scopes.clone().unwrap_or_default();
    let width = label_width(&block.human_lines, &scopes_owned, &block.secrets);
    for (lbl, value) in &block.human_lines {
        println!("    {lbl:width$} : {value}");
    }
    let scopes_label = "scopes";
    let scopes_value = if scopes_owned.is_empty() {
        "(none)".to_string()
    } else {
        scopes_owned.join(", ")
    };
    println!("    {scopes_label:width$} : {scopes_value}");
    for s in &block.secrets {
        let lbl = s.label;
        let state = if s.present { "stored" } else { "missing" };
        let account = &s.account;
        println!("    {lbl:width$} : {state} (service=\"zad\", account=\"{account}\")");
    }
    let _ = T::NAME; // keep the type bound live for parity
}

// ---------------------------------------------------------------------------
// CLI driver: status
// ---------------------------------------------------------------------------

/// Run `zad service status --service <svc>` for service `T`. Emits
/// JSON or human output, then exits the process with code 1 if the
/// effective scope failed its live ping.
pub async fn run_status<T: LifecycleService>(args: StatusArgs) -> Result<()> {
    let mut out = lifecycle::status_for::<T>().await?;
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
// CLI driver: delete
// ---------------------------------------------------------------------------

pub fn run_delete<T: LifecycleService>(args: DeleteArgs) -> Result<()> {
    let (config_path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) =
        if args.local {
            let slug = zad::config::path::project_slug()?;
            let p = zad::config::path::project_service_config_path_for(&slug, T::NAME)?;
            (
                p,
                "local (project-scoped)".to_string(),
                "local",
                Scope::Project(leak(slug)),
            )
        } else {
            (
                zad::config::path::global_service_config_path(T::NAME)?,
                "global".to_string(),
                "global",
                Scope::Global,
            )
        };

    let outcome: DeleteOutcome = lifecycle::delete::<T>(DeleteOpts {
        scope_label: scope_machine,
        scope: keychain_scope,
        config_path: config_path.clone(),
        force: args.force,
    })?;

    if args.json {
        let env = DeleteEnvelope {
            command: format!("service.delete.{}", T::NAME),
            scope: scope_machine,
            config_path: outcome.config_path.display().to_string(),
            config_removed: outcome.config_removed,
            secrets: &outcome.secrets,
            project_still_references: outcome.project_still_references,
        };
        println!("{}", serde_json::to_string_pretty(&env).unwrap());
        return Ok(());
    }

    println!("{} credentials deleted ({scope_label}).", T::DISPLAY);
    println!(
        "  config : {} ({})",
        outcome.config_path.display(),
        if outcome.config_removed {
            "removed"
        } else {
            "not present"
        }
    );
    for s in &outcome.secrets {
        println!("  {} : OS keychain entry `{}` cleared", s.label, s.account);
    }

    if outcome.project_still_references {
        let project_path = zad::config::path::project_config_path()?;
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
// Shared prompt helpers (the only place this module touches dialoguer)
// ---------------------------------------------------------------------------

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
        .interact()
        .into_zad()?;
    Ok(v)
}

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
        .interact()
        .into_zad()?;
    Ok(picks
        .into_iter()
        .map(|i| all_scopes[i].to_string())
        .collect())
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

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
