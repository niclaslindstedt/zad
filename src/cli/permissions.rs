//! Shared CLI plumbing for the staged-commit permissions workflow.
//!
//! Each service's clap tree pulls in [`StagingAction`] as an enum
//! variant and forwards matched subcommands to the generic dispatchers
//! here. The per-service CLI stays tiny — it knows its service type
//! ([`PermissionsService`] impl) and lets the shared code handle
//! mutation parsing, staging, signing, and JSON formatting.

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;

use crate::error::{Result, ZadError};
use crate::permissions::mutation::{ListKind, Mutation};
use crate::permissions::service::{PermissionsService, global_path, local_path_current};
use crate::permissions::signing;
use crate::permissions::staging;

// ---------------------------------------------------------------------------
// clap types
// ---------------------------------------------------------------------------

/// Staging-workflow subcommands common to every permissions-bearing
/// service. Each service embeds this under its `permissions` subgroup.
#[derive(Debug, Subcommand)]
pub enum StagingAction {
    /// Print whether a pending policy exists at each scope.
    Status(ScopeArgs),
    /// Show the unified diff between live and pending (if any).
    Diff(ScopeArgs),
    /// Discard the pending policy without touching the live file.
    Discard(ScopeArgs),
    /// Sign the pending policy with the keychain-held signing key and
    /// atomically replace the live file.
    Commit(ScopeArgs),
    /// Re-sign the live file in place. Use after a hand edit.
    Sign(ScopeArgs),

    /// Queue an allow/deny pattern change. Writes to the pending file.
    #[command(alias = "add-pattern")]
    Add(PatternMutationArgs),
    /// Queue an allow/deny pattern removal.
    #[command(alias = "remove-pattern")]
    Remove(PatternMutationArgs),

    /// Queue a content `deny_words` / `deny_patterns` / `max_length`
    /// change.
    Content(ContentMutationArgs),
    /// Queue a time-window change.
    Time(TimeMutationArgs),
}

