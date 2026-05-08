//! Slack-specific permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/slack/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/slack/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both files
//! are optional; when both exist, a call must pass **both** — local can
//! only add restrictions, never loosen the global baseline.
//!
//! ```toml
//! [content]
//! deny_words    = ["password", "api_key"]
//! deny_patterns = ["(?i)bearer\\s+[a-z0-9]+"]
//! max_length    = 2000
//!
//! [time]
//! days    = ["mon","tue","wed","thu","fri"]
//! windows = ["09:00-18:00"]
//!
//! [send]
//! channels.allow = ["general", "team-*"]
//! channels.deny  = ["*admin*"]
//! users.allow    = []
//!
//! [read]
//! channels.deny = ["*private*"]
//!
//! [channels]
//! workspaces.allow = []
//!
//! [discover]
//! workspaces.allow = []
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::directory::Directory;
use crate::error::{Result, ZadError};
use crate::permissions::{
    content::{ContentRules, ContentRulesRaw},
    mutation::{self, Mutation},
    pattern::{PatternList, PatternListRaw},
    service::HasSignature,
    signing::{self, Signature, SigningKey},
    time::{TimeWindow, TimeWindowRaw},
};

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlackPermissionsRaw {
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
    pub discover: FunctionBlockRaw,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl HasSignature for SlackPermissionsRaw {
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
    pub channels: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub users: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub workspaces: PatternListRaw,
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
    pub workspaces: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            channels: PatternList::compile(&raw.channels).map_err(ZadError::Invalid)?,
            users: PatternList::compile(&raw.users).map_err(ZadError::Invalid)?,
            workspaces: PatternList::compile(&raw.workspaces).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct SlackPermissions {
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub send: FunctionBlock,
    pub read: FunctionBlock,
    pub channels: FunctionBlock,
    pub discover: FunctionBlock,
}

impl SlackPermissions {
    fn compile(raw: &SlackPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(SlackPermissions {
            source,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            send: FunctionBlock::compile(&raw.send)?,
            read: FunctionBlock::compile(&raw.read)?,
            channels: FunctionBlock::compile(&raw.channels)?,
            discover: FunctionBlock::compile(&raw.discover)?,
        })
    }

    fn block(&self, f: SlackFunction) -> &FunctionBlock {
        match f {
            SlackFunction::Send => &self.send,
            SlackFunction::Read => &self.read,
            SlackFunction::Channels => &self.channels,
            SlackFunction::Discover => &self.discover,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackFunction {
    Send,
    Read,
    Channels,
    Discover,
}

impl SlackFunction {
    pub fn name(self) -> &'static str {
        match self {
            SlackFunction::Send => "send",
            SlackFunction::Read => "read",
            SlackFunction::Channels => "channels",
            SlackFunction::Discover => "discover",
        }
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<SlackPermissions>,
    pub local: Option<SlackPermissions>,
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

    fn layers(&self) -> impl Iterator<Item = &SlackPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    pub fn check_send_channel(&self, input: &str, directory: &Directory) -> Result<()> {
        self.check_target(SlackFunction::Send, TargetKind::Channel, input, directory)
    }

    pub fn check_send_dm(&self, input: &str, directory: &Directory) -> Result<()> {
        self.check_target(SlackFunction::Send, TargetKind::User, input, directory)
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

    pub fn check_read_channel(&self, input: &str, directory: &Directory) -> Result<()> {
        self.check_target(SlackFunction::Read, TargetKind::Channel, input, directory)
    }

    pub fn check_channels_workspace(&self, input: &str, directory: &Directory) -> Result<()> {
        self.check_target(
            SlackFunction::Channels,
            TargetKind::Workspace,
            input,
            directory,
        )
    }

    pub fn check_discover_workspace(&self, input: &str, directory: &Directory) -> Result<()> {
        self.check_target(
            SlackFunction::Discover,
            TargetKind::Workspace,
            input,
            directory,
        )
    }

    pub fn check_time(&self, f: SlackFunction) -> Result<()> {
        for p in self.layers() {
            let merged = p.time.clone().merge(p.block(f).time.clone());
            if let Err(e) = merged.evaluate_now() {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    fn check_target(
        &self,
        f: SlackFunction,
        kind: TargetKind,
        input: &str,
        directory: &Directory,
    ) -> Result<()> {
        let stripped = input
            .strip_prefix('#')
            .or_else(|| input.strip_prefix('@'))
            .unwrap_or(input);

        let mut names: Vec<String> = Vec::with_capacity(4);
        names.push(stripped.to_string());
        names.extend(kind.names_for(directory, stripped));
        names.sort();
        names.dedup();

        for p in self.layers() {
            let list = match kind {
                TargetKind::Channel => &p.block(f).channels,
                TargetKind::User => &p.block(f).users,
                TargetKind::Workspace => &p.block(f).workspaces,
            };
            if list.is_empty() {
                continue;
            }
            let aliases: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            if let Err(e) = list.evaluate(aliases.iter().copied()) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(&format!("{} `{}`", kind.label(), input)),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Channel,
    User,
    Workspace,
}

impl TargetKind {
    fn label(self) -> &'static str {
        match self {
            TargetKind::Channel => "channel",
            TargetKind::User => "user",
            TargetKind::Workspace => "workspace",
        }
    }

    fn names_for(self, directory: &Directory, id: &str) -> Vec<String> {
        match self {
            TargetKind::Channel => directory
                .channels
                .iter()
                .filter(|(_, v)| v.as_str() == id)
                .map(|(k, _)| k.clone())
                .collect(),
            TargetKind::User => directory
                .users
                .iter()
                .filter(|(_, v)| v.as_str() == id)
                .map(|(k, _)| k.clone())
                .collect(),
            TargetKind::Workspace => directory
                .guilds
                .iter()
                .filter(|(_, v)| v.as_str() == id)
                .map(|(k, _)| k.clone())
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// paths + load
// ---------------------------------------------------------------------------

pub fn global_path() -> Result<PathBuf> {
    crate::permissions::service::global_path::<PermissionsService>()
}

pub fn local_path_for(slug: &str) -> Result<PathBuf> {
    crate::permissions::service::local_path_for::<PermissionsService>(slug)
}

pub fn local_path_current() -> Result<PathBuf> {
    crate::permissions::service::local_path_current::<PermissionsService>()
}

pub fn load_file(path: &Path) -> Result<Option<SlackPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: SlackPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    signing::verify_raw(&raw, path)?;
    let compiled = SlackPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

pub fn load_raw_file(path: &Path) -> Result<Option<SlackPermissionsRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: SlackPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
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

pub fn load_effective() -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_current()?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn load_effective_for(slug: &str) -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_for(slug)?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(path: &Path, raw: &SlackPermissionsRaw, key: &SigningKey) -> Result<()> {
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

pub fn save_unsigned(path: &Path, raw: &SlackPermissionsRaw) -> Result<()> {
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

pub fn starter_template() -> SlackPermissionsRaw {
    SlackPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into(), "secret".into()],
            deny_patterns: vec![],
            max_length: None,
        },
        time: TimeWindowRaw::default(),
        send: FunctionBlockRaw {
            channels: PatternListRaw {
                allow: vec![],
                deny: vec!["*admin*".into(), "*ops*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        read: FunctionBlockRaw::default(),
        channels: FunctionBlockRaw::default(),
        discover: FunctionBlockRaw::default(),
        signature: None,
    }
}

// ---------------------------------------------------------------------------
// PermissionsService binding
// ---------------------------------------------------------------------------

pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = "slack";
    type Raw = SlackPermissionsRaw;

    fn starter_template() -> Self::Raw {
        starter_template()
    }

    fn all_functions() -> &'static [&'static str] {
        &["send", "read", "channels", "discover"]
    }

    fn target_kinds() -> &'static [&'static str] {
        &["channel", "user", "workspace"]
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
            other => Err(mutation::unsupported("slack", other)),
        }
    }
}

fn function_block_mut<'a>(
    raw: &'a mut SlackPermissionsRaw,
    function: &str,
) -> Result<&'a mut FunctionBlockRaw> {
    Ok(match function {
        "send" => &mut raw.send,
        "read" => &mut raw.read,
        "channels" => &mut raw.channels,
        "discover" => &mut raw.discover,
        other => {
            return Err(ZadError::Invalid(format!(
                "slack permissions: unknown function `{other}`; expected one of \
                 send, read, channels, discover"
            )));
        }
    })
}

fn block_refs_mut<'a>(
    raw: &'a mut SlackPermissionsRaw,
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
    raw: &'a mut SlackPermissionsRaw,
    function: Option<&str>,
    target: &str,
) -> Result<&'a mut PatternListRaw> {
    let Some(name) = function else {
        return Err(ZadError::Invalid(format!(
            "slack permissions: pattern mutations require --function \
             (top-level {target} lists are not a Slack schema field)"
        )));
    };
    let block = function_block_mut(raw, name)?;
    Ok(match target {
        "channel" => &mut block.channels,
        "user" => &mut block.users,
        "workspace" => &mut block.workspaces,
        other => {
            return Err(ZadError::Invalid(format!(
                "slack permissions: unknown target `{other}`; \
                 expected one of channel, user, workspace"
            )));
        }
    })
}
