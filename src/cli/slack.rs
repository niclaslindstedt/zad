//! `zad slack <verb>` — runtime commands against a configured Slack bot.

use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config::directory::{self as dir, Directory};
use crate::config::{self, SlackServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::default_dry_run_sink;
use crate::service::slack::permissions::{self as perms, SlackFunction};
use crate::service::slack::{DryRunSlackTransport, SlackHttp, SlackTransport};

// ---------------------------------------------------------------------------
// subcommand plumbing
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SlackArgs {
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Send a message to a channel or DM.
    Send(SendArgs),
    /// Read recent messages from a channel.
    Read(ReadArgs),
    /// List channels in the workspace.
    Channels(ChannelsArgs),
    /// Best-effort walk of the workspace's channels and members, writing a
    /// name -> ID map to this project's `directory.toml`.
    Discover(DiscoverArgs),
    /// Inspect or hand-edit the name -> ID directory.
    Directory(DirectoryArgs),
    /// Inspect, scaffold, or dry-run the permissions policy.
    Permissions(PermissionsArgs),
    /// Manage the Slack user ID resolved from the literal `@me` in targets.
    #[command(name = "self")]
    SelfCmd(SelfArgs),
}

pub async fn run(args: SlackArgs) -> Result<()> {
    let action = args
        .action
        .ok_or_else(|| ZadError::Invalid("missing subcommand. Run `zad slack --help`.".into()))?;
    match action {
        Action::Send(a) => run_send(a).await,
        Action::Read(a) => run_read(a).await,
        Action::Channels(a) => run_channels(a).await,
        Action::Discover(a) => run_discover(a).await,
        Action::Directory(a) => run_directory(a),
        Action::Permissions(a) => run_permissions(a),
        Action::SelfCmd(a) => run_self(a),
    }
}

// ---------------------------------------------------------------------------
// send
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Destination channel ID (`C...`) or name. Mutually exclusive with `--dm`.
    #[arg(long, conflicts_with = "dm")]
    pub channel: Option<String>,

    /// Destination user ID (`U...`) or name for a direct message. Mutually
    /// exclusive with `--channel`.
    #[arg(long, conflicts_with = "channel")]
    pub dm: Option<String>,

    /// Read the message body from stdin instead of the positional argument.
    #[arg(long, conflicts_with = "body")]
    pub stdin: bool,

    /// Message body.
    pub body: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,

    /// Preview the call without contacting Slack. Scope and permission
    /// checks still run; no bot token is loaded.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
struct SendOutput {
    command: &'static str,
    target: &'static str,
    target_id: String,
    ts: String,
}

async fn run_send(args: SendArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let permissions = crate::cli::echo::load_effective_or_echo(perms::load_effective)?;
    permissions.check_time(SlackFunction::Send)?;

    let body = resolve_body(args.body.as_deref(), args.stdin)?;
    permissions.check_send_body(&body)?;

    enum SendTarget {
        Channel(String),
        Dm(String),
    }

    let target = match (&args.channel, &args.dm) {
        (Some(c), None) => {
            let id = resolve_channel(c, &cfg, &directory)?;
            permissions.check_send_channel(&id, &directory)?;
            SendTarget::Channel(id)
        }
        (None, Some(u)) => {
            let id = resolve_user_or_self(u, cfg.self_user_id.as_deref(), &directory)?;
            permissions.check_send_dm(&id, &directory)?;
            SendTarget::Dm(id)
        }
        (None, None) => {
            return Err(ZadError::Invalid(
                "missing destination: pass --channel <ID|name> or --dm <USER_ID|name>".into(),
            ));
        }
        (Some(_), Some(_)) => unreachable!("clap enforces mutual exclusion"),
    };

    let http = slack_http_for("chat:write", args.dry_run)?;
    let ts = match &target {
        SendTarget::Channel(id) => http.send(id, &body).await?,
        SendTarget::Dm(id) => http.send_dm(id, &body).await?,
    };

    if args.dry_run {
        return Ok(());
    }
    if crate::cli::echo::echo_active() {
        crate::cli::echo::render_and_clear(args.json);
        return Ok(());
    }

    let (kind, tid) = match &target {
        SendTarget::Channel(id) => ("channel", id.clone()),
        SendTarget::Dm(id) => ("dm", id.clone()),
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&SendOutput {
                command: "slack.send",
                target: kind,
                target_id: tid,
                ts,
            })
            .unwrap()
        );
    } else {
        println!("Sent message (ts={ts}) to {kind} {tid}.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Channel ID (`C...`) or name to read from.
    #[arg(long)]
    pub channel: String,

    /// Maximum number of messages to fetch (1–200). Defaults to 20.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ReadOutput {
    command: &'static str,
    channel: String,
    count: usize,
    messages: Vec<ReadMessage>,
}

#[derive(Debug, Serialize)]
struct ReadMessage {
    ts: String,
    user: String,
    text: String,
}

async fn run_read(args: ReadArgs) -> Result<()> {
    if args.limit == 0 || args.limit > 200 {
        return Err(ZadError::Invalid(
            "--limit must be between 1 and 200".into(),
        ));
    }
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let permissions = crate::cli::echo::load_effective_or_echo(perms::load_effective)?;
    permissions.check_time(SlackFunction::Read)?;
    let channel_id = resolve_channel(&args.channel, &cfg, &directory)?;
    permissions.check_read_channel(&channel_id, &directory)?;
    let http = slack_http_for("channels:history", false)?;
    let msgs = http.history(&channel_id, args.limit).await?;

    if crate::cli::echo::echo_active() {
        crate::cli::echo::render_and_clear(args.json);
        return Ok(());
    }

    if args.json {
        let out = ReadOutput {
            command: "slack.read",
            channel: channel_id,
            count: msgs.len(),
            messages: msgs
                .iter()
                .map(|m| ReadMessage {
                    ts: m.ts.clone(),
                    user: m.user.clone(),
                    text: m.text.clone(),
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    if msgs.is_empty() {
        println!("(no messages)");
        return Ok(());
    }
    // Slack returns newest-first; print oldest-first.
    for m in msgs.iter().rev() {
        println!("[{}] <{}> {}", m.ts, m.user, m.text);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// channels
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ChannelsArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ChannelsOutput {
    command: &'static str,
    count: usize,
    channels: Vec<ChannelRow>,
}

#[derive(Debug, Serialize)]
struct ChannelRow {
    id: String,
    name: String,
    kind: String,
}

async fn run_channels(args: ChannelsArgs) -> Result<()> {
    let permissions = crate::cli::echo::load_effective_or_echo(perms::load_effective)?;
    permissions.check_time(SlackFunction::Channels)?;
    let directory = dir::load().unwrap_or_default();
    let workspace_input = "workspace";
    permissions.check_channels_workspace(workspace_input, &directory)?;
    if crate::cli::echo::echo_active() {
        crate::cli::echo::render_and_clear(args.json);
        return Ok(());
    }
    let http = slack_http_for("channels:read", false)?;

    let mut all_channels = vec![];
    let mut cursor: Option<String> = None;
    loop {
        let (batch, next) = http.list_channels(cursor.as_deref()).await?;
        all_channels.extend(batch);
        if next.is_none() {
            break;
        }
        cursor = next;
    }

    if args.json {
        let rows: Vec<ChannelRow> = all_channels
            .iter()
            .map(|c| ChannelRow {
                id: c.id.clone(),
                name: c.name.clone(),
                kind: if c.is_private {
                    "private".into()
                } else {
                    "public".into()
                },
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&ChannelsOutput {
                command: "slack.channels",
                count: rows.len(),
                channels: rows,
            })
            .unwrap()
        );
        return Ok(());
    }

    if all_channels.is_empty() {
        println!("(no channels)");
        return Ok(());
    }
    println!("{:<20}  {:<10}  NAME", "ID", "KIND");
    for c in &all_channels {
        let kind = if c.is_private { "private" } else { "public" };
        println!("{:<20}  {:<10}  {}", c.id, kind, c.name);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Skip the member-listing phase.
    #[arg(long)]
    pub skip_members: bool,

    /// Emit machine-readable JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct DiscoverOutput {
    command: &'static str,
    channels: usize,
    users: usize,
    warnings: Vec<String>,
}

async fn run_discover(args: DiscoverArgs) -> Result<()> {
    let permissions = crate::cli::echo::load_effective_or_echo(perms::load_effective)?;
    permissions.check_time(SlackFunction::Discover)?;
    if crate::cli::echo::echo_active() {
        crate::cli::echo::render_and_clear(args.json);
        return Ok(());
    }
    let http = slack_http_for("channels:read", false)?;
    let mut directory = dir::load().unwrap_or_default();
    let mut warnings: Vec<String> = vec![];

    // Channels
    let mut cursor: Option<String> = None;
    loop {
        match http.list_channels(cursor.as_deref()).await {
            Ok((batch, next)) => {
                for c in &batch {
                    directory.channels.insert(c.name.clone(), c.id.clone());
                }
                if next.is_none() {
                    break;
                }
                cursor = next;
            }
            Err(e) => {
                warnings.push(format!("list channels: {e}"));
                break;
            }
        }
    }

    // Members
    if !args.skip_members {
        let mut user_cursor: Option<String> = None;
        loop {
            match http.list_users(user_cursor.as_deref()).await {
                Ok((batch, next)) => {
                    for u in &batch {
                        directory.users.insert(u.display.clone(), u.id.clone());
                        if u.name != u.display {
                            directory.users.insert(u.name.clone(), u.id.clone());
                        }
                    }
                    if next.is_none() {
                        break;
                    }
                    user_cursor = next;
                }
                Err(e) => {
                    warnings.push(format!("list users (needs users:read scope): {e}"));
                    break;
                }
            }
        }
    }

    directory.generated_at_unix = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    dir::save(&directory)?;

    let channels_n = directory.channels.len();
    let users_n = directory.users.len();

    if args.json {
        let out = DiscoverOutput {
            command: "slack.discover",
            channels: channels_n,
            users: users_n,
            warnings: warnings.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Wrote directory: {channels_n} channel entries, {users_n} users.");
        for w in &warnings {
            crate::output::warn(w);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// directory
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DirectoryArgs {
    #[command(subcommand)]
    pub action: Option<DirectoryAction>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum DirectoryAction {
    /// Upsert a name -> ID mapping. `<kind>` is one of `channel` or `user`.
    Set(DirectorySetArgs),
    /// Remove a single mapping.
    Remove(DirectoryRemoveArgs),
    /// Wipe every entry. Use with `--force`.
    Clear(DirectoryClearArgs),
}

#[derive(Debug, Args)]
pub struct DirectorySetArgs {
    pub kind: DirectoryKind,
    pub name: String,
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DirectoryRemoveArgs {
    pub kind: DirectoryKind,
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DirectoryClearArgs {
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum DirectoryKind {
    Channel,
    User,
}

#[derive(Debug, Serialize)]
struct DirectoryOutput<'a> {
    command: &'static str,
    path: String,
    generated_at_unix: Option<u64>,
    channels: &'a std::collections::BTreeMap<String, String>,
    users: &'a std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct DirectoryMutation {
    command: &'static str,
    kind: &'static str,
    name: String,
    id: Option<String>,
    removed: bool,
}

fn require_slack_enabled() -> Result<()> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("slack") {
        return Err(ZadError::Invalid(format!(
            "slack is not enabled for this project ({}). \
             Run `zad service enable slack` first.",
            project_path.display()
        )));
    }
    Ok(())
}

fn kind_as_str(k: DirectoryKind) -> &'static str {
    match k {
        DirectoryKind::Channel => "channel",
        DirectoryKind::User => "user",
    }
}

fn run_directory(args: DirectoryArgs) -> Result<()> {
    require_slack_enabled()?;
    match args.action {
        None => run_directory_list(args.json),
        Some(DirectoryAction::Set(a)) => run_directory_set(a),
        Some(DirectoryAction::Remove(a)) => run_directory_remove(a),
        Some(DirectoryAction::Clear(a)) => run_directory_clear(a),
    }
}

fn run_directory_list(json: bool) -> Result<()> {
    let path = dir::path_current()?;
    let directory = dir::load_from(&path)?;
    if json {
        let out = DirectoryOutput {
            command: "slack.directory",
            path: path.display().to_string(),
            generated_at_unix: directory.generated_at_unix,
            channels: &directory.channels,
            users: &directory.users,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    if directory.channels.is_empty() && directory.users.is_empty() {
        println!("(empty) {}", path.display());
        println!("Run `zad slack discover` to populate it.");
        return Ok(());
    }
    println!("# {}", path.display());
    if !directory.channels.is_empty() {
        println!("\n[channels]");
        for (n, id) in &directory.channels {
            println!("  {n:<40}  {id}");
        }
    }
    if !directory.users.is_empty() {
        println!("\n[users]");
        for (n, id) in &directory.users {
            println!("  {n:<24}  {id}");
        }
    }
    Ok(())
}

fn run_directory_set(args: DirectorySetArgs) -> Result<()> {
    let path = dir::path_current()?;
    let mut directory = dir::load_from(&path)?;
    let bucket = match args.kind {
        DirectoryKind::Channel => &mut directory.channels,
        DirectoryKind::User => &mut directory.users,
    };
    bucket.insert(args.name.clone(), args.id.clone());
    dir::save_to(&path, &directory)?;
    if args.json {
        let out = DirectoryMutation {
            command: "slack.directory.set",
            kind: kind_as_str(args.kind),
            name: args.name,
            id: Some(args.id),
            removed: false,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!(
            "Mapped {} `{}` in {}.",
            kind_as_str(args.kind),
            args.name,
            path.display()
        );
    }
    Ok(())
}

fn run_directory_remove(args: DirectoryRemoveArgs) -> Result<()> {
    let path = dir::path_current()?;
    let mut directory = dir::load_from(&path)?;
    let bucket = match args.kind {
        DirectoryKind::Channel => &mut directory.channels,
        DirectoryKind::User => &mut directory.users,
    };
    let removed = bucket.remove(&args.name).is_some();
    if removed {
        dir::save_to(&path, &directory)?;
    }
    if args.json {
        let out = DirectoryMutation {
            command: "slack.directory.remove",
            kind: kind_as_str(args.kind),
            name: args.name,
            id: None,
            removed,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if removed {
        println!(
            "Removed {} `{}` from {}.",
            kind_as_str(args.kind),
            args.name,
            path.display()
        );
    } else {
        println!("No {} entry named `{}`.", kind_as_str(args.kind), args.name);
    }
    Ok(())
}

fn run_directory_clear(args: DirectoryClearArgs) -> Result<()> {
    if !args.force {
        return Err(ZadError::Invalid(
            "refusing to clear the directory without --force".into(),
        ));
    }
    let path = dir::path_current()?;
    let directory = Directory::default();
    dir::save_to(&path, &directory)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "slack.directory.clear",
                "path": path.display().to_string(),
            }))
            .unwrap()
        );
    } else {
        println!("Cleared {}.", path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// permissions
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum PermissionsAction {
    Show(PermissionsShowArgs),
    Init(PermissionsInitArgs),
    Path(PermissionsPathArgs),
    Check(PermissionsCheckArgs),
    #[command(flatten)]
    Staging(crate::cli::permissions::StagingAction),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
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
pub struct PermissionsPathArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsCheckArgs {
    /// Function to check: `send`, `read`, `channels`, `discover`.
    #[arg(long)]
    pub function: String,

    /// Channel name or ID for `send` / `read`.
    #[arg(long, conflicts_with = "user")]
    pub channel: Option<String>,

    /// User name or ID for `send` DM checks.
    #[arg(long, conflicts_with = "channel")]
    pub user: Option<String>,

    /// Body to test against content rules (applies only to `send`).
    #[arg(long)]
    pub body: Option<String>,

    #[arg(long)]
    pub json: bool,
}

fn run_permissions(args: PermissionsArgs) -> Result<()> {
    match args.action {
        None => run_permissions_show(PermissionsShowArgs { json: args.json }),
        Some(PermissionsAction::Show(a)) => run_permissions_show(a),
        Some(PermissionsAction::Init(a)) => run_permissions_init(a),
        Some(PermissionsAction::Path(a)) => run_permissions_path(a),
        Some(PermissionsAction::Check(a)) => run_permissions_check(a),
        Some(PermissionsAction::Staging(a)) => {
            crate::cli::permissions::run::<perms::PermissionsService>(a)
        }
    }
}

#[derive(Debug, Serialize)]
struct PermissionsShowOutput {
    command: &'static str,
    global: PermissionsScopeBlock,
    local: PermissionsScopeBlock,
}

#[derive(Debug, Serialize)]
struct PermissionsScopeBlock {
    path: String,
    present: bool,
}

fn run_permissions_show(args: PermissionsShowArgs) -> Result<()> {
    let global_p = perms::global_path()?;
    let local_p = perms::local_path_current()?;
    let global_present = global_p.exists();
    let local_present = local_p.exists();
    let effective = perms::load_effective()?;
    let _ = effective;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&PermissionsShowOutput {
                command: "slack.permissions.show",
                global: PermissionsScopeBlock {
                    path: global_p.display().to_string(),
                    present: global_present,
                },
                local: PermissionsScopeBlock {
                    path: local_p.display().to_string(),
                    present: local_present,
                },
            })
            .unwrap()
        );
        return Ok(());
    }
    println!("# permissions");
    println!(
        "  global : {} ({})",
        global_p.display(),
        if global_present {
            "present"
        } else {
            "not present (no restrictions at this scope)"
        }
    );
    println!(
        "  local  : {} ({})",
        local_p.display(),
        if local_present {
            "present"
        } else {
            "not present (no restrictions at this scope)"
        }
    );
    println!();
    if !global_present && !local_present {
        println!("No permission files found. Every declared scope is currently unrestricted.");
        println!("Run `zad slack permissions init` to scaffold a starter policy.");
        return Ok(());
    }
    for p in [&global_p, &local_p] {
        if !p.exists() {
            continue;
        }
        println!("## {}", p.display());
        match std::fs::read_to_string(p) {
            Ok(body) => {
                for line in body.lines() {
                    println!("  {line}");
                }
            }
            Err(e) => println!("  (failed to read: {e})"),
        }
        println!();
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsInitOutput {
    command: &'static str,
    scope: &'static str,
    path: String,
    written: bool,
}

fn run_permissions_init(args: PermissionsInitArgs) -> Result<()> {
    let (path, scope) = if args.local {
        (perms::local_path_current()?, "local")
    } else {
        (perms::global_path()?, "global")
    };
    if path.exists() && !args.force {
        return Err(ZadError::Invalid(format!(
            "permissions file already exists at {}. Pass --force to overwrite.",
            path.display()
        )));
    }
    let template = perms::starter_template();
    let key = crate::permissions::signing::load_or_create_from_keychain()?;
    crate::permissions::signing::write_public_key_cache(&key)?;
    perms::save_file(&path, &template, &key)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&PermissionsInitOutput {
                command: "slack.permissions.init",
                scope,
                path: path.display().to_string(),
                written: true,
            })
            .unwrap()
        );
    } else {
        println!("Wrote starter permissions ({scope}): {}", path.display());
        println!("Signed with key {}.", key.fingerprint());
        println!("Review it; the defaults deny admin-like channels.");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsPathOutput {
    command: &'static str,
    global: String,
    local: String,
}

fn run_permissions_path(args: PermissionsPathArgs) -> Result<()> {
    let global_p = perms::global_path()?;
    let local_p = perms::local_path_current()?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&PermissionsPathOutput {
                command: "slack.permissions.path",
                global: global_p.display().to_string(),
                local: local_p.display().to_string(),
            })
            .unwrap()
        );
    } else {
        println!("{}", global_p.display());
        println!("{}", local_p.display());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsCheckOutput {
    command: &'static str,
    function: String,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_path: Option<String>,
}

fn run_permissions_check(args: PermissionsCheckArgs) -> Result<()> {
    let function = parse_function(&args.function)?;
    let permissions = perms::load_effective()?;
    let directory = dir::load().unwrap_or_default();

    let mut outcome: Result<()> = Ok(());
    outcome = outcome.and_then(|()| permissions.check_time(function));

    if outcome.is_ok() {
        outcome = match (function, &args.channel, &args.user) {
            (SlackFunction::Send, Some(c), None) => permissions.check_send_channel(c, &directory),
            (SlackFunction::Send, None, Some(u)) => permissions.check_send_dm(u, &directory),
            (SlackFunction::Read, Some(c), _) => permissions.check_read_channel(c, &directory),
            (SlackFunction::Channels, _, _) => {
                permissions.check_channels_workspace("workspace", &directory)
            }
            (SlackFunction::Discover, _, _) => {
                permissions.check_discover_workspace("workspace", &directory)
            }
            _ => Ok(()),
        };
    }

    if outcome.is_ok()
        && function == SlackFunction::Send
        && let Some(body) = &args.body
    {
        outcome = permissions.check_send_body(body);
    }

    let (allowed, reason, config_path) = match outcome {
        Ok(()) => (true, None, None),
        Err(ZadError::PermissionDenied {
            reason,
            config_path,
            ..
        }) => (false, Some(reason), Some(config_path.display().to_string())),
        Err(e) => return Err(e),
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&PermissionsCheckOutput {
                command: "slack.permissions.check",
                function: args.function.clone(),
                allowed,
                reason,
                config_path,
            })
            .unwrap()
        );
    } else if allowed {
        println!("allow");
    } else {
        println!(
            "deny — {}",
            reason.as_deref().unwrap_or("unspecified reason")
        );
        if let Some(p) = &config_path {
            println!("  config: {p}");
        }
    }
    if !allowed {
        std::process::exit(1);
    }
    Ok(())
}

fn parse_function(name: &str) -> Result<SlackFunction> {
    match name {
        "send" => Ok(SlackFunction::Send),
        "read" => Ok(SlackFunction::Read),
        "channels" => Ok(SlackFunction::Channels),
        "discover" => Ok(SlackFunction::Discover),
        other => Err(ZadError::Invalid(format!(
            "unknown function `{other}`. Expected one of: send, read, channels, discover."
        ))),
    }
}

// ---------------------------------------------------------------------------
// self — manage the `@me` resolution target
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SelfArgs {
    #[command(subcommand)]
    pub action: Option<SelfAction>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum SelfAction {
    Show(SelfShowArgs),
    Set(SelfSetArgs),
    Clear(SelfClearArgs),
}

#[derive(Debug, Args)]
pub struct SelfShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfSetArgs {
    /// Your Slack user ID (`U...`).
    pub user_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfClearArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct SelfOutput {
    command: &'static str,
    self_user_id: Option<String>,
}

fn run_self(args: SelfArgs) -> Result<()> {
    match args.action {
        None => run_self_show(SelfShowArgs { json: args.json }),
        Some(SelfAction::Show(a)) => run_self_show(a),
        Some(SelfAction::Set(a)) => run_self_set(a),
        Some(SelfAction::Clear(a)) => run_self_clear(a),
    }
}

fn run_self_show(args: SelfShowArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    emit_self(args.json, "slack.self.show", cfg.self_user_id)
}

fn run_self_set(args: SelfSetArgs) -> Result<()> {
    let (mut cfg, scope) = effective_config()?;
    cfg.self_user_id = Some(args.user_id.trim().to_string());
    save_effective_config(&cfg, &scope)?;
    emit_self(args.json, "slack.self.set", cfg.self_user_id)
}

fn run_self_clear(args: SelfClearArgs) -> Result<()> {
    let (mut cfg, scope) = effective_config()?;
    cfg.self_user_id = None;
    save_effective_config(&cfg, &scope)?;
    emit_self(args.json, "slack.self.clear", None)
}

fn emit_self(json: bool, command: &'static str, self_user_id: Option<String>) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&SelfOutput {
                command,
                self_user_id
            })
            .unwrap()
        );
    } else {
        match self_user_id {
            Some(id) => println!("self user id: {id}"),
            None => println!("self user id: not configured"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// credential / config plumbing
// ---------------------------------------------------------------------------

enum EffectiveScope {
    Global,
    Local(String),
}

fn effective_config() -> Result<(SlackServiceCfg, EffectiveScope)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("slack") {
        return Err(ZadError::Invalid(format!(
            "slack is not enabled for this project ({}). \
             Run `zad service enable slack` first.",
            project_path.display()
        )));
    }
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "slack")?;
    if let Some(cfg) = config::load_flat::<SlackServiceCfg>(&local_path)? {
        return Ok((cfg, EffectiveScope::Local(slug)));
    }
    let global_path = config::path::global_service_config_path("slack")?;
    if let Some(cfg) = config::load_flat::<SlackServiceCfg>(&global_path)? {
        return Ok((cfg, EffectiveScope::Global));
    }
    Err(ZadError::Invalid(format!(
        "no Slack credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_token(scope: &EffectiveScope) -> Result<String> {
    let account = match scope {
        EffectiveScope::Global => secrets::account("slack", "bot", Scope::Global),
        EffectiveScope::Local(slug) => secrets::account("slack", "bot", Scope::Project(slug)),
    };
    secrets::load(&account)?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "bot token missing from keychain (account `{account}`). \
             Re-run `zad service create slack` to reinstall it."
        ))
    })
}

fn slack_http_for(required: &'static str, dry_run: bool) -> Result<Box<dyn SlackTransport>> {
    let (cfg, scope) = effective_config()?;
    let config_path = match &scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "slack")?
        }
        EffectiveScope::Global => config::path::global_service_config_path("slack")?,
    };
    let scopes: std::collections::BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    if !scopes.contains(required) {
        return Err(ZadError::ScopeDenied {
            service: "slack",
            scope: required,
            config_path,
        });
    }
    if dry_run || crate::cli::echo::echo_active() {
        let sink = if crate::cli::echo::echo_active() {
            crate::cli::echo::dry_run_sink_for_echo()
        } else {
            default_dry_run_sink()
        };
        return Ok(Box::new(DryRunSlackTransport::new(sink)));
    }
    let token = load_token(&scope)?;
    Ok(Box::new(SlackHttp::new(&token, scopes, config_path)))
}

fn resolve_channel(input: &str, cfg: &SlackServiceCfg, directory: &Directory) -> Result<String> {
    // If it looks like a Slack channel ID (starts with C or D), use as-is.
    if is_slack_id(input) {
        return Ok(input.to_string());
    }
    // Try the default_channel shorthand.
    if input.eq_ignore_ascii_case("default") {
        if let Some(ch) = &cfg.default_channel {
            return resolve_channel(ch, cfg, directory);
        }
        return Err(ZadError::Invalid(
            "no default_channel configured; pass --channel explicitly".into(),
        ));
    }
    let key = input.strip_prefix('#').unwrap_or(input);
    if let Some(id) = directory.channels.get(key) {
        return Ok(id.clone());
    }
    Err(ZadError::Invalid(format!(
        "--channel `{input}` is neither a Slack channel ID nor a known directory entry. \
         Run `zad slack discover` or map it manually with \
         `zad slack directory set channel {key} <ID>`."
    )))
}

fn resolve_user(input: &str, directory: &Directory) -> Result<String> {
    if is_slack_id(input) {
        return Ok(input.to_string());
    }
    let key = input.strip_prefix('@').unwrap_or(input);
    if let Some(id) = directory.users.get(key) {
        return Ok(id.clone());
    }
    Err(ZadError::Invalid(format!(
        "--dm `{input}` is neither a Slack user ID nor a known directory entry. \
         Run `zad slack discover` or map it manually with \
         `zad slack directory set user {key} <ID>`."
    )))
}

fn resolve_user_or_self(
    input: &str,
    self_user_id: Option<&str>,
    directory: &Directory,
) -> Result<String> {
    if input.eq_ignore_ascii_case("@me") {
        return match self_user_id {
            Some(id) => Ok(id.to_string()),
            None => Err(ZadError::Invalid(
                "`@me` has no self-user configured. Run \
                 `zad slack self set <U...>` with your Slack user ID."
                    .into(),
            )),
        };
    }
    resolve_user(input, directory)
}

fn is_slack_id(s: &str) -> bool {
    matches!(s.chars().next(), Some('C' | 'D' | 'G' | 'U' | 'W' | 'T'))
        && s.len() >= 8
        && s.chars().skip(1).all(|c| c.is_ascii_alphanumeric())
}

fn save_effective_config(cfg: &SlackServiceCfg, scope: &EffectiveScope) -> Result<()> {
    let path = match scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "slack")?
        }
        EffectiveScope::Global => config::path::global_service_config_path("slack")?,
    };
    config::save_flat(&path, cfg)
}

fn resolve_body(positional: Option<&str>, from_stdin: bool) -> Result<String> {
    if from_stdin {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            ZadError::Invalid(format!("failed to read message body from stdin: {e}"))
        })?;
        let trimmed = buf.trim_end_matches(['\n', '\r']).to_string();
        if trimmed.is_empty() {
            return Err(ZadError::Invalid("message body is empty (stdin)".into()));
        }
        return Ok(trimmed);
    }
    match positional {
        Some(b) if !b.is_empty() => Ok(b.to_string()),
        _ => Err(ZadError::Invalid(
            "missing message body: pass it as a positional arg or --stdin".into(),
        )),
    }
}
