//! YouTube Music–specific permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/ymusic/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/ymusic/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both
//! files are optional; when both exist, a call must pass **both** —
//! local can only add restrictions, never loosen the global baseline.
//!
//! YouTube Music exposes five runtime functions, each gating one
//! verb group:
//!
//! - `search` — `zad ymusic search` (target: query string).
//! - `playlists_read` — `zad ymusic playlists list/show` (target:
//!   playlist title or ID).
//! - `playlists_write` — `zad ymusic playlists
//!   create/rename/delete/add/remove` (target: playlist or video ID).
//! - `library_read` — `zad ymusic library list` (target: video ID).
//! - `library_write` — `zad ymusic library {like,unlike}` (target:
//!   video ID).
//!
//! Each per-function block carries a single `targets` allow / deny
//! list. Top-level `[content]` and `[time]` defaults cascade into
//! every block; per-block overrides narrow them further.
//!
//! ```toml
//! [content]
//! deny_words    = ["password", "api_key"]
//! max_length    = 256
//!
//! [time]
//! days    = ["mon","tue","wed","thu","fri"]
//! windows = ["09:00-22:00"]
//!
//! [search]
//!
//! [playlists_read]
//!
//! [playlists_write]
//! targets.allow = ["zad-*", "scratch-*"]
//! targets.deny  = ["*release*", "*official*"]
//!
//! [library_read]
//!
//! [library_write]
//! targets.deny = ["dQw4w9WgXcQ"]
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZadError};
use crate::permissions::{
    content::{ContentRules, ContentRulesRaw},
    mutation::{self, Mutation},
    pattern::{PatternList, PatternListRaw},
    signing::{self, SigningKey},
    time::{TimeWindow, TimeWindowRaw},
};

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct YmusicPermissionsRaw {
    #[serde(default)]
    pub content: ContentRulesRaw,
    #[serde(default)]
    pub time: TimeWindowRaw,

    #[serde(default)]
    pub search: FunctionBlockRaw,
    #[serde(default)]
    pub playlists_read: FunctionBlockRaw,
    #[serde(default)]
    pub playlists_write: FunctionBlockRaw,
    #[serde(default)]
    pub library_read: FunctionBlockRaw,
    #[serde(default)]
    pub library_write: FunctionBlockRaw,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionBlockRaw {
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub targets: PatternListRaw,
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
    pub targets: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            targets: PatternList::compile(&raw.targets).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
        })
    }
}

/// One file's worth of rules, compiled.
#[derive(Debug, Clone, Default)]
pub struct YmusicPermissions {
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub search: FunctionBlock,
    pub playlists_read: FunctionBlock,
    pub playlists_write: FunctionBlock,
    pub library_read: FunctionBlock,
    pub library_write: FunctionBlock,
}

impl YmusicPermissions {
    fn compile(raw: &YmusicPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(YmusicPermissions {
            source,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            search: FunctionBlock::compile(&raw.search)?,
            playlists_read: FunctionBlock::compile(&raw.playlists_read)?,
            playlists_write: FunctionBlock::compile(&raw.playlists_write)?,
            library_read: FunctionBlock::compile(&raw.library_read)?,
            library_write: FunctionBlock::compile(&raw.library_write)?,
        })
    }

    fn block(&self, f: YmusicFunction) -> &FunctionBlock {
        match f {
            YmusicFunction::Search => &self.search,
            YmusicFunction::PlaylistsRead => &self.playlists_read,
            YmusicFunction::PlaylistsWrite => &self.playlists_write,
            YmusicFunction::LibraryRead => &self.library_read,
            YmusicFunction::LibraryWrite => &self.library_write,
        }
    }
}

/// Identifier for every YouTube Music runtime function permissions
/// gate. Closed enum so the compiler catches a new verb being added
/// without a matching permission block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YmusicFunction {
    Search,
    PlaylistsRead,
    PlaylistsWrite,
    LibraryRead,
    LibraryWrite,
}

impl YmusicFunction {
    pub fn name(self) -> &'static str {
        match self {
            YmusicFunction::Search => "search",
            YmusicFunction::PlaylistsRead => "playlists_read",
            YmusicFunction::PlaylistsWrite => "playlists_write",
            YmusicFunction::LibraryRead => "library_read",
            YmusicFunction::LibraryWrite => "library_write",
        }
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<YmusicPermissions>,
    pub local: Option<YmusicPermissions>,
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

