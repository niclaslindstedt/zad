use clap::{Args, Subcommand};

use crate::error::Result;

use super::{service_discord, service_list};

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Create credentials for a service.
    Create(CreateArgs),
    /// Enable a service in the current project (using existing credentials).
    Enable(EnableArgs),
    /// Disable a service in the current project (inverse of `enable`).
    Disable(DisableArgs),
    /// List all services with credential and project-enablement status.
    List(service_list::ListArgs),
    /// Show details for a configured service.
    Show(ShowArgs),
    /// Delete credentials for a service (inverse of `create`).
    Delete(DeleteArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(subcommand)]
    pub service: CreateService,
}

#[derive(Debug, Subcommand)]
pub enum CreateService {
    /// Create Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(service_discord::CreateArgs),
}

#[derive(Debug, Args)]
pub struct EnableArgs {
    #[command(subcommand)]
    pub service: EnableService,
}

#[derive(Debug, Subcommand)]
pub enum EnableService {
    /// Enable the Discord service in the current project.
    Discord(service_discord::EnableArgs),
}

#[derive(Debug, Args)]
pub struct DisableArgs {
    #[command(subcommand)]
    pub service: DisableService,
}

#[derive(Debug, Subcommand)]
pub enum DisableService {
    /// Disable the Discord service in the current project.
    Discord(service_discord::DisableArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    #[command(subcommand)]
    pub service: ShowService,
}

#[derive(Debug, Subcommand)]
pub enum ShowService {
    /// Show the Discord service's effective configuration.
    Discord(service_discord::ShowArgs),
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    #[command(subcommand)]
    pub service: DeleteService,
}

#[derive(Debug, Subcommand)]
pub enum DeleteService {
    /// Delete Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(service_discord::DeleteArgs),
}

pub async fn run(args: ServiceArgs) -> Result<()> {
    match args.action {
        Action::Create(c) => match c.service {
            CreateService::Discord(a) => service_discord::run_create(a).await,
        },
        Action::Enable(a) => match a.service {
            EnableService::Discord(a) => service_discord::run_enable(a),
        },
        Action::Disable(d) => match d.service {
            DisableService::Discord(a) => service_discord::run_disable(a),
        },
        Action::List(a) => service_list::run(a),
        Action::Show(s) => match s.service {
            ShowService::Discord(a) => service_discord::run_show(a),
        },
        Action::Delete(d) => match d.service {
            DeleteService::Discord(a) => service_discord::run_delete(a),
        },
    }
}
