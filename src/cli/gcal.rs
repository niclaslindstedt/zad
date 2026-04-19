//! `zad gcal <verb>` — runtime surface for the Google Calendar
//! service.
//!
//! Wires together:
//! - per-verb clap args (`calendars list/show`, `events list/show/
//!   create/update/delete`, plus the `permissions` and `self`
//!   subgroups every service ships);
//! - credential + scope resolution from the effective config (local
//!   wins over global);
//! - permission gating (time window → calendar → attendees → content
//!   → numeric caps) executed **before** any network call;
//! - `--dry-run` wrapping via the [`GcalTransport`] trait so previews
//!   never hit the keychain.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::lifecycle::leak;
use crate::config::{self, GcalServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::default_dry_run_sink;
use crate::service::gcal::client::{CalendarEntry, Event, EventsListParams, GcalHttp};
use crate::service::gcal::permissions::{self as perms, EffectivePermissions, GcalFunction};
use crate::service::gcal::transport::{DryRunGcalTransport, GcalTransport};

// ---------------------------------------------------------------------------
// top-level args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct GcalArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Calendar directory operations (list / show).
    Calendars(CalendarsArgs),
    /// Event read + write operations.
    Events(EventsArgs),
    /// Inspect or scaffold the permissions policy.
    Permissions(PermissionsArgs),
    /// Configure the `@me` alias that resolves to your own email.
    #[command(name = "self")]
    SelfAction(SelfArgs),
}

// ---------------------------------------------------------------------------
// `zad gcal calendars …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CalendarsArgs {
    #[command(subcommand)]
    pub action: CalendarsAction,
}

#[derive(Debug, Subcommand)]
pub enum CalendarsAction {
    /// List every calendar the authenticated user can see.
    List(CalendarsListArgs),
    /// Show metadata for one calendar.
    Show(CalendarsShowArgs),
}

