//! Telegram-specific permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/telegram/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/telegram/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both files
//! are optional; when both exist, a call must pass **both** — local can
//! only add restrictions, never loosen the global baseline.
//!
//! Telegram has one kind of target (a chat), so each per-function
//! block carries a single `chats` allow/deny list — no separate axes
//! for users, servers, or guilds.
//!
//! The TOML surface:
//!
//! ```toml
//! # Top-level defaults. Each per-function block inherits from these.
//! [content]
//! deny_words    = ["password", "api_key"]
//! deny_patterns = ["(?i)bearer\\s+[a-z0-9]+"]
//! max_length    = 3500
//!
//! [time]
//! days    = ["mon","tue","wed","thu","fri"]
//! windows = ["09:00-18:00"]
//!
//! [send]
//! chats.allow = ["general", "bot-*", "@team_notifications"]
//! chats.deny  = ["*admin*"]
//!
//! [read]
//! chats.deny = ["*private*"]
//!
//! [chats]
//! # No restrictions on listing the cached chats.
//!
//! [discover]
//! chats.allow = ["re:^[0-9-]+$"]
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::{Result, ZadError};
use crate::permissions::{
    attachments::{AttachmentInfo, AttachmentRules, AttachmentRulesRaw},
    content::{ContentRules, ContentRulesRaw},
    mutation::{self, Mutation},
    pattern::{PatternList, PatternListRaw},
    service::HasSignature,
    signing::{self, Signature, SigningKey},
    time::{TimeWindow, TimeWindowRaw},
};
use crate::service::telegram::directory::Directory;

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramPermissionsRaw {
    #[serde(default)]
    pub content: ContentRulesRaw,
    #[serde(default)]
    pub time: TimeWindowRaw,

    #[serde(default)]
    pub send: FunctionBlockRaw,
    #[serde(default)]
    pub read: FunctionBlockRaw,
    #[serde(default)]
    pub chats: FunctionBlockRaw,
    #[serde(default)]
    pub discover: FunctionBlockRaw,

    /// Ed25519 signature over the canonical serialization of every
    /// other field. See [`crate::permissions::signing`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl HasSignature for TelegramPermissionsRaw {
    fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }
    fn set_signature(&mut self, sig: Option<Signature>) {
        self.signature = sig;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionBlockRaw {
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub chats: PatternListRaw,
    #[serde(default, skip_serializing_if = "ContentRulesRaw_is_default")]
    pub content: ContentRulesRaw,
    #[serde(default, skip_serializing_if = "TimeWindowRaw_is_default")]
    pub time: TimeWindowRaw,
    #[serde(default, skip_serializing_if = "AttachmentRulesRaw_is_default")]
    pub attachments: AttachmentRulesRaw,
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
#[allow(non_snake_case)]
fn AttachmentRulesRaw_is_default(v: &AttachmentRulesRaw) -> bool {
    v.max_count.is_none()
        && v.max_size_bytes.is_none()
        && v.extensions.allow.is_empty()
        && v.extensions.deny.is_empty()
        && v.deny_filenames.allow.is_empty()
        && v.deny_filenames.deny.is_empty()
}

// ---------------------------------------------------------------------------
// compiled form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FunctionBlock {
    pub chats: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub attachments: AttachmentRules,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            chats: PatternList::compile(&raw.chats).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            attachments: AttachmentRules::compile(&raw.attachments).map_err(ZadError::Invalid)?,
        })
    }
}

/// One file's worth of rules, compiled.
#[derive(Debug, Clone, Default)]
pub struct TelegramPermissions {
    /// Absolute path the rules were loaded from — embedded in every
    /// `PermissionDenied` error so the operator can find and edit the
    /// offending line without grep.
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub send: FunctionBlock,
    pub read: FunctionBlock,
    pub chats: FunctionBlock,
    pub discover: FunctionBlock,
}

impl TelegramPermissions {
    fn compile(raw: &TelegramPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(TelegramPermissions {
            source,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            send: FunctionBlock::compile(&raw.send)?,
            read: FunctionBlock::compile(&raw.read)?,
            chats: FunctionBlock::compile(&raw.chats)?,
            discover: FunctionBlock::compile(&raw.discover)?,
        })
    }

    fn block(&self, f: TelegramFunction) -> &FunctionBlock {
        match f {
            TelegramFunction::Send => &self.send,
            TelegramFunction::Read => &self.read,
            TelegramFunction::Chats => &self.chats,
            TelegramFunction::Discover => &self.discover,
        }
    }
}

