//! `zad telegram <verb>` — runtime commands against a configured
//! Telegram bot.
//!
//! Credential resolution mirrors `zad service enable telegram`: the
//! project-local config wins over the global one, and the matching
//! keychain entry holds the bot token. The project must already have
//! enabled the Telegram service.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config::{self, TelegramServiceCfg};
use crate::error::{Result, ZadError};
use crate::permissions::attachments::AttachmentInfo;
use crate::secrets::{self, Scope};
use crate::service::default_dry_run_sink;
use crate::service::telegram::client::{
    TELEGRAM_MAX_CAPTION_LEN, TELEGRAM_MAX_MEDIA_GROUP, TELEGRAM_MAX_MESSAGE_LEN,
};
use crate::service::telegram::directory::{self as dir, Directory};
use crate::service::telegram::permissions::{self as perms, TelegramFunction};
use crate::service::telegram::{DryRunTelegramTransport, TelegramHttp, TelegramTransport};

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
    /// Send a message to a chat (private, group, supergroup, or channel).
    Send(SendArgs),
    /// Fetch recent messages the bot has buffered for a chat.
    Read(ReadArgs),
    /// List chats the bot has seen (local directory + recent updates).
    Chats(ChatsArgs),
    /// Poll the Bot API for recent updates and upsert chat aliases
    /// into this project's `directory.toml`.
    Discover(DiscoverArgs),
    /// Inspect or hand-edit the name -> chat_id directory.
    Directory(DirectoryArgs),
    /// Inspect, scaffold, or dry-run the permissions policy that
    /// narrows what this service may actually do.
    Permissions(PermissionsArgs),
    /// Manage the private-chat ID resolved from the literal `@me` in
    /// send/read targets. Capture (by polling for your first message
    /// to the bot), show, set, or clear.
    #[command(name = "self")]
    SelfCmd(SelfArgs),
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
        Action::SelfCmd(a) => run_self(a).await,
    }
}

// ---------------------------------------------------------------------------
// send
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

    /// Attach a file to the message. Repeat up to Telegram's
    /// `sendMediaGroup` cap of 10 to attach multiple files. With one
    /// file the message is sent via `sendDocument`; with 2+ files it
    /// becomes a `sendMediaGroup`. The body (if any) is sent as the
    /// caption on the first item; with attachments present Telegram's
    /// 1024-character caption cap applies instead of the 4096-character
    /// plain-text cap.
    #[arg(long = "file", value_name = "PATH", action = clap::ArgAction::Append)]
    pub files: Vec<PathBuf>,

    /// Message body. Required unless `--stdin` is set or at least one
    /// `--file` is attached.
    pub body: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,

    /// Preview the outgoing call without contacting the Bot API.
    /// Scope and permission checks still run; no bot token is loaded.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
struct SendOutput {
    command: &'static str,
    chat_id: String,
    message_id: String,
}

