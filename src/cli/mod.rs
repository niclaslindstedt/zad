pub mod commands;
pub mod debug_agent;
pub mod discord;
pub mod docs;
pub mod gcal;
pub mod help_agent;
pub mod lifecycle;
pub mod man;
pub mod onepass;
pub mod service;
pub mod service_discord;
pub mod service_gcal;
pub mod service_list;
pub mod service_onepass;
pub mod service_telegram;
pub mod status;
pub mod telegram;

use clap::{Parser, Subcommand};

use crate::error::Result;

#[derive(Debug, Parser)]
#[command(
    name = "zad",
    version,
    about = "Connect AI agents to external services via scoped service configs.",
    disable_help_subcommand = true,
    propagate_version = true
)]
pub struct Cli {
    /// Enable debug-level logging on stderr (file log is always on).
    #[arg(long, global = true)]
    pub debug: bool,

    /// Print a compact, prompt-injectable description of this CLI suitable
    /// for splicing into an agent prompt via command substitution. See
    /// OSS_SPEC.md §12.1.
    #[arg(long, global = true)]
    pub help_agent: bool,

    /// Print a troubleshooting block (log paths, config precedence, env
    /// vars, diagnostic commands, version). See OSS_SPEC.md §12.2.
    #[arg(long, global = true)]
    pub debug_agent: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Configure or inspect external services.
    Service(service::ServiceArgs),
    /// Operate the 1Password service (vaults, items, get, read, inject, create).
    #[command(name = "1pass")]
    OnePass(onepass::OnePassArgs),
    /// Operate the Discord service (send, read, channels, join, leave).
    Discord(discord::DiscordArgs),
    /// Operate the Google Calendar service (calendars, events, permissions).
    Gcal(gcal::GcalArgs),
    /// Operate the Telegram service (send, read, chats, discover).
    Telegram(telegram::TelegramArgs),
    /// Check the live status of every configured service in one go.
    Status(status::StatusArgs),
    /// Enumerate CLI commands, flags, and realistic examples.
    Commands(commands::CommandsArgs),
    /// Print topic documentation embedded at build time.
    Docs(docs::DocsArgs),
    /// Print reference manpages embedded at build time.
    Man(man::ManArgs),
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    crate::logging::init(cli.debug);

    if cli.help_agent {
        print!("{}", help_agent::render());
        return Ok(());
    }

    if cli.debug_agent {
        print!("{}", debug_agent::render());
        return Ok(());
    }

    match cli.command {
        Some(Command::Service(args)) => service::run(args).await,
        Some(Command::OnePass(args)) => onepass::run(args).await,
        Some(Command::Discord(args)) => discord::run(args).await,
        Some(Command::Gcal(args)) => gcal::run(args).await,
        Some(Command::Telegram(args)) => telegram::run(args).await,
        Some(Command::Status(args)) => status::run(args).await,
        Some(Command::Commands(args)) => commands::run(args),
        Some(Command::Docs(args)) => docs::run(args),
        Some(Command::Man(args)) => man::run(args),
        None => {
            println!("zad {}", crate::version());
            println!("Run `zad --help` for usage.");
            Ok(())
        }
    }
}
