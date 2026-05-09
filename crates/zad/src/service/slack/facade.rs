//! Typed library facade for Slack. See
//! `service::discord::facade` for the full pattern documentation —
//! this module follows the same shape: three constructors
//! (`from_default_config`, `with_token`, `with_paths`), validating
//! `*Request` types per verb, automatic permission enforcement.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::directory::{self as dir, Directory};
use crate::config::{self, SlackServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::ChannelInfo;
use crate::service::slack::client::{
    ListChannelsResult, SLACK_MAX_MESSAGE_LEN, SlackHttp, SlackMessage,
};
use crate::service::slack::permissions::{self as perms, EffectivePermissions, SlackFunction};

/// Slack channel ID newtype (`C…` for channels, `D…` for DMs, `G…` for
/// private channels). Distinct from raw `String` so a function that
/// takes a `SlackChannelId` can't be called with a username or any
/// other free-form string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlackChannelId(pub String);

/// Slack user ID newtype (`U…`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlackUserId(pub String);

/// Where a Slack message should be delivered.
#[derive(Debug, Clone)]
pub enum SlackTarget {
    Channel(SlackChannelId),
    Dm(SlackUserId),
}

/// Typed library entry point for Slack.
pub struct Slack {
    http: SlackHttp,
    permissions: Option<EffectivePermissions>,
    directory: Directory,
}

impl Slack {
    /// CLI-equivalent: load project-or-global Slack config + bot
    /// token from the keychain + effective permissions from default
    /// paths. **Honors `ZAD_HOME_OVERRIDE` and friends.**
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope) = effective_config()?;
        let config_path = config_path_for(&scope)?;
        let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
        let token = load_token(&scope)?;
        let http = SlackHttp::new(&token, scopes, config_path);
        let permissions = perms::load_effective().ok();
        let directory = dir::load().unwrap_or_default();
        Ok(Self {
            http,
            permissions,
            directory,
        })
    }

    /// Explicit token + scope set + config path. No env reads, no
    /// permission enforcement (layer back on with
    /// [`Slack::with_permissions`]).
    pub fn with_token(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        let http = SlackHttp::new(token.into(), scopes, config_path);
        Self {
            http,
            permissions: None,
            directory: Directory::default(),
        }
    }

    /// Fully explicit, env-free constructor. Recommended for library
    /// code. `global_permissions` / `local_permissions` are optional;
    /// pass `None` for both to skip on-disk policy enforcement.
    pub fn with_paths(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        global_permissions: Option<&Path>,
        local_permissions: Option<&Path>,
    ) -> Result<Self> {
        let http = SlackHttp::new(token.into(), scopes, config_path);
        let permissions = perms::load_from(global_permissions, local_permissions)?;
        let permissions = if permissions.any() {
            Some(permissions)
        } else {
            None
        };
        Ok(Self {
            http,
            permissions,
            directory: Directory::default(),
        })
    }

    pub fn with_permissions(mut self, permissions: EffectivePermissions) -> Self {
        self.permissions = Some(permissions);
        self
    }

    pub fn with_directory(mut self, directory: Directory) -> Self {
        self.directory = directory;
        self
    }

    /// Send a message. Validation runs at [`SendRequest::new`];
    /// permission and time checks run here before the network call.
    pub async fn send(&self, req: SendRequest) -> Result<SendResponse> {
        if let Some(p) = &self.permissions {
            p.check_time(SlackFunction::Send)?;
            match &req.target {
                SlackTarget::Channel(SlackChannelId(id)) => {
                    p.check_send_channel(id, &self.directory)?;
                }
                SlackTarget::Dm(SlackUserId(id)) => {
                    p.check_send_dm(id, &self.directory)?;
                }
            }
            p.check_send_body(&req.body)?;
        }
        let ts = match &req.target {
            SlackTarget::Channel(SlackChannelId(id)) => self.http.send(id, &req.body).await?,
            SlackTarget::Dm(SlackUserId(id)) => self.http.send_dm(id, &req.body).await?,
        };
        Ok(SendResponse {
            ts,
            target: req.target,
        })
    }

    /// Read recent messages from a channel.
    pub async fn read(&self, req: ReadRequest) -> Result<Vec<SlackMessage>> {
        if let Some(p) = &self.permissions {
            p.check_time(SlackFunction::Read)?;
            p.check_read_channel(&req.channel.0, &self.directory)?;
        }
        self.http.history(&req.channel.0, req.limit).await
    }

    /// List channels in the workspace (one page; pass the cursor from
    /// the previous response to paginate).
    pub async fn channels(&self, req: ChannelsRequest) -> Result<ListChannelsResult> {
        if let Some(p) = &self.permissions {
            p.check_time(SlackFunction::Channels)?;
            p.check_channels_workspace("workspace", &self.directory)?;
        }
        self.http.list_channels(req.cursor.as_deref()).await
    }
}

// Conversions for callers who want the service-agnostic `ChannelInfo`.
const _: fn(&crate::service::slack::client::SlackChannel) -> ChannelInfo =
    SlackHttp::channel_info_to_domain;

// ---------------------------------------------------------------------------
// Typed Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub target: SlackTarget,
    pub body: String,
}

impl SendRequest {
    /// Validates body length against [`SLACK_MAX_MESSAGE_LEN`] and
    /// rejects empty bodies (Slack's `chat.postMessage` rejects them).
    pub fn new(target: SlackTarget, body: impl Into<String>) -> Result<Self> {
        let body = body.into();
        if body.is_empty() {
            return Err(ZadError::Invalid(
                "Slack message body must not be empty".into(),
            ));
        }
        let len = body.chars().count();
        if len > SLACK_MAX_MESSAGE_LEN {
            return Err(ZadError::Invalid(format!(
                "message body is {len} characters; Slack's hard limit is {SLACK_MAX_MESSAGE_LEN}"
            )));
        }
        Ok(Self { target, body })
    }
}

#[derive(Debug, Clone)]
pub struct SendResponse {
    pub ts: String,
    pub target: SlackTarget,
}

#[derive(Debug, Clone)]
pub struct ReadRequest {
    pub channel: SlackChannelId,
    pub limit: usize,
}

impl ReadRequest {
    /// Slack caps `conversations.history` at 1000; keep the API
    /// envelope to `1..=200` (Slack's recommended page size).
    pub fn new(channel: SlackChannelId, limit: usize) -> Result<Self> {
        if !(1..=200).contains(&limit) {
            return Err(ZadError::Invalid(format!(
                "limit must be between 1 and 200; got {limit}"
            )));
        }
        Ok(Self { channel, limit })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChannelsRequest {
    pub cursor: Option<String>,
}

impl ChannelsRequest {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_cursor(cursor: impl Into<String>) -> Self {
        Self {
            cursor: Some(cursor.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Config / token plumbing — mirrors `cli/slack.rs`.
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

fn config_path_for(scope: &EffectiveScope) -> Result<PathBuf> {
    match scope {
        EffectiveScope::Local(slug) => config::path::project_service_config_path_for(slug, "slack"),
        EffectiveScope::Global => config::path::global_service_config_path("slack"),
    }
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
