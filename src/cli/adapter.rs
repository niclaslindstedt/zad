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
    Enable(EnableArgs),
    /// Disable an adapter in the current project (inverse of `enable`).
    Disable(DisableArgs),
    /// List all adapters with credential and project-enablement status.
    List(adapter_list::ListArgs),
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
pub struct EnableArgs {
    #[command(subcommand)]
    pub adapter: EnableAdapter,
}

#[derive(Debug, Subcommand)]
pub enum EnableAdapter {
    /// Enable the Discord adapter in the current project.
    Discord(adapter_discord::EnableArgs),
}

#[derive(Debug, Args)]
pub struct DisableArgs {
    #[command(subcommand)]
    pub adapter: DisableAdapter,
}

#[derive(Debug, Subcommand)]
pub enum DisableAdapter {
    /// Disable the Discord adapter in the current project.
    Discord(adapter_discord::DisableArgs),
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
        Action::Enable(a) => match a.adapter {
            EnableAdapter::Discord(a) => adapter_discord::run_enable(a),
        },
        Action::Disable(d) => match d.adapter {
            DisableAdapter::Discord(a) => adapter_discord::run_disable(a),
        },
        Action::List(a) => adapter_list::run(a),
        Action::Show(s) => match s.adapter {
            ShowAdapter::Discord(a) => adapter_discord::run_show(a),
        },
        Action::Delete(d) => match d.adapter {
            DeleteAdapter::Discord(a) => adapter_discord::run_delete(a),
        },
    }
}
