//! Side-effectful surface for `zad slack <verb>`.
//!
//! [`SlackTransport`] is a thin trait over the Slack Web API call set.
//! [`DryRunSlackTransport`] intercepts mutating verbs for `--dry-run`
//! preview, following the same pattern as `src/service/discord/transport.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::Result;
use crate::service::slack::client::{SlackChannel, SlackHttp, SlackMessage, SlackUser};
use crate::service::{DryRunOp, DryRunSink};

#[async_trait]
pub trait SlackTransport: Send + Sync {
    async fn send(&self, channel_id: &str, body: &str) -> Result<String>;
    async fn send_dm(&self, user_id: &str, body: &str) -> Result<String>;
    async fn history(&self, channel_id: &str, limit: usize) -> Result<Vec<SlackMessage>>;
    async fn list_channels(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<SlackChannel>, Option<String>)>;
    async fn list_users(&self, cursor: Option<&str>) -> Result<(Vec<SlackUser>, Option<String>)>;
    async fn join_channel(&self, channel_id: &str) -> Result<()>;
}

#[async_trait]
impl SlackTransport for SlackHttp {
    async fn send(&self, channel_id: &str, body: &str) -> Result<String> {
        SlackHttp::send(self, channel_id, body).await
    }
    async fn send_dm(&self, user_id: &str, body: &str) -> Result<String> {
        SlackHttp::send_dm(self, user_id, body).await
    }
    async fn history(&self, channel_id: &str, limit: usize) -> Result<Vec<SlackMessage>> {
        SlackHttp::history(self, channel_id, limit).await
    }
    async fn list_channels(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<SlackChannel>, Option<String>)> {
        let r = SlackHttp::list_channels(self, cursor).await?;
        Ok((r.channels, r.next_cursor))
    }
    async fn list_users(&self, cursor: Option<&str>) -> Result<(Vec<SlackUser>, Option<String>)> {
        let r = SlackHttp::list_users(self, cursor).await?;
        Ok((r.users, r.next_cursor))
    }
    async fn join_channel(&self, channel_id: &str) -> Result<()> {
        SlackHttp::join_channel(self, channel_id).await
    }
}

pub struct DryRunSlackTransport {
    sink: Arc<dyn DryRunSink>,
}

impl DryRunSlackTransport {
    pub fn new(sink: Arc<dyn DryRunSink>) -> Self {
        Self { sink }
    }

    fn record(&self, verb: &'static str, summary: String, details: serde_json::Value) {
        self.sink.record(DryRunOp {
            service: "slack",
            verb,
            summary,
            details,
        });
    }
}

#[async_trait]
impl SlackTransport for DryRunSlackTransport {
    async fn send(&self, channel_id: &str, body: &str) -> Result<String> {
        let len = body.chars().count();
        self.record(
            "send",
            format!("would send {len} chars to channel {channel_id}"),
            json!({
                "command": "slack.send",
                "channel": channel_id,
                "body": body,
                "body_chars": len,
            }),
        );
        Ok("0".to_string())
    }

    async fn send_dm(&self, user_id: &str, body: &str) -> Result<String> {
        let len = body.chars().count();
        self.record(
            "send_dm",
            format!("would send {len} chars as DM to user {user_id}"),
            json!({
                "command": "slack.send",
                "target": "dm",
                "user": user_id,
                "body": body,
                "body_chars": len,
            }),
        );
        Ok("0".to_string())
    }

    async fn history(&self, _channel_id: &str, _limit: usize) -> Result<Vec<SlackMessage>> {
        Ok(vec![])
    }

    async fn list_channels(
        &self,
        _cursor: Option<&str>,
    ) -> Result<(Vec<SlackChannel>, Option<String>)> {
        Ok((vec![], None))
    }

    async fn list_users(&self, _cursor: Option<&str>) -> Result<(Vec<SlackUser>, Option<String>)> {
        Ok((vec![], None))
    }

    async fn join_channel(&self, channel_id: &str) -> Result<()> {
        self.record(
            "join_channel",
            format!("would join channel {channel_id}"),
            json!({
                "command": "slack.join",
                "channel": channel_id,
            }),
        );
        Ok(())
    }
}
