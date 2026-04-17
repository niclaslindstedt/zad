//! Discord-specific permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/discord/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/discord/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both files
//! are optional; when both exist, a call must pass **both** — local can
//! only add restrictions, never loosen the global baseline.
//!
//! The TOML surface:
//!
//! ```toml
//! # Top-level defaults. Each per-function block inherits from these.
//! [content]
//! deny_words    = ["password", "api_key"]
//! deny_patterns = ["(?i)bearer\\s+[a-z0-9]+"]
//! max_length    = 1500
//!
//! [time]
//! days    = ["mon","tue","wed","thu","fri"]
//! windows = ["09:00-18:00"]
//!
//! # Per-function blocks. Every function has `channels`, `users`,
//! # `guilds` sublists where applicable, plus optional content/time
//! # overrides that **narrow** the top-level defaults.
//! [send]
//! channels.allow = ["general", "bot-*", "team/*"]
//! channels.deny  = ["*admin*"]
//! users.allow    = ["alice", "bob"]
//!
//! [read]
//! channels.deny = ["*private*"]
//!
//! [channels]
//! guilds.allow = ["main-server"]
//!
//! [join]
//! channels.deny = ["*admin*"]
//!
//! [leave]
//! # no restrictions
//!
//! [discover]
//! guilds.allow = ["main-server"]
//!
//! [manage]
//! # default-deny for channels.manage: block everything unless explicitly allowed
//! channels.allow = []
//! channels.deny  = ["*"]
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{self, directory::Directory};
use crate::error::{Result, ZadError};
use crate::permissions::{
    content::{ContentRules, ContentRulesRaw},
    pattern::{PatternList, PatternListRaw},
    time::{TimeWindow, TimeWindowRaw},
};

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordPermissionsRaw {
    #[serde(default)]
    pub content: ContentRulesRaw,
    #[serde(default)]
    pub time: TimeWindowRaw,

    #[serde(default)]
    pub send: FunctionBlockRaw,
    #[serde(default)]
    pub read: FunctionBlockRaw,
    #[serde(default)]
    pub channels: FunctionBlockRaw,
    #[serde(default)]
    pub join: FunctionBlockRaw,
    #[serde(default)]
    pub leave: FunctionBlockRaw,
    #[serde(default)]
    pub discover: FunctionBlockRaw,
    #[serde(default)]
    pub manage: FunctionBlockRaw,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionBlockRaw {
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub channels: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub users: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub guilds: PatternListRaw,
    #[serde(default, skip_serializing_if = "ContentRulesRaw_is_default")]
    pub content: ContentRulesRaw,
    #[serde(default, skip_serializing_if = "TimeWindowRaw_is_default")]
    pub time: TimeWindowRaw,
}

#[allow(non_snake_case)]
fn PatternListRaw_is_default(v: &PatternListRaw) -> bool {
    v.allow.is_empty() && v.deny.is_empty()
}
#[allow(non_snake_case)]
fn ContentRulesRaw_is_default(v: &ContentRulesRaw) -> bool {
    v.deny_words.is_empty() && v.deny_patterns.is_empty() && v.max_length.is_none()
}
#[allow(non_snake_case)]
fn TimeWindowRaw_is_default(v: &TimeWindowRaw) -> bool {
    v.days.is_empty() && v.windows.is_empty()
}

// ---------------------------------------------------------------------------
// compiled form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FunctionBlock {
    pub channels: PatternList,
    pub users: PatternList,
    pub guilds: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            channels: PatternList::compile(&raw.channels).map_err(ZadError::Invalid)?,
            users: PatternList::compile(&raw.users).map_err(ZadError::Invalid)?,
            guilds: PatternList::compile(&raw.guilds).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
        })
    }
}

/// One file's worth of rules, compiled. The fields match
/// `DiscordPermissionsRaw` 1:1 so lookups are O(1).
#[derive(Debug, Clone, Default)]
pub struct DiscordPermissions {
    /// Absolute path the rules were loaded from — embedded in every
    /// `PermissionDenied` error so the operator can find and edit the
    /// offending line without grep.
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub send: FunctionBlock,
    pub read: FunctionBlock,
    pub channels: FunctionBlock,
    pub join: FunctionBlock,
    pub leave: FunctionBlock,
    pub discover: FunctionBlock,
    pub manage: FunctionBlock,
}

