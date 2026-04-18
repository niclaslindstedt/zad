//! Project-local name → chat_id directory for the Telegram service.
//!
//! Telegram addresses every target — DMs, groups, supergroups, and
//! channels — with a single signed `chat_id`, so this directory holds
//! one `chats` map rather than splitting by target kind. Chat IDs are
//! stored as strings to preserve the leading `-` that (super)groups
//! carry without needing a custom deserializer.
//!
//! The file at
//! `~/.zad/projects/<slug>/services/telegram/directory.toml` is an
//! optional cache. Runtime verbs accept raw IDs or `@usernames`
//! directly — the directory is a convenience layer on top, and
//! hand-authored entries are preserved across re-discovery.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::path;
use crate::error::{Result, ZadError};

/// Telegram-specific name → chat_id mapping persisted in TOML.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Directory {
    /// Seconds since the Unix epoch at which `discover` last wrote this
    /// file. `None` if the file was only ever hand-authored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at_unix: Option<u64>,

    /// Human alias → chat_id (as a decimal string). The value carries
    /// a leading `-` for group/supergroup chats.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chats: BTreeMap<String, String>,
}

impl Directory {
    pub fn total(&self) -> usize {
        self.chats.len()
    }

    /// Resolve a chat reference in priority order:
    ///
    /// 1. Decimal integer (optionally leading `-`) → parse as `i64`.
    /// 2. `@username` → look up under the stripped name.
    /// 3. Bare alias → look up verbatim.
    ///
    /// Returns the signed chat_id on success. The Bot API passes the
    /// chat_id through as an integer on every request, so we keep the
    /// sign rather than losing it in a `u64`.
    pub fn resolve_chat(&self, input: &str) -> Option<i64> {
        if let Ok(id) = input.parse::<i64>() {
            return Some(id);
        }
        let key = input.strip_prefix('@').unwrap_or(input);
        self.chats.get(key).and_then(|s| s.parse::<i64>().ok())
    }

    /// Reverse-lookup: names the directory records for a given chat_id.
    /// Used by the permissions layer so a deny on `*admin*` fires even
    /// when the agent pasted the raw ID.
    pub fn names_for_chat(&self, id: i64) -> Vec<String> {
        let id_s = id.to_string();
        self.chats
            .iter()
            .filter(|(_, v)| **v == id_s)
            .map(|(k, _)| k.clone())
            .collect()
    }
}

pub fn path_for(slug: &str) -> Result<PathBuf> {
    Ok(path::project_service_dir_for(slug, "telegram")?.join("directory.toml"))
}

pub fn path_current() -> Result<PathBuf> {
    path_for(&path::project_slug()?)
}

pub fn load_from(p: &Path) -> Result<Directory> {
    if !p.exists() {
        return Ok(Directory::default());
    }
    let raw = std::fs::read_to_string(p).map_err(|e| ZadError::Io {
        path: p.to_owned(),
        source: e,
    })?;
    toml::from_str(&raw).map_err(|e| ZadError::TomlParse {
        path: p.to_owned(),
        source: e,
    })
}

pub fn save_to(p: &Path, dir: &Directory) -> Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_owned(),
            source: e,
        })?;
    }
    let body = toml::to_string_pretty(dir)?;
    std::fs::write(p, body).map_err(|e| ZadError::Io {
        path: p.to_owned(),
        source: e,
    })
}

pub fn load() -> Result<Directory> {
    load_from(&path_current()?)
}

pub fn save(dir: &Directory) -> Result<()> {
    save_to(&path_current()?, dir)
}
