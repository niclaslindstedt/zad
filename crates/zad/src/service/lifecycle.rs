//! Service-lifecycle primitives — the typed library core that the
//! CLI's `service create / enable / disable / show / status / delete`
//! commands sit on top of.
//!
//! Everything in this module is library-shaped: typed inputs, typed
//! outputs, no `clap`, no `dialoguer`, no `println!`. Library callers
//! that want to provision a service programmatically (multi-tenant
//! servers, infrastructure-as-code tools, integration tests) can
//! depend on `zad` and call these functions directly. The CLI in
//! `zad-cli` wraps each driver with arg parsing, interactive prompts,
//! and human/JSON output formatting.
//!
//! ## Trait split
//!
//! - [`LifecycleService`] — the **library** trait. Per-service
//!   implementors describe how to enable/disable a service in a
//!   project config, validate credentials, store/load/inspect secrets,
//!   and render config to the human/JSON shapes. **No `clap` bound.**
//! - The CLI extends this with a `CliLifecycle` trait that adds
//!   `clap::Args`-derived `CreateArgs` and the interactive `resolve`
//!   step that turns CLI flags into `(Cfg, Secrets)`. CLI driver
//!   functions (`run_create`, etc.) compose the two.
//!
//! ## Driver functions
//!
//! The free functions [`create`], [`enable`], [`disable`], [`show`],
//! [`delete`], and [`status_for`] are the typed library entry points.
//! Each takes pre-validated inputs and returns a typed `*Outcome` that
//! the caller (CLI or library code) can render however it likes.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

use crate::config::{self, ProjectConfig};
use crate::error::{Result, ZadError};
use crate::secrets::Scope;

// ---------------------------------------------------------------------------
// The library trait
// ---------------------------------------------------------------------------

