//! Spotify-specific permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/spotify/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/spotify/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both files
//! are optional; when both exist, a call must pass **both** — local
//! can only add restrictions, never loosen the global baseline.
//!
//! Spotify exposes five runtime functions, each gating one verb group:
//!
//! - `search` — `zad spotify search` (target: query string).
//! - `playlists_read` — `zad spotify playlists list/show` (target:
//!   playlist name, ID, or `spotify:playlist:<id>` URI).
//! - `playlists_write` — `zad spotify playlists create/rename/delete/
//!   add/remove` (target: playlist).
//! - `library_read` — `zad spotify library {tracks,albums} list` (no
//!   target axis — the verb lists everything saved by the
//!   authenticated user).
//! - `library_write` — `zad spotify library {tracks,albums}
//!   {save,unsave}` (target: track / album URI or ID).
//!
//! Each per-function block carries a single `targets` allow / deny
//! list. The semantics of "target" is the thing being acted on, as
//! described above. Top-level `[content]` and `[time]` defaults
//! cascade into every block; per-block overrides narrow them
//! further. The TOML surface:
//!
//! ```toml
//! [content]
//! deny_words    = ["password", "api_key"]
//! deny_patterns = ["(?i)bearer\\s+[a-z0-9]+"]
//! max_length    = 256
//!
//! [time]
//! days    = ["mon","tue","wed","thu","fri"]
//! windows = ["09:00-22:00"]
//!
//! [search]
//! # No target restrictions; just inherit the global content/time.
//!
//! [playlists_read]
//! # All playlists allowed.
//!
//! [playlists_write]
//! targets.allow = ["zad-*", "scratch-*"]
//! targets.deny  = ["*release*", "*official*"]
//!
//! [library_read]
//!
//! [library_write]
//! targets.deny = ["spotify:track:5p..."]
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
pub struct SpotifyPermissionsRaw {
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
pub struct SpotifyPermissions {
    /// Absolute path the rules were loaded from — embedded in every
    /// `PermissionDenied` error so the operator can find and edit the
    /// offending line without grep.
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub search: FunctionBlock,
    pub playlists_read: FunctionBlock,
    pub playlists_write: FunctionBlock,
    pub library_read: FunctionBlock,
    pub library_write: FunctionBlock,
}

impl SpotifyPermissions {
    fn compile(raw: &SpotifyPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(SpotifyPermissions {
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

    fn block(&self, f: SpotifyFunction) -> &FunctionBlock {
        match f {
            SpotifyFunction::Search => &self.search,
            SpotifyFunction::PlaylistsRead => &self.playlists_read,
            SpotifyFunction::PlaylistsWrite => &self.playlists_write,
            SpotifyFunction::LibraryRead => &self.library_read,
            SpotifyFunction::LibraryWrite => &self.library_write,
        }
    }
}

/// Identifier for every Spotify runtime function permissions gate.
/// Closed enum so the compiler catches a new verb being added without
/// a matching permission block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyFunction {
    Search,
    PlaylistsRead,
    PlaylistsWrite,
    LibraryRead,
    LibraryWrite,
}

impl SpotifyFunction {
    pub fn name(self) -> &'static str {
        match self {
            SpotifyFunction::Search => "search",
            SpotifyFunction::PlaylistsRead => "playlists_read",
            SpotifyFunction::PlaylistsWrite => "playlists_write",
            SpotifyFunction::LibraryRead => "library_read",
            SpotifyFunction::LibraryWrite => "library_write",
        }
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<SpotifyPermissions>,
    pub local: Option<SpotifyPermissions>,
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

    fn layers(&self) -> impl Iterator<Item = &SpotifyPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    /// Time-window check for a given function. Callers invoke this at
    /// the top of every verb that could issue a network call, so the
    /// "denied" answer never leaks a target name on failure.
    pub fn check_time(&self, f: SpotifyFunction) -> Result<()> {
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

    /// Evaluate a target (playlist name/ID/URI, or track/album URI/ID,
    /// or query string) against the per-function `targets` allow/deny
    /// list in each layer. Empty lists in a layer contribute nothing
    /// (no positive constraint).
    pub fn check_target(&self, f: SpotifyFunction, target: &str) -> Result<()> {
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
    pub fn check_body(&self, f: SpotifyFunction, body: &str) -> Result<()> {
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

fn static_name(f: SpotifyFunction) -> &'static str {
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
pub fn load_file(path: &Path) -> Result<Option<SpotifyPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: SpotifyPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    signing::verify_raw(&raw, path)?;
    let compiled = SpotifyPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

/// Read a file's raw policy (signature included) without compiling.
pub fn load_raw_file(path: &Path) -> Result<Option<SpotifyPermissionsRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: SpotifyPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
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

/// Load the effective permissions for the current project, honoring
/// any `ZAD_PERMISSIONS_PATH` / `ZAD_PERMISSIONS_ROOT` override.
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

/// Load `EffectivePermissions` from explicit paths. Reads no env vars.
/// Recommended for library callers; the CLI uses [`load_effective`].
pub fn load_from(global: Option<&Path>, local: Option<&Path>) -> Result<EffectivePermissions> {
    let global = match global {
        Some(p) => load_file(p)?,
        None => None,
    };
    let local = match local {
        Some(p) => load_file(p)?,
        None => None,
    };
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(path: &Path, raw: &SpotifyPermissionsRaw, key: &SigningKey) -> Result<()> {
    save_unsigned(path, raw)?;
    let sig = signing::sign_unsigned(raw, key)?;
    let path_key = crate::permissions::trust::canonical_path_key(path)?;
    let entry = crate::permissions::TrustEntry::from_signature(path_key, sig);
    let mut store = crate::permissions::TrustStore::load()?;
    store.upsert(entry);
    store.save(key)
}

/// Write `raw` without signing. Staging-only.
pub fn save_unsigned(path: &Path, raw: &SpotifyPermissionsRaw) -> Result<()> {
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

/// A starter policy emitted by `zad spotify permissions init`. Biased
/// toward safe defaults — content rules that catch obvious leak
/// vectors and a write-side deny that prevents the agent from
/// blindly mutating "official"-looking playlists.
pub fn starter_template() -> SpotifyPermissionsRaw {
    SpotifyPermissionsRaw {
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

/// Zero-sized type used to feed the shared permissions runner with
/// Spotify-specific bindings. See
/// [`crate::permissions::service::PermissionsService`].
pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = "spotify";
    type Raw = SpotifyPermissionsRaw;

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
            other => Err(mutation::unsupported("spotify", other)),
        }
    }
}

fn function_block_mut<'a>(
    raw: &'a mut SpotifyPermissionsRaw,
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
                "spotify permissions: unknown function `{other}`; expected one of \
                 search, playlists_read, playlists_write, library_read, library_write"
            )));
        }
    })
}

fn block_refs_mut<'a>(
    raw: &'a mut SpotifyPermissionsRaw,
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
    raw: &'a mut SpotifyPermissionsRaw,
    function: Option<&str>,
    target: &str,
) -> Result<&'a mut PatternListRaw> {
    let Some(name) = function else {
        return Err(ZadError::Invalid(format!(
            "spotify permissions: pattern mutations require --function (top-level {target} lists are not a Spotify schema field)"
        )));
    };
    let block = function_block_mut(raw, name)?;
    Ok(match target {
        "target" => &mut block.targets,
        other => {
            return Err(ZadError::Invalid(format!(
                "spotify permissions: unknown target `{other}`; expected `target`"
            )));
        }
    })
}