    fn layers(&self) -> impl Iterator<Item = &YmusicPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    /// Time-window check for a given function.
    pub fn check_time(&self, f: YmusicFunction) -> Result<()> {
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

    /// Evaluate a target (playlist title/ID, video ID, or query
    /// string) against the per-function `targets` allow/deny list.
    pub fn check_target(&self, f: YmusicFunction, target: &str) -> Result<()> {
        for p in self.layers() {
            let list = &p.block(f).targets;
            if list.is_empty() {
                continue;
            }
            let aliases = std::iter::once(target);
            if let Err(e) = list.evaluate(aliases) {
                return Err(ZadError::PermissionDenied {
                    function: static_name(f),
                    reason: e.as_sentence(&format!("target `{target}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Evaluate a body of text (e.g. a new playlist's `description`,
    /// or a search query) against the merged content rules of the
    /// given function.
    pub fn check_body(&self, f: YmusicFunction, body: &str) -> Result<()> {
        for p in self.layers() {
            let merged = p.content.clone().merge(p.block(f).content.clone());
            if let Err(e) = merged.evaluate(body) {
                return Err(ZadError::PermissionDenied {
                    function: static_name(f),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }
}

fn static_name(f: YmusicFunction) -> &'static str {
    f.name()
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

/// Load a single file by path. Absent file → `Ok(None)`. Parse / compile
/// errors surface with the file path embedded in the message.
pub fn load_file(path: &Path) -> Result<Option<YmusicPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: YmusicPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    signing::verify_raw(&raw, path)?;
    let compiled = YmusicPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

/// Read a file's raw policy (signature included) without compiling.
pub fn load_raw_file(path: &Path) -> Result<Option<YmusicPermissionsRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: YmusicPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
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
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_current()?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn load_effective_for(slug: &str) -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_for(slug)?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(path: &Path, raw: &YmusicPermissionsRaw, key: &SigningKey) -> Result<()> {
    save_unsigned(path, raw)?;
    let sig = signing::sign_unsigned(raw, key)?;
    let path_key = crate::permissions::trust::canonical_path_key(path)?;
    let entry = crate::permissions::TrustEntry::from_signature(path_key, sig);
    let mut store = crate::permissions::TrustStore::load()?;
    store.upsert(entry);
    store.save(key)
}

/// Write `raw` without signing. Staging-only.
pub fn save_unsigned(path: &Path, raw: &YmusicPermissionsRaw) -> Result<()> {
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

/// Starter policy emitted by `zad ymusic permissions init`. Biased
/// toward safe defaults — content rules that catch obvious leak
/// vectors and a write-side deny that prevents the agent from
/// blindly mutating "official"-looking playlists.
pub fn starter_template() -> YmusicPermissionsRaw {
    YmusicPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into(), "secret".into()],
            deny_patterns: vec![],
            max_length: None,
        },
        time: TimeWindowRaw::default(),
        search: FunctionBlockRaw::default(),
        playlists_read: FunctionBlockRaw::default(),
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec![],
                deny: vec!["*release*".into(), "*official*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        library_read: FunctionBlockRaw::default(),
        library_write: FunctionBlockRaw::default(),
    }
}

// ---------------------------------------------------------------------------
// PermissionsService binding
// ---------------------------------------------------------------------------

pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = "ymusic";
    type Raw = YmusicPermissionsRaw;

    fn starter_template() -> Self::Raw {
        starter_template()
    }

    fn all_functions() -> &'static [&'static str] {
        &[
            "search",
            "playlists_read",
            "playlists_write",
            "library_read",
            "library_write",
        ]
    }

    fn target_kinds() -> &'static [&'static str] {
        &["target"]
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
            other => Err(mutation::unsupported("ymusic", other)),
        }
    }
}

fn function_block_mut<'a>(
    raw: &'a mut YmusicPermissionsRaw,
    function: &str,
) -> Result<&'a mut FunctionBlockRaw> {
    Ok(match function {
        "search" => &mut raw.search,
        "playlists_read" => &mut raw.playlists_read,
        "playlists_write" => &mut raw.playlists_write,
        "library_read" => &mut raw.library_read,
        "library_write" => &mut raw.library_write,
        other => {
            return Err(ZadError::Invalid(format!(
                "ymusic permissions: unknown function `{other}`; expected one of \
                 search, playlists_read, playlists_write, library_read, library_write"
            )));
        }
    })
}

fn block_refs_mut<'a>(
    raw: &'a mut YmusicPermissionsRaw,
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
    raw: &'a mut YmusicPermissionsRaw,
    function: Option<&str>,
    target: &str,
) -> Result<&'a mut PatternListRaw> {
    let Some(name) = function else {
        return Err(ZadError::Invalid(format!(
            "ymusic permissions: pattern mutations require --function (top-level {target} \
             lists are not a YouTube Music schema field)"
        )));
    };
    let block = function_block_mut(raw, name)?;
    Ok(match target {
        "target" => &mut block.targets,
        other => {
            return Err(ZadError::Invalid(format!(
                "ymusic permissions: unknown target `{other}`; expected `target`"
            )));
        }
    })
}