#[derive(Debug, Args)]
pub struct CalendarsListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CalendarsShowArgs {
    /// Calendar ID, `primary`, or a directory alias.
    pub calendar: String,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad gcal events …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct EventsArgs {
    #[command(subcommand)]
    pub action: EventsAction,
}

#[derive(Debug, Subcommand)]
pub enum EventsAction {
    /// List events on a calendar, optionally filtered by time or
    /// free-text query.
    List(EventsListArgs),
    /// Show one event.
    Show(EventsShowArgs),
    /// Create a new event.
    Create(EventsCreateArgs),
    /// Patch an existing event. `--add-attendee` and
    /// `--add-reminder-minutes` are additive; `--remove-attendee` is
    /// subtractive.
    Update(EventsUpdateArgs),
    /// Delete an event.
    Delete(EventsDeleteArgs),
}

#[derive(Debug, Args)]
pub struct EventsListArgs {
    /// Calendar ID, `primary`, or a directory alias. Defaults to
    /// `default_calendar` from config if set.
    #[arg(long)]
    pub calendar: Option<String>,
    #[arg(long)]
    pub time_min: Option<String>,
    #[arg(long)]
    pub time_max: Option<String>,
    #[arg(long)]
    pub query: Option<String>,
    #[arg(long, default_value_t = 25)]
    pub max: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EventsShowArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub calendar: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EventsCreateArgs {
    #[arg(long)]
    pub calendar: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub location: Option<String>,
    #[arg(long)]
    pub start: Option<String>,
    #[arg(long)]
    pub end: Option<String>,
    /// IANA timezone. Defaults to the calendar's default.
    #[arg(long)]
    pub tz: Option<String>,
    #[arg(long = "attendee")]
    pub attendees: Vec<String>,
    /// Reminder override minutes-before-start (repeatable).
    #[arg(long = "reminder-minutes")]
    pub reminder_minutes: Vec<u32>,
    #[arg(long, value_parser = ["default", "public", "private"])]
    pub visibility: Option<String>,
    #[arg(long, value_parser = ["none", "external", "all"])]
    pub send_updates: Option<String>,
    /// `RRULE:` string (repeatable — emitted as `recurrence` array).
    #[arg(long = "recurrence")]
    pub recurrence: Vec<String>,
    /// Read the full event payload from a file (`-` for stdin).
    /// Merged with the flag-derived fields; the flags win on
    /// conflict.
    #[arg(long)]
    pub from_json: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EventsUpdateArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub calendar: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub location: Option<String>,
    #[arg(long)]
    pub start: Option<String>,
    #[arg(long)]
    pub end: Option<String>,
    #[arg(long)]
    pub tz: Option<String>,
    #[arg(long = "add-attendee")]
    pub add_attendees: Vec<String>,
    #[arg(long = "remove-attendee")]
    pub remove_attendees: Vec<String>,
    #[arg(long = "add-reminder-minutes")]
    pub add_reminder_minutes: Vec<u32>,
    #[arg(long, value_parser = ["default", "public", "private"])]
    pub visibility: Option<String>,
    #[arg(long, value_parser = ["none", "external", "all"])]
    pub send_updates: Option<String>,
    #[arg(long)]
    pub from_json: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EventsDeleteArgs {
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub calendar: Option<String>,
    #[arg(long, value_parser = ["none", "external", "all"])]
    pub send_updates: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad gcal permissions …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum PermissionsAction {
    /// Print the effective policy (both file paths + bodies).
    Show(PermissionsShowArgs),
    /// Print the two candidate file paths, one per line.
    Path(PermissionsPathArgs),
    /// Write a starter policy to the selected scope.
    Init(PermissionsInitArgs),
    /// Dry-run a permissions check without hitting the network.
    Check(PermissionsCheckArgs),
    /// Staged-commit workflow: queue mutations in a `.pending` file and
    /// only sign on `commit`. See `cli::permissions`.
    #[command(flatten)]
    Staging(crate::cli::permissions::StagingAction),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsPathArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsInitArgs {
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsCheckArgs {
    #[arg(long)]
    pub function: String,
    #[arg(long)]
    pub calendar: Option<String>,
    #[arg(long = "attendee")]
    pub attendees: Vec<String>,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub start: Option<String>,
    #[arg(long)]
    pub end: Option<String>,
    /// Total attendee count after the hypothetical write.
    #[arg(long = "attendee-count")]
    pub attendee_count: Option<u32>,
    #[arg(long, value_parser = ["none", "external", "all"])]
    pub send_updates: Option<String>,
    #[arg(long = "reminder-minutes")]
    pub reminder_minutes: Vec<u32>,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad gcal self …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SelfArgs {
    #[command(subcommand)]
    pub action: SelfAction,
}

#[derive(Debug, Subcommand)]
pub enum SelfAction {
    /// Print the currently configured self email.
    Show(SelfShowArgs),
    /// Set the self email (overwrites whatever's stored).
    Set(SelfSetArgs),
    /// Clear the self email.
    Clear(SelfClearArgs),
}

#[derive(Debug, Args)]
pub struct SelfShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfSetArgs {
    #[arg(long)]
    pub email: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfClearArgs {
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

pub async fn run(args: GcalArgs) -> Result<()> {
    match args.action {
        Action::Calendars(a) => match a.action {
            CalendarsAction::List(a) => run_calendars_list(a).await,
            CalendarsAction::Show(a) => run_calendars_show(a).await,
        },
        Action::Events(a) => match a.action {
            EventsAction::List(a) => run_events_list(a).await,
            EventsAction::Show(a) => run_events_show(a).await,
            EventsAction::Create(a) => run_events_create(a).await,
            EventsAction::Update(a) => run_events_update(a).await,
            EventsAction::Delete(a) => run_events_delete(a).await,
        },
        Action::Permissions(a) => run_permissions(a),
        Action::SelfAction(a) => match a.action {
            SelfAction::Show(a) => run_self_show(a),
            SelfAction::Set(a) => run_self_set(a),
            SelfAction::Clear(a) => run_self_clear(a),
        },
    }
}

// ---------------------------------------------------------------------------
// stubs — populated in subsequent parts
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// read-side verbs — calendars list/show, events list/show
// ---------------------------------------------------------------------------

async fn run_calendars_list(args: CalendarsListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(GcalFunction::ListCalendars)?;

    let (transport, _cfg, _config_path) = transport_for(false)?;
    let items = transport.list_calendars().await?;

    // Per-calendar permission check — filters out calendars the
    // policy denies. We filter silently (not as a hard error) so
    // `list` remains useful even when some calendars are restricted.
    let mut filtered: Vec<CalendarEntry> = Vec::with_capacity(items.len());
    for c in items {
        if permissions
            .check_calendar(GcalFunction::ListCalendars, &c.id, &c.id)
            .is_ok()
        {
            filtered.push(c);
        }
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
        return Ok(());
    }
    if filtered.is_empty() {
        println!("No calendars visible (or all filtered by permissions).");
        return Ok(());
    }
    for c in &filtered {
        let primary = if c.primary == Some(true) {
            " (primary)"
        } else {
            ""
        };
        let role = c.access_role.as_deref().unwrap_or("?");
        let tz = c.time_zone.as_deref().unwrap_or("?");
        println!(
            "{:40}  role={role:6}  tz={tz}  {}{primary}",
            c.id, c.summary
        );
    }
    Ok(())
}

async fn run_calendars_show(args: CalendarsShowArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(GcalFunction::GetCalendar)?;
    let (raw, resolved) = (args.calendar.clone(), args.calendar.clone());
    permissions.check_calendar(GcalFunction::GetCalendar, &raw, &resolved)?;

    let (transport, _cfg, _p) = transport_for(false)?;
    let cal = transport.get_calendar(&resolved).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&cal).unwrap());
        return Ok(());
    }
    println!("id       : {}", cal.id);
    println!("summary  : {}", cal.summary);
    if let Some(tz) = &cal.time_zone {
        println!("timezone : {tz}");
    }
    Ok(())
}

async fn run_events_list(args: EventsListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(GcalFunction::ListEvents)?;
    let (cfg, _label, _scope, _path) = effective_config()?;
    let (raw, resolved) =
        resolve_calendar(args.calendar.as_deref(), cfg.default_calendar.as_deref())?;
    permissions.check_calendar(GcalFunction::ListEvents, &raw, &resolved)?;

    let params = EventsListParams {
        time_min: args.time_min,
        time_max: args.time_max,
        query: args.query,
        max_results: Some(args.max),
    };
    let (transport, _cfg2, _p) = transport_for(false)?;
    let events = transport.list_events(&resolved, &params).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&events).unwrap());
        return Ok(());
    }
    if events.is_empty() {
        println!("No events.");
        return Ok(());
    }
    for e in &events {
        let summary = e.summary.as_deref().unwrap_or("(no summary)");
        let when = event_start_string(e);
        println!("{:25}  {}  {}", when, e.id, summary);
    }
    Ok(())
}

async fn run_events_show(args: EventsShowArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(GcalFunction::GetEvent)?;
    let (cfg, _label, _scope, _path) = effective_config()?;
    let (raw, resolved) =
        resolve_calendar(args.calendar.as_deref(), cfg.default_calendar.as_deref())?;
    permissions.check_calendar(GcalFunction::GetEvent, &raw, &resolved)?;

    let (transport, _c, _p) = transport_for(false)?;
    let event = transport.get_event(&resolved, &args.id).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&event).unwrap());
        return Ok(());
    }
    println!("id          : {}", event.id);
    if let Some(s) = &event.summary {
        println!("summary     : {s}");
    }
    if let Some(s) = &event.description {
        println!("description : {s}");
    }
    if let Some(s) = &event.location {
        println!("location    : {s}");
    }
    println!("start       : {}", event_start_string(&event));
    println!("end         : {}", event_end_string(&event));
    if let Some(attendees) = &event.attendees
        && !attendees.is_empty()
    {
        println!("attendees   :");
        for a in attendees {
            let name = a.display_name.as_deref().unwrap_or("");
            let resp = a.response_status.as_deref().unwrap_or("?");
            println!("  - {} {name} ({resp})", a.email);
        }
    }
    if let Some(link) = &event.html_link {
        println!("link        : {link}");
    }
    Ok(())
}