impl DiscordPermissions {
    fn compile(raw: &DiscordPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(DiscordPermissions {
            source,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            send: FunctionBlock::compile(&raw.send)?,
            read: FunctionBlock::compile(&raw.read)?,
            channels: FunctionBlock::compile(&raw.channels)?,
            join: FunctionBlock::compile(&raw.join)?,
            leave: FunctionBlock::compile(&raw.leave)?,
            discover: FunctionBlock::compile(&raw.discover)?,
            manage: FunctionBlock::compile(&raw.manage)?,
        })
    }

    fn block(&self, f: DiscordFunction) -> &FunctionBlock {
        match f {
            DiscordFunction::Send => &self.send,
            DiscordFunction::Read => &self.read,
            DiscordFunction::Channels => &self.channels,
            DiscordFunction::Join => &self.join,
            DiscordFunction::Leave => &self.leave,
            DiscordFunction::Discover => &self.discover,
            DiscordFunction::Manage => &self.manage,
        }
    }
}

/// Identifier for every Discord runtime function permissions gate. Kept
/// as a small closed enum (rather than strings) so the compiler catches
/// a new verb being added without a matching permission block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscordFunction {
    Send,
    Read,
    Channels,
    Join,
    Leave,
    Discover,
    Manage,
}

impl DiscordFunction {
    pub fn name(self) -> &'static str {
        match self {
            DiscordFunction::Send => "send",
            DiscordFunction::Read => "read",
            DiscordFunction::Channels => "channels",
            DiscordFunction::Join => "join",
            DiscordFunction::Leave => "leave",
            DiscordFunction::Discover => "discover",
            DiscordFunction::Manage => "manage",
        }
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

/// The pair of files that might apply to this project. Either or both
/// may be absent; a missing file is represented by `None` and contributes
/// no restrictions.
#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<DiscordPermissions>,
    pub local: Option<DiscordPermissions>,
}

impl EffectivePermissions {
    pub fn any(&self) -> bool {
        self.global.is_some() || self.local.is_some()
    }

    /// Paths that were considered, in the order they were considered.
    /// Returned by the `permissions path` CLI verb and embedded in
    /// `show --json` output.
    pub fn sources(&self) -> Vec<&Path> {
        let mut out: Vec<&Path> = vec![];
        if let Some(g) = &self.global {
            out.push(&g.source);
        }
        if let Some(l) = &self.local {
            out.push(&l.source);
        }
        out
    }

    /// Iterate compiled permission files in evaluation order (global
    /// first, then local). Both must admit the call.
    fn layers(&self) -> impl Iterator<Item = &DiscordPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    // ---- per-function enforcement ----