#[derive(Debug, Args)]
pub struct ScopeArgs {
    /// Operate on the project-local file instead of the global one.
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PatternMutationArgs {
    /// Function block to edit. Omit to edit the top-level defaults
    /// (only valid for services that expose top-level target lists).
    #[arg(long)]
    pub function: Option<String>,

    /// Target kind to edit. Must be one of the service's known target
    /// kinds (`channel`, `user`, `guild`, `chat`, `calendar`,
    /// `attendee`, `vault`, `item`, `tag`, `category`, `field`).
    #[arg(long)]
    pub target: String,

    /// Which list to edit.
    #[arg(long, value_enum)]
    pub list: CliListKind,

    /// Pattern value (exact match, glob, `re:<regex>`, or numeric ID).
    pub value: String,

    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum CliListKind {
    Allow,
    Deny,
}

impl From<CliListKind> for ListKind {
    fn from(v: CliListKind) -> ListKind {
        match v {
            CliListKind::Allow => ListKind::Allow,
            CliListKind::Deny => ListKind::Deny,
        }
    }
}

#[derive(Debug, Args)]
pub struct ContentMutationArgs {
    /// Scope the edit to one function's block. Omit for the top-level
    /// defaults.
    #[arg(long)]
    pub function: Option<String>,

    #[command(subcommand)]
    pub action: ContentAction,

    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum ContentAction {
    /// Append a case-insensitive deny-word.
    AddDenyWord { word: String },
    /// Remove a deny-word.
    RemoveDenyWord { word: String },
    /// Append a deny regex.
    AddDenyRegex { pattern: String },
    /// Remove a deny regex.
    RemoveDenyRegex { pattern: String },
    /// Set the `max_length` codepoint cap. Pass `--clear` to remove.
    SetMaxLength {
        #[arg(long, conflicts_with = "clear")]
        value: Option<u32>,
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Debug, Args)]
pub struct TimeMutationArgs {
    #[arg(long)]
    pub function: Option<String>,

    #[command(subcommand)]
    pub action: TimeAction,

    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum TimeAction {
    /// Replace the weekday allow-list. Comma-separated: `mon,tue,wed`.
    SetDays {
        #[arg(long, value_delimiter = ',')]
        days: Vec<String>,
    },
    /// Replace the HH:MM-HH:MM window list. Comma-separated.
    SetWindows {
        #[arg(long, value_delimiter = ',')]
        windows: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

/// Entry point a service's CLI module calls after matching the shared
/// subcommand.
pub fn run<S: PermissionsService>(action: StagingAction) -> Result<()> {
    match action {
        StagingAction::Status(a) => run_status::<S>(a),
        StagingAction::Diff(a) => run_diff::<S>(a),
        StagingAction::Discard(a) => run_discard::<S>(a),
        StagingAction::Commit(a) => run_commit::<S>(a),
        StagingAction::Sign(a) => run_sign::<S>(a),
        StagingAction::Add(a) => run_pattern_mutation::<S>(a, /* add */ true),
        StagingAction::Remove(a) => run_pattern_mutation::<S>(a, /* add */ false),
        StagingAction::Content(a) => run_content_mutation::<S>(a),
        StagingAction::Time(a) => run_time_mutation::<S>(a),
    }
}

fn scope_path<S: PermissionsService>(local: bool) -> Result<std::path::PathBuf> {
    if local {
        local_path_current::<S>()
    } else {
        global_path::<S>()
    }
}

#[derive(Debug, Serialize)]
struct StatusOut {
    command: String,
    scope: &'static str,
    live_path: String,
    live_exists: bool,
    pending_path: String,
    pending_exists: bool,
}

fn run_status<S: PermissionsService>(args: ScopeArgs) -> Result<()> {
    let scope = if args.local { "local" } else { "global" };
    let live = scope_path::<S>(args.local)?;
    let pending = staging::pending_path_for(&live);
    let st = staging::status(&live);
    if args.json {
        let out = StatusOut {
            command: format!("{}.permissions.status", S::NAME),
            scope,
            live_path: live.display().to_string(),
            live_exists: st.live_exists,
            pending_path: pending.display().to_string(),
            pending_exists: st.pending_exists,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("# {} permissions ({scope})", S::NAME);
        println!(
            "  live    : {} ({})",
            live.display(),
            if st.live_exists { "present" } else { "absent" }
        );
        println!(
            "  pending : {} ({})",
            pending.display(),
            if st.pending_exists {
                "present"
            } else {
                "absent"
            }
        );
    }
    Ok(())
}

fn run_diff<S: PermissionsService>(args: ScopeArgs) -> Result<()> {
    let live = scope_path::<S>(args.local)?;
    match staging::diff(&live)? {
        Some(body) if body.is_empty() => {
            println!("# no effective change (pending matches live)");
        }
        Some(body) => {
            print!("{body}");
        }
        None => {
            if args.json {
                println!(
                    "{{\"command\": \"{}.permissions.diff\", \"pending\": false}}",
                    S::NAME
                );
            } else {
                println!("# no pending changes at {}", live.display());
            }
        }
    }
    Ok(())
}

fn run_discard<S: PermissionsService>(args: ScopeArgs) -> Result<()> {
    let live = scope_path::<S>(args.local)?;
    let removed = staging::discard(&live)?;
    if args.json {
        println!(
            "{{\"command\": \"{}.permissions.discard\", \"removed\": {removed}}}",
            S::NAME
        );
    } else if removed {
        println!(
            "Discarded pending changes at {}",
            staging::pending_path_for(&live).display()
        );
    } else {
        println!("No pending changes to discard.");
    }
    Ok(())
}

fn run_commit<S: PermissionsService>(args: ScopeArgs) -> Result<()> {
    let live = scope_path::<S>(args.local)?;
    let key = signing::load_from_keychain()?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "no signing key in keychain; run `zad {} permissions init` to generate one",
            S::NAME
        ))
    })?;
    staging::commit::<S>(&live, &key)?;
    if args.json {
        println!(
            "{{\"command\": \"{}.permissions.commit\", \"signed_with\": \"{}\"}}",
            S::NAME,
            key.fingerprint()
        );
    } else {
        println!(
            "Committed and signed: {} (key {}).",
            live.display(),
            key.fingerprint()
        );
    }
    Ok(())
}

fn run_sign<S: PermissionsService>(args: ScopeArgs) -> Result<()> {
    let live = scope_path::<S>(args.local)?;
    let key = signing::load_or_create_from_keychain()?;
    signing::write_public_key_cache(&key)?;
    staging::sign_in_place::<S>(&live, &key)?;
    if args.json {
        println!(
            "{{\"command\": \"{}.permissions.sign\", \"signed_with\": \"{}\"}}",
            S::NAME,
            key.fingerprint()
        );
    } else {
        println!("Re-signed: {} (key {}).", live.display(), key.fingerprint());
    }
    Ok(())
}

fn run_pattern_mutation<S: PermissionsService>(args: PatternMutationArgs, add: bool) -> Result<()> {
    validate_function::<S>(args.function.as_deref())?;
    validate_target::<S>(&args.target)?;
    let mutation = if add {
        Mutation::AddPattern {
            function: args.function,
            target: args.target,
            list: args.list.into(),
            value: args.value,
        }
    } else {
        Mutation::RemovePattern {
            function: args.function,
            target: args.target,
            list: args.list.into(),
            value: args.value,
        }
    };
    queue_and_report::<S>(&mutation, args.local, args.json)
}

fn run_content_mutation<S: PermissionsService>(args: ContentMutationArgs) -> Result<()> {
    validate_function::<S>(args.function.as_deref())?;
    let mutation = match args.action {
        ContentAction::AddDenyWord { word } => Mutation::AddDenyWord {
            function: args.function,
            word,
        },
        ContentAction::RemoveDenyWord { word } => Mutation::RemoveDenyWord {
            function: args.function,
            word,
        },
        ContentAction::AddDenyRegex { pattern } => Mutation::AddDenyRegex {
            function: args.function,
            pattern,
        },
        ContentAction::RemoveDenyRegex { pattern } => Mutation::RemoveDenyRegex {
            function: args.function,
            pattern,
        },
        ContentAction::SetMaxLength { value, clear } => {
            if clear && value.is_some() {
                return Err(ZadError::Invalid(
                    "pass --value or --clear, not both".into(),
                ));
            }
            Mutation::SetMaxLength {
                function: args.function,
                value: if clear { None } else { value },
            }
        }
    };
    queue_and_report::<S>(&mutation, args.local, args.json)
}

fn run_time_mutation<S: PermissionsService>(args: TimeMutationArgs) -> Result<()> {
    validate_function::<S>(args.function.as_deref())?;
    let mutation = match args.action {
        TimeAction::SetDays { days } => Mutation::SetTimeDays {
            function: args.function,
            days,
        },
        TimeAction::SetWindows { windows } => Mutation::SetTimeWindows {
            function: args.function,
            windows,
        },
    };
    queue_and_report::<S>(&mutation, args.local, args.json)
}

fn queue_and_report<S: PermissionsService>(
    mutation: &Mutation,
    local: bool,
    json: bool,
) -> Result<()> {
    let live = scope_path::<S>(local)?;
    staging::mutate_pending::<S>(&live, mutation)?;
    let pending = staging::pending_path_for(&live);
    if json {
        println!(
            "{{\"command\": \"{}.permissions.queue\", \"mutation\": {}, \"pending\": \"{}\"}}",
            S::NAME,
            serde_json::to_string(mutation).unwrap(),
            pending.display()
        );
    } else {
        println!("Queued: {}", mutation.summary());
        println!("  pending: {}", pending.display());
        println!(
            "  review with `zad {} permissions diff{}`, commit with \
             `zad {} permissions commit{}`.",
            S::NAME,
            if local { " --local" } else { "" },
            S::NAME,
            if local { " --local" } else { "" }
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// validation helpers
// ---------------------------------------------------------------------------

fn validate_function<S: PermissionsService>(function: Option<&str>) -> Result<()> {
    let Some(name) = function else {
        return Ok(());
    };
    if S::all_functions().contains(&name) {
        return Ok(());
    }
    Err(ZadError::Invalid(format!(
        "{}: unknown function `{name}`; expected one of {}",
        S::NAME,
        S::all_functions().join(", ")
    )))
}

fn validate_target<S: PermissionsService>(target: &str) -> Result<()> {
    if S::target_kinds().contains(&target) {
        return Ok(());
    }
    Err(ZadError::Invalid(format!(
        "{}: unknown target `{target}`; expected one of {}",
        S::NAME,
        S::target_kinds().join(", ")
    )))
}
