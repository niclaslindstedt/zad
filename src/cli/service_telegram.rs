//! `zad service <action> telegram` — lifecycle verbs for the Telegram
//! integration. Mirrors `service_discord.rs` in layout and behaviour so
//! the CLI surface is symmetric across services. See
//! [`crate::service::telegram`] for the (still-TODO) runtime client.

use clap::Args;
use dialoguer::{Input, MultiSelect, Password, theme::ColorfulTheme};
use serde::Serialize;

use crate::config::{self, TelegramServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};

const DEFAULT_SCOPES: &[&str] = &["messages.read", "messages.send"];
const ALL_SCOPES: &[&str] = &[
    "messages.read",
    "messages.send",
    "chats",
    "chats.manage",
    "gateway.listen",
];

// ---------------------------------------------------------------------------
// create — writes credentials, either globally (default) or project-locally
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Write credentials to this project's private service directory
    /// (`~/.zad/projects/<slug>/services/telegram/config.toml`) instead
    /// of the shared global location. Local credentials take precedence
    /// over global ones for this project.
    #[arg(long)]
    pub local: bool,

    /// Telegram bot token issued by @BotFather. Stored in the OS
    /// keychain, never in the TOML.
    #[arg(long, conflicts_with = "bot_token_env")]
    pub bot_token: Option<String>,

    /// Read the bot token from the named environment variable.
    #[arg(long, conflicts_with = "bot_token")]
    pub bot_token_env: Option<String>,

    /// Optional default chat identifier. Accepts a numeric Telegram
    /// chat ID (negative for groups/channels) or an `@username` handle.
    #[arg(long)]
    pub default_chat: Option<String>,

    /// Capabilities to enable.
    #[arg(long, value_delimiter = ',')]
    pub scopes: Option<Vec<String>>,

    /// Overwrite any existing configuration at the chosen scope.
    #[arg(long)]
    pub force: bool,

    /// Fail instead of prompting for any missing value.
    #[arg(long)]
    pub non_interactive: bool,

    /// Skip the `getMe` token validation step. Note: validation is not
    /// yet wired up (the runtime client is TODO), so this flag is a
    /// no-op today and is accepted only to keep the CLI shape stable.
    // TODO: implement getMe-based validation once `TelegramHttp` lands
    //       and drop this flag's "no-op" caveat from the help text.
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
    scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_chat: Option<String>,
    token_account: String,
}

