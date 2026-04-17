use clap::Args;
use dialoguer::{Input, MultiSelect, Password, theme::ColorfulTheme};
use serde::Serialize;

use crate::adapter::discord::DiscordHttp;
use crate::config::{self, DiscordAdapterCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};

const DEFAULT_SCOPES: &[&str] = &["guilds", "messages.read", "messages.send"];
const ALL_SCOPES: &[&str] = &[
    "guilds",
    "messages.read",
    "messages.send",
    "channels.manage",
    "gateway.listen",
];

// ---------------------------------------------------------------------------
// create — writes credentials, either globally (default) or project-locally
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Write credentials to this project's private adapter directory
    /// (`~/.zad/projects/<slug>/adapters/discord/config.toml`) instead
    /// of the shared global location. Local credentials take precedence
    /// over global ones for this project.
    #[arg(long)]
    pub local: bool,

    /// Discord application (bot) ID.
    #[arg(long)]
    pub application_id: Option<String>,

    /// Discord bot token. Stored in the OS keychain, never in the TOML.
    #[arg(long, conflicts_with = "bot_token_env")]
    pub bot_token: Option<String>,

    /// Read the bot token from the named environment variable.
    #[arg(long, conflicts_with = "bot_token")]
    pub bot_token_env: Option<String>,

    /// Optional default guild (server) ID.
    #[arg(long)]
    pub default_guild: Option<String>,

    /// Capabilities to enable.
    #[arg(long, value_delimiter = ',')]
    pub scopes: Option<Vec<String>>,

    /// Overwrite any existing configuration at the chosen scope.
    #[arg(long)]
    pub force: bool,

    /// Fail instead of prompting for any missing value.
    #[arg(long)]
    pub non_interactive: bool,

    /// Skip the `GET /users/@me` token validation step.
    #[arg(long)]
    pub no_validate: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct CreateOutput {
    command: &'static str,
    scope: &'static str,
    config_path: String,
    application_id: String,
    scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_guild: Option<String>,
    token_account: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authenticated_as: Option<String>,
}

