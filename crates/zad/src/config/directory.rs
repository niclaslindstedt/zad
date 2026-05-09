//! Project-local name → snowflake directory for the Discord service.
//!
//! `zad discord discover` walks the bot's visible guilds/channels/members
//! and writes `~/.zad/projects/<slug>/services/discord/directory.toml`;
//! runtime verbs (`send`, `read`, `channels`, `join`, `leave`) consult
//! the same file so a human or agent can say `--channel general` or
//! `--dm alice` instead of pasting a 19-digit snowflake.
//!
//! The file is plain TOML with three string→string maps. Hand-edited
//! entries round-trip: `discover` loads the existing directory, upserts
//! whatever the API returned, and writes the union back. Entries the
//! API no longer knows about are left alone — deletion is an explicit
//! operator decision, not a side effect of rediscovery.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::path;
use crate::error::{Result, ZadError};

/// Discord-specific name → snowflake mapping persisted in TOML.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Directory {
    /// Seconds since the Unix epoch at which `discover` last wrote this
    /// file. `None` if the file was only ever hand-authored. Agents can
    /// use it to decide when to re-run discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at_unix: Option<u64>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub guilds: BTreeMap<String, String>,

    /// Channel entries are keyed by either a bare name (`general`) or a
    /// guild-qualified form (`main-server/general`). The qualified form
    /// always wins at lookup time when both exist and the caller has a
    /// guild context. Both forms coexist so `--channel general` works
    /// without disambiguation in a single-guild setup.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub users: BTreeMap<String, String>,
}

impl Directory {
    pub fn total(&self) -> usize {
        self.guilds.len() + self.channels.len() + self.users.len()
    }

    /// Resolve a channel reference, trying (in order) numeric snowflake,
    /// qualified `guild/name` lookup, guild-scoped lookup using
    /// `context_guild`, then bare-name lookup. A leading `#` is stripped.
    pub fn resolve_channel(&self, input: &str, context_guild: Option<&str>) -> Option<u64> {
        if let Ok(id) = input.parse::<u64>() {
            return Some(id);
        }
        let key = input.strip_prefix('#').unwrap_or(input);
        if key.contains('/')
            && let Some(id) = self.channels.get(key).and_then(|s| s.parse().ok())
        {
            return Some(id);
        }
        if let Some(g) = context_guild {
            let qualified = format!("{g}/{key}");
            if let Some(id) = self.channels.get(&qualified).and_then(|s| s.parse().ok()) {
                return Some(id);
            }
        }
        self.channels.get(key).and_then(|s| s.parse().ok())
    }

    /// Resolve a user reference. Strips a leading `@`.
    pub fn resolve_user(&self, input: &str) -> Option<u64> {
        if let Ok(id) = input.parse::<u64>() {
            return Some(id);
        }
        let key = input.strip_prefix('@').unwrap_or(input);
        self.users.get(key).and_then(|s| s.parse().ok())
    }

    /// Resolve a guild reference.
    pub fn resolve_guild(&self, input: &str) -> Option<u64> {
        if let Ok(id) = input.parse::<u64>() {
            return Some(id);
        }
        self.guilds.get(input).and_then(|s| s.parse().ok())
    }

    /// Reverse-lookup: find the name a guild snowflake was discovered
    /// under, for producing guild-qualified channel keys during
    /// discovery.
    pub fn guild_name_for(&self, id: u64) -> Option<&str> {
        self.guilds
            .iter()
            .find(|(_, v)| v.parse::<u64>() == Ok(id))
            .map(|(k, _)| k.as_str())
    }
}

pub fn path_for(slug: &str) -> Result<PathBuf> {
    Ok(path::project_service_dir_for(slug, "discord")?.join("directory.toml"))
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