fn event_start_string(e: &Event) -> String {
    match &e.start {
        Some(dt) => dt
            .date_time
            .clone()
            .or_else(|| dt.date.clone())
            .unwrap_or_else(|| "(unknown)".into()),
        None => "(unknown)".into(),
    }
}
fn event_end_string(e: &Event) -> String {
    match &e.end {
        Some(dt) => dt
            .date_time
            .clone()
            .or_else(|| dt.date.clone())
            .unwrap_or_else(|| "(unknown)".into()),
        None => "(unknown)".into(),
    }
}

// ---------------------------------------------------------------------------
// write-side verbs — create / update / delete
//
// Enforcement order, every verb:
//   1. time window
//   2. calendar allow/deny + block_shared_calendars
//   3. attendee allow/deny (for each)
//   4. content rules on summary + description
//   5. send_updates allow/deny (when set)
//   6. reminder minutes hard cap (for each)
//   7. numeric caps (max_future_days, min_notice_minutes, max_attendees)
// Only after every check succeeds does the transport touch the network.
// ---------------------------------------------------------------------------

async fn run_events_create(args: EventsCreateArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    let func = GcalFunction::CreateEvent;
    permissions.check_time(func)?;

    // Config is best-effort under dry-run so previews work pre-create.
    let (cfg, _scope_label, _scope, _path) = if args.dry_run {
        match effective_config() {
            Ok(t) => t,
            Err(_) => (
                GcalServiceCfg {
                    scopes: vec![],
                    default_calendar: None,
                    self_email: None,
                },
                "global",
                Scope::Global,
                PathBuf::new(),
            ),
        }
    } else {
        effective_config()?
    };

    let (raw_cal, resolved_cal) =
        resolve_calendar(args.calendar.as_deref(), cfg.default_calendar.as_deref())?;
    permissions.check_calendar(func, &raw_cal, &resolved_cal)?;

    // Build the event payload. Start from `--from-json` (if any),
    // then layer flag-derived fields on top.
    let mut payload = match &args.from_json {
        Some(p) => read_json_input(p)?,
        None => serde_json::json!({}),
    };

    if let Some(s) = &args.summary {
        payload["summary"] = serde_json::Value::String(s.clone());
    }
    if let Some(s) = &args.description {
        payload["description"] = serde_json::Value::String(s.clone());
    }
    if let Some(s) = &args.location {
        payload["location"] = serde_json::Value::String(s.clone());
    }
    if let Some(s) = &args.visibility {
        payload["visibility"] = serde_json::Value::String(s.clone());
    }
    if !args.recurrence.is_empty() {
        payload["recurrence"] = serde_json::Value::Array(
            args.recurrence
                .iter()
                .map(|r| serde_json::Value::String(r.clone()))
                .collect(),
        );
    }

    // start / end — flag-provided start/end overwrite whatever came
    // from --from-json. We need these to run the numeric-cap check.
    let start_et = match &args.start {
        Some(s) => Some(crate::service::gcal::time::parse_event_time(s)?),
        None => None,
    };
    let end_et = match &args.end {
        Some(s) => Some(crate::service::gcal::time::parse_event_time(s)?),
        None => None,
    };
    if let Some(t) = &start_et {
        payload["start"] = t.to_api_json(args.tz.as_deref());
    }
    if let Some(t) = &end_et {
        payload["end"] = t.to_api_json(args.tz.as_deref());
    }

    // Attendees: build the array (from-json attendees, if any, are
    // preserved; flag-provided ones append).
    let mut attendee_list: Vec<serde_json::Value> = match payload.get("attendees") {
        Some(serde_json::Value::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    for email in &args.attendees {
        let resolved_email = if email.eq_ignore_ascii_case("@me") {
            cfg.self_email.clone().ok_or_else(|| {
                ZadError::Invalid(
                    "`@me` used but self_email is not set. Run `zad gcal self set --email <addr>`."
                        .into(),
                )
            })?
        } else {
            email.clone()
        };
        permissions.check_attendee(func, email, cfg.self_email.as_deref())?;
        permissions.check_attendee(GcalFunction::Invite, email, cfg.self_email.as_deref())?;
        attendee_list.push(serde_json::json!({ "email": resolved_email }));
    }
    if !attendee_list.is_empty() {
        payload["attendees"] = serde_json::Value::Array(attendee_list.clone());
    }

    // Content checks on summary + description.
    if let Some(s) = payload.get("summary").and_then(|v| v.as_str()) {
        permissions.check_body(func, s)?;
    }
    if let Some(s) = payload.get("description").and_then(|v| v.as_str()) {
        permissions.check_body(func, s)?;
    }

    // Reminders — override + minutes.
    if !args.reminder_minutes.is_empty() {
        for m in &args.reminder_minutes {
            permissions.check_reminder_minutes(func, *m)?;
            permissions.check_reminder_minutes(GcalFunction::Remind, *m)?;
        }
        let overrides: Vec<serde_json::Value> = args
            .reminder_minutes
            .iter()
            .map(|m| serde_json::json!({ "method": "popup", "minutes": m }))
            .collect();
        payload["reminders"] = serde_json::json!({
            "useDefault": false,
            "overrides": overrides,
        });
    }

    if let Some(su) = args.send_updates.as_deref() {
        permissions.check_send_updates(func, su)?;
    }

    // block_shared_calendars — re-use the live transport to fetch the
    // calendar's `accessRole` unless we're in dry-run mode.
    if permissions.block_shared_calendars(func).is_some() && !args.dry_run {
        enforce_owner_only(&resolved_cal, func, &permissions).await?;
    }

    // Numeric caps.
    let (days_future, mins_future) = match &start_et {
        Some(t) => (
            crate::service::gcal::time::days_from_now(t),
            crate::service::gcal::time::minutes_from_now(t),
        ),
        None => (None, None),
    };
    let attendee_count = if attendee_list.is_empty() {
        None
    } else {
        Some(attendee_list.len() as u32)
    };
    permissions.check_event_caps(func, days_future, mins_future, attendee_count)?;

    // All checks passed; hit the network (or the dry-run sink).
    let (transport, _cfg2, _p) = transport_for(args.dry_run)?;
    let event = transport
        .create_event(&resolved_cal, &payload, args.send_updates.as_deref())
        .await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&event).unwrap());
    } else {
        println!("created event {}", event.id);
        if let Some(link) = &event.html_link {
            println!("  {link}");
        }
    }
    Ok(())
}

