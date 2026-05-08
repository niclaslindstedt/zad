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
    service_discord, service_gcal, service_list, service_onepass, service_slack, service_spotify,
    service_status, service_telegram, service_ymusic,
};
use service_discord::DiscordLifecycle;
use service_gcal::GcalLifecycle;
use service_onepass::OnePassLifecycle;
use service_slack::SlackLifecycle;
use service_spotify::SpotifyLifecycle;
use service_telegram::TelegramLifecycle;
use service_ymusic::YmusicLifecycle;

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
    /// Create Spotify credentials (global by default, `--local` for
    /// project-scoped).
    Spotify(service_spotify::CreateArgs),
    /// Create Slack credentials (global by default, `--local` for
    /// project-scoped).
    Slack(service_slack::CreateArgs),
    /// Create Telegram credentials (global by default, `--local` for
    /// project-scoped).
    Telegram(service_telegram::CreateArgs),
    /// Create YouTube Music credentials (global by default, `--local`
    /// for project-scoped).
    Ymusic(service_ymusic::CreateArgs),
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
    /// Enable the Slack service in the current project.
    Slack(EnableArgs),
    /// Enable the Spotify service in the current project.
    Spotify(EnableArgs),
    /// Enable the Telegram service in the current project.
    Telegram(EnableArgs),
    /// Enable the YouTube Music service in the current project.
    Ymusic(EnableArgs),
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
    /// Disable the Slack service in the current project.
    Slack(DisableArgs),
    /// Disable the Spotify service in the current project.
    Spotify(DisableArgs),
    /// Disable the Telegram service in the current project.
    Telegram(DisableArgs),
    /// Disable the YouTube Music service in the current project.
    Ymusic(DisableArgs),
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
    /// Show the Slack service's effective configuration.
    Slack(ShowArgs),
    /// Show the Spotify service's effective configuration.
    Spotify(ShowArgs),
    /// Show the Telegram service's effective configuration.
    Telegram(ShowArgs),
    /// Show the YouTube Music service's effective configuration.
    Ymusic(ShowArgs),
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
    /// Delete Slack credentials (global by default, `--local` for
    /// project-scoped).
    Slack(DeleteArgs),
    /// Delete Spotify credentials (global by default, `--local` for
    /// project-scoped).
    Spotify(DeleteArgs),
    /// Delete Telegram credentials (global by default, `--local` for
    /// project-scoped).
    Telegram(DeleteArgs),
    /// Delete YouTube Music credentials (global by default, `--local`
    /// for project-scoped).
    Ymusic(DeleteArgs),
}

pub async fn run(args: ServiceArgs) -> Result<()> {
    match args.action {
        Action::Create(c) => match c.service {
            CreateService::OnePass(a) => lifecycle::run_create::<OnePassLifecycle>(a).await,
            CreateService::Discord(a) => lifecycle::run_create::<DiscordLifecycle>(a).await,
            CreateService::Gcal(a) => lifecycle::run_create::<GcalLifecycle>(a).await,
            CreateService::Slack(a) => lifecycle::run_create::<SlackLifecycle>(a).await,
            CreateService::Spotify(a) => lifecycle::run_create::<SpotifyLifecycle>(a).await,
            CreateService::Telegram(a) => lifecycle::run_create::<TelegramLifecycle>(a).await,
            CreateService::Ymusic(a) => lifecycle::run_create::<YmusicLifecycle>(a).await,
        },
        Action::Enable(a) => match a.service {
            EnableService::OnePass(a) => lifecycle::run_enable::<OnePassLifecycle>(a),
            EnableService::Discord(a) => lifecycle::run_enable::<DiscordLifecycle>(a),
            EnableService::Gcal(a) => lifecycle::run_enable::<GcalLifecycle>(a),
            EnableService::Slack(a) => lifecycle::run_enable::<SlackLifecycle>(a),
            EnableService::Spotify(a) => lifecycle::run_enable::<SpotifyLifecycle>(a),
            EnableService::Telegram(a) => lifecycle::run_enable::<TelegramLifecycle>(a),
            EnableService::Ymusic(a) => lifecycle::run_enable::<YmusicLifecycle>(a),
        },
        Action::Disable(d) => match d.service {
            DisableService::OnePass(a) => lifecycle::run_disable::<OnePassLifecycle>(a),
            DisableService::Discord(a) => lifecycle::run_disable::<DiscordLifecycle>(a),
            DisableService::Gcal(a) => lifecycle::run_disable::<GcalLifecycle>(a),
            DisableService::Slack(a) => lifecycle::run_disable::<SlackLifecycle>(a),
            DisableService::Spotify(a) => lifecycle::run_disable::<SpotifyLifecycle>(a),
            DisableService::Telegram(a) => lifecycle::run_disable::<TelegramLifecycle>(a),
            DisableService::Ymusic(a) => lifecycle::run_disable::<YmusicLifecycle>(a),
        },
        Action::List(a) => service_list::run(a),
        Action::Show(s) => match s.service {
            ShowService::OnePass(a) => lifecycle::run_show::<OnePassLifecycle>(a),
            ShowService::Discord(a) => lifecycle::run_show::<DiscordLifecycle>(a),
            ShowService::Gcal(a) => lifecycle::run_show::<GcalLifecycle>(a),
            ShowService::Slack(a) => lifecycle::run_show::<SlackLifecycle>(a),
            ShowService::Spotify(a) => lifecycle::run_show::<SpotifyLifecycle>(a),
            ShowService::Telegram(a) => lifecycle::run_show::<TelegramLifecycle>(a),
            ShowService::Ymusic(a) => lifecycle::run_show::<YmusicLifecycle>(a),
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
            Some("slack") => {
                lifecycle::run_status::<SlackLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            Some("spotify") => {
                lifecycle::run_status::<SpotifyLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            Some("telegram") => {
                lifecycle::run_status::<TelegramLifecycle>(lifecycle::StatusArgs { json: s.json })
                    .await
            }
            Some("ymusic") => {
                lifecycle::run_status::<YmusicLifecycle>(lifecycle::StatusArgs { json: s.json })
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
            DeleteService::Slack(a) => lifecycle::run_delete::<SlackLifecycle>(a),
            DeleteService::Spotify(a) => lifecycle::run_delete::<SpotifyLifecycle>(a),
            DeleteService::Telegram(a) => lifecycle::run_delete::<TelegramLifecycle>(a),
            DeleteService::Ymusic(a) => lifecycle::run_delete::<YmusicLifecycle>(a),
        },
    }
}
