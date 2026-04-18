use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-project config stored at `~/.zad/projects/<slug>/config.toml`.
/// This file only records *which* services the project uses. Credentials
/// for those services live in the corresponding global service config
/// under `~/.zad/services/<service>/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub service: BTreeMap<String, ServiceProjectRef>,
}

/// Project-level reference to a service. All services share the same
/// on-disk shape here — the map key (`"discord"`, `"telegram"`, …) is
/// authoritative, and service-specific credentials live elsewhere
/// (`~/.zad/services/<name>/config.toml` + the OS keychain). Keeping
/// one struct avoids the `#[serde(untagged)]` ambiguity that would
/// bite if multiple services had identically-shaped project refs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceProjectRef {
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
    pub fn discord(&self) -> Option<&ServiceProjectRef> {
        self.service.get("discord")
    }

    pub fn enable_discord(&mut self) {
        self.service
            .insert("discord".to_string(), ServiceProjectRef { enabled: true });
    }

    pub fn disable_discord(&mut self) {
        self.service.remove("discord");
    }

    pub fn has_service(&self, name: &str) -> bool {
        self.service.contains_key(name)
    }
}
