//! Dispatch for `zad status` — aggregate status check across every service.
//!
//! This is the agent-facing entrypoint: one command that pings every
//! service registered in [`crate::service::registry::SERVICES`] and
//! returns a single JSON envelope describing which ones work. Pings
//! run in parallel so adding a third service doesn't linearly inflate
//! startup time.
//!
//! Each row is a [`lifecycle::ServiceStatusOutput`] — the same shape
//! emitted by `zad service status <svc>` — so an agent that already
//! handles the per-service command can consume this output unchanged.

use clap::Args;
use serde::Serialize;

use crate::cli::lifecycle::{self, ServiceStatusOutput};
use crate::cli::{service_discord::DiscordLifecycle, service_telegram::TelegramLifecycle};
use crate::error::Result;

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    /// Recommended for agents — the envelope is stable and lists every
    /// service by name with its ping result.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct AggregateOutput {
    command: &'static str,
    /// True iff every service that has an effective scope pinged OK.
    /// Services with no credentials at all (`effective: null`) don't
    /// affect this value — they're reported but not counted as
    /// failures, since "not configured" isn't the same as "broken".
    ok: bool,
    services: Vec<ServiceStatusOutput>,
}

pub async fn run(args: StatusArgs) -> Result<()> {
    // Independent network calls — fan them out. Two services today;
    // more will slot in without restructuring.
    let (discord, telegram) = tokio::join!(
        lifecycle::status_for::<DiscordLifecycle>(),
        lifecycle::status_for::<TelegramLifecycle>(),
    );
    let services = vec![discord?, telegram?];

    let ok = services
        .iter()
        .filter(|s| s.effective.is_some())
        .all(|s| s.ok);

    if args.json {
        let out = AggregateOutput {
            command: "status",
            ok,
            services,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        print_human(ok, &services);
    }

    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_human(ok: bool, services: &[ServiceStatusOutput]) {
    println!(
        "zad status: {}",
        if ok {
            "all configured services ok"
        } else {
            "one or more services FAILED"
        }
    );
    for svc in services {
        let effective = svc.effective.unwrap_or("(not configured)");
        let state = match svc.effective {
            None => "not configured".to_string(),
            Some(_) if svc.ok => {
                let name = svc
                    .global
                    .check
                    .as_ref()
                    .or(svc.local.check.as_ref())
                    .and_then(|c| c.authenticated_as.as_deref())
                    .unwrap_or("(unknown)");
                format!("ok (authenticated as `{name}`)")
            }
            Some(_) => {
                let err = svc
                    .global
                    .check
                    .as_ref()
                    .or(svc.local.check.as_ref())
                    .and_then(|c| c.error.as_deref())
                    .unwrap_or("(no detail)");
                format!("FAILED ({err})")
            }
        };
        println!("  {:<10} [{effective:<5}]  {state}", svc.service);
    }
}