/// Library-shaped lifecycle plumbing for one service. Implement this
/// when adding a new service so the typed `create` / `enable` / …
/// driver functions below work for it.
///
/// The CLI extends this with `CliLifecycle` (in `zad-cli`) that adds
/// the `CreateArgs: clap::Args` associated type plus an interactive
/// `resolve` step. Library callers don't need that — they construct
/// `(Cfg, Secrets)` themselves and call [`create`] directly.
#[async_trait]
pub trait LifecycleService: Send + Sync + 'static {
    /// Lowercase identifier used in paths, commands, and keychain
    /// account names (`"discord"`, `"telegram"`, …). Must match the
    /// entry in [`crate::service::registry::SERVICES`].
    const NAME: &'static str;

    /// Capitalized display name for human-facing output (`"Discord"`).
    const DISPLAY: &'static str;

    /// Non-secret per-service config persisted to the service's
    /// `config.toml`. Anything that isn't a credential belongs here.
    type Cfg: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;

    /// Credential material held in the OS keychain. Shape is up to
    /// the service: one bot token, three OAuth fields, a PEM blob —
    /// whatever the provider needs.
    type Secrets: Send + Sync;

    /// Mark the current project as using this service.
    fn enable_in_project(cfg: &mut ProjectConfig);

    /// Remove this service's entry from the current project config.
    fn disable_in_project(cfg: &mut ProjectConfig);

    /// Confirm the credentials work by pinging the provider. Returns
    /// a short identifier (bot username, GitHub App slug) on success.
    async fn validate(cfg: &Self::Cfg, secrets: &Self::Secrets) -> Result<String>;

    /// Write each piece of secret material to the OS keychain at
    /// `scope`. Returns one `SecretRef` per account written.
    fn store_secrets(secrets: &Self::Secrets, scope: Scope<'_>) -> Result<Vec<SecretRef>>;

    /// Remove every keychain entry for this service at `scope`.
    /// Idempotent.
    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>>;

    /// Report keychain presence per account this service expects.
    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>>;

    /// Load the full secret material from the keychain at `scope`.
    /// Returns `Ok(None)` if any required account is missing.
    fn load_secrets(scope: Scope<'_>) -> Result<Option<Self::Secrets>>;

    /// Human-readable non-secret fields, as `(label, value)` pairs.
    fn cfg_human(cfg: &Self::Cfg) -> Vec<(&'static str, String)>;

    /// Non-secret fields rendered for `--json`.
    fn cfg_json(cfg: &Self::Cfg) -> serde_json::Value;

    /// Declared scopes — stored verbatim in the TOML config's
    /// `scopes` array.
    fn scopes_of(cfg: &Self::Cfg) -> &[String];

    /// Optional URL to surface immediately after `create` succeeds.
    /// Default: no hint.
    fn post_create_hint(_cfg: &Self::Cfg) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// One OS-keychain entry belonging to a service.
#[derive(Debug, Clone, Serialize)]
pub struct SecretRef {
    /// Human label, e.g. `"token"`, `"bot token"`, `"client secret"`.
    pub label: &'static str,
    /// Full keychain account string passed to [`crate::secrets::store`].
    pub account: String,
    /// Whether the OS keychain currently has this entry.
    pub present: bool,
}

/// Per-scope view of credentials, for `show` / `status` output.
#[derive(Debug, Clone, Serialize)]
pub struct ScopeBlock {
    pub path: String,
    pub configured: bool,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub service: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub secrets: Vec<SecretRef>,
    /// Human-readable `(label, value)` pairs derived from the typed
    /// `Cfg` via [`LifecycleService::cfg_human`]. Carried in the
    /// outcome so CLI rendering doesn't need to re-parse the config
    /// file or re-instantiate the typed Cfg.
    #[serde(skip)]
    pub human_lines: Vec<(&'static str, String)>,
}

/// Per-service envelope for `zad service status --service <svc>`.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatusOutput {
    /// `"service.status.<name>"` for the per-service command; left
    /// unset when the value is embedded in the aggregate output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub service: &'static str,
    /// Which scope would be used at runtime (local wins over global).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<&'static str>,
    /// Overall health: `true` iff the effective scope pinged OK.
    pub ok: bool,
    pub global: StatusBlock,
    pub local: StatusBlock,
    pub project: ProjectBlock,
}

/// One scope's view for status. `check` is populated only for the
/// effective scope.
#[derive(Debug, Clone, Serialize)]
pub struct StatusBlock {
    pub path: String,
    pub configured: bool,
    pub credentials_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<StatusCheck>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCheck {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticated_as: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectBlock {
    pub config: String,
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Driver options
// ---------------------------------------------------------------------------

/// Options for [`create`]. The library doesn't deal with browser
/// auto-open or interactive prompts — those are CLI concerns. The
/// caller is responsible for choosing `Scope::Global` vs
/// `Scope::Project(slug)` and the matching config path.
#[derive(Debug)]
pub struct CreateOpts<'a> {
    pub scope_label: &'static str,
    pub scope: Scope<'a>,
    pub config_path: PathBuf,
    /// Overwrite existing config at this scope.
    pub force: bool,
    /// Run [`LifecycleService::validate`] before storing secrets.
    pub validate: bool,
}

#[derive(Debug)]
pub struct EnableOpts {
    pub force: bool,
}

#[derive(Debug)]
pub struct DisableOpts {
    pub force: bool,
}

#[derive(Debug)]
pub struct ShowOpts;

#[derive(Debug)]
pub struct DeleteOpts<'a> {
    pub scope_label: &'static str,
    pub scope: Scope<'a>,
    pub config_path: PathBuf,
    pub force: bool,
}

// ---------------------------------------------------------------------------
// Driver outputs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    pub config_path: PathBuf,
    pub scope: &'static str,
    pub secrets: Vec<SecretRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticated_as: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnableOutcome {
    pub project_config: PathBuf,
    pub credentials_path: PathBuf,
    pub credentials_scope: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisableOutcome {
    pub project_config: PathBuf,
    pub was_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShowOutcome {
    pub effective: Option<&'static str>,
    pub global: ScopeBlock,
    pub local: ScopeBlock,
    pub project: ProjectBlock,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteOutcome {
    pub config_path: PathBuf,
    pub config_removed: bool,
    pub secrets: Vec<SecretRef>,
    pub project_still_references: bool,
}

// ---------------------------------------------------------------------------
// Driver: create
// ---------------------------------------------------------------------------

/// Provision the service: optionally validate, store secrets, write
/// the config file. Caller resolves `(cfg, secrets)` first (the CLI
/// uses interactive prompts; library callers construct them directly).
pub async fn create<T: LifecycleService>(
    cfg: &T::Cfg,
    secrets: &T::Secrets,
    opts: CreateOpts<'_>,
) -> Result<CreateOutcome> {
    let existing: Option<T::Cfg> = config::load_flat(&opts.config_path)?;
    if existing.is_some() && !opts.force {
        return Err(ZadError::ServiceAlreadyConfigured {
            name: format!("{} ({})", T::NAME, opts.scope_label),
        });
    }

    let authenticated_as = if opts.validate {
        Some(T::validate(cfg, secrets).await?)
    } else {
        None
    };

    let secret_refs = T::store_secrets(secrets, opts.scope)?;
    config::save_flat(&opts.config_path, cfg)?;
    let hint = T::post_create_hint(cfg);

    Ok(CreateOutcome {
        config_path: opts.config_path,
        scope: scope_machine_label(opts.scope_label),
        secrets: secret_refs,
        authenticated_as,
        hint,
    })
}

// ---------------------------------------------------------------------------
// Driver: enable
// ---------------------------------------------------------------------------

pub fn enable<T: LifecycleService>(opts: EnableOpts) -> Result<EnableOutcome> {
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
    if project_cfg.has_service(T::NAME) && !opts.force {
        return Err(ZadError::ServiceAlreadyConfigured {
            name: T::NAME.to_string(),
        });
    }

    T::enable_in_project(&mut project_cfg);
    config::save_to(&project_path, &project_cfg)?;

    Ok(EnableOutcome {
        project_config: project_path,
        credentials_path: creds_path,
        credentials_scope: scope_label,
    })
}

// ---------------------------------------------------------------------------
// Driver: disable
// ---------------------------------------------------------------------------

pub fn disable<T: LifecycleService>(opts: DisableOpts) -> Result<DisableOutcome> {
    let project_path = config::path::project_config_path()?;
    let mut project_cfg = config::load_from(&project_path)?;
    let was_enabled = project_cfg.has_service(T::NAME);

    if !was_enabled && !opts.force {
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

    Ok(DisableOutcome {
        project_config: project_path,
        was_enabled,
    })
}

// ---------------------------------------------------------------------------
// Driver: show
// ---------------------------------------------------------------------------

pub fn show<T: LifecycleService>(_opts: ShowOpts) -> Result<ShowOutcome> {
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

    Ok(ShowOutcome {
        effective,
        global: scope_block::<T>(&global_path, global_cfg.as_ref(), Scope::Global)?,
        local: scope_block::<T>(&local_path, local_cfg.as_ref(), Scope::Project(&slug))?,
        project: ProjectBlock {
            config: project_path.display().to_string(),
            enabled: project_enabled,
        },
    })
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
        human_lines: Vec::new(),
    };
    if let Some(c) = cfg {
        block.service = Some(T::cfg_json(c));
        block.scopes = Some(T::scopes_of(c).to_vec());
        block.secrets = T::inspect_secrets(scope)?;
        block.human_lines = T::cfg_human(c);
    }
    Ok(block)
}

// ---------------------------------------------------------------------------
// Driver: delete
// ---------------------------------------------------------------------------

pub fn delete<T: LifecycleService>(opts: DeleteOpts<'_>) -> Result<DeleteOutcome> {
    let existed = opts.config_path.exists();
    if !existed && !opts.force {
        return Err(ZadError::Invalid(format!(
            "no {} credentials at {} scope ({}). Pass --force to ignore.",
            T::NAME,
            opts.scope_label,
            opts.config_path.display()
        )));
    }

    if existed {
        std::fs::remove_file(&opts.config_path).map_err(|e| ZadError::Io {
            path: opts.config_path.clone(),
            source: e,
        })?;
        if let Some(parent) = opts.config_path.parent() {
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

    let secret_refs = T::delete_secrets(opts.scope)?;

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_still_references = project_cfg.has_service(T::NAME);

    Ok(DeleteOutcome {
        config_path: opts.config_path,
        config_removed: existed,
        secrets: secret_refs,
        project_still_references,
    })
}

// ---------------------------------------------------------------------------
// Driver: status
// ---------------------------------------------------------------------------

/// Collect the status envelope for service `T` without emitting
/// anything. Pings only the effective scope.
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scope_machine_label(label: &str) -> &'static str {
    if label.starts_with("local") {
        "local"
    } else {
        "global"
    }
}

/// Leak an owned `String` to satisfy `Scope::Project(&'a str)`. Safe
/// in fire-and-forget binaries: the process runs one command and
/// exits, so the "leak" ends with the process. Library callers in
/// long-lived processes should construct `Scope::Project(&slug)` from
/// a string they own and avoid this helper.
pub fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
