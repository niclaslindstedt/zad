use clap::{Args, Subcommand};

use crate::error::Result;

use super::{adapter_discord, adapter_list};

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
    /// List all adapters with credential and project-enablement status.
    List,
    /// Show details for a configured adapter.
    Show(ShowArgs),
    /// Delete credentials for an adapter (inverse of `create`).
    Delete(DeleteArgs),
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

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[command(subcommand)]
    pub adapter: ShowAdapter,
}

#[derive(Debug, Subcommand)]
pub enum ShowAdapter {
    /// Show the Discord adapter's effective configuration.
    Discord(adapter_discord::ShowArgs),
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    #[command(subcommand)]
    pub adapter: DeleteAdapter,
}

#[derive(Debug, Subcommand)]
pub enum DeleteAdapter {
    /// Delete Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(adapter_discord::DeleteArgs),
}

pub async fn run(args: AdapterArgs) -> Result<()> {
    match args.action {
        Action::Create(c) => match c.adapter {
            CreateAdapter::Discord(a) => adapter_discord::run_create(a).await,
        },
        Action::Add(a) => match a.adapter {
            AddAdapter::Discord(a) => adapter_discord::run_add(a),
        },
        Action::List => adapter_list::run(),
        Action::Show(s) => match s.adapter {
            ShowAdapter::Discord(a) => adapter_discord::run_show(a),
        },
        Action::Delete(d) => match d.adapter {
            DeleteAdapter::Discord(a) => adapter_discord::run_delete(a),
        },
    }
}
