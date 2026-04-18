//! `zad telegram <verb>` — runtime verbs for a configured Telegram bot.
//!
//! This file wires the CLI surface (subcommand enum, flag parsing, error
//! messages pointing at the right config files) but the actual Bot API
//! calls are still **TODO**. Every verb currently fails fast with a
//! `ZadError::Unsupported` so lifecycle work (`zad service create
//! telegram`, `enable`, `list`, `show`, `delete`) can ship first and be
//! exercised end-to-end.
//!
//! Follow-up: implement each `run_*` helper on top of a new
//! `service::telegram::TelegramHttp` (see the TODO block at the top of
//! `service/telegram/mod.rs` for the concrete plan). The CLI shape
//! should stay stable; the bodies are what change.
//!
//! ## Mapping to Discord verbs
//!
//! The verb set below deliberately mirrors `zad discord` so an agent
//! that has learned one surface can switch services without relearning
//! the shape:
//!
//! | Discord verb | Telegram equivalent | Bot API call (planned) |
//! |---|---|---|
//! | `send --channel/--dm`  | `send --chat <ID\|@handle\|name>`   | `sendMessage` |
//! | `read --channel`       | `read --chat <ID\|@handle\|name>`   | `getUpdates` / message forwarding |
//! | `channels --guild`     | `chats`                              | state snapshot built from the directory + `getChat` |
//! | `join --channel`       | `join --chat <@handle>`              | `joinChat` *(requires the bot to be invited)* |
//! | `leave --channel`      | `leave --chat <ID\|@handle>`         | `leaveChat` |
//! | `discover`             | `discover`                           | best-effort walk via `getUpdates` + `getChat` |
//! | `directory`            | `directory`                          | identical — shared directory.toml schema |
//! | `permissions`          | `permissions`                        | identical — shared permissions primitives, Telegram-specific function names |

use clap::{Args, Subcommand};

use crate::config;
use crate::error::{Result, ZadError};

// ---------------------------------------------------------------------------
// subcommand plumbing
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct TelegramArgs {
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Send a message to a chat (user, group, supergroup, or channel).
    Send(SendArgs),
    /// Read recent messages from a chat.
    Read(ReadArgs),
    /// List chats the bot is currently a member of.
    Chats(ChatsArgs),
    /// Join a public group or channel by `@username`.
    Join(JoinArgs),
    /// Leave a chat the bot is a member of.
    Leave(LeaveArgs),
    /// Best-effort walk of the bot's recent updates to cache a name ->
    /// chat_id map in this project's `directory.toml`.
    Discover(DiscoverArgs),
    /// Inspect or hand-edit the name -> chat_id directory.
    Directory(DirectoryArgs),
    /// Inspect, scaffold, or dry-run the Telegram permissions policy.
    Permissions(PermissionsArgs),
}

