use clap::Args;
use dialoguer::{Input, MultiSelect, Password, theme::ColorfulTheme};

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
}

pub async fn run_create(args: CreateArgs) -> Result<()> {
    let (path, scope_label, keychain_scope): (_, _, Scope<'_>) = if args.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_adapter_config_path_for(&slug, "discord")?;
        (
            p,
            "local (project-scoped)".to_string(),
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_adapter_config_path("discord")?,
            "global".to_string(),
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

    if !args.no_validate {
        tracing::info!("validating Discord bot token");
        let http = DiscordHttp::new(&token);
        match http.validate_token().await {
            Ok(name) => println!("  ✓ authenticated as bot `{name}`"),
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
    println!("Next: run `zad adapter add discord` in each project that should use Discord.");
    Ok(())
}

// ---------------------------------------------------------------------------
// add — enables the adapter in the current project
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Overwrite an existing `[adapter.discord]` entry in the project
    /// config.
    #[arg(long)]
    pub force: bool,

    /// Fail instead of prompting (reserved; `add` has no prompts today).
    #[arg(long)]
    pub non_interactive: bool,
}

pub fn run_add(args: AddArgs) -> Result<()> {
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

    println!("Discord adapter enabled for this project.");
    println!("  project config : {}", project_path.display());
    println!(
        "  credentials    : {} ({scope_label})",
        creds_path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// show — prints the effective config and both scopes' details
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ShowArgs {}

pub fn run_show(_args: ShowArgs) -> Result<()> {
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
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if project_cfg.has_adapter("discord") {
        println!("  enabled : yes");
    } else {
        println!("  enabled : no");
    }
    println!("  config  : {}", project_path.display());

    Ok(())
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
}

pub fn run_delete(args: DeleteArgs) -> Result<()> {
    let (path, scope_label, keychain_scope): (_, _, Scope<'_>) = if args.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_adapter_config_path_for(&slug, "discord")?;
        (
            p,
            "local (project-scoped)".to_string(),
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_adapter_config_path("discord")?,
            "global".to_string(),
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

    println!("Discord credentials deleted ({scope_label}).");
    println!(
        "  config : {} ({})",
        path.display(),
        if existed { "removed" } else { "not present" }
    );
    println!("  token  : OS keychain entry `{account}` cleared");

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if project_cfg.has_adapter("discord") {
        println!();
        println!(
            "warning: this project still references the discord adapter ({}).",
            project_path.display()
        );
        println!("         Remove the `[adapter.discord]` entry manually if desired.");
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
