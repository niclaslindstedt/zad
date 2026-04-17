use clap::{Args, Subcommand};

use crate::error::Result;

use super::adapter_discord;

#[derive(Debug, Args)]
pub struct AdapterArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Create credentials for an adapter.
    Create(CreateArgs),
    /// Enable an adapter in the current project (using existing credentials).
    Add(AddArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(subcommand)]
    pub adapter: CreateAdapter,
}

#[derive(Debug, Subcommand)]
pub enum CreateAdapter {
    /// Create Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(adapter_discord::CreateArgs),
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[command(subcommand)]
    pub adapter: AddAdapter,
}

#[derive(Debug, Subcommand)]
pub enum AddAdapter {
    /// Enable the Discord adapter in the current project.
    Discord(adapter_discord::AddArgs),
}

pub async fn run(args: AdapterArgs) -> Result<()> {
    match args.action {
        Action::Create(c) => match c.adapter {
            CreateAdapter::Discord(a) => adapter_discord::run_create(a).await,
        },
        Action::Add(a) => match a.adapter {
            AddAdapter::Discord(a) => adapter_discord::run_add(a),
        },
    }
}
