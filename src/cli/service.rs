//! Dispatch for `zad service <action> <name>`.
//!
//! Each action is a thin clap enum: one variant per service that
//! routes to the generic `lifecycle::run_*::<T>()` driver with the
//! service's `LifecycleService` impl as the type parameter. Adding a
//! new service means adding one variant to each enum below plus one
//! dispatch arm in `run()` — about 10 lines total.

use clap::{Args, Subcommand};

use crate::cli::lifecycle::{self, DeleteArgs, DisableArgs, EnableArgs, ShowArgs, StatusArgs};
use crate::error::Result;

use super::{service_discord, service_gcal, service_list, service_telegram};
use service_discord::DiscordLifecycle;
use service_gcal::GcalLifecycle;
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
    /// Check whether a service's credentials work by pinging the provider.
    Status(StatusAction),
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
    /// Create Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(service_discord::CreateArgs),
    /// Create Google Calendar credentials (global by default,
    /// `--local` for project-scoped).
    Gcal(service_gcal::CreateArgs),
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
    /// Enable the Discord service in the current project.
    Discord(EnableArgs),
    /// Enable the Google Calendar service in the current project.
    Gcal(EnableArgs),
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
    /// Disable the Discord service in the current project.
    Discord(DisableArgs),
    /// Disable the Google Calendar service in the current project.
    Gcal(DisableArgs),
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
    /// Show the Discord service's effective configuration.
    Discord(ShowArgs),
    /// Show the Google Calendar service's effective configuration.
    Gcal(ShowArgs),
    /// Show the Telegram service's effective configuration.
    Telegram(ShowArgs),
}

#[derive(Debug, Args)]
pub struct StatusAction {
    #[command(subcommand)]
    pub service: StatusService,
}

#[derive(Debug, Subcommand)]
pub enum StatusService {
    /// Check whether Discord credentials work (pings `GET /users/@me`).
    Discord(StatusArgs),
    /// Check whether Google Calendar credentials work (pings the
    /// OpenID userinfo endpoint and `calendarList`).
    Gcal(StatusArgs),
    /// Check whether Telegram credentials work (pings `getMe`).
    Telegram(StatusArgs),
}

#[derive(Debug, Args)]
pub struct DeleteAction {
    #[command(subcommand)]
    pub service: DeleteService,
}

#[derive(Debug, Subcommand)]
pub enum DeleteService {
    /// Delete Discord credentials (global by default, `--local` for
    /// project-scoped).
    Discord(DeleteArgs),
    /// Delete Google Calendar credentials (global by default,
    /// `--local` for project-scoped).
    Gcal(DeleteArgs),
    /// Delete Telegram credentials (global by default, `--local` for
    /// project-scoped).
    Telegram(DeleteArgs),
}

pub async fn run(args: ServiceArgs) -> Result<()> {
    match args.action {
        Action::Create(c) => match c.service {
            CreateService::Discord(a) => lifecycle::run_create::<DiscordLifecycle>(a).await,
            CreateService::Gcal(a) => lifecycle::run_create::<GcalLifecycle>(a).await,
            CreateService::Telegram(a) => lifecycle::run_create::<TelegramLifecycle>(a).await,
        },
        Action::Enable(a) => match a.service {
            EnableService::Discord(a) => lifecycle::run_enable::<DiscordLifecycle>(a),
            EnableService::Gcal(a) => lifecycle::run_enable::<GcalLifecycle>(a),
            EnableService::Telegram(a) => lifecycle::run_enable::<TelegramLifecycle>(a),
        },
        Action::Disable(d) => match d.service {
            DisableService::Discord(a) => lifecycle::run_disable::<DiscordLifecycle>(a),
            DisableService::Gcal(a) => lifecycle::run_disable::<GcalLifecycle>(a),
            DisableService::Telegram(a) => lifecycle::run_disable::<TelegramLifecycle>(a),
        },
        Action::List(a) => service_list::run(a),
        Action::Show(s) => match s.service {
            ShowService::Discord(a) => lifecycle::run_show::<DiscordLifecycle>(a),
            ShowService::Gcal(a) => lifecycle::run_show::<GcalLifecycle>(a),
            ShowService::Telegram(a) => lifecycle::run_show::<TelegramLifecycle>(a),
        },
        Action::Status(s) => match s.service {
            StatusService::Discord(a) => lifecycle::run_status::<DiscordLifecycle>(a).await,
            StatusService::Gcal(a) => lifecycle::run_status::<GcalLifecycle>(a).await,
            StatusService::Telegram(a) => lifecycle::run_status::<TelegramLifecycle>(a).await,
        },
        Action::Delete(d) => match d.service {
            DeleteService::Discord(a) => lifecycle::run_delete::<DiscordLifecycle>(a),
            DeleteService::Gcal(a) => lifecycle::run_delete::<GcalLifecycle>(a),
            DeleteService::Telegram(a) => lifecycle::run_delete::<TelegramLifecycle>(a),
        },
    }
}