pub async fn run_create(args: CreateArgs) -> Result<()> {
    let (path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) = if args.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_adapter_config_path_for(&slug, "discord")?;
        (
            p,
            "local (project-scoped)".to_string(),
            "local",
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_adapter_config_path("discord")?,
            "global".to_string(),
            "global",
            Scope::Global,
        )
    };

    let existing: Option<DiscordAdapterCfg> = config::load_flat(&path)?;
    if existing.is_some() && !args.force {
        return Err(ZadError::AdapterAlreadyConfigured {
            name: format!("discord ({scope_label})"),
        });
    }

    let application_id =
        resolve_application_id(args.application_id.as_deref(), args.non_interactive)?;
    let token = resolve_token(
        args.bot_token.as_deref(),
        args.bot_token_env.as_deref(),
        args.non_interactive,
    )?;
    let default_guild = resolve_default_guild(args.default_guild.as_deref(), args.non_interactive)?;
    let scopes = resolve_scopes(args.scopes.as_deref(), args.non_interactive)?;

    let mut authenticated_as: Option<String> = None;
    if !args.no_validate {
        tracing::info!("validating Discord bot token");
        let http = DiscordHttp::new(&token);
        match http.validate_token().await {
            Ok(name) => {
                if !args.json {
                    println!("  ✓ authenticated as bot `{name}`");
                }
                authenticated_as = Some(name);
            }
            Err(e) => return Err(ZadError::Discord(format!("token validation failed: {e}"))),
        }
    }

    let account = secrets::discord_bot_account(keychain_scope);
    secrets::store(&account, &token)?;

    let cfg = DiscordAdapterCfg {
        application_id: application_id.clone(),
        scopes: scopes.clone(),
        default_guild: default_guild.clone(),
    };
    config::save_flat(&path, &cfg)?;

    if args.json {
        let out = CreateOutput {
            command: "adapter.create.discord",
            scope: scope_machine,
            config_path: path.display().to_string(),
            application_id,
            scopes,
            default_guild,
            token_account: account,
            authenticated_as,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!();
        println!("Discord credentials created ({scope_label}).");
        println!("  config : {}", path.display());
        println!("  app id : {application_id}");
        println!("  scopes : {}", scopes.join(", "));
        if let Some(g) = &default_guild {
            println!("  guild  : {g}");
        }
        println!("  token  : OS keychain (service=\"zad\", account=\"{account}\")");
        println!();
        println!("Next: run `zad adapter enable discord` in each project that should use Discord.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// enable — enables the adapter in the current project
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct EnableArgs {
    /// Overwrite an existing `[adapter.discord]` entry in the project
    /// config.
    #[arg(long)]
    pub force: bool,

    /// Fail instead of prompting (reserved; `enable` has no prompts today).
    #[arg(long)]
    pub non_interactive: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct EnableOutput {
    command: &'static str,
    project_config: String,
    credentials_path: String,
    credentials_scope: &'static str,
}

pub fn run_enable(args: EnableArgs) -> Result<()> {
    let slug = config::path::project_slug()?;
    let local_creds = config::path::project_adapter_config_path_for(&slug, "discord")?;
    let global_creds = config::path::global_adapter_config_path("discord")?;

    let (creds_path, scope_label) = if local_creds.exists() {
        (local_creds.clone(), "local")
    } else if global_creds.exists() {
        (global_creds.clone(), "global")
    } else {
        return Err(ZadError::Invalid(format!(
            "no Discord credentials found. Run `zad adapter create discord` \
             (or with `--local`) to register credentials first.\n\
             looked in:\n  {}\n  {}",
            local_creds.display(),
            global_creds.display()
        )));
    };

    let project_path = config::path::project_config_path()?;
    let mut project_cfg = config::load_from(&project_path)?;
    if project_cfg.has_adapter("discord") && !args.force {
        return Err(ZadError::AdapterAlreadyConfigured {
            name: "discord".to_string(),
        });
    }

    project_cfg.enable_discord();
    config::save_to(&project_path, &project_cfg)?;

    if args.json {
        let out = EnableOutput {
            command: "adapter.enable.discord",
            project_config: project_path.display().to_string(),
            credentials_path: creds_path.display().to_string(),
            credentials_scope: scope_label,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Discord adapter enabled for this project.");
        println!("  project config : {}", project_path.display());
        println!(
            "  credentials    : {} ({scope_label})",
            creds_path.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// disable — removes the adapter entry from the project config
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DisableArgs {
    /// Succeed silently even if the adapter is not currently enabled in
    /// this project.
    #[arg(long)]
    pub force: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct DisableOutput {
    command: &'static str,
    project_config: String,
    was_enabled: bool,
}

pub fn run_disable(args: DisableArgs) -> Result<()> {
    let project_path = config::path::project_config_path()?;
    let mut project_cfg = config::load_from(&project_path)?;
    let was_enabled = project_cfg.has_adapter("discord");

    if !was_enabled && !args.force {
        return Err(ZadError::Invalid(format!(
            "discord adapter is not enabled for this project ({}). \
             Pass --force to ignore.",
            project_path.display()
        )));
    }

    if was_enabled {
        project_cfg.disable_discord();
        config::save_to(&project_path, &project_cfg)?;
    }

    if args.json {
        let out = DisableOutput {
            command: "adapter.disable.discord",
            project_config: project_path.display().to_string(),
            was_enabled,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if was_enabled {
        println!("Discord adapter disabled for this project.");
        println!("  project config : {}", project_path.display());
    } else {
        println!("Discord adapter was not enabled for this project (nothing to do).");
        println!("  project config : {}", project_path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// show — prints the effective config and both scopes' details
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ShowOutput {
    command: &'static str,
    adapter: &'static str,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    application_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_guild: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_present: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ProjectBlock {
    config: String,
    enabled: bool,
}

pub fn run_show(args: ShowArgs) -> Result<()> {
    let slug = config::path::project_slug()?;
    let global_path = config::path::global_adapter_config_path("discord")?;
    let local_path = config::path::project_adapter_config_path_for(&slug, "discord")?;

    let global_cfg: Option<DiscordAdapterCfg> = config::load_flat(&global_path)?;
    let local_cfg: Option<DiscordAdapterCfg> = config::load_flat(&local_path)?;

    let effective = if local_cfg.is_some() {
        Some("local")
    } else if global_cfg.is_some() {
        Some("global")
    } else {
        None
    };

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_enabled = project_cfg.has_adapter("discord");

    if args.json {
        let out = ShowOutput {
            command: "adapter.show.discord",
            adapter: "discord",
            effective,
            global: scope_block(&global_path, global_cfg.as_ref(), Scope::Global)?,
            local: scope_block(&local_path, local_cfg.as_ref(), Scope::Project(&slug))?,
            project: ProjectBlock {
                config: project_path.display().to_string(),
                enabled: project_enabled,
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    println!("Adapter: discord");
    println!();
    println!("## Credentials");
    if let Some(label) = effective {
        println!("  effective : {label}");
    } else {
        println!("  effective : (none — run `zad adapter create discord`)");
    }

    print_scope_block("global", &global_path, global_cfg.as_ref(), Scope::Global)?;
    print_scope_block(
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

fn scope_block(
    path: &std::path::Path,
    cfg: Option<&DiscordAdapterCfg>,
    scope: Scope<'_>,
) -> Result<ScopeBlock> {
    let mut block = ScopeBlock {
        path: path.display().to_string(),
        configured: cfg.is_some(),
        application_id: None,
        scopes: None,
        default_guild: None,
        token_account: None,
        token_present: None,
    };
    if let Some(c) = cfg {
        block.application_id = Some(c.application_id.clone());
        block.scopes = Some(c.scopes.clone());
        block.default_guild = c.default_guild.clone();
        let account = secrets::discord_bot_account(scope);
        let present = secrets::load(&account)?.is_some();
        block.token_account = Some(account);
        block.token_present = Some(present);
    }
    Ok(block)
}

fn print_scope_block(
    label: &str,
    path: &std::path::Path,
    cfg: Option<&DiscordAdapterCfg>,
    scope: Scope<'_>,
) -> Result<()> {
    println!();
    println!("  [{label}] {}", path.display());
    match cfg {
        None => println!("    status : not configured"),
        Some(c) => {
            println!("    app id : {}", c.application_id);
            println!(
                "    scopes : {}",
                if c.scopes.is_empty() {
                    "(none)".to_string()
                } else {
                    c.scopes.join(", ")
                }
            );
            if let Some(g) = &c.default_guild {
                println!("    guild  : {g}");
            }
            let account = secrets::discord_bot_account(scope);
            let present = secrets::load(&account)?.is_some();
            println!(
                "    token  : {} (service=\"zad\", account=\"{account}\")",
                if present { "stored" } else { "missing" }
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// delete — removes credentials at the chosen scope (inverse of `create`)
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// Delete the project-scoped credentials instead of the global ones.
    #[arg(long)]
    pub local: bool,

    /// Succeed silently even if no config file exists at the chosen scope.
    #[arg(long)]
    pub force: bool,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct DeleteOutput {
    command: &'static str,
    scope: &'static str,
    config_path: String,
    config_removed: bool,
    token_account: String,
    project_still_references: bool,
}

pub fn run_delete(args: DeleteArgs) -> Result<()> {
    let (path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) = if args.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_adapter_config_path_for(&slug, "discord")?;
        (
            p,
            "local (project-scoped)".to_string(),
            "local",
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_adapter_config_path("discord")?,
            "global".to_string(),
            "global",
            Scope::Global,
        )
    };

    let existed = path.exists();
    if !existed && !args.force {
        return Err(ZadError::Invalid(format!(
            "no discord credentials at {scope_label} scope ({}). \
             Pass --force to ignore.",
            path.display()
        )));
    }

    if existed {
        std::fs::remove_file(&path).map_err(|e| ZadError::Io {
            path: path.clone(),
            source: e,
        })?;
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

    let account = secrets::discord_bot_account(keychain_scope);
    secrets::delete(&account)?;

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_still_references = project_cfg.has_adapter("discord");

    if args.json {
        let out = DeleteOutput {
            command: "adapter.delete.discord",
            scope: scope_machine,
            config_path: path.display().to_string(),
            config_removed: existed,
            token_account: account,
            project_still_references,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    println!("Discord credentials deleted ({scope_label}).");
    println!(
        "  config : {} ({})",
        path.display(),
        if existed { "removed" } else { "not present" }
    );
    println!("  token  : OS keychain entry `{account}` cleared");

    if project_still_references {
        println!();
        println!(
            "warning: this project still references the discord adapter ({}).",
            project_path.display()
        );
        println!(
            "         Run `zad adapter disable discord` to remove the `[adapter.discord]` entry."
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// prompt helpers (shared by `create`)
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
        return Err(ZadError::MissingRequired("--bot-token or --bot-token-env"));
    }
    let v = Password::with_theme(&theme())
        .with_prompt("Discord bot token")
        .interact()?;
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

fn resolve_scopes(flag: Option<&[String]>, non_interactive: bool) -> Result<Vec<String>> {
    if let Some(list) = flag {
        let cleaned: Vec<String> = list
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for s in &cleaned {
            if !ALL_SCOPES.contains(&s.as_str()) {
                return Err(ZadError::Invalid(format!("unknown scope: {s}")));
            }
        }
        return Ok(cleaned);
    }
    if non_interactive {
        return Ok(DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect());
    }
    let defaults: Vec<bool> = ALL_SCOPES
        .iter()
        .map(|s| DEFAULT_SCOPES.contains(s))
        .collect();
    let picks = MultiSelect::with_theme(&theme())
        .with_prompt("Scopes (space to toggle, enter to confirm)")
        .items(ALL_SCOPES)
        .defaults(&defaults)
        .interact()?;
    Ok(picks
        .into_iter()
        .map(|i| ALL_SCOPES[i].to_string())
        .collect())
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

/// Leak a single owned String to satisfy the `Scope::Project(&'a str)`
/// lifetime requirement in a fire-and-forget CLI context. The binary
/// runs one command and exits, so this is not a real leak.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
