use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{Result, ZadError};
use crate::service::{ChannelId, ChannelInfo, GuildInfo, MemberInfo, Message, MessageId, UserId};

const SLACK_MAX_MESSAGE_LEN: usize = 40_000;
const SLACK_API_BASE: &str = "https://slack.com/api";

/// Thin reqwest wrapper around the Slack Web API. All methods check the
/// declared `scopes` before touching the network and map `"ok": false`
/// API errors to `ZadError::Service { name: "slack" }`.
#[derive(Clone)]
pub struct SlackHttp {
    client: reqwest::Client,
    token: String,
    scopes: BTreeSet<String>,
    pub(crate) config_path: PathBuf,
}

impl SlackHttp {
    pub fn new(token: impl Into<String>, scopes: BTreeSet<String>, config_path: PathBuf) -> Self {
        Self {
            client: reqwest::Client::new(),
            token: token.into(),
            scopes,
            config_path,
        }
    }

    pub fn unscoped(token: impl Into<String>) -> Self {
        Self::new(token, BTreeSet::new(), PathBuf::new())
    }

    fn require_scope(&self, scope: &'static str) -> Result<()> {
        if self.scopes.contains(scope) {
            return Ok(());
        }
        Err(ZadError::ScopeDenied {
            service: "slack",
            scope,
            config_path: self.config_path.clone(),
        })
    }

