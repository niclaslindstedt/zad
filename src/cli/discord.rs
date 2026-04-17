//! `zad discord <verb>` — runtime commands against a configured Discord
//! bot. Credential resolution mirrors `zad service enable discord`: the
//! project-local config wins over the global one, and the matching
//! keychain entry holds the bot token. The project must already have
//! enabled the Discord service.

use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config::directory::{self as dir, Directory};
use crate::config::{self, DiscordServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::discord::DiscordHttp;
use crate::service::discord::permissions::{self as perms, DiscordFunction};
use crate::service::{ChannelId, Target, UserId};

// ---------------------------------------------------------------------------
// subcommand plumbing
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DiscordArgs {
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Send a message to a channel or DM.
    Send(SendArgs),
    /// Read recent messages from a channel.
    Read(ReadArgs),
    /// List channels in a guild.
    Channels(ChannelsArgs),
    /// Join a thread channel (Discord only allows explicit joins on threads).
    Join(JoinArgs),
    /// Leave a thread channel.
    Leave(LeaveArgs),
    /// Best-effort walk of the bot's visible guilds, channels, and
    /// members, writing a name -> snowflake map to this project's
    /// `directory.toml`. Safe to re-run; preserves hand-authored entries.
    Discover(DiscoverArgs),
    /// Inspect or hand-edit the name -> snowflake directory.
    Directory(DirectoryArgs),
    /// Inspect, scaffold, or dry-run the permissions policy that narrows
    /// what this service may actually do.
    Permissions(PermissionsArgs),
}

pub async fn run(args: DiscordArgs) -> Result<()> {
    let action = args
        .action
        .ok_or_else(|| ZadError::Invalid("missing subcommand. Run `zad discord --help`.".into()))?;
    match action {
        Action::Send(a) => run_send(a).await,
        Action::Read(a) => run_read(a).await,
        Action::Channels(a) => run_channels(a).await,
        Action::Join(a) => run_join(a).await,
        Action::Leave(a) => run_leave(a).await,
        Action::Discover(a) => run_discover(a).await,
        Action::Directory(a) => run_directory(a),
        Action::Permissions(a) => run_permissions(a),
    }
}

// ---------------------------------------------------------------------------
// send
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Destination channel ID (snowflake). Mutually exclusive with `--dm`.
    #[arg(long, conflicts_with = "dm")]
    pub channel: Option<String>,

    /// Destination user ID (snowflake) for a direct message. Mutually
    /// exclusive with `--channel`.
    #[arg(long, conflicts_with = "channel")]
    pub dm: Option<String>,

    /// Read the message body from standard input instead of the positional
    /// argument.
    #[arg(long, conflicts_with = "body")]
    pub stdin: bool,

    /// Message body. Required unless `--stdin` is set.
    pub body: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct SendOutput {
    command: &'static str,
    target: &'static str,
    target_id: String,
    message_id: String,
}

async fn run_send(args: SendArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let context_guild = default_guild_name(&cfg, &directory);
    let permissions = perms::load_effective()?;
    permissions.check_time(DiscordFunction::Send)?;
    let target = match (&args.channel, &args.dm) {
        (Some(c), None) => {
            let id = resolve_channel(c, &directory, context_guild.as_deref())?;
            permissions.check_send_channel(c, id, &directory)?;
            Target::Channel(ChannelId(id))
        }
        (None, Some(u)) => {
            let id = resolve_user(u, &directory)?;
            permissions.check_send_dm(u, id, &directory)?;
            Target::Dm(UserId(id))
        }
        (None, None) => {
            return Err(ZadError::Invalid(
                "missing destination: pass --channel <ID> or --dm <USER_ID>".into(),
            ));
        }
        (Some(_), Some(_)) => unreachable!("clap enforces mutual exclusion"),
    };

    let body = resolve_body(args.body.as_deref(), args.stdin)?;
    let len = body.chars().count();
    if len > crate::service::discord::client::DISCORD_MAX_MESSAGE_LEN {
        return Err(ZadError::Invalid(format!(
            "message body is {len} characters; Discord's hard limit is {}",
            crate::service::discord::client::DISCORD_MAX_MESSAGE_LEN
        )));
    }
    permissions.check_send_body(&body)?;
    let http = discord_http_for("messages.send")?;
    let msg_id = http.send(target.clone(), &body).await?;

    let (kind, tid) = match &target {
        Target::Channel(ChannelId(id)) => ("channel", id.to_string()),
        Target::Dm(UserId(id)) => ("dm", id.to_string()),
    };

    if args.json {
        let out = SendOutput {
            command: "discord.send",
            target: kind,
            target_id: tid,
            message_id: msg_id.0.to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Sent message {} to {kind} {tid}.", msg_id.0);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Channel ID (snowflake) to read from.
    #[arg(long)]
    pub channel: String,

    /// Maximum number of messages to fetch (1–100). Defaults to 20.
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
    id: String,
    author: String,
    body: String,
}

async fn run_read(args: ReadArgs) -> Result<()> {
    if args.limit == 0 || args.limit > 100 {
        return Err(ZadError::Invalid(
            "--limit must be between 1 and 100 (Discord API maximum)".into(),
        ));
    }
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let context_guild = default_guild_name(&cfg, &directory);
    let permissions = perms::load_effective()?;
    permissions.check_time(DiscordFunction::Read)?;
    let id = resolve_channel(&args.channel, &directory, context_guild.as_deref())?;
    permissions.check_read_channel(&args.channel, id, &directory)?;
    let channel_id = ChannelId(id);
    let http = discord_http_for("messages.read")?;
    let msgs = http.history(channel_id.clone(), args.limit).await?;

    if args.json {
        let out = ReadOutput {
            command: "discord.read",
            channel: channel_id.0.to_string(),
            count: msgs.len(),
            messages: msgs
                .iter()
                .map(|m| ReadMessage {
                    id: m.id.0.to_string(),
                    author: m.author.0.to_string(),
                    body: m.body.clone(),
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
    // Discord returns newest-first; print oldest-first so a human reads
    // top-to-bottom in chronological order.
    for m in msgs.iter().rev() {
        println!("[{}] <{}> {}", m.id.0, m.author.0, m.body);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// channels
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ChannelsArgs {
    /// Guild (server) ID. Defaults to the configured `default_guild` if
    /// unset.
    #[arg(long)]
    pub guild: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ChannelsOutput {
    command: &'static str,
    guild: String,
    count: usize,
    channels: Vec<ChannelRow>,
}

#[derive(Debug, Serialize)]
struct ChannelRow {
    id: String,
    name: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    position: u16,
}

async fn run_channels(args: ChannelsArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let permissions = perms::load_effective()?;
    permissions.check_time(DiscordFunction::Channels)?;
    let guild = resolve_guild_arg(
        args.guild.as_deref(),
        cfg.default_guild.as_deref(),
        &directory,
    )?;
    let guild_input = args
        .guild
        .clone()
        .or_else(|| cfg.default_guild.clone())
        .unwrap_or_else(|| guild.to_string());
    permissions.check_channels_guild(&guild_input, guild, &directory)?;
    let http = discord_http_for("guilds")?;
    let channels = http.list_channels(guild).await?;

    if args.json {
        let rows: Vec<ChannelRow> = channels
            .iter()
            .map(|c| ChannelRow {
                id: c.id.0.to_string(),
                name: c.name.clone(),
                kind: c.kind.clone(),
                parent: c.parent.as_ref().map(|p| p.0.to_string()),
                position: c.position,
            })
            .collect();
        let out = ChannelsOutput {
            command: "discord.channels",
            guild: guild.to_string(),
            count: rows.len(),
            channels: rows,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    if channels.is_empty() {
        println!("(no channels in guild {guild})");
        return Ok(());
    }
    println!("{:<20}  {:<14}  NAME", "ID", "KIND");
    for c in &channels {
        println!("{:<20}  {:<14}  {}", c.id.0, c.kind, c.name);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// join / leave (thread members)
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct JoinArgs {
    /// Channel ID (snowflake). Must refer to a thread channel.
    #[arg(long)]
    pub channel: String,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LeaveArgs {
    /// Channel ID (snowflake). Must refer to a thread channel.
    #[arg(long)]
    pub channel: String,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct MembershipOutput {
    command: &'static str,
    channel: String,
}

async fn run_join(args: JoinArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let context_guild = default_guild_name(&cfg, &directory);
    let permissions = perms::load_effective()?;
    permissions.check_time(DiscordFunction::Join)?;
    let id = resolve_channel(&args.channel, &directory, context_guild.as_deref())?;
    permissions.check_join_channel(&args.channel, id, &directory)?;
    let channel = ChannelId(id);
    let http = discord_http_for("guilds")?;
    http.join_channel(channel.clone()).await?;
    if args.json {
        let out = MembershipOutput {
            command: "discord.join",
            channel: channel.0.to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Joined channel {}.", channel.0);
    }
    Ok(())
}

async fn run_leave(args: LeaveArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let context_guild = default_guild_name(&cfg, &directory);
    let permissions = perms::load_effective()?;
    permissions.check_time(DiscordFunction::Leave)?;
    let id = resolve_channel(&args.channel, &directory, context_guild.as_deref())?;
    permissions.check_leave_channel(&args.channel, id, &directory)?;
    let channel = ChannelId(id);
    let http = discord_http_for("guilds")?;
    http.leave_channel(channel.clone()).await?;
    if args.json {
        let out = MembershipOutput {
            command: "discord.leave",
            channel: channel.0.to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Left channel {}.", channel.0);
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

fn effective_config() -> Result<(DiscordServiceCfg, EffectiveScope)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("discord") {
        return Err(ZadError::Invalid(format!(
            "discord is not enabled for this project ({}). \
             Run `zad service enable discord` first.",
            project_path.display()
        )));
    }

    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "discord")?;
    if let Some(cfg) = config::load_flat::<DiscordServiceCfg>(&local_path)? {
        return Ok((cfg, EffectiveScope::Local(slug)));
    }
    let global_path = config::path::global_service_config_path("discord")?;
    if let Some(cfg) = config::load_flat::<DiscordServiceCfg>(&global_path)? {
        return Ok((cfg, EffectiveScope::Global));
    }
    Err(ZadError::Invalid(format!(
        "no Discord credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_token(scope: &EffectiveScope) -> Result<String> {
    let account = match scope {
        EffectiveScope::Global => secrets::discord_bot_account(Scope::Global),
        EffectiveScope::Local(slug) => secrets::discord_bot_account(Scope::Project(slug)),
    };
    secrets::load(&account)?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "bot token missing from keychain (account `{account}`). \
             Re-run `zad service create discord` to reinstall it."
        ))
    })
}

/// Resolve config + token + scope set into a ready-to-call client, and
/// fail fast with [`ZadError::ScopeDenied`] if `required` isn't declared.
/// The fail-fast scope check happens *before* the keychain read so a
/// denied op never touches secrets; [`DiscordHttp`] still guards the
/// same scope internally, which covers library callers (`DiscordService`)
/// that bypass this helper.
fn discord_http_for(required: &'static str) -> Result<DiscordHttp> {
    let (cfg, scope) = effective_config()?;
    let config_path = match &scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "discord")?
        }
        EffectiveScope::Global => config::path::global_service_config_path("discord")?,
    };
    let scopes: std::collections::BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    if !scopes.contains(required) {
        return Err(ZadError::ScopeDenied {
            scope: required,
            config_path,
        });
    }
    let token = load_token(&scope)?;
    Ok(DiscordHttp::new(&token, scopes, config_path))
}

fn resolve_guild_arg(
    flag: Option<&str>,
    default: Option<&str>,
    directory: &Directory,
) -> Result<u64> {
    let raw = flag.or(default).ok_or_else(|| {
        ZadError::Invalid(
            "no guild specified: pass --guild <ID|name> or set `default_guild` in the config"
                .into(),
        )
    })?;
    directory.resolve_guild(raw).ok_or_else(|| {
        ZadError::Invalid(format!(
            "--guild `{raw}` is neither a numeric snowflake nor a known directory entry. \
             Run `zad discord discover` or map it manually with \
             `zad discord directory set guild {raw} <id>`."
        ))
    })
}

fn resolve_channel(input: &str, directory: &Directory, context_guild: Option<&str>) -> Result<u64> {
    directory
        .resolve_channel(input, context_guild)
        .ok_or_else(|| {
            let key = input.strip_prefix('#').unwrap_or(input);
            ZadError::Invalid(format!(
                "--channel `{input}` is neither a numeric snowflake nor a known directory entry. \
             Run `zad discord discover` or map it manually with \
             `zad discord directory set channel {key} <id>`."
            ))
        })
}

fn resolve_user(input: &str, directory: &Directory) -> Result<u64> {
    directory.resolve_user(input).ok_or_else(|| {
        let key = input.strip_prefix('@').unwrap_or(input);
        ZadError::Invalid(format!(
            "--dm `{input}` is neither a numeric snowflake nor a known directory entry. \
             Run `zad discord discover` or map it manually with \
             `zad discord directory set user {key} <id>`."
        ))
    })
}

fn parse_snowflake(v: &str, field: &'static str) -> Result<u64> {
    v.parse::<u64>().map_err(|_| {
        ZadError::Invalid(format!(
            "{field} must be a numeric Discord snowflake, got `{v}`"
        ))
    })
}

fn default_guild_name(cfg: &DiscordServiceCfg, directory: &Directory) -> Option<String> {
    let raw = cfg.default_guild.as_deref()?;
    if let Ok(id) = raw.parse::<u64>() {
        return directory.guild_name_for(id).map(str::to_owned);
    }
    if directory.guilds.contains_key(raw) {
        return Some(raw.to_owned());
    }
    None
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Scope discovery to a single guild (by ID or known name). Without
    /// this flag, every guild the bot can see is walked.
    #[arg(long)]
    pub guild: Option<String>,

    /// Skip the member-listing phase. Use this when the bot doesn't have
    /// the privileged `GUILD_MEMBERS` intent enabled and you want to
    /// suppress the warning it would otherwise emit.
    #[arg(long)]
    pub skip_members: bool,

    /// Emit machine-readable JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct DiscoverOutput {
    command: &'static str,
    guilds: usize,
    channels: usize,
    users: usize,
    warnings: Vec<String>,
}

async fn run_discover(args: DiscoverArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(DiscordFunction::Discover)?;
    let http = discord_http_for("guilds")?;
    let mut directory = dir::load().unwrap_or_default();
    let mut warnings: Vec<String> = vec![];

    let guilds = match http.list_guilds().await {
        Ok(g) => g,
        Err(e) => {
            warnings.push(format!("list guilds: {e}"));
            vec![]
        }
    };

    let scoped: Option<u64> = args
        .guild
        .as_deref()
        .map(|raw| -> Result<u64> {
            directory.resolve_guild(raw).ok_or_else(|| {
                ZadError::Invalid(format!(
                    "--guild `{raw}` is not numeric and not in the directory; \
                     run `zad discord discover` without --guild first, or pass an ID."
                ))
            })
        })
        .transpose()?;

    // Filter the walk to guilds the operator actually allowed discovery
    // into. `guilds.allow`/`guilds.deny` for the `discover` block narrows
    // the walk; silently skipping a denied guild is the right shape
    // because `discover` is already best-effort.
    let targets: Vec<_> = match scoped {
        Some(id) => guilds.iter().filter(|g| g.id == id).cloned().collect(),
        None => guilds
            .iter()
            .filter(|g| {
                permissions
                    .check_discover_guild(&g.name, g.id, &directory)
                    .is_ok()
            })
            .cloned()
            .collect(),
    };
    if let Some(id) = scoped
        && let Some(g) = guilds.iter().find(|g| g.id == id)
    {
        permissions.check_discover_guild(&g.name, g.id, &directory)?;
    }

    for g in &guilds {
        directory.guilds.insert(g.name.clone(), g.id.to_string());
    }

    for g in &targets {
        match http.list_channels(g.id).await {
            Ok(chans) => {
                for c in chans {
                    let qualified = format!("{}/{}", g.name, c.name);
                    directory.channels.insert(qualified, c.id.0.to_string());
                    // Bare-name convenience key. If multiple guilds share
                    // a channel name (e.g. `general`), the last one
                    // written wins; the qualified key always
                    // disambiguates when the caller needs it to.
                    directory.channels.insert(c.name, c.id.0.to_string());
                }
            }
            Err(e) => warnings.push(format!("channels for `{}`: {e}", g.name)),
        }

        if args.skip_members {
            continue;
        }
        match http.list_members(g.id, 1000).await {
            Ok(members) => {
                for m in members {
                    directory
                        .users
                        .insert(m.display_name.clone(), m.id.0.to_string());
                }
            }
            Err(e) => warnings.push(format!(
                "members for `{}` (needs GUILD_MEMBERS privileged intent): {e}",
                g.name
            )),
        }
    }

    directory.generated_at_unix = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    dir::save(&directory)?;

    let guilds_n = directory.guilds.len();
    let channels_n = directory.channels.len();
    let users_n = directory.users.len();

    if args.json {
        let out = DiscoverOutput {
            command: "discord.discover",
            guilds: guilds_n,
            channels: channels_n,
            users: users_n,
            warnings: warnings.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!(
            "Wrote directory: {guilds_n} guilds, {channels_n} channel entries, {users_n} users."
        );
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

    /// When no subcommand is given, print the directory as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum DirectoryAction {
    /// Upsert a name -> snowflake mapping. `<kind>` is one of
    /// `guild`, `channel`, or `user`. Channel keys may include a
    /// `guild/channel` qualifier.
    Set(DirectorySetArgs),
    /// Remove a single mapping. Silent no-op if the key is absent.
    Remove(DirectoryRemoveArgs),
    /// Wipe every entry. Use with `--force`.
    Clear(DirectoryClearArgs),
}

#[derive(Debug, Args)]
pub struct DirectorySetArgs {
    /// One of `guild`, `channel`, `user`.
    pub kind: DirectoryKind,
    /// Human-readable name to map from.
    pub name: String,
    /// Numeric snowflake to map to.
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
    Guild,
    Channel,
    User,
}

#[derive(Debug, Serialize)]
struct DirectoryOutput<'a> {
    command: &'static str,
    path: String,
    generated_at_unix: Option<u64>,
    guilds: &'a std::collections::BTreeMap<String, String>,
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

fn require_discord_enabled() -> Result<()> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("discord") {
        return Err(ZadError::Invalid(format!(
            "discord is not enabled for this project ({}). \
             Run `zad service enable discord` first.",
            project_path.display()
        )));
    }
    Ok(())
}

fn kind_as_str(k: DirectoryKind) -> &'static str {
    match k {
        DirectoryKind::Guild => "guild",
        DirectoryKind::Channel => "channel",
        DirectoryKind::User => "user",
    }
}

fn run_directory(args: DirectoryArgs) -> Result<()> {
    require_discord_enabled()?;
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
            command: "discord.directory",
            path: path.display().to_string(),
            generated_at_unix: directory.generated_at_unix,
            guilds: &directory.guilds,
            channels: &directory.channels,
            users: &directory.users,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    if directory.total() == 0 {
        println!("(empty) {}", path.display());
        println!("Run `zad discord discover` to populate it.");
        return Ok(());
    }
    println!("# {}", path.display());
    if !directory.guilds.is_empty() {
        println!("\n[guilds]");
        for (n, id) in &directory.guilds {
            println!("  {n:<24}  {id}");
        }
    }
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
    let id = parse_snowflake(&args.id, "<id>")?;
    let path = dir::path_current()?;
    let mut directory = dir::load_from(&path)?;
    let bucket = match args.kind {
        DirectoryKind::Guild => &mut directory.guilds,
        DirectoryKind::Channel => &mut directory.channels,
        DirectoryKind::User => &mut directory.users,
    };
    bucket.insert(args.name.clone(), id.to_string());
    dir::save_to(&path, &directory)?;

    if args.json {
        let out = DirectoryMutation {
            command: "discord.directory.set",
            kind: kind_as_str(args.kind),
            name: args.name,
            id: Some(id.to_string()),
            removed: false,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!(
            "Mapped {} `{}` -> {id} in {}.",
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
        DirectoryKind::Guild => &mut directory.guilds,
        DirectoryKind::Channel => &mut directory.channels,
        DirectoryKind::User => &mut directory.users,
    };
    let removed = bucket.remove(&args.name).is_some();
    if removed {
        dir::save_to(&path, &directory)?;
    }

    if args.json {
        let out = DirectoryMutation {
            command: "discord.directory.remove",
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
                "command": "discord.directory.clear",
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
// permissions — inspect / scaffold / dry-run the permissions policy
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,

    /// When no subcommand is given, behave like `show`.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum PermissionsAction {
    /// Print the effective policy (global + local) for this project.
    Show(PermissionsShowArgs),
    /// Write a starter `permissions.toml` at the selected scope.
    Init(PermissionsInitArgs),
    /// Print the paths considered for this project, in precedence order.
    Path(PermissionsPathArgs),
    /// Dry-run: ask whether a proposed action would be admitted *without*
    /// hitting Discord. Useful for agents that want to pre-flight.
    Check(PermissionsCheckArgs),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsInitArgs {
    /// Write to the project-local `permissions.toml`. Default is global.
    #[arg(long)]
    pub local: bool,

    /// Overwrite any existing file at that scope.
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
    /// Function to check: `send`, `read`, `channels`, `join`, `leave`,
    /// `discover`, `manage`.
    #[arg(long)]
    pub function: String,

    /// Channel name or snowflake to test against the channel list for
    /// `send` / `read` / `join` / `leave`.
    #[arg(long, conflicts_with = "user")]
    pub channel: Option<String>,

    /// User name or snowflake to test against the DM list for `send`.
    #[arg(long, conflicts_with = "channel")]
    pub user: Option<String>,

    /// Guild name or snowflake to test against the guild list for
    /// `channels` / `discover`.
    #[arg(long)]
    pub guild: Option<String>,

    /// Body to test against `content` rules (applies only to `send`).
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

    // Pre-load to surface any compile errors up front, before printing.
    let effective = perms::load_effective()?;
    let _ = effective;

    if args.json {
        let out = PermissionsShowOutput {
            command: "discord.permissions.show",
            global: PermissionsScopeBlock {
                path: global_p.display().to_string(),
                present: global_present,
            },
            local: PermissionsScopeBlock {
                path: local_p.display().to_string(),
                present: local_present,
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
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
        println!("Run `zad discord permissions init` to scaffold a starter policy.");
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
    perms::save_file(&path, &template)?;
    if args.json {
        let out = PermissionsInitOutput {
            command: "discord.permissions.init",
            scope,
            path: path.display().to_string(),
            written: true,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Wrote starter permissions ({scope}): {}", path.display());
        println!("Review it; the defaults deny admin-like channels and channels.manage.");
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
        let out = PermissionsPathOutput {
            command: "discord.permissions.path",
            global: global_p.display().to_string(),
            local: local_p.display().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
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
        outcome = match (function, &args.channel, &args.user, &args.guild) {
            (DiscordFunction::Send, Some(c), None, _) => {
                let id = directory.resolve_channel(c, None).unwrap_or(0);
                permissions.check_send_channel(c, id, &directory)
            }
            (DiscordFunction::Send, None, Some(u), _) => {
                let id = directory.resolve_user(u).unwrap_or(0);
                permissions.check_send_dm(u, id, &directory)
            }
            (DiscordFunction::Read, Some(c), None, _) => {
                let id = directory.resolve_channel(c, None).unwrap_or(0);
                permissions.check_read_channel(c, id, &directory)
            }
            (DiscordFunction::Channels, _, _, Some(g)) => {
                let id = directory.resolve_guild(g).unwrap_or(0);
                permissions.check_channels_guild(g, id, &directory)
            }
            (DiscordFunction::Join, Some(c), None, _) => {
                let id = directory.resolve_channel(c, None).unwrap_or(0);
                permissions.check_join_channel(c, id, &directory)
            }
            (DiscordFunction::Leave, Some(c), None, _) => {
                let id = directory.resolve_channel(c, None).unwrap_or(0);
                permissions.check_leave_channel(c, id, &directory)
            }
            (DiscordFunction::Discover, _, _, Some(g)) => {
                let id = directory.resolve_guild(g).unwrap_or(0);
                permissions.check_discover_guild(g, id, &directory)
            }
            _ => Ok(()),
        };
    }

    if outcome.is_ok()
        && function == DiscordFunction::Send
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
        let out = PermissionsCheckOutput {
            command: "discord.permissions.check",
            function: args.function.clone(),
            allowed,
            reason,
            config_path,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
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

fn parse_function(name: &str) -> Result<DiscordFunction> {
    match name {
        "send" => Ok(DiscordFunction::Send),
        "read" => Ok(DiscordFunction::Read),
        "channels" => Ok(DiscordFunction::Channels),
        "join" => Ok(DiscordFunction::Join),
        "leave" => Ok(DiscordFunction::Leave),
        "discover" => Ok(DiscordFunction::Discover),
        "manage" => Ok(DiscordFunction::Manage),
        other => Err(ZadError::Invalid(format!(
            "unknown function `{other}`. Expected one of: send, read, channels, join, leave, discover, manage."
        ))),
    }
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
            "missing message body: pass it as a positional arg or use --stdin".into(),
        )),
    }
}
