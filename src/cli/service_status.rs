//! Aggregate dispatch for `zad service status` (no `--service` filter).
//!
//! This is the agent-facing entrypoint: one command that pings every
//! service registered in [`crate::service::registry::SERVICES`] and
//! returns a single JSON envelope describing which ones work. Pings
//! run in parallel so adding a third service doesn't linearly inflate
//! startup time.
//!
//! Each row is a [`lifecycle::ServiceStatusOutput`] — the same shape
//! emitted by `zad service status --service <svc>` — so an agent that
//! already handles the single-service form can consume this output
//! unchanged.

use serde::Serialize;

use crate::cli::lifecycle::{self, ServiceStatusOutput};
use crate::cli::service::StatusArgs;
use crate::cli::{
    service_discord::DiscordLifecycle, service_gcal::GcalLifecycle,
    service_onepass::OnePassLifecycle, service_telegram::TelegramLifecycle,
};
use crate::error::Result;

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

pub async fn run_all(args: StatusArgs) -> Result<()> {
    // Independent network calls — fan them out. Order here matches the
    // alphabetical order of `crate::service::registry::SERVICES`;
    // adding a new service means adding one line here and one match
    // arm in `service::run()`.
    let (onepass, discord, gcal, telegram) = tokio::join!(
        lifecycle::status_for::<OnePassLifecycle>(),
        lifecycle::status_for::<DiscordLifecycle>(),
        lifecycle::status_for::<GcalLifecycle>(),
        lifecycle::status_for::<TelegramLifecycle>(),
    );
    let services = vec![onepass?, discord?, gcal?, telegram?];

    let ok = services
        .iter()
        .filter(|s| s.effective.is_some())
        .all(|s| s.ok);

    if args.json {
        let out = AggregateOutput {
            command: "service.status",
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
        "zad service status: {}",
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