async fn run_send(args: SendArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let permissions = perms::load_effective()?;
    permissions.check_time(TelegramFunction::Send)?;

    let (chat_input, chat_id) = resolve_chat_arg(
        args.chat.as_deref(),
        cfg.default_chat.as_deref(),
        cfg.self_chat_id,
        &directory,
    )?;
    permissions.check_send_chat(&chat_input, chat_id, &directory)?;

    let body = if args.files.is_empty() {
        resolve_body(args.body.as_deref(), args.stdin)?
    } else {
        resolve_body_or_empty(args.body.as_deref(), args.stdin)?
    };
    let len = body.chars().count();
    // Bot API caption cap (1024) is stricter than the plain-text cap
    // (4096); when attachments are present the body rides as the
    // caption, so narrow the check accordingly.
    let body_cap = if args.files.is_empty() {
        TELEGRAM_MAX_MESSAGE_LEN
    } else {
        TELEGRAM_MAX_CAPTION_LEN
    };
    if len > body_cap {
        let label = if args.files.is_empty() {
            "hard limit"
        } else {
            "caption cap (attachments present)"
        };
        return Err(ZadError::Invalid(format!(
            "message body is {len} characters; Telegram's {label} is {body_cap}"
        )));
    }
    if args.files.len() > TELEGRAM_MAX_MEDIA_GROUP {
        return Err(ZadError::Invalid(format!(
            "{} attachments is above Telegram's per-message cap of {TELEGRAM_MAX_MEDIA_GROUP}",
            args.files.len()
        )));
    }
    permissions.check_send_body(&body)?;

    let infos: Vec<AttachmentInfo> = args
        .files
        .iter()
        .map(|p| {
            AttachmentInfo::probe(p).map_err(|e| {
                ZadError::Invalid(format!("attachment `{}` not readable: {e}", p.display()))
            })
        })
        .collect::<Result<_>>()?;
    permissions.check_send_attachments(&infos)?;

    let http = telegram_http_for("messages.send", args.dry_run)?;
    let message_id = http.send(chat_id, &body, &args.files).await?;

    // When --dry-run is active the transport already emitted a preview
    // record (human summary via `tracing::info!`, JSON payload on
    // stdout). Skip the trailing "Sent …" / SendOutput print so we
    // never claim success for an operation we didn't actually perform.
    if args.dry_run {
        return Ok(());
    }

    if args.json {
        let out = SendOutput {
            command: "telegram.send",
            chat_id: chat_id.to_string(),
            message_id: message_id.to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Sent message {message_id} to chat {chat_id}.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// Chat to read from. Same accepted formats as `--chat` on `send`.
    #[arg(long)]
    pub chat: String,

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
    chat_id: String,
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
            "--limit must be between 1 and 100".into(),
        ));
    }
    let (cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let permissions = perms::load_effective()?;
    permissions.check_time(TelegramFunction::Read)?;

    let (chat_input, chat_id) =
        resolve_chat_arg(Some(&args.chat), None, cfg.self_chat_id, &directory)?;
    permissions.check_read_chat(&chat_input, chat_id, &directory)?;

    let http = telegram_http_for("messages.read", false)?;
    let msgs = http.history(chat_id, args.limit).await?;

    if args.json {
        let out = ReadOutput {
            command: "telegram.read",
            chat_id: chat_id.to_string(),
            count: msgs.len(),
            messages: msgs
                .iter()
                .map(|m| ReadMessage {
                    id: m.id.to_string(),
                    author: m.author.clone(),
                    body: m.body.clone(),
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    if msgs.is_empty() {
        println!("(no messages — `getUpdates` is forward-only; see `zad man telegram`)");
        return Ok(());
    }
    // Print oldest-first so a human reads top-to-bottom in chronological
    // order. `history` returned newest-first.
    for m in msgs.iter().rev() {
        println!("[{}] <{}> {}", m.id, m.author, m.body);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// chats
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct ChatsArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct ChatsOutput {
    command: &'static str,
    count: usize,
    chats: Vec<ChatRow>,
}

#[derive(Debug, Serialize)]
struct ChatRow {
    id: String,
    title: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    source: &'static str,
}

async fn run_chats(args: ChatsArgs) -> Result<()> {
    let (_cfg, _scope) = effective_config()?;
    let directory = dir::load().unwrap_or_default();
    let permissions = perms::load_effective()?;
    permissions.check_time(TelegramFunction::Chats)?;

    let http = telegram_http_for("chats", false)?;
    let observed = http.list_chats().await?;

    // Merge observed chats with the local directory cache so an
    // operator sees every chat zad knows about, not just the ones
    // whose updates happen to be buffered right now. Observed entries
    // override directory rows when they share an id so kind/username
    // come from the live data where possible.
    let mut by_id: std::collections::BTreeMap<i64, ChatRow> = std::collections::BTreeMap::new();
    for (name, id_s) in &directory.chats {
        if let Ok(id) = id_s.parse::<i64>() {
            by_id.entry(id).or_insert_with(|| ChatRow {
                id: id.to_string(),
                title: name.clone(),
                kind: "unknown".into(),
                username: None,
                source: "directory",
            });
        }
    }
    for c in &observed {
        by_id.insert(
            c.id,
            ChatRow {
                id: c.id.to_string(),
                title: c.title.clone(),
                kind: c.kind.clone(),
                username: c.username.clone(),
                source: "observed",
            },
        );
    }
    let rows: Vec<ChatRow> = by_id.into_values().collect();

    if args.json {
        let out = ChatsOutput {
            command: "telegram.chats",
            count: rows.len(),
            chats: rows,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    if rows.is_empty() {
        println!("(no chats — run `zad telegram discover` once the bot has seen traffic)");
        return Ok(());
    }
    println!("{:<20}  {:<10}  {:<10}  TITLE", "ID", "KIND", "SOURCE");
    for r in &rows {
        println!(
            "{:<20}  {:<10}  {:<10}  {}",
            r.id, r.kind, r.source, r.title
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Emit machine-readable JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct DiscoverOutput {
    command: &'static str,
    chats: usize,
    added: usize,
    skipped: usize,
    warnings: Vec<String>,
}

async fn run_discover(args: DiscoverArgs) -> Result<()> {
    let (_cfg, _scope) = effective_config()?;
    let permissions = perms::load_effective()?;
    permissions.check_time(TelegramFunction::Discover)?;

    let http = telegram_http_for("chats", false)?;
    let observed = http.list_chats().await?;

    let mut directory = dir::load().unwrap_or_default();
    let mut added = 0usize;
    let mut skipped = 0usize;
    let warnings: Vec<String> = vec![];

    for c in &observed {
        // Silently skip chats the policy denies from discovery — the
        // walk is best-effort and shouldn't fail the whole call.
        if permissions
            .check_discover_chat(&c.title, c.id, &directory)
            .is_err()
        {
            skipped += 1;
            continue;
        }
        let key = c.title.clone();
        let id_s = c.id.to_string();
        match directory.chats.get(&key) {
            Some(existing) if existing == &id_s => {}
            _ => {
                directory.chats.insert(key, id_s);
                added += 1;
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

    if args.json {
        let out = DiscoverOutput {
            command: "telegram.discover",
            chats: observed.len(),
            added,
            skipped,
            warnings: warnings.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        let total = observed.len();
        println!("Observed {total} chat(s); added {added}, skipped {skipped} (denied by policy).");
        for w in &warnings {
            crate::output::warn(w);
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

fn effective_config() -> Result<(TelegramServiceCfg, EffectiveScope)> {
    require_telegram_enabled()?;

    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "telegram")?;
    if let Some(cfg) = config::load_flat::<TelegramServiceCfg>(&local_path)? {
        return Ok((cfg, EffectiveScope::Local(slug)));
    }
    let global_path = config::path::global_service_config_path("telegram")?;
    if let Some(cfg) = config::load_flat::<TelegramServiceCfg>(&global_path)? {
        return Ok((cfg, EffectiveScope::Global));
    }
    Err(ZadError::Invalid(format!(
        "no Telegram credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_token(scope: &EffectiveScope) -> Result<String> {
    let account = match scope {
        EffectiveScope::Global => secrets::account("telegram", "bot", Scope::Global),
        EffectiveScope::Local(slug) => secrets::account("telegram", "bot", Scope::Project(slug)),
    };
    secrets::load(&account)?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "bot token missing from keychain (account `{account}`). \
             Re-run `zad service create telegram` to reinstall it."
        ))
    })
}

/// Resolve config + token + scope set into a ready-to-call transport,
/// failing fast with [`ZadError::ScopeDenied`] if `required` isn't
/// declared. The fail-fast scope check happens *before* the keychain
/// read, so a denied op never touches secrets; [`TelegramHttp`] also
/// guards the same scope internally for library-level callers.
///
/// When `dry_run` is `true` the scope check still runs (so preview
/// respects the caller's policy boundary), but the keychain read is
/// skipped and a [`DryRunTelegramTransport`] is returned instead of a
/// live client. That lets `--dry-run` work before the operator has
/// configured a bot, and guarantees no token is ever loaded into
/// memory for a preview.
fn telegram_http_for(required: &'static str, dry_run: bool) -> Result<Box<dyn TelegramTransport>> {
    let (cfg, scope) = effective_config()?;
    let config_path = match &scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "telegram")?
        }
        EffectiveScope::Global => config::path::global_service_config_path("telegram")?,
    };
    let scopes: std::collections::BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    if !scopes.contains(required) {
        return Err(ZadError::ScopeDenied {
            service: "telegram",
            scope: required,
            config_path,
        });
    }
    if dry_run {
        return Ok(Box::new(DryRunTelegramTransport::new(
            default_dry_run_sink(),
        )));
    }
    let token = load_token(&scope)?;
    Ok(Box::new(TelegramHttp::new(&token, scopes, config_path)))
}

fn resolve_chat_arg(
    flag: Option<&str>,
    default: Option<&str>,
    self_chat_id: Option<i64>,
    directory: &Directory,
) -> Result<(String, i64)> {
    let raw = flag.or(default).ok_or_else(|| {
        ZadError::Invalid(
            "no chat specified: pass --chat <ID|@username|name> or set `default_chat` in the config"
                .into(),
        )
    })?;
    if raw.eq_ignore_ascii_case("@me") {
        return match self_chat_id {
            Some(id) => Ok((raw.to_string(), id)),
            None => Err(ZadError::Invalid(
                "`@me` has no self-chat configured. Run `zad telegram self capture` \
                 to poll for your first message to the bot, or \
                 `zad telegram self set <id>` if you already know the id."
                    .into(),
            )),
        };
    }
    let id = directory.resolve_chat(raw).ok_or_else(|| {
        let key = raw.strip_prefix('@').unwrap_or(raw);
        ZadError::Invalid(format!(
            "--chat `{raw}` is neither a chat_id nor a known directory entry. \
             Run `zad telegram discover` or map it manually with \
             `zad telegram directory set {key} <id>`."
        ))
    })?;
    Ok((raw.to_string(), id))
}

fn resolve_body(positional: Option<&str>, from_stdin: bool) -> Result<String> {
    resolve_body_inner(positional, from_stdin, false)
}

/// Same as [`resolve_body`] but tolerates an empty result, for send
/// paths that carry at least one attachment (the caption on a
/// `sendDocument` / `sendMediaGroup` is optional).
fn resolve_body_or_empty(positional: Option<&str>, from_stdin: bool) -> Result<String> {
    resolve_body_inner(positional, from_stdin, true)
}

fn resolve_body_inner(
    positional: Option<&str>,
    from_stdin: bool,
    allow_empty: bool,
) -> Result<String> {
    if from_stdin {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            ZadError::Invalid(format!("failed to read message body from stdin: {e}"))
        })?;
        let trimmed = buf.trim_end_matches(['\n', '\r']).to_string();
        if trimmed.is_empty() && !allow_empty {
            return Err(ZadError::Invalid("message body is empty (stdin)".into()));
        }
        return Ok(trimmed);
    }
    match positional {
        Some(b) if !b.is_empty() => Ok(b.to_string()),
        Some(_) if allow_empty => Ok(String::new()),
        None if allow_empty => Ok(String::new()),
        _ => Err(ZadError::Invalid(
            "missing message body: pass it as a positional arg, --stdin, or attach at least one --file".into(),
        )),
    }
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
    /// Print the paths considered for this project, in precedence
    /// order.
    Path(PermissionsPathArgs),
    /// Dry-run: ask whether a proposed action would be admitted
    /// *without* hitting the Bot API. Useful for agents that want to
    /// pre-flight.
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
    let key = crate::permissions::signing::load_or_create_from_keychain()?;
    crate::permissions::signing::write_public_key_cache(&key)?;
    perms::save_file(&path, &template, &key)?;
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
        println!("Signed with key {}.", key.fingerprint());
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

// ---------------------------------------------------------------------------
// self — manage the `@me` resolution target
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SelfArgs {
    #[command(subcommand)]
    pub action: Option<SelfAction>,

    /// When no subcommand is given, behave like `show`.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum SelfAction {
    /// Print the stored self-chat ID (or note that it's not set).
    Show(SelfShowArgs),
    /// Set the self-chat ID directly. No validation — use `capture`
    /// for a validated setup.
    Set(SelfSetArgs),
    /// Clear the stored self-chat ID.
    Clear(SelfClearArgs),
    /// Poll `getUpdates` for up to 60s waiting for your first message
    /// to the bot, then store that private-chat ID.
    Capture(SelfCaptureArgs),
}

#[derive(Debug, Args)]
pub struct SelfShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfSetArgs {
    /// Your private-chat ID (a signed integer).
    pub chat_id: i64,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfClearArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SelfCaptureArgs {
    /// Don't open the bot's `https://t.me/<username>` link in the
    /// system browser. The link is still printed either way.
    #[arg(long)]
    pub no_browser: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Serialize)]
struct SelfOutput {
    command: &'static str,
    self_chat_id: Option<i64>,
}

async fn run_self(args: SelfArgs) -> Result<()> {
    match args.action {
        None => run_self_show(SelfShowArgs { json: args.json }),
        Some(SelfAction::Show(a)) => run_self_show(a),
        Some(SelfAction::Set(a)) => run_self_set(a),
        Some(SelfAction::Clear(a)) => run_self_clear(a),
        Some(SelfAction::Capture(a)) => run_self_capture(a).await,
    }
}

fn run_self_show(args: SelfShowArgs) -> Result<()> {
    let (cfg, _scope) = effective_config()?;
    emit_self(args.json, "telegram.self.show", cfg.self_chat_id)
}

fn run_self_set(args: SelfSetArgs) -> Result<()> {
    let (mut cfg, scope) = effective_config()?;
    cfg.self_chat_id = Some(args.chat_id);
    save_effective_config(&cfg, &scope)?;
    emit_self(args.json, "telegram.self.set", cfg.self_chat_id)
}

fn run_self_clear(args: SelfClearArgs) -> Result<()> {
    let (mut cfg, scope) = effective_config()?;
    cfg.self_chat_id = None;
    save_effective_config(&cfg, &scope)?;
    emit_self(args.json, "telegram.self.clear", None)
}

async fn run_self_capture(args: SelfCaptureArgs) -> Result<()> {
    let (mut cfg, scope) = effective_config()?;
    let token = load_token(&scope)?;
    let client = TelegramHttp::unscoped(&token);
    let identity = client.get_me().await?;
    let captured =
        crate::cli::service_telegram::capture_self_chat(&client, &identity, !args.no_browser)
            .await?;
    match captured {
        Some(c) => {
            cfg.self_chat_id = Some(c.chat_id);
            save_effective_config(&cfg, &scope)?;
            emit_self(args.json, "telegram.self.capture", cfg.self_chat_id)
        }
        None => {
            // User declined at the confirmation prompt or timeout
            // expired. Report the unchanged state rather than erroring —
            // `capture_self_chat` already printed the reason.
            emit_self(args.json, "telegram.self.capture", cfg.self_chat_id)
        }
    }
}

fn emit_self(json: bool, command: &'static str, self_chat_id: Option<i64>) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&SelfOutput {
                command,
                self_chat_id
            })
            .unwrap()
        );
    } else {
        match self_chat_id {
            Some(id) => println!("self chat id: {id}"),
            None => println!("self chat id: not configured"),
        }
    }
    Ok(())
}

fn save_effective_config(cfg: &TelegramServiceCfg, scope: &EffectiveScope) -> Result<()> {
    let path = match scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "telegram")?
        }
        EffectiveScope::Global => config::path::global_service_config_path("telegram")?,
    };
    config::save_flat(&path, cfg)
}
