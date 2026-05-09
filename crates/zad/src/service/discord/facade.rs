//! High-level, typed Discord facade for library callers.
//!
//! The CLI ([`zad-cli::cli::discord`](../../../../zad_cli/cli/discord/index.html))
//! drives Discord by hand: load config, load permissions, validate input
//! lengths, build a [`Target`], call the transport, render output. This
//! module hosts the same flow behind a typed API so a Rust crate that
//! depends on `zad` can call `Discord::from_default_config()?.send(req)`
//! and get a typed [`SendResponse`] back.
//!
//! ## Typed-input contract
//!
//! Every `*Request` is validated at construction, so a value of the
//! type is always a sendable request:
//!
//! - [`SendRequest::new`] enforces the 2000-character body limit
//!   ([`crate::service::discord::client::DISCORD_MAX_MESSAGE_LEN`]),
//!   the 10-attachment cap
//!   ([`crate::service::discord::client::DISCORD_MAX_ATTACHMENTS`]),
//!   and the rule that a `MessageBody::Empty` must come with at least
//!   one attachment.
//! - [`ReadRequest::new`] enforces Discord's `1..=100` history limit.
//! - Newtypes ([`ChannelId`], [`UserId`], [`MessageId`]) prevent
//!   passing the wrong kind of snowflake at a call site.
//!
//! Construction errors surface as [`ZadError::Invalid`]; runtime errors
//! (network, permission, scope) come back through the same `ZadError`
//! variants the CLI handles today.
//!
//! ## Permission and scope enforcement
//!
//! Library callers get the **same** scope and permission enforcement
//! the CLI does. Both gates run inside the facade methods before any
//! network call:
//!
//! 1. **Scope.** The bot's declared scopes (`messages.send`, `guilds`,
//!    …) come from the Discord service config. The transport rejects
//!    any call to a verb whose scope isn't in that set with
//!    [`ZadError::ScopeDenied`], naming the file to edit.
//! 2. **Permissions.** The signed `permissions.toml` files (global +
//!    local) intersect: every call must pass every file that is
//!    present. The facade runs the `check_*` for the verb (channel
//!    allow/deny, content rules, time windows, attachment caps) and
//!    surfaces denials as
//!    [`ZadError::PermissionDenied { function, reason, config_path }`]
//!    — the same error variant the CLI emits.
//!
//! ## Choosing a constructor
//!
//! Three entry points, each with a different env-var policy:
//!
//! | Constructor | Reads env vars? | When to use |
//! |---|---|---|
//! | [`Discord::with_paths`] | **No.** | Production library code. Multi-tenant servers, embed-zad-in-another-app scenarios, deterministic tests. The caller hands every path in, including which `permissions.toml` files to enforce (or none). |
//! | [`Discord::with_token`] | **No.** | Quick tests, scripts, or programs that build their own permission policy at runtime. Enforces scope only; layer permissions back on with [`Discord::with_permissions`]. |
//! | [`Discord::from_default_config`] | **Yes** — honors `ZAD_HOME_OVERRIDE`, `ZAD_PERMISSIONS_PATH`, `ZAD_PERMISSIONS_ROOT`, `ZAD_SECRETS_MEMORY` via the underlying `config::path::*` and `secrets::*` helpers. | CLI-equivalent shortcut. Use when you want exactly what `zad discord <verb>` does. |
//!
//! Best practice for a Rust library that embeds zad: prefer
//! [`Discord::with_paths`]. It guarantees that nothing in the
//! surrounding process — env vars set by other tools, env vars set
//! by your test harness — can change what your code does.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::config::directory::{self as dir, Directory};
use crate::config::{self, DiscordServiceCfg};
use crate::error::{Result, ZadError};
use crate::permissions::attachments::AttachmentInfo;
use crate::secrets::{self, Scope};
use crate::service::discord::client::{DISCORD_MAX_ATTACHMENTS, DISCORD_MAX_MESSAGE_LEN};
use crate::service::discord::permissions::{self as perms, DiscordFunction, EffectivePermissions};
use crate::service::discord::{DiscordHttp, DiscordTransport};
use crate::service::{ChannelId, ChannelInfo, GuildInfo, Message, MessageId, Target, UserId};

