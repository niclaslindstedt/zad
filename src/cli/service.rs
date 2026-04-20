//! Dispatch for `zad service <action> <name>`.
//!
//! Each action is a thin clap enum: one variant per service that
//! routes to the generic `lifecycle::run_*::<T>()` driver with the
//! service's `LifecycleService` impl as the type parameter. Adding a
//! new service means adding one variant to each enum below plus one
//! dispatch arm in `run()` — about 10 lines total.

use clap::{Args, Subcommand, builder::PossibleValuesParser};

use crate::cli::lifecycle::{self, DeleteArgs, DisableArgs, EnableArgs, ShowArgs};
use crate::error::Result;
use crate::service::registry::SERVICES;

use super::{
    service_discord, service_gcal, service_github, service_list, service_onepass, service_status,
    service_telegram,
};
use service_discord::DiscordLifecycle;
use service_gcal::GcalLifecycle;
use service_github::GithubLifecycle;
use service_onepass::OnePassLifecycle;
use service_telegram::TelegramLifecycle;

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Action {
    /// Create credentials for a service.
    Create(CreateArgs),
    /// Enable a service in the current project (using existing credentials).
    Enable(EnableAction),
    /// Disable a service in the current project (inverse of `enable`).
    Disable(DisableAction),
    /// List all services with credential and project-enablement status.
    List(service_list::ListArgs),
    /// Show details for a configured service.
    Show(ShowAction),
    /// Check whether service credentials work by pinging the provider.
    /// Without `--service`, every configured service is pinged in parallel.
    Status(StatusArgs),
    /// Delete credentials for a service (inverse of `create`).
    Delete(DeleteAction),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(subcommand)]
    pub service: CreateService,
}

#[derive(Debug, Subcommand)]
pub enum CreateService {
    /// Create 1Password (1pass) credentials (global by default,
    /// `--local` for project-scoped).
    #[command(name = "1pass")]
    OnePass(service_onepass::CreateArgs),
    /// Create Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(service_discord::CreateArgs),
    /// Create Google Calendar credentials (global by default,
    /// `--local` for project-scoped).
    Gcal(service_gcal::CreateArgs),
    /// Create GitHub credentials (global by default, `--local` for
    /// project-scoped).
    Github(service_github::CreateArgs),
    /// Create Telegram credentials (global by default, `--local` for
    /// project-scoped).
    Telegram(service_telegram::CreateArgs),
}

#[derive(Debug, Args)]
pub struct EnableAction {
    #[command(subcommand)]
    pub service: EnableService,
}

#[derive(Debug, Subcommand)]
pub enum EnableService {
    /// Enable the 1Password service in the current project.
    #[command(name = "1pass")]
    OnePass(EnableArgs),
    /// Enable the Discord service in the current project.
    Discord(EnableArgs),
    /// Enable the Google Calendar service in the current project.
    Gcal(EnableArgs),
    /// Enable the GitHub service in the current project.
    Github(EnableArgs),
    /// Enable the Telegram service in the current project.
    Telegram(EnableArgs),
}

#[derive(Debug, Args)]
pub struct DisableAction {
    #[command(subcommand)]
    pub service: DisableService,
}

#[derive(Debug, Subcommand)]
pub enum DisableService {
    /// Disable the 1Password service in the current project.
    #[command(name = "1pass")]
    OnePass(DisableArgs),
    /// Disable the Discord service in the current project.
    Discord(DisableArgs),
    /// Disable the Google Calendar service in the current project.
    Gcal(DisableArgs),
    /// Disable the GitHub service in the current project.
    Github(DisableArgs),
    /// Disable the Telegram service in the current project.
    Telegram(DisableArgs),
}

#[derive(Debug, Args)]
pub struct ShowAction {
    #[command(subcommand)]
    pub service: ShowService,
}

#[derive(Debug, Subcommand)]
pub enum ShowService {
    /// Show the 1Password service's effective configuration.
    #[command(name = "1pass")]
    OnePass(ShowArgs),
    /// Show the Discord service's effective configuration.
    Discord(ShowArgs),
    /// Show the Google Calendar service's effective configuration.
    Gcal(ShowArgs),
    /// Show the GitHub service's effective configuration.
    Github(ShowArgs),
    /// Show the Telegram service's effective configuration.
    Telegram(ShowArgs),
}

/// Args for `zad service status [--service <NAME>] [--json]`.
///
/// Without `--service`, every service registered in
/// [`crate::service::registry::SERVICES`] is pinged in parallel and a
/// single aggregate envelope is emitted. With `--service`, only the
/// named service is pinged and the per-service envelope is emitted.
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Limit the check to a single service (e.g. `discord`, `telegram`).
    /// Without this flag, every service in the registry is pinged.
    #[arg(long, value_name = "NAME", value_parser = PossibleValuesParser::new(SERVICES))]
    pub service: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text.
    /// Recommended for agents — the envelope is stable.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteAction {
    #[command(subcommand)]
    pub service: DeleteService,
}

