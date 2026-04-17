use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-project config stored at `~/.zad/projects/<slug>/config.toml`.
/// This file only records *which* services the project uses. Credentials
/// for those services live in the corresponding global service config
/// under `~/.zad/services/<service>/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub service: BTreeMap<String, ServiceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ServiceRef {
    Discord(DiscordProjectRef),
}

/// Project-level reference to the Discord service. Credentials live in
/// the global service config; this struct only records that the project
/// has opted in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordProjectRef {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Global Discord service config stored at
/// `~/.zad/services/discord/config.toml`. Written as flat top-level keys
/// (no `[service.discord]` wrapper) because the file's location already
/// identifies the service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordServiceCfg {
    pub application_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_guild: Option<String>,
}

impl ProjectConfig {
    pub fn discord(&self) -> Option<&DiscordProjectRef> {
        self.service.get("discord").map(|a| match a {
            ServiceRef::Discord(d) => d,
        })
    }

    pub fn enable_discord(&mut self) {
        self.service.insert(
            "discord".to_string(),
            ServiceRef::Discord(DiscordProjectRef { enabled: true }),
        );
    }

    pub fn disable_discord(&mut self) {
        self.service.remove("discord");
    }

    pub fn has_service(&self, name: &str) -> bool {
        self.service.contains_key(name)
    }
}