async fn run_events_update(args: EventsUpdateArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    let func = GcalFunction::UpdateEvent;
    permissions.check_time(func)?;

    let (cfg, _scope_label, _scope, _path) = if args.dry_run {
        match effective_config() {
            Ok(t) => t,
            Err(_) => (
                GcalServiceCfg {
                    scopes: vec![],
                    default_calendar: None,
                    self_email: None,
                },
                "global",
                Scope::Global,
                PathBuf::new(),
            ),
        }
    } else {
        effective_config()?
    };
    let (raw_cal, resolved_cal) =
        resolve_calendar(args.calendar.as_deref(), cfg.default_calendar.as_deref())?;
    permissions.check_calendar(func, &raw_cal, &resolved_cal)?;

    let mut patch = match &args.from_json {
        Some(p) => read_json_input(p)?,
        None => serde_json::json!({}),
    };
    if let Some(s) = &args.summary {
        patch["summary"] = serde_json::Value::String(s.clone());
    }
    if let Some(s) = &args.description {
        patch["description"] = serde_json::Value::String(s.clone());
    }
    if let Some(s) = &args.location {
        patch["location"] = serde_json::Value::String(s.clone());
    }
    if let Some(s) = &args.visibility {
        patch["visibility"] = serde_json::Value::String(s.clone());
    }

    let start_et = match &args.start {
        Some(s) => Some(crate::service::gcal::time::parse_event_time(s)?),
        None => None,
    };
    let end_et = match &args.end {
        Some(s) => Some(crate::service::gcal::time::parse_event_time(s)?),
        None => None,
    };
    if let Some(t) = &start_et {
        patch["start"] = t.to_api_json(args.tz.as_deref());
    }
    if let Some(t) = &end_et {
        patch["end"] = t.to_api_json(args.tz.as_deref());
    }

    // For attendee adds/removes we need the current attendee list
    // from the server (unless dry-run — then we work with just the
    // add delta).
    let mut current_attendees: Vec<serde_json::Value> = match patch.get("attendees") {
        Some(serde_json::Value::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    };
    if !args.dry_run && (!args.add_attendees.is_empty() || !args.remove_attendees.is_empty()) {
        let (transport, _c, _p) = transport_for(false)?;
        let ev = transport.get_event(&resolved_cal, &args.id).await?;
        if let Some(list) = ev.attendees {
            current_attendees = list
                .into_iter()
                .map(|a| serde_json::json!({ "email": a.email }))
                .collect();
        }
    }
    for email in &args.add_attendees {
        let resolved_email = if email.eq_ignore_ascii_case("@me") {
            cfg.self_email.clone().ok_or_else(|| {
                ZadError::Invalid(
                    "`@me` used but self_email is not set. Run `zad gcal self set --email <addr>`."
                        .into(),
                )
            })?
        } else {
            email.clone()
        };
        permissions.check_attendee(func, email, cfg.self_email.as_deref())?;
        permissions.check_attendee(GcalFunction::Invite, email, cfg.self_email.as_deref())?;
        // Avoid duplicates.
        if !current_attendees
            .iter()
            .any(|v| v.get("email").and_then(|e| e.as_str()) == Some(resolved_email.as_str()))
        {
            current_attendees.push(serde_json::json!({ "email": resolved_email }));
        }
    }
    for email in &args.remove_attendees {
        current_attendees
            .retain(|v| v.get("email").and_then(|e| e.as_str()) != Some(email.as_str()));
    }
    if !args.add_attendees.is_empty() || !args.remove_attendees.is_empty() {
        patch["attendees"] = serde_json::Value::Array(current_attendees.clone());
    }

    if let Some(s) = patch.get("summary").and_then(|v| v.as_str()) {
        permissions.check_body(func, s)?;
    }
    if let Some(s) = patch.get("description").and_then(|v| v.as_str()) {
        permissions.check_body(func, s)?;
    }

    if !args.add_reminder_minutes.is_empty() {
        for m in &args.add_reminder_minutes {
            permissions.check_reminder_minutes(func, *m)?;
            permissions.check_reminder_minutes(GcalFunction::Remind, *m)?;
        }
        let overrides: Vec<serde_json::Value> = args
            .add_reminder_minutes
            .iter()
            .map(|m| serde_json::json!({ "method": "popup", "minutes": m }))
            .collect();
        patch["reminders"] = serde_json::json!({
            "useDefault": false,
            "overrides": overrides,
        });
    }

    if let Some(su) = args.send_updates.as_deref() {
        permissions.check_send_updates(func, su)?;
    }

    if permissions.block_shared_calendars(func).is_some() && !args.dry_run {
        enforce_owner_only(&resolved_cal, func, &permissions).await?;
    }

    let (days_future, mins_future) = match &start_et {
        Some(t) => (
            crate::service::gcal::time::days_from_now(t),
            crate::service::gcal::time::minutes_from_now(t),
        ),
        None => (None, None),
    };
    let attendee_count = if current_attendees.is_empty() {
        None
    } else {
        Some(current_attendees.len() as u32)
    };
    permissions.check_event_caps(func, days_future, mins_future, attendee_count)?;

    let (transport, _c, _p) = transport_for(args.dry_run)?;
    let event = transport
        .update_event(
            &resolved_cal,
            &args.id,
            &patch,
            args.send_updates.as_deref(),
        )
        .await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&event).unwrap());
    } else {
        println!("updated event {}", event.id);
    }
    Ok(())
}