#[derive(Debug, Subcommand)]
pub enum DeleteService {
    /// Delete 1Password credentials (global by default, `--local` for
    /// project-scoped).
    #[command(name = "1pass")]
    OnePass(DeleteArgs),
    /// Delete Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(DeleteArgs),
    /// Delete Google Calendar credentials (global by default,
    /// `--local` for project-scoped).
    Gcal(DeleteArgs),
    /// Delete GitHub credentials (global by default, `--local` for
    /// project-scoped).
    Github(DeleteArgs),
    /// Delete Telegram credentials (global by default, `--local` for
    /// project-scoped).
    Telegram(DeleteArgs),
}

pub async fn run(args: ServiceArgs) -> Result<()> {
    match args.action {
        Action::Create(c) => match c.service {
            CreateService::OnePass(a) => lifecycle::run_create::<OnePassLifecycle>(a).await,
            CreateService::Discord(a) => lifecycle::run_create::<DiscordLifecycle>(a).await,
            CreateService::Gcal(a) => lifecycle::run_create::<GcalLifecycle>(a).await,
            CreateService::Github(a) => lifecycle::run_create::<GithubLifecycle>(a).await,
            CreateService::Telegram(a) => lifecycle::run_create::<TelegramLifecycle>(a).await,
        },
        Action::Enable(a) => match a.service {
            EnableService::OnePass(a) => lifecycle::run_enable::<OnePassLifecycle>(a),
            EnableService::Discord(a) => lifecycle::run_enable::<DiscordLifecycle>(a),
            EnableService::Gcal(a) => lifecycle::run_enable::<GcalLifecycle>(a),
            EnableService::Github(a) => lifecycle::run_enable::<GithubLifecycle>(a),
            EnableService::Telegram(a) => lifecycle::run_enable::<TelegramLifecycle>(a),
        },
        Action::Disable(d) => match d.service {
            DisableService::OnePass(a) => lifecycle::run_disable::<OnePassLifecycle>(a),
            DisableService::Discord(a) => lifecycle::run_disable::<DiscordLifecycle>(a),
            DisableService::Gcal(a) => lifecycle::run_disable::<GcalLifecycle>(a),
            DisableService::Github(a) => lifecycle::run_disable::<GithubLifecycle>(a),
            DisableService::Telegram(a) => lifecycle::run_disable::<TelegramLifecycle>(a),
        },
        Action::List(a) => service_list::run(a),
        Action::Show(s) => match s.service {
            ShowService::OnePass(a) => lifecycle::run_show::<OnePassLifecycle>(a),
            ShowService::Discord(a) => lifecycle::run_show::<DiscordLifecycle>(a),
            ShowService::Gcal(a) => lifecycle::run_show::<GcalLifecycle>(a),
            ShowService::Github(a) => lifecycle::run_show::<GithubLifecycle>(a),
            ShowService::Telegram(a) => lifecycle::run_show::<TelegramLifecycle>(a),
        },
        Action::Status(s) => match s.service.as_deref() {
            None => service_status::run_all(s).await,
            Some("1pass") => {
                lifecycle::run_status::<OnePassLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            Some("discord") => {
                lifecycle::run_status::<DiscordLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            Some("gcal") => {
                lifecycle::run_status::<GcalLifecycle>(lifecycle::StatusArgs { json: s.json }).await
            }
            Some("github") => {
                lifecycle::run_status::<GithubLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            Some("telegram") => {
                lifecycle::run_status::<TelegramLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            // PossibleValuesParser rejects unknown values before we get
            // here, so this arm only fires if a new entry is added to
            // `SERVICES` without a matching match arm.
            Some(other) => Err(crate::error::ZadError::Invalid(format!(
                "unhandled service in status dispatch: `{other}`"
            ))),
        },
        Action::Delete(d) => match d.service {
            DeleteService::OnePass(a) => lifecycle::run_delete::<OnePassLifecycle>(a),
            DeleteService::Discord(a) => lifecycle::run_delete::<DiscordLifecycle>(a),
            DeleteService::Gcal(a) => lifecycle::run_delete::<GcalLifecycle>(a),
            DeleteService::Github(a) => lifecycle::run_delete::<GithubLifecycle>(a),
            DeleteService::Telegram(a) => lifecycle::run_delete::<TelegramLifecycle>(a),
        },
    }
}