/// Identifier for every Telegram runtime function permissions gate.
/// Kept as a small closed enum (rather than strings) so the compiler
/// catches a new verb being added without a matching permission block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramFunction {
    Send,
    Read,
    Chats,
    Discover,
}

impl TelegramFunction {
    pub fn name(self) -> &'static str {
        match self {
            TelegramFunction::Send => "send",
            TelegramFunction::Read => "read",
            TelegramFunction::Chats => "chats",
            TelegramFunction::Discover => "discover",
        }
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<TelegramPermissions>,
    pub local: Option<TelegramPermissions>,
}

impl EffectivePermissions {
    pub fn any(&self) -> bool {
        self.global.is_some() || self.local.is_some()
    }

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

    fn layers(&self) -> impl Iterator<Item = &TelegramPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    pub fn check_send_chat(
        &self,
        input: &str,
        resolved_id: i64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_chat(TelegramFunction::Send, input, resolved_id, directory)
    }

    pub fn check_read_chat(
        &self,
        input: &str,
        resolved_id: i64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_chat(TelegramFunction::Read, input, resolved_id, directory)
    }

    pub fn check_chats_chat(
        &self,
        input: &str,
        resolved_id: i64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_chat(TelegramFunction::Chats, input, resolved_id, directory)
    }