async fn run_events_delete(args: EventsDeleteArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    let func = GcalFunction::DeleteEvent;
    permissions.check_time(func)?;

    let (cfg, _scope_label, _scope, _path) = if args.dry_run {
        match effective_config() {
            Ok(t) => t,
            Err(_) => (
                GcalServiceCfg {
                    scopes: vec![],
                    default_calendar: None,
                    self_email: None,
                },
                "global",
                Scope::Global,
                PathBuf::new(),
            ),
        }
    } else {
        effective_config()?
    };
    let (raw_cal, resolved_cal) =
        resolve_calendar(args.calendar.as_deref(), cfg.default_calendar.as_deref())?;
    permissions.check_calendar(func, &raw_cal, &resolved_cal)?;

    if let Some(su) = args.send_updates.as_deref() {
        permissions.check_send_updates(func, su)?;
    }

    if permissions.block_shared_calendars(func).is_some() && !args.dry_run {
        enforce_owner_only(&resolved_cal, func, &permissions).await?;
    }

    let (transport, _c, _p) = transport_for(args.dry_run)?;
    transport
        .delete_event(&resolved_cal, &args.id, args.send_updates.as_deref())
        .await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        println!(
            r#"{{"command":"gcal.events.delete","calendar":"{resolved_cal}","event_id":"{}"}}"#,
            args.id
        );
    } else {
        println!("deleted event {}", args.id);
    }
    Ok(())
}

