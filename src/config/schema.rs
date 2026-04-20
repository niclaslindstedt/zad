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
    /// Numeric snowflake of the human user this bot belongs to. Populated
    /// at `zad service create discord` time (or later via
    /// `zad discord self set`) and resolved from the literal `@me` in
    /// send targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_user_id: Option<String>,
}

/// Global Google Calendar (gcal) service config stored at
/// `~/.zad/services/gcal/config.toml`. gcal authenticates via OAuth
/// 2.0 rather than a bot token, so all three credential pieces
/// (`client_id`, `client_secret`, `refresh_token`) live in the OS
/// keychain; this struct carries only the non-secret metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcalServiceCfg {
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional calendar the runtime verbs default to when `--calendar`
    /// is omitted. Accepts a bare calendar ID, `primary`, or a directory
    /// alias. Absent means every verb must name its calendar
    /// explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_calendar: Option<String>,
    /// The authenticated user's primary email, captured during
    /// `zad service create gcal` from Google's userinfo endpoint.
    /// Resolves the literal `@me` in attendee targets, same role
    /// `self_user_id` plays for Discord.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_email: Option<String>,
}

/// Global 1Password (1pass) service config stored at
/// `~/.zad/services/1pass/config.toml`. Authentication is always via a
/// 1Password Service Account token (agent-first; no desktop / biometric
/// flow), stored in the OS keychain. This struct carries only the
/// non-secret fields: the sign-in host (`account`), an optional
/// `default_vault` convenience for commands that omit `--vault`, and
/// the declared scopes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnePassServiceCfg {
    /// 1Password sign-in address — e.g. `my.1password.com` or
    /// `team.1password.eu`. `op` reads this via the `OP_ACCOUNT`
    /// environment variable so we don't need to pass a `--account`
    /// flag on every shell-out.
    pub account: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional vault the runtime verbs default to when `--vault` is
    /// omitted. Accepts a vault name or UUID. Absent means every verb
    /// must name its vault explicitly (or accept `op`'s own search
    /// across all visible vaults).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_vault: Option<String>,
}

/// Global GitHub service config stored at
/// `~/.zad/services/github/config.toml`. Authentication is a single
/// Personal Access Token held in the OS keychain; this struct carries
/// only the non-secret metadata. Runtime verbs shell out to the `gh`
/// CLI with `GH_TOKEN` set from the keychain entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubServiceCfg {
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional default repository for verbs that omit `--repo`.
    /// Format: `owner/name` (e.g. `octocat/hello-world`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_repo: Option<String>,
    /// Optional default owner (user or org) for verbs like `code search`
    /// that scope to an org. Used when `--org` is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_owner: Option<String>,
    /// The authenticated user's GitHub login, captured during
    /// `zad service create github` from `gh api user`. Displayed in
    /// `show` output; not currently resolved as a target in any verb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_login: Option<String>,
}

/// Global Telegram service config stored at
/// `~/.zad/services/telegram/config.toml`. Telegram bots carry their
/// identity inside the bot token itself (no separate app ID), and
/// address every target — private chat, group, supergroup, channel —
/// through a single `chat_id`. So the config only needs the declared
/// scopes plus an optional `default_chat` for verbs that omit the
/// flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramServiceCfg {
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional default chat for commands that omit `--chat`. Can be a
    /// numeric chat ID (groups/supergroups are negative), a `@username`
    /// for public channels, or a directory alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_chat: Option<String>,
    /// The private-chat ID Telegram assigns to the human user this bot
    /// belongs to. Populated at `zad service create telegram` time (or
    /// later via `zad telegram self capture|set`) once the user has
    /// sent a message to the bot, and resolved from the literal `@me`
    /// in send targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_chat_id: Option<i64>,
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

    pub fn telegram(&self) -> Option<&ServiceProjectRef> {
        self.service.get("telegram")
    }

    pub fn enable_telegram(&mut self) {
        self.service
            .insert("telegram".to_string(), ServiceProjectRef { enabled: true });
    }

    pub fn disable_telegram(&mut self) {
        self.service.remove("telegram");
    }

    pub fn one_pass(&self) -> Option<&ServiceProjectRef> {
        self.service.get("1pass")
    }

    pub fn enable_one_pass(&mut self) {
        self.service
            .insert("1pass".to_string(), ServiceProjectRef { enabled: true });
    }

    pub fn disable_one_pass(&mut self) {
        self.service.remove("1pass");
    }

    pub fn gcal(&self) -> Option<&ServiceProjectRef> {
        self.service.get("gcal")
    }

    pub fn enable_gcal(&mut self) {
        self.service
            .insert("gcal".to_string(), ServiceProjectRef { enabled: true });
    }

    pub fn disable_gcal(&mut self) {
        self.service.remove("gcal");
    }

    pub fn github(&self) -> Option<&ServiceProjectRef> {
        self.service.get("github")
    }

    pub fn enable_github(&mut self) {
        self.service
            .insert("github".to_string(), ServiceProjectRef { enabled: true });
    }

    pub fn disable_github(&mut self) {
        self.service.remove("github");
    }

    pub fn has_service(&self, name: &str) -> bool {
        self.service.contains_key(name)
    }
}