    /// Evaluate a send call: pattern-match on the channel/DM target,
    /// scan the body for content violations, and check the time window.
    /// The `directory` is used only to collect reverse-lookup names
    /// when the caller resolved a raw ID — so `*admin*` catches both
    /// the name and the snowflake form.
    pub fn check_send_channel(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Send,
            TargetKind::Channel,
            input,
            resolved_id,
            directory,
        )
    }

    pub fn check_send_dm(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Send,
            TargetKind::User,
            input,
            resolved_id,
            directory,
        )
    }

    pub fn check_send_body(&self, body: &str) -> Result<()> {
        for p in self.layers() {
            let merged = p.content.clone().merge(p.send.content.clone());
            if let Err(e) = merged.evaluate(body) {
                return Err(ZadError::PermissionDenied {
                    function: "send",
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn check_read_channel(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Read,
            TargetKind::Channel,
            input,
            resolved_id,
            directory,
        )
    }

    pub fn check_channels_guild(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Channels,
            TargetKind::Guild,
            input,
            resolved_id,
            directory,
        )
    }

    pub fn check_join_channel(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Join,
            TargetKind::Channel,
            input,
            resolved_id,
            directory,
        )
    }

    pub fn check_leave_channel(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Leave,
            TargetKind::Channel,
            input,
            resolved_id,
            directory,
        )
    }

    pub fn check_discover_guild(
        &self,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_target(
            DiscordFunction::Discover,
            TargetKind::Guild,
            input,
            resolved_id,
            directory,
        )
    }

    /// Time-window check for a given function. Callers invoke this at
    /// the top of every verb that could issue a network call, so the
    /// "denied" answer never leaks a target name on failure.
    pub fn check_time(&self, f: DiscordFunction) -> Result<()> {
        for p in self.layers() {
            let merged = p.time.clone().merge(p.block(f).time.clone());
            if let Err(e) = merged.evaluate_now() {
                return Err(ZadError::PermissionDenied {
                    function: static_name(f),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    fn check_target(
        &self,
        f: DiscordFunction,
        kind: TargetKind,
        input: &str,
        resolved_id: u64,
        directory: &Directory,
    ) -> Result<()> {
        // Strip the ergonomic sigils so `#general` and `general` match
        // the same allow pattern `general`.
        let stripped = input
            .strip_prefix('#')
            .or_else(|| input.strip_prefix('@'))
            .unwrap_or(input);
        let id_str = resolved_id.to_string();

        let mut names: Vec<String> = Vec::with_capacity(4);
        names.push(stripped.to_string());
        names.push(id_str);
        names.extend(kind.names_for(directory, resolved_id));
        // De-dup so operators don't see the same pattern reported twice
        // for files with both a bare and a qualified channel key.
        names.sort();
        names.dedup();

        for p in self.layers() {
            let list = match kind {
                TargetKind::Channel => &p.block(f).channels,
                TargetKind::User => &p.block(f).users,
                TargetKind::Guild => &p.block(f).guilds,
            };
            if list.is_empty() {
                continue;
            }
            let aliases: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            if let Err(e) = list.evaluate(aliases.iter().copied()) {
                return Err(ZadError::PermissionDenied {
                    function: static_name(f),
                    reason: e.as_sentence(&format!("{} `{}`", kind.label(), input)),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }
}

fn static_name(f: DiscordFunction) -> &'static str {
    // Needed because `ZadError::PermissionDenied::function` is a
    // `&'static str` — it's used for machine-readable JSON output and
    // for grep patterns in tests.
    f.name()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Channel,
    User,
    Guild,
}

impl TargetKind {
    fn label(self) -> &'static str {
        match self {
            TargetKind::Channel => "channel",
            TargetKind::User => "user",
            TargetKind::Guild => "guild",
        }
    }

    fn names_for(self, directory: &Directory, id: u64) -> Vec<String> {
        let id_s = id.to_string();
        match self {
            TargetKind::Channel => directory
                .channels
                .iter()
                .filter(|(_, v)| **v == id_s)
                .map(|(k, _)| k.clone())
                .collect(),
            TargetKind::User => directory
                .users
                .iter()
                .filter(|(_, v)| **v == id_s)
                .map(|(k, _)| k.clone())
                .collect(),
            TargetKind::Guild => directory
                .guilds
                .iter()
                .filter(|(_, v)| **v == id_s)
                .map(|(k, _)| k.clone())
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// paths + load
// ---------------------------------------------------------------------------

pub fn global_path() -> Result<PathBuf> {
    Ok(config::path::global_service_dir("discord")?.join("permissions.toml"))
}

pub fn local_path_for(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, "discord")?.join("permissions.toml"))
}

pub fn local_path_current() -> Result<PathBuf> {
    local_path_for(&config::path::project_slug()?)
}

/// Load a single file by path. Absent file → `Ok(None)`. Parse/compile
/// errors surface with the file path embedded in the message.
pub fn load_file(path: &Path) -> Result<Option<DiscordPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: DiscordPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    let compiled = DiscordPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

fn wrap_compile_error(err: ZadError, path: &Path) -> ZadError {
    match err {
        ZadError::Invalid(msg) => ZadError::Invalid(format!(
            "invalid permissions file {}: {msg}",
            path.display()
        )),
        other => other,
    }
}

/// Load the effective permissions for the current project.
pub fn load_effective() -> Result<EffectivePermissions> {
    let slug = config::path::project_slug()?;
    load_effective_for(&slug)
}

pub fn load_effective_for(slug: &str) -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_for(slug)?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(path: &Path, raw: &DiscordPermissionsRaw) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let body = toml::to_string_pretty(raw)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// A starter policy emitted by `zad discord permissions init`. Biased
/// toward safe defaults — non-empty comment header, an illustrative
/// allow list, and content rules that catch the most obvious leak
/// vectors.
pub fn starter_template() -> DiscordPermissionsRaw {
    DiscordPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into(), "secret".into()],
            deny_patterns: vec![],
            max_length: None,
        },
        time: TimeWindowRaw::default(),
        send: FunctionBlockRaw {
            channels: PatternListRaw {
                allow: vec![],
                deny: vec!["*admin*".into(), "*mod-*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        read: FunctionBlockRaw::default(),
        channels: FunctionBlockRaw::default(),
        join: FunctionBlockRaw::default(),
        leave: FunctionBlockRaw::default(),
        discover: FunctionBlockRaw::default(),
        manage: FunctionBlockRaw {
            channels: PatternListRaw {
                allow: vec![],
                deny: vec!["*".into()],
            },
            ..FunctionBlockRaw::default()
        },
    }
}