/// Fetch the calendar's `accessRole` once and fail the verb if it
/// isn't `owner`. Honours `block_shared_calendars = true`.
async fn enforce_owner_only(
    calendar_id: &str,
    func: GcalFunction,
    permissions: &EffectivePermissions,
) -> Result<()> {
    // The cheapest way to learn `accessRole` is a single calendarList
    // lookup. We read every entry because the Calendar API's
    // `calendarList/get` endpoint is keyed by ID but returns the same
    // `accessRole` field.
    let (transport, _c, _p) = transport_for(false)?;
    let entries = transport.list_calendars().await?;
    let role = entries
        .iter()
        .find(|c| c.id == calendar_id)
        .and_then(|c| c.access_role.clone());
    match role.as_deref() {
        Some("owner") => Ok(()),
        other => {
            let path = permissions.block_shared_calendars(func).unwrap_or_default();
            Err(ZadError::PermissionDenied {
                function: func.name(),
                reason: format!(
                    "calendar `{calendar_id}` has accessRole `{}`, not `owner`; block_shared_calendars = true",
                    other.unwrap_or("unknown")
                ),
                config_path: path,
            })
        }
    }
}

/// Read a JSON document from `path` (or stdin when `path == "-"`).
fn read_json_input(path: &str) -> Result<serde_json::Value> {
    let body = if path == "-" {
        let mut buf = String::new();
        use std::io::Read;
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| ZadError::Io {
                path: PathBuf::from("<stdin>"),
                source: e,
            })?;
        buf
    } else {
        std::fs::read_to_string(path).map_err(|e| ZadError::Io {
            path: PathBuf::from(path),
            source: e,
        })?
    };
    serde_json::from_str(&body)
        .map_err(|e| ZadError::Invalid(format!("failed to parse JSON from `{path}`: {e}")))
}

// ---------------------------------------------------------------------------
// permissions subgroup — show / path / init / check
// ---------------------------------------------------------------------------

fn run_permissions(args: PermissionsArgs) -> Result<()> {
    match args.action {
        None => run_permissions_show(PermissionsShowArgs { json: args.json }),
        Some(PermissionsAction::Show(a)) => run_permissions_show(a),
        Some(PermissionsAction::Path(a)) => run_permissions_path(a),
        Some(PermissionsAction::Init(a)) => run_permissions_init(a),
        Some(PermissionsAction::Check(a)) => run_permissions_check(a),
        Some(PermissionsAction::Staging(a)) => {
            crate::cli::permissions::run::<perms::PermissionsService>(a)
        }
    }
}

#[derive(Debug, Serialize)]
struct PermissionsScopeOut {
    path: String,
    present: bool,
}

#[derive(Debug, Serialize)]
struct PermissionsShowOut {
    command: &'static str,
    global: PermissionsScopeOut,
    local: PermissionsScopeOut,
}

