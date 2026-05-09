//! Typed library facade for Telegram. Same shape as
//! `service::discord::facade`: three constructors, validating
//! `*Request` types, automatic permission enforcement.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::{self, TelegramServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::telegram::client::{TELEGRAM_MAX_MESSAGE_LEN, TelegramHttp, Update};
use crate::service::telegram::directory::{self as dir, Directory};
use crate::service::telegram::permissions::{
    self as perms, EffectivePermissions, TelegramFunction,
};

/// Telegram chat ID newtype. Telegram's flat `chat_id` address space
/// covers private chats, groups (negative IDs), supergroups (also
/// negative), and channels — all encoded as `i64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChatId(pub i64);

/// Typed library entry point for Telegram.
pub struct Telegram {
    http: TelegramHttp,
    permissions: Option<EffectivePermissions>,
    directory: Directory,
}

impl Telegram {
    /// CLI-equivalent: project-or-global config + bot token + default
    /// permission paths. **Honors `ZAD_HOME_OVERRIDE` and friends.**
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope) = effective_config()?;
        let config_path = config_path_for(&scope)?;
        let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
        let token = load_token(&scope)?;
        let http = TelegramHttp::new(&token, scopes, config_path);
        let permissions = perms::load_effective().ok();
        let directory = dir::load().unwrap_or_default();
        Ok(Self {
            http,
            permissions,
            directory,
        })
    }

    /// Explicit token + scope set + config path. Reads no env vars,
    /// no permission enforcement (`with_permissions` to layer back on).
    pub fn with_token(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        let token = token.into();
        let http = TelegramHttp::new(&token, scopes, config_path);
        Self {
            http,
            permissions: None,
            directory: Directory::default(),
        }
    }

    /// Fully explicit, env-free. Recommended for library code.
    pub fn with_paths(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        global_permissions: Option<&Path>,
        local_permissions: Option<&Path>,
    ) -> Result<Self> {
        let token = token.into();
        let http = TelegramHttp::new(&token, scopes, config_path);
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

    pub async fn send(&self, req: SendRequest) -> Result<SendResponse> {
        if let Some(p) = &self.permissions {
            p.check_time(TelegramFunction::Send)?;
            p.check_send_chat(&req.chat.0.to_string(), req.chat.0, &self.directory)?;
            p.check_send_body(&req.body)?;
        }
        let message_id = self.http.send_message(req.chat.0, &req.body).await?;
        Ok(SendResponse {
            message_id,
            chat: req.chat,
        })
    }

    pub async fn read(&self, req: ReadRequest) -> Result<Vec<Update>> {
        if let Some(p) = &self.permissions {
            p.check_time(TelegramFunction::Read)?;
            // Telegram has no per-chat read endpoint; getUpdates is
            // global to the bot. The chat allow/deny check still runs
            // so a denied chat surfaces consistently.
            p.check_read_chat(
                &req.chat.map(|c| c.0.to_string()).unwrap_or_default(),
                req.chat.map(|c| c.0).unwrap_or(0),
                &self.directory,
            )?;
        }
        self.http.get_updates(req.offset).await
    }

    pub async fn chats(&self, _req: ChatsRequest) -> Result<Vec<Update>> {
        if let Some(p) = &self.permissions {
            p.check_time(TelegramFunction::Chats)?;
        }
        // Telegram's "list chats" surface is implicit through getUpdates;
        // the CLI exposes it under `zad telegram chats` with the same
        // backing call.
        self.http.get_updates(None).await
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub chat: ChatId,
    pub body: String,
}

impl SendRequest {
    pub fn new(chat: ChatId, body: impl Into<String>) -> Result<Self> {
        let body = body.into();
        if body.is_empty() {
            return Err(ZadError::Invalid(
                "Telegram message body must not be empty".into(),
            ));
        }
        let len = body.chars().count();
        if len > TELEGRAM_MAX_MESSAGE_LEN {
            return Err(ZadError::Invalid(format!(
                "message body is {len} characters; Telegram's hard limit is {TELEGRAM_MAX_MESSAGE_LEN}"
            )));
        }
        Ok(Self { chat, body })
    }
}

#[derive(Debug, Clone)]
pub struct SendResponse {
    pub message_id: i64,
    pub chat: ChatId,
}

#[derive(Debug, Clone, Default)]
pub struct ReadRequest {
    pub chat: Option<ChatId>,
    pub offset: Option<i64>,
}

impl ReadRequest {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatsRequest;

impl ChatsRequest {
    pub fn new() -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
// Config / token plumbing — mirrors `cli/telegram.rs`.
// ---------------------------------------------------------------------------

enum EffectiveScope {
    Global,
    Local(String),
}

fn effective_config() -> Result<(TelegramServiceCfg, EffectiveScope)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("telegram") {
        return Err(ZadError::Invalid(format!(
            "telegram is not enabled for this project ({}). \
             Run `zad service enable telegram` first.",
            project_path.display()
        )));
    }
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

fn config_path_for(scope: &EffectiveScope) -> Result<PathBuf> {
    match scope {
        EffectiveScope::Local(slug) => {
            config::path::project_service_config_path_for(slug, "telegram")
        }
        EffectiveScope::Global => config::path::global_service_config_path("telegram"),
    }
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
