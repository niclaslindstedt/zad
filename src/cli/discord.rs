//! `zad discord <verb>` — runtime commands against a configured Discord
//! bot. Credential resolution mirrors `zad service enable discord`: the
//! project-local config wins over the global one, and the matching
//! keychain entry holds the bot token. The project must already have
//! enabled the Discord service.

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config::{self, DiscordServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::discord::DiscordHttp;
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
    let target = match (&args.channel, &args.dm) {
        (Some(c), None) => Target::Channel(ChannelId(parse_snowflake(c, "--channel")?)),
        (None, Some(u)) => Target::Dm(UserId(parse_snowflake(u, "--dm")?)),
        (None, None) => {
            return Err(ZadError::Invalid(
                "missing destination: pass --channel <ID> or --dm <USER_ID>".into(),
            ));
        }
        (Some(_), Some(_)) => unreachable!("clap enforces mutual exclusion"),
    };

    let body = resolve_body(args.body.as_deref(), args.stdin)?;
    let http = discord_http()?;
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
    let channel_id = ChannelId(parse_snowflake(&args.channel, "--channel")?);
    if args.limit == 0 {
        return Err(ZadError::Invalid("--limit must be at least 1".into()));
    }
    let http = discord_http()?;
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
    let guild = resolve_guild(args.guild.as_deref(), cfg.default_guild.as_deref())?;
    let http = discord_http()?;
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
    let channel = ChannelId(parse_snowflake(&args.channel, "--channel")?);
    let http = discord_http()?;
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
    let channel = ChannelId(parse_snowflake(&args.channel, "--channel")?);
    let http = discord_http()?;
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

fn discord_http() -> Result<DiscordHttp> {
    let (_cfg, scope) = effective_config()?;
    let token = load_token(&scope)?;
    Ok(DiscordHttp::new(&token))
}

fn resolve_guild(flag: Option<&str>, default: Option<&str>) -> Result<u64> {
    let raw = flag.or(default).ok_or_else(|| {
        ZadError::Invalid(
            "no guild specified: pass --guild <ID> or set `default_guild` in the config".into(),
        )
    })?;
    parse_snowflake(raw, "--guild")
}

fn parse_snowflake(v: &str, field: &'static str) -> Result<u64> {
    v.parse::<u64>().map_err(|_| {
        ZadError::Invalid(format!(
            "{field} must be a numeric Discord snowflake, got `{v}`"
        ))
    })
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
