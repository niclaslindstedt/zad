pub mod adapter;
pub mod adapter_discord;
pub mod adapter_list;

use clap::{Parser, Subcommand};

use crate::error::Result;

#[derive(Debug, Parser)]
#[command(
    name = "zad",
    version,
    about = "Connect AI agents to external services via scoped adapter configs.",
    disable_help_subcommand = true,
    propagate_version = true
)]
pub struct Cli {
    /// Enable debug-level logging on stderr (file log is always on).
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Configure or inspect external-service adapters.
    Adapter(adapter::AdapterArgs),
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    crate::logging::init(cli.debug);

    match cli.command {
        Some(Command::Adapter(args)) => adapter::run(args).await,
        None => {
            println!("zad {}", crate::version());
            println!("Run `zad --help` for usage.");
            Ok(())
        }
    }
}