fn run_permissions_show(args: PermissionsShowArgs) -> Result<()> {
    let global_path = perms::global_path()?;
    let local_path = perms::local_path_current()?;
    // Force compile so syntax errors surface immediately.
    let _ = perms::load_effective()?;

    if args.json {
        let out = PermissionsShowOut {
            command: "gcal.permissions.show",
            global: PermissionsScopeOut {
                path: global_path.display().to_string(),
                present: global_path.exists(),
            },
            local: PermissionsScopeOut {
                path: local_path.display().to_string(),
                present: local_path.exists(),
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("Google Calendar permissions");
    print_scope_block("global", &global_path);
    print_scope_block("local", &local_path);
    Ok(())
}

fn print_scope_block(label: &str, path: &Path) {
    println!();
    println!("  [{label}] {}", path.display());
    if !path.exists() {
        println!("    status : not present (no restrictions from this scope)");
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(body) => {
            for line in body.lines() {
                println!("    {line}");
            }
        }
        Err(e) => println!("    status : read error — {e}"),
    }
}

#[derive(Debug, Serialize)]
struct PermissionsPathOut {
    command: &'static str,
    global: String,
    local: String,
}

fn run_permissions_path(args: PermissionsPathArgs) -> Result<()> {
    let global_path = perms::global_path()?;
    let local_path = perms::local_path_current()?;
    if args.json {
        let out = PermissionsPathOut {
            command: "gcal.permissions.path",
            global: global_path.display().to_string(),
            local: local_path.display().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("{}", global_path.display());
    println!("{}", local_path.display());
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsInitOut {
    command: &'static str,
    scope: &'static str,
    path: String,
    written: bool,
}

fn run_permissions_init(args: PermissionsInitArgs) -> Result<()> {
    let (path, scope_label): (PathBuf, &'static str) = if args.local {
        (perms::local_path_current()?, "local")
    } else {
        (perms::global_path()?, "global")
    };
    if path.exists() && !args.force {
        return Err(ZadError::Invalid(format!(
            "{} already exists — pass --force to overwrite",
            path.display()
        )));
    }
    let key = crate::permissions::signing::load_or_create_from_keychain()?;
    crate::permissions::signing::write_public_key_cache(&key)?;
    perms::save_file(&path, &perms::starter_template(), &key)?;
    if args.json {
        let out = PermissionsInitOut {
            command: "gcal.permissions.init",
            scope: scope_label,
            path: path.display().to_string(),
            written: true,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!(
        "Wrote gcal permissions starter policy to {} ({scope_label}).",
        path.display()
    );
    println!("Signed with key {}.", key.fingerprint());
    println!("Edit to narrow further; re-run `zad gcal permissions show` to inspect.");
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsCheckOut {
    command: &'static str,
    function: String,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_path: Option<String>,
}

fn run_permissions_check(args: PermissionsCheckArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    let func = GcalFunction::parse(&args.function)?;
    match check_hypothetical(&permissions, func, &args) {
        Ok(()) => {
            if args.json {
                let out = PermissionsCheckOut {
                    command: "gcal.permissions.check",
                    function: args.function,
                    allowed: true,
                    reason: None,
                    config_path: None,
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                println!("allowed");
            }
            Ok(())
        }
        Err(ZadError::PermissionDenied {
            function,
            reason,
            config_path,
        }) => {
            if args.json {
                let out = PermissionsCheckOut {
                    command: "gcal.permissions.check",
                    function: function.to_string(),
                    allowed: false,
                    reason: Some(reason.clone()),
                    config_path: Some(config_path.display().to_string()),
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                println!("denied: {reason}");
                println!("  edit: {}", config_path.display());
            }
            std::process::exit(1);
        }
        Err(other) => Err(other),
    }
}

/// Body of the check: dry-run every applicable permission predicate
/// for `func` against `args`. Mirrors the enforcement sequence the
/// write-side verbs use in Part 3.
fn check_hypothetical(
    permissions: &EffectivePermissions,
    func: GcalFunction,
    args: &PermissionsCheckArgs,
) -> Result<()> {
    permissions.check_time(func)?;

    if let Some(cal) = args.calendar.as_deref() {
        let resolved = cal.strip_prefix('@').unwrap_or(cal);
        permissions.check_calendar(func, cal, resolved)?;
    }

    // Attendee checks — use `self_email` if the effective config has
    // one (so `@me` resolves against policy).
    let self_email = effective_config()
        .ok()
        .and_then(|(c, _, _, _)| c.self_email);
    for a in &args.attendees {
        permissions.check_attendee(func, a, self_email.as_deref())?;
    }

    if let Some(b) = args.body.as_deref() {
        permissions.check_body(func, b)?;
    }

    if let Some(su) = args.send_updates.as_deref() {
        permissions.check_send_updates(func, su)?;
    }

    for m in &args.reminder_minutes {
        permissions.check_reminder_minutes(func, *m)?;
    }

    // Numeric caps: if caller gave both start and end we compute
    // them; otherwise leave the caps check unconstrained.
    let (days_future, mins_future) = match args.start.as_deref() {
        Some(s) => {
            let t = crate::service::gcal::time::parse_event_time(s)?;
            (
                crate::service::gcal::time::days_from_now(&t),
                crate::service::gcal::time::minutes_from_now(&t),
            )
        }
        None => (None, None),
    };
    permissions.check_event_caps(func, days_future, mins_future, args.attendee_count)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// self subgroup
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct SelfShowOut {
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    scope: &'static str,
    config_path: String,
}

fn run_self_show(args: SelfShowArgs) -> Result<()> {
    let (cfg, scope_label, _scope, path) = effective_config()?;
    if args.json {
        let out = SelfShowOut {
            command: "gcal.self.show",
            email: cfg.self_email.clone(),
            scope: scope_label,
            config_path: path.display().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    match cfg.self_email {
        Some(e) => println!("self email: {e} ({scope_label})"),
        None => println!("self email: (unset; use `zad gcal self set --email <addr>`)"),
    }
    Ok(())
}

fn run_self_set(args: SelfSetArgs) -> Result<()> {
    let (mut cfg, _label, _scope, path) = effective_config()?;
    cfg.self_email = Some(args.email.clone());
    config::save_flat(&path, &cfg)?;
    if args.json {
        println!(
            r#"{{"command":"gcal.self.set","email":"{}","config_path":"{}"}}"#,
            args.email,
            path.display()
        );
        return Ok(());
    }
    println!("self email set to {} in {}", args.email, path.display());
    Ok(())
}

fn run_self_clear(args: SelfClearArgs) -> Result<()> {
    let (mut cfg, _label, _scope, path) = effective_config()?;
    cfg.self_email = None;
    config::save_flat(&path, &cfg)?;
    if args.json {
        println!(
            r#"{{"command":"gcal.self.clear","config_path":"{}"}}"#,
            path.display()
        );
        return Ok(());
    }
    println!("self email cleared in {}", path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// shared helpers (used by every verb in parts 2/3)
// ---------------------------------------------------------------------------

/// Load the effective `GcalServiceCfg` (local wins over global) plus
/// the scope label for error messages and the keychain scope to read
/// secrets from.
pub(crate) fn effective_config() -> Result<(GcalServiceCfg, &'static str, Scope<'static>, PathBuf)>
{
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "gcal")?;
    let global_path = config::path::global_service_config_path("gcal")?;

    let project_cfg = config::load()?;
    if !project_cfg.has_service("gcal") {
        return Err(ZadError::Invalid(format!(
            "gcal is not enabled for this project ({}). Run `zad service enable gcal` first.",
            config::path::project_config_path()?.display()
        )));
    }

    if let Some(cfg) = config::load_flat::<GcalServiceCfg>(&local_path)? {
        let slug_leaked = leak(slug);
        return Ok((cfg, "local", Scope::Project(slug_leaked), local_path));
    }
    if let Some(cfg) = config::load_flat::<GcalServiceCfg>(&global_path)? {
        return Ok((cfg, "global", Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no gcal credentials found.\n  looked in:\n    {}\n    {}\n  Run `zad service create gcal`.",
        local_path.display(),
        global_path.display()
    )))
}

/// Factory that returns either a live [`GcalHttp`]-backed transport
/// or a dry-run preview transport. Honours `dry_run`: when true, the
/// keychain is never read, so previews work even before credentials
/// are registered. When false, credentials are loaded and the scope
/// set is pinned to whatever the config declared.
pub(crate) fn transport_for(
    dry_run: bool,
) -> Result<(Box<dyn GcalTransport>, GcalServiceCfg, PathBuf)> {
    if dry_run {
        // Best-effort config load for dry-run: if we have one, use it
        // for default_calendar / self_email; otherwise return a
        // synthetic stub so the preview path works uncredentialed.
        let cfg = effective_config()
            .map(|(c, _, _, p)| (c, p))
            .unwrap_or_else(|_| {
                (
                    GcalServiceCfg {
                        scopes: vec![],
                        default_calendar: None,
                        self_email: None,
                    },
                    PathBuf::new(),
                )
            });
        let transport: Box<dyn GcalTransport> =
            Box::new(DryRunGcalTransport::new(default_dry_run_sink()));
        return Ok((transport, cfg.0, cfg.1));
    }

    let (cfg, _scope_label, scope, path) = effective_config()?;
    let client_id = secrets::load(&secrets::account("gcal", "client-id", scope.clone()))?.ok_or(
        ZadError::Service {
            name: "gcal",
            message: "client-id missing from keychain; re-run `zad service create gcal`".into(),
        },
    )?;
    let client_secret = secrets::load(&secrets::account("gcal", "client-secret", scope.clone()))?
        .ok_or(ZadError::Service {
        name: "gcal",
        message: "client-secret missing from keychain; re-run `zad service create gcal`".into(),
    })?;
    let refresh_token =
        secrets::load(&secrets::account("gcal", "refresh", scope))?.ok_or(ZadError::Service {
            name: "gcal",
            message: "refresh token missing from keychain; re-run `zad service create gcal`".into(),
        })?;
    let scope_set: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    let http = GcalHttp::new(
        client_id,
        client_secret,
        refresh_token,
        scope_set,
        path.clone(),
    );
    Ok((Box::new(http), cfg, path))
}

/// Resolve `--calendar <raw>` against `default_calendar` fallback.
/// Returns `(raw_input, resolved_id)` — the two aliases every
/// permission check wants.
pub(crate) fn resolve_calendar(
    flag: Option<&str>,
    default: Option<&str>,
) -> Result<(String, String)> {
    let raw = match flag {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => default.map(str::to_string).ok_or_else(|| {
            ZadError::Invalid(
                "no calendar specified and no `default_calendar` set — pass --calendar <id>".into(),
            )
        })?,
    };
    // Google calendar IDs are already resolved. We just strip an
    // optional leading `@` for ergonomic pastes like `@primary`.
    let resolved = raw.strip_prefix('@').unwrap_or(&raw).to_string();
    Ok((raw, resolved))
}