/// Typed library entry point for Discord.
///
/// Wraps a [`DiscordHttp`] transport with the same scope and
/// permission guarantees the CLI provides. Construct with
/// [`Discord::from_default_config`] to mirror what
/// `zad discord <verb>` does, or with [`Discord::with_token`] when
/// you want to control credentials directly (tests, multi-tenant
/// servers).
pub struct Discord {
    http: Box<dyn DiscordTransport>,
    /// Loaded once at construction. `None` means "no
    /// `permissions.toml` to enforce" — either the caller used
    /// [`Discord::with_token`] without [`Discord::with_permissions`],
    /// or no policy files exist on disk.
    permissions: Option<EffectivePermissions>,
    /// The name → snowflake directory used by permission rules to
    /// match aliases (e.g. a deny on `*admin*` should fire when the
    /// caller pastes the raw snowflake of an `admin-only` channel).
    directory: Directory,
}

impl Discord {
    /// Load `~/.zad/projects/<slug>/services/discord/config.toml` (with
    /// fallback to the global service config) and the bot token from
    /// the OS keychain, exactly as `zad discord <verb>` does. The
    /// project must already have Discord enabled
    /// (`zad service enable discord`).
    ///
    /// Also loads the effective `permissions.toml` (intersection of
    /// global + local) and the project directory, so every facade
    /// method enforces the same policy as the CLI.
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope) = effective_config()?;
        let config_path = config_path_for(&scope)?;
        let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
        let token = load_token(&scope)?;
        let http = DiscordHttp::new(&token, scopes, config_path);
        let permissions = perms::load_effective().ok();
        let directory = dir::load().unwrap_or_default();
        Ok(Self {
            http: Box::new(http),
            permissions,
            directory,
        })
    }

    /// Construct from an explicit bot token, declared scope set, and
    /// `config_path` (referenced only in scope-violation error
    /// messages). Does not touch the filesystem or the keychain.
    /// Skips on-disk `permissions.toml` enforcement; layer policy on
    /// top with [`Discord::with_permissions`] if you need it.
    pub fn with_token(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        let http = DiscordHttp::new(&token.into(), scopes, config_path);
        Self {
            http: Box::new(http),
            permissions: None,
            directory: Directory::default(),
        }
    }

    /// Fully explicit, env-var-free constructor — the recommended
    /// entry point for library code. Every input the facade needs
    /// (token, scopes, config path for scope-error messages, the two
    /// `permissions.toml` paths) is passed in directly. Reads no env
    /// vars; touches only the permission files the caller named.
    ///
    /// `global_permissions` and `local_permissions` are each
    /// `Option<&Path>`:
    /// - `Some(path)` that exists is loaded and enforced.
    /// - `Some(path)` that does not exist contributes no restrictions
    ///   (mirrors the CLI's "missing file = empty layer" semantics).
    /// - `None` means "no permission file at this scope".
    ///
    /// Pass `None` for both to get scope-only enforcement (equivalent
    /// to [`Discord::with_token`]).
    pub fn with_paths(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        global_permissions: Option<&std::path::Path>,
        local_permissions: Option<&std::path::Path>,
    ) -> Result<Self> {
        let http = DiscordHttp::new(&token.into(), scopes, config_path);
        let permissions = perms::load_from(global_permissions, local_permissions)?;
        let permissions = if permissions.any() {
            Some(permissions)
        } else {
            None
        };
        Ok(Self {
            http: Box::new(http),
            permissions,
            directory: Directory::default(),
        })
    }

    /// Replace the loaded permission policy. Useful for tests, for
    /// programs that compute their own policy at runtime, or for
    /// callers that built the [`Discord`] with [`Discord::with_token`]
    /// and want permission enforcement layered back on.
    pub fn with_permissions(mut self, permissions: EffectivePermissions) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Replace the directory used to resolve aliases for permission
    /// rules (so a deny on `*admin*` matches both the channel name and
    /// the snowflake form).
    pub fn with_directory(mut self, directory: Directory) -> Self {
        self.directory = directory;
        self
    }

    /// Send a message to a channel or DM. Runs every gate the CLI
    /// runs — scope (via the transport), then permissions (time
    /// window, channel allow/deny, content rules, attachment caps),
    /// then the network call. Returns the message ID Discord
    /// assigned.
    pub async fn send(&self, req: SendRequest) -> Result<SendResponse> {
        if let Some(p) = &self.permissions {
            p.check_time(DiscordFunction::Send)?;
            match &req.target {
                Target::Channel(ChannelId(id)) => {
                    p.check_send_channel(&id.to_string(), *id, &self.directory)?;
                }
                Target::Dm(UserId(id)) => {
                    p.check_send_dm(&id.to_string(), *id, &self.directory)?;
                }
            }
            p.check_send_body(req.body.as_str())?;
            if !req.attachments.is_empty() {
                let infos: Vec<AttachmentInfo> = req
                    .attachments
                    .iter()
                    .map(|path| {
                        AttachmentInfo::probe(path).map_err(|e| {
                            ZadError::Invalid(format!(
                                "attachment `{}` not readable: {e}",
                                path.display()
                            ))
                        })
                    })
                    .collect::<Result<_>>()?;
                p.check_send_attachments(&infos)?;
            }
        }
        let body = req.body.as_str();
        let message_id = self
            .http
            .send(req.target.clone(), body, &req.attachments)
            .await?;
        Ok(SendResponse {
            message_id,
            target: req.target,
        })
    }

    /// Fetch recent messages from a channel. Returns oldest-first
    /// (Discord's API returns newest-first; the facade reverses for
    /// callers who want chronological order). Limit is bounded by
    /// [`ReadRequest::new`] to `1..=100`.
    pub async fn read(&self, req: ReadRequest) -> Result<Vec<Message>> {
        if let Some(p) = &self.permissions {
            p.check_time(DiscordFunction::Read)?;
            p.check_read_channel(&req.channel.0.to_string(), req.channel.0, &self.directory)?;
        }
        let mut msgs = self.http.history(req.channel, req.limit).await?;
        msgs.reverse();
        Ok(msgs)
    }

    /// List the channels in a guild.
    pub async fn channels(&self, req: ChannelsRequest) -> Result<Vec<ChannelInfo>> {
        if let Some(p) = &self.permissions {
            p.check_time(DiscordFunction::Channels)?;
            p.check_channels_guild(&req.guild.to_string(), req.guild, &self.directory)?;
        }
        self.http.list_channels(req.guild).await
    }

    /// Join a thread channel. Discord only accepts explicit joins on
    /// thread channels — the API call is a no-op on regular channels
    /// but the type system can't tell them apart.
    pub async fn join(&self, req: JoinRequest) -> Result<()> {
        if let Some(p) = &self.permissions {
            p.check_time(DiscordFunction::Join)?;
            p.check_join_channel(&req.channel.0.to_string(), req.channel.0, &self.directory)?;
        }
        self.http.join_channel(req.channel).await
    }

    /// Leave a thread channel.
    pub async fn leave(&self, req: LeaveRequest) -> Result<()> {
        if let Some(p) = &self.permissions {
            p.check_time(DiscordFunction::Leave)?;
            p.check_leave_channel(&req.channel.0.to_string(), req.channel.0, &self.directory)?;
        }
        self.http.leave_channel(req.channel).await
    }

    /// List the guilds the bot can see.
    pub async fn guilds(&self) -> Result<Vec<GuildInfo>> {
        if let Some(p) = &self.permissions {
            p.check_time(DiscordFunction::Discover)?;
        }
        self.http.list_guilds().await
    }
}