pub async fn run(args: TelegramArgs) -> Result<()> {
    let action = args.action.ok_or_else(|| {
        ZadError::Invalid("missing subcommand. Run `zad telegram --help`.".into())
    })?;

    // Every runtime verb requires the project to have enabled Telegram,
    // the same way `zad discord …` requires discord enablement. This
    // check runs up front so TODO stubs still surface the right error
    // when the operator hasn't completed the lifecycle dance.
    require_telegram_enabled()?;

    match action {
        Action::Send(a) => run_send(a).await,
        Action::Read(a) => run_read(a).await,
        Action::Chats(a) => run_chats(a).await,
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
    /// Destination chat. Accepts a numeric chat ID (negative for
    /// groups/channels, positive for private chats), an `@username`
    /// handle, or a name from this project's directory.
    #[arg(long)]
    pub chat: Option<String>,

    /// Read the message body from standard input instead of the
    /// positional argument.
    #[arg(long, conflicts_with = "body")]
    pub stdin: bool,

    /// Message body. Required unless `--stdin` is set.
    pub body: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,

    /// Preview the outgoing call without contacting Telegram. Scope and
    /// permission checks still run; no bot token is loaded.
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_send(_args: SendArgs) -> Result<()> {
    // TODO: Resolve --chat against directory.toml (supports names,
    //       @handles, and numeric IDs), load the permissions policy,
    //       check time/chat/body rules, then POST /bot<TOKEN>/sendMessage
    //       via TelegramHttp. Enforce Telegram's 4096-codepoint body
    //       cap locally. Use DryRunTelegramTransport when --dry-run is
    //       set so the keychain is not touched.
    Err(unimplemented("send"))
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Chat to read from. Same resolution rules as `send --chat`.
    #[arg(long)]
    pub chat: String,

    /// Maximum number of messages to fetch.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

async fn run_read(_args: ReadArgs) -> Result<()> {
    // TODO: Telegram does not expose a "get messages by chat" endpoint
    //       for bots. The plan is to long-poll `getUpdates`, filter by
    //       chat_id, and buffer up to `limit` messages. For chats the
    //       bot admins, a later enhancement can use the
    //       `forwardMessages` + `copyMessages` endpoints or the Bot API
    //       7.x message-history extensions where available.
    Err(unimplemented("read"))
}

// ---------------------------------------------------------------------------
// chats
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ChatsArgs {
    /// Emit machine-readable JSON instead of the human-readable table.
    #[arg(long)]
    pub json: bool,
}

async fn run_chats(_args: ChatsArgs) -> Result<()> {
    // TODO: Compose a snapshot from (a) the project directory and (b)
    //       a fresh `getChat` per known chat_id. There is no
    //       list-all-chats endpoint in the Bot API — this command is
    //       primarily a view over whatever `discover` has cached.
    Err(unimplemented("chats"))
}

// ---------------------------------------------------------------------------
// join / leave
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct JoinArgs {
    /// Chat to join. Must be an `@username` handle for a public group
    /// or channel — the Bot API does not let bots accept private invite
    /// links.
    #[arg(long)]
    pub chat: String,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,

    /// Preview the outgoing call without contacting Telegram.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct LeaveArgs {
    /// Chat to leave. Accepts a numeric chat ID, an `@username` handle,
    /// or a directory name.
    #[arg(long)]
    pub chat: String,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,

    /// Preview the outgoing call without contacting Telegram.
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_join(_args: JoinArgs) -> Result<()> {
    // TODO: POST /bot<TOKEN>/joinChat with the resolved @username.
    Err(unimplemented("join"))
}

async fn run_leave(_args: LeaveArgs) -> Result<()> {
    // TODO: POST /bot<TOKEN>/leaveChat with the resolved chat_id.
    Err(unimplemented("leave"))
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Skip the user-enumeration phase (only useful once we walk
    /// `getChatAdministrators` for known chats).
    #[arg(long)]
    pub skip_users: bool,

    /// Emit machine-readable JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

async fn run_discover(_args: DiscoverArgs) -> Result<()> {
    // TODO: Walk `getUpdates` with offset=0 to harvest every chat and
    //       user the bot has seen, then optionally call
    //       `getChatAdministrators` per known group/channel. Write the
    //       resulting name -> chat_id / user_id map into this project's
    //       `directory.toml`, merged on top of existing entries.
    //       This is explicitly best-effort: Telegram's `getUpdates`
    //       window is bounded (~24h by default) so the command should
    //       be safe to re-run and should preserve hand-authored keys.
    Err(unimplemented("discover"))
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
    /// Upsert a name -> chat_id/user_id mapping. `<kind>` is one of
    /// `chat` or `user`.
    Set(DirectorySetArgs),
    /// Remove a single mapping. Silent no-op if the key is absent.
    Remove(DirectoryRemoveArgs),
    /// Wipe every entry. Use with `--force`.
    Clear(DirectoryClearArgs),
}

#[derive(Debug, Args)]
pub struct DirectorySetArgs {
    /// One of `chat` or `user`.
    pub kind: DirectoryKind,
    /// Human-readable name to map from.
    pub name: String,
    /// Numeric chat_id / user_id (Telegram allows negative IDs for
    /// groups and channels, so this is parsed as a signed integer).
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
    Chat,
    User,
}

fn run_directory(_args: DirectoryArgs) -> Result<()> {
    // TODO: Re-use `config::directory` (the same shared name -> id
    //       store Discord writes to). The only telegram-specific
    //       concern is that chat IDs are signed 64-bit integers, so
    //       the parser here should accept leading `-` (unlike Discord
    //       snowflakes which are unsigned). Writes should preserve
    //       any hand-authored keys, same as the Discord directory
    //       command.
    Err(unimplemented("directory"))
}

// ---------------------------------------------------------------------------
// permissions
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
    /// Print the paths considered for this project, in precedence
    /// order.
    Path(PermissionsPathArgs),
    /// Dry-run: ask whether a proposed action would be admitted
    /// *without* hitting Telegram.
    Check(PermissionsCheckArgs),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsInitArgs {
    /// Write to the project-local `permissions.toml`. Default is
    /// global.
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
    /// Function to check: `send`, `read`, `chats`, `join`, `leave`,
    /// `discover`, `manage`.
    #[arg(long)]
    pub function: String,

    /// Chat name, @handle, or chat_id to test against the chat list
    /// for `send` / `read` / `join` / `leave`.
    #[arg(long, conflicts_with = "user")]
    pub chat: Option<String>,

    /// User name or user_id to test against the user list for `send`.
    #[arg(long, conflicts_with = "chat")]
    pub user: Option<String>,

    /// Body to test against `content` rules (applies only to `send`).
    #[arg(long)]
    pub body: Option<String>,

    #[arg(long)]
    pub json: bool,
}

fn run_permissions(_args: PermissionsArgs) -> Result<()> {
    // TODO: Ship a `service/telegram/permissions.rs` built on the
    //       generic `permissions::{pattern, content, time}` primitives,
    //       with per-function blocks for `send`, `read`, `chats`,
    //       `join`, `leave`, `discover`, and `manage`. The starter
    //       template should deny public-channel sends and any
    //       `manage`-level action unless explicitly allowed, matching
    //       the safe defaults the Discord template ships with.
    Err(unimplemented("permissions"))
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

fn require_telegram_enabled() -> Result<()> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("telegram") {
        return Err(ZadError::Invalid(format!(
            "telegram is not enabled for this project ({}). \
             Run `zad service enable telegram` first.",
            project_path.display()
        )));
    }
    Ok(())
}

/// Uniform "not implemented yet" error so every stubbed verb points the
/// operator at the same follow-up change.
fn unimplemented(verb: &'static str) -> ZadError {
    ZadError::Unsupported(match verb {
        "send" => "telegram: `send` is not implemented yet — see TODO in src/cli/telegram.rs",
        "read" => "telegram: `read` is not implemented yet — see TODO in src/cli/telegram.rs",
        "chats" => "telegram: `chats` is not implemented yet — see TODO in src/cli/telegram.rs",
        "join" => "telegram: `join` is not implemented yet — see TODO in src/cli/telegram.rs",
        "leave" => "telegram: `leave` is not implemented yet — see TODO in src/cli/telegram.rs",
        "discover" => {
            "telegram: `discover` is not implemented yet — see TODO in src/cli/telegram.rs"
        }
        "directory" => {
            "telegram: `directory` is not implemented yet — see TODO in src/cli/telegram.rs"
        }
        "permissions" => {
            "telegram: `permissions` is not implemented yet — see TODO in src/cli/telegram.rs"
        }
        _ => "telegram: not implemented yet",
    })
}
