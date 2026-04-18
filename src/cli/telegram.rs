//! `zad telegram <verb>` — runtime commands against a configured
//! Telegram bot.
//!
//! Credential resolution mirrors `zad service enable telegram`: the
//! project-local config wins over the global one, and the matching
//! keychain entry holds the bot token. The project must already have
//! enabled the Telegram service.
//!
//! ## Implementation status
//!
//! The **clap surface** here is complete — subcommand names, flags,
//! help text, and the manpage are all real so `zad --help`, `zad man
//! telegram`, and `zad commands` advertise the planned interface
//! today. The **runtime bodies** (`run_send`, `run_read`, …) are
//! stubbed: each returns `ZadError::Invalid("... not yet implemented
//! ...")` and carries a block-comment describing the Bot API call it
//! should make. The stub shape is grep-friendly: search for
//! `not_yet_implemented(` to find every gap.
//!
//! The three exceptions are `directory` and `permissions` — those are
//! local-state operations (no network I/O) and are implemented
//! end-to-end. An operator can scaffold a permissions policy and
//! hand-populate chat aliases today, before the send/read verbs
//! land.

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config;
use crate::error::{Result, ZadError};
use crate::service::telegram::directory::{self as dir, Directory};
use crate::service::telegram::permissions::{self as perms, TelegramFunction};

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
    /// Send a message to a chat. [PLANNED — clap surface only]
    Send(SendArgs),
    /// Fetch recent messages from a chat. [PLANNED — clap surface only]
    Read(ReadArgs),
    /// List chats the bot has seen. [PLANNED — clap surface only]
    Chats(ChatsArgs),
    /// Poll the Bot API for recent updates and cache chat aliases.
    /// [PLANNED — clap surface only]
    Discover(DiscoverArgs),
    /// Inspect or hand-edit the name -> chat_id directory. [IMPLEMENTED]
    Directory(DirectoryArgs),
    /// Inspect, scaffold, or dry-run the permissions policy.
    /// [IMPLEMENTED]
    Permissions(PermissionsArgs),
}

pub async fn run(args: TelegramArgs) -> Result<()> {
    let action = args.action.ok_or_else(|| {
        ZadError::Invalid("missing subcommand. Run `zad telegram --help`.".into())
    })?;
    match action {
        Action::Send(a) => run_send(a).await,
        Action::Read(a) => run_read(a).await,
        Action::Chats(a) => run_chats(a).await,
        Action::Discover(a) => run_discover(a).await,
        Action::Directory(a) => run_directory(a),
        Action::Permissions(a) => run_permissions(a),
    }
}

/// Stub error shape shared by every runtime verb that's still to be
/// implemented. Grep for call-sites (`not_yet_implemented(`) to find
/// every gap.
fn not_yet_implemented(verb: &str) -> ZadError {
    ZadError::Invalid(format!(
        "`zad telegram {verb}` is not yet implemented; \
         see the TODO block in src/cli/telegram.rs::run_{verb} \
         and src/service/telegram/transport.rs"
    ))
}

// ---------------------------------------------------------------------------
// send [stub]
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Destination chat: a signed integer chat_id (groups/supergroups
    /// are negative), a `@username` for public channels, or a
    /// directory alias.
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

    /// Preview the outgoing call without contacting the Bot API.
    /// Scope and permission checks still run; no bot token is loaded.
    #[arg(long)]
    pub dry_run: bool,
}

async fn run_send(_args: SendArgs) -> Result<()> {
    // TODO: end-to-end outline (mirror `src/cli/discord.rs::run_send`):
    //   1. `effective_config()` → TelegramServiceCfg + scope
    //   2. Load directory + permissions
    //   3. `permissions.check_time(TelegramFunction::Send)`
    //   4. Resolve `--chat` (or default_chat) to a signed chat_id
    //   5. `permissions.check_send_chat(input, id, &directory)`
    //   6. Resolve the body (positional / stdin), enforce
    //      `TELEGRAM_MAX_MESSAGE_LEN`
    //   7. `permissions.check_send_body(&body)`
    //   8. `telegram_http_for("messages.send", args.dry_run)`
    //   9. `transport.send(chat_id, &body).await`
    //  10. Print human / JSON output, suppress "sent …" under
    //      `--dry-run` (the sink already emitted the preview record).
    Err(not_yet_implemented("send"))
}