// ---------------------------------------------------------------------------
// SendRequest — the canonical typed-input shape
// ---------------------------------------------------------------------------

/// Request to [`Discord::send`].
///
/// Construct with [`SendRequest::new`]; that constructor enforces every
/// invariant Discord's send endpoint cares about (body length cap,
/// attachment count cap, "non-empty body or at least one attachment"),
/// so a `SendRequest` value is always a sendable request.
#[derive(Debug, Clone)]
pub struct SendRequest {
    pub target: Target,
    pub body: MessageBody,
    pub attachments: Vec<PathBuf>,
}

/// A message body. Discord accepts an empty body iff at least one
/// attachment is also present — the [`MessageBody::Empty`] variant
/// captures that special case at the type level so [`SendRequest::new`]
/// can validate it once.
#[derive(Debug, Clone)]
pub enum MessageBody {
    Text(String),
    Empty,
}

impl MessageBody {
    /// Helper for the common case of a plain text body.
    pub fn text(s: impl Into<String>) -> Self {
        MessageBody::Text(s.into())
    }

    fn as_str(&self) -> &str {
        match self {
            MessageBody::Text(s) => s.as_str(),
            MessageBody::Empty => "",
        }
    }
}

impl SendRequest {
    /// Build a validated send request.
    ///
    /// Returns [`ZadError::Invalid`] if:
    /// - the body is over [`DISCORD_MAX_MESSAGE_LEN`] characters,
    /// - more than [`DISCORD_MAX_ATTACHMENTS`] attachments are
    ///   provided, or
    /// - the body is [`MessageBody::Empty`] and no attachments are
    ///   provided (Discord rejects messages with no payload).
    pub fn new(target: Target, body: MessageBody, attachments: Vec<PathBuf>) -> Result<Self> {
        let len = body.as_str().chars().count();
        if len > DISCORD_MAX_MESSAGE_LEN {
            return Err(ZadError::Invalid(format!(
                "message body is {len} characters; Discord's hard limit is {DISCORD_MAX_MESSAGE_LEN}"
            )));
        }
        if attachments.len() > DISCORD_MAX_ATTACHMENTS {
            return Err(ZadError::Invalid(format!(
                "{} attachments is above Discord's per-message cap of {DISCORD_MAX_ATTACHMENTS}",
                attachments.len()
            )));
        }
        if matches!(body, MessageBody::Empty) && attachments.is_empty() {
            return Err(ZadError::Invalid(
                "message body is empty and no attachments are present; pass at least one of them"
                    .into(),
            ));
        }
        Ok(Self {
            target,
            body,
            attachments,
        })
    }
}

