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

/// Project-level entry for a single service under the `[service.*]`
/// table. The map key (e.g. `"discord"`, `"telegram"`) is the real
/// discriminator; the enum exists so each service can evolve its
/// per-project fields independently.
///
/// Note: `#[serde(untagged)]` means every variant is currently
/// structurally interchangeable (all just `{ enabled: bool }`), so a
/// round-tripped value may deserialize as the first matching variant.
/// Lookups therefore always go through [`ProjectConfig::has_service`] by
/// name — never by matching the variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ServiceRef {
    Discord(DiscordProjectRef),
    Telegram(TelegramProjectRef),
}

/// Project-level reference to the Discord service. Credentials live in
/// the global service config; this struct only records that the project
/// has opted in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordProjectRef {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Project-level reference to the Telegram service. Credentials live in
/// the global service config; this struct only records that the project
/// has opted in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramProjectRef {
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

/// Global Telegram service config stored at
/// `~/.zad/services/telegram/config.toml`. Written as flat top-level
/// keys (no `[service.telegram]` wrapper) because the file's location
/// already identifies the service.
///
/// Telegram bots are identified solely by the bot token issued by
/// @BotFather — there is no separate "application ID" like Discord's
/// snowflake. The bot ID is the numeric prefix of the token itself and
/// can be recovered at runtime via `getMe`, so zad only records the
/// declared scopes plus an optional default chat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramServiceCfg {
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional default chat ID (integer) or `@username` for public
    /// channels / groups. Resolved at runtime against the project's
    /// directory.toml when the caller omits `--chat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_chat: Option<String>,
}

impl ProjectConfig {
    pub fn discord(&self) -> Option<&DiscordProjectRef> {
        match self.service.get("discord")? {
            ServiceRef::Discord(d) => Some(d),
            ServiceRef::Telegram(_) => None,
        }
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

    pub fn enable_telegram(&mut self) {
        self.service.insert(
            "telegram".to_string(),
            ServiceRef::Telegram(TelegramProjectRef { enabled: true }),
        );
    }

    pub fn disable_telegram(&mut self) {
        self.service.remove("telegram");
    }

    pub fn has_service(&self, name: &str) -> bool {
        self.service.contains_key(name)
    }
}
