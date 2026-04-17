use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-project config stored at `~/.zad/projects/<slug>/config.toml`.
/// This file only records *which* adapters the project uses. Credentials
/// for those adapters live in the corresponding global adapter config
/// under `~/.zad/adapters/<adapter>/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter: BTreeMap<String, AdapterRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AdapterRef {
    Discord(DiscordProjectRef),
}

/// Project-level reference to the Discord adapter. Credentials live in
/// the global adapter config; this struct only records that the project
/// has opted in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordProjectRef {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Global Discord adapter config stored at
/// `~/.zad/adapters/discord/config.toml`. Written as flat top-level keys
/// (no `[adapter.discord]` wrapper) because the file's location already
/// identifies the adapter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordAdapterCfg {
    pub application_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_guild: Option<String>,
}

impl ProjectConfig {
    pub fn discord(&self) -> Option<&DiscordProjectRef> {
        self.adapter.get("discord").map(|a| match a {
            AdapterRef::Discord(d) => d,
        })
    }

    pub fn enable_discord(&mut self) {
        self.adapter.insert(
            "discord".to_string(),
            AdapterRef::Discord(DiscordProjectRef { enabled: true }),
        );
    }

    pub fn disable_discord(&mut self) {
        self.adapter.remove("discord");
    }

    pub fn has_adapter(&self, name: &str) -> bool {
        self.adapter.contains_key(name)
    }
}