// ---------------------------------------------------------------------------
// read [stub]
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Chat to read from. Same accepted formats as `--chat` on `send`.
    #[arg(long)]
    pub chat: String,

    /// Maximum number of messages to fetch. Defaults to 20.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

async fn run_read(_args: ReadArgs) -> Result<()> {
    // TODO: The Bot API's `getUpdates` is long-poll and forward-only
    // — it returns new updates since the last call, never historical
    // backfill. First cut will call `getUpdates` with a short timeout
    // and filter client-side to messages matching `--chat`. Document
    // the "new messages only" caveat in the manpage. Scope:
    // `messages.read`.
    Err(not_yet_implemented("read"))
}

// ---------------------------------------------------------------------------
// chats [stub]
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ChatsArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

async fn run_chats(_args: ChatsArgs) -> Result<()> {
    // TODO: no "list every chat the bot is in" endpoint exists. First
    // cut reads from the local directory (`dir::load()`), optionally
    // supplemented with chats observed in a short `getUpdates` poll.
    // Scope: `chats`.
    Err(not_yet_implemented("chats"))
}

// ---------------------------------------------------------------------------
// discover [stub]
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Emit machine-readable JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

async fn run_discover(_args: DiscoverArgs) -> Result<()> {
    // TODO: call `getUpdates` once with a short timeout, extract every
    // `Chat` from the envelope's `message.chat`, `my_chat_member.chat`,
    // `channel_post.chat`, etc., and upsert `(title, id)` pairs into
    // the directory. Hand-authored entries must round-trip untouched.
    // Scope: `chats`.
    Err(not_yet_implemented("discover"))
}

// ---------------------------------------------------------------------------
// credential / config plumbing [partial — only the shape used by the
// IMPLEMENTED verbs below is live; the runtime stubs will extend it]
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

// TODO: when send/read/chats/discover land, add:
//   enum EffectiveScope { Global, Local(String) }
//   fn effective_config() -> Result<(TelegramServiceCfg, EffectiveScope)>
//   fn load_token(scope: &EffectiveScope) -> Result<String>
//   fn telegram_http_for(required: &'static str, dry_run: bool)
//       -> Result<Box<dyn TelegramTransport>>
// mirroring the Discord helpers. The `telegram` keychain account is
// `secrets::account("telegram", "bot", scope)`.

// ---------------------------------------------------------------------------
// directory [implemented]
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
    /// Upsert a name -> chat_id mapping.
    Set(DirectorySetArgs),
    /// Remove a single mapping. Silent no-op if the key is absent.
    Remove(DirectoryRemoveArgs),
    /// Wipe every entry. Use with `--force`.
    Clear(DirectoryClearArgs),
}