    pub fn check_discover_chat(
        &self,
        input: &str,
        resolved_id: i64,
        directory: &Directory,
    ) -> Result<()> {
        self.check_chat(TelegramFunction::Discover, input, resolved_id, directory)
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

    /// Evaluate outbound attachments against the merged
    /// `[send.attachments]` policy in each layer. Caps and pattern lists
    /// apply independently per layer (global must admit, then local must
    /// admit) so a project can only tighten the global baseline.
    pub fn check_send_attachments(&self, files: &[AttachmentInfo]) -> Result<()> {
        for p in self.layers() {
            if let Err(e) = p.send.attachments.evaluate(files) {
                return Err(ZadError::PermissionDenied {
                    function: "send",
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Time-window check for a given function. Callers invoke this at
    /// the top of every verb that could issue a network call, so the
    /// "denied" answer never leaks a target name on failure.
    pub fn check_time(&self, f: TelegramFunction) -> Result<()> {
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

    fn check_chat(
        &self,
        f: TelegramFunction,
        input: &str,
        resolved_id: i64,
        directory: &Directory,
    ) -> Result<()> {
        let stripped = input.strip_prefix('@').unwrap_or(input);
        let id_str = resolved_id.to_string();

        let mut names: Vec<String> = Vec::with_capacity(4);
        names.push(stripped.to_string());
        names.push(id_str);
        names.extend(directory.names_for_chat(resolved_id));
        names.sort();
        names.dedup();

        for p in self.layers() {
            let list = &p.block(f).chats;
            if list.is_empty() {
                continue;
            }
            let aliases: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            if let Err(e) = list.evaluate(aliases.iter().copied()) {
                return Err(ZadError::PermissionDenied {
                    function: static_name(f),
                    reason: e.as_sentence(&format!("chat `{input}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }
}

fn static_name(f: TelegramFunction) -> &'static str {
    // `ZadError::PermissionDenied::function` is a `&'static str` — used
    // for machine-readable JSON output and grep patterns in tests.
    f.name()
}

// ---------------------------------------------------------------------------
// paths + load
// ---------------------------------------------------------------------------

pub fn global_path() -> Result<PathBuf> {
    Ok(config::path::global_service_dir("telegram")?.join("permissions.toml"))
}

pub fn local_path_for(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, "telegram")?.join("permissions.toml"))
}

pub fn local_path_current() -> Result<PathBuf> {
    local_path_for(&config::path::project_slug()?)
}

/// Load a single file by path. Absent file → `Ok(None)`. Parse/compile
/// errors surface with the file path embedded in the message.
pub fn load_file(path: &Path) -> Result<Option<TelegramPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: TelegramPermissionsRaw =
        toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
            path: path.to_path_buf(),
            source: e,
        })?;
    signing::verify_raw(&raw, path)?;
    let compiled = TelegramPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

/// Read a file's raw policy (signature included) without compiling.
pub fn load_raw_file(path: &Path) -> Result<Option<TelegramPermissionsRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: TelegramPermissionsRaw =
        toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
            path: path.to_path_buf(),
            source: e,
        })?;
    Ok(Some(raw))
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

pub fn save_file(path: &Path, raw: &TelegramPermissionsRaw, key: &SigningKey) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let sig = signing::sign_raw(&to_write, key)?;
    to_write.set_signature(Some(sig));
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Write `raw` without signing. Staging-only.
pub fn save_unsigned(path: &Path, raw: &TelegramPermissionsRaw) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// A starter policy emitted by `zad telegram permissions init`. Biased
/// toward safe defaults — non-empty comment header, an illustrative
/// allow list, and content rules that catch the most obvious leak
/// vectors.
pub fn starter_template() -> TelegramPermissionsRaw {
    TelegramPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into(), "secret".into()],
            deny_patterns: vec![],
            max_length: None,
        },
        time: TimeWindowRaw::default(),
        send: FunctionBlockRaw {
            chats: PatternListRaw {
                allow: vec![],
                deny: vec!["*admin*".into(), "*mod-*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        read: FunctionBlockRaw::default(),
        chats: FunctionBlockRaw::default(),
        discover: FunctionBlockRaw::default(),
        signature: None,
    }
}

// ---------------------------------------------------------------------------
// PermissionsService binding
// ---------------------------------------------------------------------------

/// Zero-sized type used to feed the shared permissions runner with
/// Telegram-specific bindings. See
/// [`crate::permissions::service::PermissionsService`].
pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = "telegram";
    type Raw = TelegramPermissionsRaw;

    fn starter_template() -> Self::Raw {
        starter_template()
    }

    fn all_functions() -> &'static [&'static str] {
        &["send", "read", "chats", "discover"]
    }

    fn target_kinds() -> &'static [&'static str] {
        &["chat"]
    }

    fn apply_mutation(raw: &mut Self::Raw, m: &Mutation) -> Result<()> {
        let function = match m {
            Mutation::AddPattern { function, .. }
            | Mutation::RemovePattern { function, .. }
            | Mutation::AddDenyWord { function, .. }
            | Mutation::RemoveDenyWord { function, .. }
            | Mutation::AddDenyRegex { function, .. }
            | Mutation::RemoveDenyRegex { function, .. }
            | Mutation::SetMaxLength { function, .. }
            | Mutation::SetTimeDays { function, .. }
            | Mutation::SetTimeWindows { function, .. } => function.as_deref(),
        };

        let (content, time) = block_refs_mut(raw, function)?;
        if mutation::apply_content(content, m)? {
            return Ok(());
        }
        if mutation::apply_time(time, m)? {
            return Ok(());
        }

        match m {
            Mutation::AddPattern {
                function,
                target,
                list,
                value,
            }
            | Mutation::RemovePattern {
                function,
                target,
                list,
                value,
            } => {
                let add = matches!(m, Mutation::AddPattern { .. });
                let plist = pattern_list_mut(raw, function.as_deref(), target)?;
                mutation::apply_pattern_list(plist, *list, value, add);
                Ok(())
            }
            other => Err(mutation::unsupported("telegram", other)),
        }
    }
}

fn function_block_mut<'a>(
    raw: &'a mut TelegramPermissionsRaw,
    function: &str,
) -> Result<&'a mut FunctionBlockRaw> {
    Ok(match function {
        "send" => &mut raw.send,
        "read" => &mut raw.read,
        "chats" => &mut raw.chats,
        "discover" => &mut raw.discover,
        other => {
            return Err(ZadError::Invalid(format!(
                "telegram permissions: unknown function `{other}`; expected one of \
                 send, read, chats, discover"
            )));
        }
    })
}

fn block_refs_mut<'a>(
    raw: &'a mut TelegramPermissionsRaw,
    function: Option<&str>,
) -> Result<(&'a mut ContentRulesRaw, &'a mut TimeWindowRaw)> {
    match function {
        None => Ok((&mut raw.content, &mut raw.time)),
        Some(name) => {
            let block = function_block_mut(raw, name)?;
            Ok((&mut block.content, &mut block.time))
        }
    }
}

fn pattern_list_mut<'a>(
    raw: &'a mut TelegramPermissionsRaw,
    function: Option<&str>,
    target: &str,
) -> Result<&'a mut PatternListRaw> {
    let Some(name) = function else {
        return Err(ZadError::Invalid(format!(
            "telegram permissions: pattern mutations require --function (top-level {target} lists are not a Telegram schema field)"
        )));
    };
    let block = function_block_mut(raw, name)?;
    Ok(match target {
        "chat" => &mut block.chats,
        other => {
            return Err(ZadError::Invalid(format!(
                "telegram permissions: unknown target `{other}`; expected `chat`"
            )));
        }
    })
}