    async fn post<T: serde::Serialize>(&self, method: &str, body: &T) -> Result<serde_json::Value> {
        let url = format!("{SLACK_API_BASE}/{method}");
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| ZadError::Service {
                name: "slack",
                message: format!("{method}: {e}"),
            })?;
        let val: serde_json::Value = resp.json().await.map_err(|e| ZadError::Service {
            name: "slack",
            message: format!("{method}: failed to parse response: {e}"),
        })?;
        if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown_error");
            return Err(ZadError::Service {
                name: "slack",
                message: format!("{method}: {err}"),
            });
        }
        Ok(val)
    }

    async fn get(&self, method: &str, params: &[(&str, &str)]) -> Result<serde_json::Value> {
        let url = format!("{SLACK_API_BASE}/{method}");
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .query(params)
            .send()
            .await
            .map_err(|e| ZadError::Service {
                name: "slack",
                message: format!("{method}: {e}"),
            })?;
        let val: serde_json::Value = resp.json().await.map_err(|e| ZadError::Service {
            name: "slack",
            message: format!("{method}: failed to parse response: {e}"),
        })?;
        if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown_error");
            return Err(ZadError::Service {
                name: "slack",
                message: format!("{method}: {err}"),
            });
        }
        Ok(val)
    }

    /// Validate the bot token via `auth.test`. Returns the display identity
    /// string `@<username> in <workspace>`. No scope check — called before
    /// scopes are persisted during `service create slack`.
    pub async fn auth_test(&self) -> Result<AuthTestInfo> {
        let val = self.post("auth.test", &serde_json::json!({})).await?;
        let user_id = string_field(&val, "user_id")?;
        let team = string_field(&val, "team")?;
        let url = val
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let user = val
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or(&user_id)
            .to_string();
        let team_id = val
            .get("team_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(AuthTestInfo {
            user_id,
            user,
            team,
            team_id,
            url,
        })
    }

    /// Send a message to a channel by ID.
    pub async fn send(&self, channel_id: &str, body: &str) -> Result<String> {
        self.require_scope("chat:write")?;
        let len = body.chars().count();
        if len > SLACK_MAX_MESSAGE_LEN {
            return Err(ZadError::Invalid(format!(
                "message body is {len} characters; Slack's hard limit is {SLACK_MAX_MESSAGE_LEN}"
            )));
        }
        let val = self
            .post(
                "chat.postMessage",
                &serde_json::json!({ "channel": channel_id, "text": body }),
            )
            .await?;
        let ts = string_field(&val, "ts")?;
        Ok(ts)
    }

    /// Open (or retrieve) a DM channel with a user, then send to it.
    pub async fn send_dm(&self, user_id: &str, body: &str) -> Result<String> {
        self.require_scope("im:write")?;
        let open_val = self
            .post(
                "conversations.open",
                &serde_json::json!({ "users": user_id }),
            )
            .await?;
        let channel_id = open_val
            .get("channel")
            .and_then(|c| c.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ZadError::Service {
                name: "slack",
                message: "conversations.open: no channel.id in response".into(),
            })?
            .to_string();
        // Switch to chat:write scope (already checked im:write above).
        self.require_scope("chat:write")?;
        let val = self
            .post(
                "chat.postMessage",
                &serde_json::json!({ "channel": channel_id, "text": body }),
            )
            .await?;
        string_field(&val, "ts")
    }

    /// Fetch message history from a channel.
    pub async fn history(&self, channel_id: &str, limit: usize) -> Result<Vec<SlackMessage>> {
        let needs = if channel_id.starts_with('D') {
            "im:history"
        } else {
            "channels:history"
        };
        self.require_scope(needs)?;
        let limit_s = limit.min(200).to_string();
        let val = self
            .get(
                "conversations.history",
                &[("channel", channel_id), ("limit", &limit_s)],
            )
            .await?;
        let msgs = val
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(msgs.len());
        for m in msgs {
            let ts = m
                .get("ts")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let user = m
                .get("user")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let text = m
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            out.push(SlackMessage {
                ts,
                user,
                text,
                channel: channel_id.to_string(),
            });
        }
        Ok(out)
    }

    /// List conversations (channels) the bot can see in the workspace.
    pub async fn list_channels(&self, cursor: Option<&str>) -> Result<ListChannelsResult> {
        self.require_scope("channels:read")?;
        let cursor_val = cursor.unwrap_or("");
        let params: &[(&str, &str)] = if cursor_val.is_empty() {
            &[
                ("types", "public_channel,private_channel"),
                ("limit", "200"),
                ("exclude_archived", "true"),
            ]
        } else {
            &[
                ("types", "public_channel,private_channel"),
                ("limit", "200"),
                ("exclude_archived", "true"),
                ("cursor", cursor_val),
            ]
        };
        let val = self.get("conversations.list", params).await?;
        let channels = val
            .get("channels")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let next_cursor = val
            .get("response_metadata")
            .and_then(|m| m.get("next_cursor"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let mut out = Vec::with_capacity(channels.len());
        for c in channels {
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = c
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_private = c
                .get("is_private")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.push(SlackChannel {
                id,
                name,
                is_private,
            });
        }
        Ok(ListChannelsResult {
            channels: out,
            next_cursor,
        })
    }

    /// List all workspace members.
    pub async fn list_users(&self, cursor: Option<&str>) -> Result<ListUsersResult> {
        self.require_scope("users:read")?;
        let cursor_val = cursor.unwrap_or("");
        let params: &[(&str, &str)] = if cursor_val.is_empty() {
            &[("limit", "200")]
        } else {
            &[("limit", "200"), ("cursor", cursor_val)]
        };
        let val = self.get("users.list", params).await?;
        let members = val
            .get("members")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let next_cursor = val
            .get("response_metadata")
            .and_then(|m| m.get("next_cursor"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let mut out = Vec::with_capacity(members.len());
        for m in members {
            if m.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            if m.get("is_bot").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            let id = m
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = m
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let display = m
                .get("profile")
                .and_then(|p| p.get("display_name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&name)
                .to_string();
            out.push(SlackUser { id, name, display });
        }
        Ok(ListUsersResult {
            users: out,
            next_cursor,
        })
    }

    /// Request the bot joins a channel. Requires `channels:join` scope.
    pub async fn join_channel(&self, channel_id: &str) -> Result<()> {
        self.require_scope("channels:join")?;
        self.post(
            "conversations.join",
            &serde_json::json!({ "channel": channel_id }),
        )
        .await?;
        Ok(())
    }

    /// Fetch the app-level token's WebSocket URL for Socket Mode.
    /// Called from `gateway.rs` with the app-level token (`xapp-...`).
    pub async fn connections_open(app_token: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{SLACK_API_BASE}/apps.connections.open"))
            .bearer_auth(app_token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| ZadError::Service {
                name: "slack",
                message: format!("apps.connections.open: {e}"),
            })?;
        let val: serde_json::Value = resp.json().await.map_err(|e| ZadError::Service {
            name: "slack",
            message: format!("apps.connections.open: {e}"),
        })?;
        if val.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown_error");
            return Err(ZadError::Service {
                name: "slack",
                message: format!("apps.connections.open: {err}"),
            });
        }
        val.get("url")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| ZadError::Service {
                name: "slack",
                message: "apps.connections.open: no url in response".into(),
            })
    }

    // ---------- domain-type adapters (for Service trait) ----------

    pub fn channel_info_to_domain(c: &SlackChannel) -> ChannelInfo {
        ChannelInfo {
            id: ChannelId(0),
            name: c.name.clone(),
            kind: if c.is_private {
                "private".to_string()
            } else {
                "public".to_string()
            },
            parent: None,
            position: 0,
        }
    }

    pub fn guild_info_workspace(team: &str, team_id: &str) -> GuildInfo {
        GuildInfo {
            id: 0,
            name: format!("{team} ({team_id})"),
        }
    }

    pub fn member_to_domain(u: &SlackUser) -> MemberInfo {
        MemberInfo {
            id: UserId(0),
            username: u.name.clone(),
            display_name: u.display.clone(),
        }
    }

    pub fn message_to_domain(m: &SlackMessage) -> Message {
        Message {
            id: MessageId(0),
            channel: ChannelId(0),
            author: UserId(0),
            body: m.text.clone(),
        }
    }
}

// ---------- response types ----------

#[derive(Debug, Clone, Deserialize)]
pub struct AuthTestInfo {
    pub user_id: String,
    pub user: String,
    pub team: String,
    pub team_id: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct SlackMessage {
    pub ts: String,
    pub user: String,
    pub text: String,
    pub channel: String,
}

#[derive(Debug, Clone)]
pub struct SlackChannel {
    pub id: String,
    pub name: String,
    pub is_private: bool,
}

#[derive(Debug, Clone)]
pub struct SlackUser {
    pub id: String,
    pub name: String,
    pub display: String,
}

#[derive(Debug)]
pub struct ListChannelsResult {
    pub channels: Vec<SlackChannel>,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub struct ListUsersResult {
    pub users: Vec<SlackUser>,
    pub next_cursor: Option<String>,
}

fn string_field(val: &serde_json::Value, key: &str) -> Result<String> {
    val.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| ZadError::Service {
            name: "slack",
            message: format!("missing `{key}` in Slack API response"),
        })
}