pub async fn run_create(args: CreateArgs) -> Result<()> {
    let (path, scope_label, scope_machine, keychain_scope): (_, _, _, Scope<'_>) = if args.local {
        let slug = config::path::project_slug()?;
        let p = config::path::project_service_config_path_for(&slug, "telegram")?;
        (
            p,
            "local (project-scoped)".to_string(),
            "local",
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_service_config_path("telegram")?,
            "global".to_string(),
            "global",
            Scope::Global,
        )
    };

    let existing: Option<TelegramServiceCfg> = config::load_flat(&path)?;
    if existing.is_some() && !args.force {
        return Err(ZadError::ServiceAlreadyConfigured {
            name: format!("telegram ({scope_label})"),
        });
    }

    let token = resolve_token(
        args.bot_token.as_deref(),
        args.bot_token_env.as_deref(),
        args.non_interactive,
    )?;
    let default_chat = resolve_default_chat(args.default_chat.as_deref(), args.non_interactive)?;
    let scopes = resolve_scopes(args.scopes.as_deref(), args.non_interactive)?;

    // TODO: call TelegramHttp::validate_token (i.e. `getMe`) here once
    //       the runtime client exists, matching the Discord flow. Until
    //       then the token is taken on faith — the keychain write still
    //       happens, but we cannot yet tell the operator the bot's
    //       username or ID in the success blurb.
    let _ = args.no_validate;

    let account = secrets::telegram_bot_account(keychain_scope);
    secrets::store(&account, &token)?;

    let cfg = TelegramServiceCfg {
        scopes: scopes.clone(),
        default_chat: default_chat.clone(),
    };
    config::save_flat(&path, &cfg)?;

    if args.json {
        let out = CreateOutput {
            command: "service.create.telegram",
            scope: scope_machine,
            config_path: path.display().to_string(),
            scopes,
            default_chat,
            token_account: account,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!();
        println!("Telegram credentials created ({scope_label}).");
        println!("  config : {}", path.display());
        println!("  scopes : {}", scopes.join(", "));
        if let Some(c) = &default_chat {
            println!("  chat   : {c}");
        }
        println!("  token  : OS keychain (service=\"zad\", account=\"{account}\")");
        println!();
        println!("Next: run `zad service enable telegram` in each project that should use Telegram.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// enable — enables the service in the current project
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct EnableArgs {
    /// Overwrite an existing `[service.telegram]` entry in the project
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
    let local_creds = config::path::project_service_config_path_for(&slug, "telegram")?;
    let global_creds = config::path::global_service_config_path("telegram")?;

    let (creds_path, scope_label) = if local_creds.exists() {
        (local_creds.clone(), "local")
    } else if global_creds.exists() {
        (global_creds.clone(), "global")
    } else {
        return Err(ZadError::Invalid(format!(
            "no Telegram credentials found. Run `zad service create telegram` \
             (or with `--local`) to register credentials first.\n\
             looked in:\n  {}\n  {}",
            local_creds.display(),
            global_creds.display()
        )));
    };

    let project_path = config::path::project_config_path()?;
    let mut project_cfg = config::load_from(&project_path)?;
    if project_cfg.has_service("telegram") && !args.force {
        return Err(ZadError::ServiceAlreadyConfigured {
            name: "telegram".to_string(),
        });
    }

    project_cfg.enable_telegram();
    config::save_to(&project_path, &project_cfg)?;

    if args.json {
        let out = EnableOutput {
            command: "service.enable.telegram",
            project_config: project_path.display().to_string(),
            credentials_path: creds_path.display().to_string(),
            credentials_scope: scope_label,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Telegram service enabled for this project.");
        println!("  project config : {}", project_path.display());
        println!(
            "  credentials    : {} ({scope_label})",
            creds_path.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// disable — removes the service entry from the project config
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DisableArgs {
    /// Succeed silently even if the service is not currently enabled in
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
    let was_enabled = project_cfg.has_service("telegram");

    if !was_enabled && !args.force {
        return Err(ZadError::Invalid(format!(
            "telegram service is not enabled for this project ({}). \
             Pass --force to ignore.",
            project_path.display()
        )));
    }

    if was_enabled {
        project_cfg.disable_telegram();
        config::save_to(&project_path, &project_cfg)?;
    }

    if args.json {
        let out = DisableOutput {
            command: "service.disable.telegram",
            project_config: project_path.display().to_string(),
            was_enabled,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if was_enabled {
        println!("Telegram service disabled for this project.");
        println!("  project config : {}", project_path.display());
    } else {
        println!("Telegram service was not enabled for this project (nothing to do).");
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
    #[serde(skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_chat: Option<String>,
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
    let global_path = config::path::global_service_config_path("telegram")?;
    let local_path = config::path::project_service_config_path_for(&slug, "telegram")?;

    let global_cfg: Option<TelegramServiceCfg> = config::load_flat(&global_path)?;
    let local_cfg: Option<TelegramServiceCfg> = config::load_flat(&local_path)?;

    let effective = if local_cfg.is_some() {
        Some("local")
    } else if global_cfg.is_some() {
        Some("global")
    } else {
        None
    };

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_enabled = project_cfg.has_service("telegram");

    if args.json {
        let out = ShowOutput {
            command: "service.show.telegram",
            service: "telegram",
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

    println!("Service: telegram");
    println!();
    println!("## Credentials");
    if let Some(label) = effective {
        println!("  effective : {label}");
    } else {
        println!("  effective : (none — run `zad service create telegram`)");
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
    cfg: Option<&TelegramServiceCfg>,
    scope: Scope<'_>,
) -> Result<ScopeBlock> {
    let mut block = ScopeBlock {
        path: path.display().to_string(),
        configured: cfg.is_some(),
        scopes: None,
        default_chat: None,
        token_account: None,
        token_present: None,
    };
    if let Some(c) = cfg {
        block.scopes = Some(c.scopes.clone());
        block.default_chat = c.default_chat.clone();
        let account = secrets::telegram_bot_account(scope);
        let present = secrets::load(&account)?.is_some();
        block.token_account = Some(account);
        block.token_present = Some(present);
    }
    Ok(block)
}

fn print_scope_block(
    label: &str,
    path: &std::path::Path,
    cfg: Option<&TelegramServiceCfg>,
    scope: Scope<'_>,
) -> Result<()> {
    println!();
    println!("  [{label}] {}", path.display());
    match cfg {
        None => println!("    status : not configured"),
        Some(c) => {
            println!(
                "    scopes : {}",
                if c.scopes.is_empty() {
                    "(none)".to_string()
                } else {
                    c.scopes.join(", ")
                }
            );
            if let Some(ch) = &c.default_chat {
                println!("    chat   : {ch}");
            }
            let account = secrets::telegram_bot_account(scope);
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
        let p = config::path::project_service_config_path_for(&slug, "telegram")?;
        (
            p,
            "local (project-scoped)".to_string(),
            "local",
            Scope::Project(leak(slug)),
        )
    } else {
        (
            config::path::global_service_config_path("telegram")?,
            "global".to_string(),
            "global",
            Scope::Global,
        )
    };

    let existed = path.exists();
    if !existed && !args.force {
        return Err(ZadError::Invalid(format!(
            "no telegram credentials at {scope_label} scope ({}). \
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

    let account = secrets::telegram_bot_account(keychain_scope);
    secrets::delete(&account)?;

    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    let project_still_references = project_cfg.has_service("telegram");

    if args.json {
        let out = DeleteOutput {
            command: "service.delete.telegram",
            scope: scope_machine,
            config_path: path.display().to_string(),
            config_removed: existed,
            token_account: account,
            project_still_references,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    println!("Telegram credentials deleted ({scope_label}).");
    println!(
        "  config : {} ({})",
        path.display(),
        if existed { "removed" } else { "not present" }
    );
    println!("  token  : OS keychain entry `{account}` cleared");

    if project_still_references {
        println!();
        println!(
            "warning: this project still references the telegram service ({}).",
            project_path.display()
        );
        println!(
            "         Run `zad service disable telegram` to remove the `[service.telegram]` entry."
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
        .with_prompt("Telegram bot token")
        .interact()?;
    Ok(v)
}

fn resolve_default_chat(flag: Option<&str>, non_interactive: bool) -> Result<Option<String>> {
    if let Some(v) = flag {
        validate_chat_id(v)?;
        return Ok(Some(v.to_string()));
    }
    if non_interactive {
        return Ok(None);
    }
    let v: String = Input::with_theme(&theme())
        .with_prompt("Default chat ID or @username (leave blank for none)")
        .allow_empty(true)
        .interact_text()?;
    if v.trim().is_empty() {
        Ok(None)
    } else {
        validate_chat_id(&v).map(|_| Some(v))
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

/// Accept either a signed integer chat ID (groups and channels are
/// negative in Telegram; users/private chats are positive) or an
/// `@username` handle for public groups, channels, and bots.
fn validate_chat_id(v: &str) -> Result<()> {
    let trimmed = v.trim();
    if trimmed.is_empty() {
        return Err(ZadError::Invalid(
            "default-chat must be a Telegram chat ID or @username".into(),
        ));
    }
    if let Some(rest) = trimmed.strip_prefix('@') {
        if rest.is_empty() {
            return Err(ZadError::Invalid(
                "default-chat `@` handle is empty — expected e.g. `@my_channel`".into(),
            ));
        }
        return Ok(());
    }
    if trimmed.parse::<i64>().is_ok() {
        return Ok(());
    }
    Err(ZadError::Invalid(format!(
        "default-chat must be a numeric Telegram chat ID (positive for users, \
         negative for groups/channels) or an @username handle, got `{v}`"
    )))
}

/// Leak a single owned String to satisfy the `Scope::Project(&'a str)`
/// lifetime requirement in a fire-and-forget CLI context. The binary
/// runs one command and exits, so this is not a real leak.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