/// Result of [`Discord::send`].
#[derive(Debug, Clone)]
pub struct SendResponse {
    pub message_id: MessageId,
    pub target: Target,
}

// ---------------------------------------------------------------------------
// ReadRequest — bounded history fetch
// ---------------------------------------------------------------------------

/// Request to [`Discord::read`].
///
/// `limit` is bounded by Discord's API to `1..=100`; [`ReadRequest::new`]
/// enforces this at construction.
#[derive(Debug, Clone)]
pub struct ReadRequest {
    pub channel: ChannelId,
    pub limit: usize,
}

impl ReadRequest {
    pub fn new(channel: ChannelId, limit: usize) -> Result<Self> {
        if !(1..=100).contains(&limit) {
            return Err(ZadError::Invalid(format!(
                "limit must be between 1 and 100 (Discord API maximum); got {limit}"
            )));
        }
        Ok(Self { channel, limit })
    }
}

// ---------------------------------------------------------------------------
// ChannelsRequest / JoinRequest / LeaveRequest — single-field typed reqs
// ---------------------------------------------------------------------------

/// Request to [`Discord::channels`].
#[derive(Debug, Clone)]
pub struct ChannelsRequest {
    pub guild: u64,
}

impl ChannelsRequest {
    pub fn new(guild: u64) -> Self {
        Self { guild }
    }
}

/// Request to [`Discord::join`].
#[derive(Debug, Clone)]
pub struct JoinRequest {
    pub channel: ChannelId,
}

impl JoinRequest {
    pub fn new(channel: ChannelId) -> Self {
        Self { channel }
    }
}

/// Request to [`Discord::leave`].
#[derive(Debug, Clone)]
pub struct LeaveRequest {
    pub channel: ChannelId,
}

impl LeaveRequest {
    pub fn new(channel: ChannelId) -> Self {
        Self { channel }
    }
}

// ---------------------------------------------------------------------------
// Config / token plumbing — mirrors `cli/discord.rs` exactly
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

fn config_path_for(scope: &EffectiveScope) -> Result<PathBuf> {
    match scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "discord")
        }
        EffectiveScope::Global => config::path::global_service_config_path("discord"),
    }
}

fn load_token(scope: &EffectiveScope) -> Result<String> {
    let account = match scope {
        EffectiveScope::Global => secrets::account("discord", "bot", Scope::Global),
        EffectiveScope::Local(slug) => secrets::account("discord", "bot", Scope::Project(slug)),
    };
    secrets::load(&account)?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "bot token missing from keychain (account `{account}`). \
             Re-run `zad service create discord` to reinstall it."
        ))
    })
}