#[derive(Debug, Args)]
pub struct DirectorySetArgs {
    /// Human-readable name to map from.
    pub name: String,
    /// Signed chat_id (groups/supergroups are negative).
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DirectoryRemoveArgs {
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

#[derive(Debug, Serialize)]
struct DirectoryOutput<'a> {
    command: &'static str,
    path: String,
    generated_at_unix: Option<u64>,
    chats: &'a std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct DirectoryMutation {
    command: &'static str,
    name: String,
    id: Option<String>,
    removed: bool,
}

fn run_directory(args: DirectoryArgs) -> Result<()> {
    require_telegram_enabled()?;
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
            command: "telegram.directory",
            path: path.display().to_string(),
            generated_at_unix: directory.generated_at_unix,
            chats: &directory.chats,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    if directory.total() == 0 {
        println!("(empty) {}", path.display());
        println!("Run `zad telegram discover` to populate it (once implemented),");
        println!("or add entries manually with `zad telegram directory set <name> <id>`.");
        return Ok(());
    }
    println!("# {}", path.display());
    if !directory.chats.is_empty() {
        println!("\n[chats]");
        for (n, id) in &directory.chats {
            println!("  {n:<32}  {id}");
        }
    }
    Ok(())
}

fn run_directory_set(args: DirectorySetArgs) -> Result<()> {
    let id = parse_chat_id(&args.id)?;
    let path = dir::path_current()?;
    let mut directory = dir::load_from(&path)?;
    directory.chats.insert(args.name.clone(), id.to_string());
    dir::save_to(&path, &directory)?;

    if args.json {
        let out = DirectoryMutation {
            command: "telegram.directory.set",
            name: args.name,
            id: Some(id.to_string()),
            removed: false,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Mapped chat `{}` -> {id} in {}.", args.name, path.display());
    }
    Ok(())
}

fn run_directory_remove(args: DirectoryRemoveArgs) -> Result<()> {
    let path = dir::path_current()?;
    let mut directory = dir::load_from(&path)?;
    let removed = directory.chats.remove(&args.name).is_some();
    if removed {
        dir::save_to(&path, &directory)?;
    }

    if args.json {
        let out = DirectoryMutation {
            command: "telegram.directory.remove",
            name: args.name,
            id: None,
            removed,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if removed {
        println!("Removed chat `{}` from {}.", args.name, path.display());
    } else {
        println!("No chat entry named `{}`.", args.name);
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
                "command": "telegram.directory.clear",
                "path": path.display().to_string(),
            }))
            .unwrap()
        );
    } else {
        println!("Cleared {}.", path.display());
    }
    Ok(())
}

fn parse_chat_id(v: &str) -> Result<i64> {
    v.parse::<i64>().map_err(|_| {
        ZadError::Invalid(format!(
            "<id> must be a signed decimal chat_id (groups are negative), got `{v}`"
        ))
    })
}

// ---------------------------------------------------------------------------
// permissions [implemented]
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
    /// *without* hitting the Bot API. Useful for agents that want to
    /// pre-flight.
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
    /// Function to check: `send`, `read`, `chats`, `discover`.
    #[arg(long)]
    pub function: String,

    /// Chat to test against the chats list.
    #[arg(long)]
    pub chat: Option<String>,

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

    // Pre-load to surface any compile errors up front, before
    // printing.
    let effective = perms::load_effective()?;
    let _ = effective;

    if args.json {
        let out = PermissionsShowOutput {
            command: "telegram.permissions.show",
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
        println!("Run `zad telegram permissions init` to scaffold a starter policy.");
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
            command: "telegram.permissions.init",
            scope,
            path: path.display().to_string(),
            written: true,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Wrote starter permissions ({scope}): {}", path.display());
        println!("Review it; the defaults deny admin-like chats.");
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
            command: "telegram.permissions.path",
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

    if outcome.is_ok()
        && let Some(c) = &args.chat
    {
        let id = directory.resolve_chat(c).unwrap_or(0);
        outcome = match function {
            TelegramFunction::Send => permissions.check_send_chat(c, id, &directory),
            TelegramFunction::Read => permissions.check_read_chat(c, id, &directory),
            TelegramFunction::Chats => permissions.check_chats_chat(c, id, &directory),
            TelegramFunction::Discover => permissions.check_discover_chat(c, id, &directory),
        };
    }

    if outcome.is_ok()
        && function == TelegramFunction::Send
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
            command: "telegram.permissions.check",
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

fn parse_function(name: &str) -> Result<TelegramFunction> {
    match name {
        "send" => Ok(TelegramFunction::Send),
        "read" => Ok(TelegramFunction::Read),
        "chats" => Ok(TelegramFunction::Chats),
        "discover" => Ok(TelegramFunction::Discover),
        other => Err(ZadError::Invalid(format!(
            "unknown function `{other}`. Expected one of: send, read, chats, discover."
        ))),
    }
}
