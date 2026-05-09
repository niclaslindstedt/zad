pub mod client;
pub mod facade;
pub mod gateway;
pub mod permissions;
pub mod transport;

use std::collections::BTreeSet;
use std::path::PathBuf;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::error::{Result, ZadError};
use crate::service::{ChannelId, Event, ManageCmd, Message, MessageId, Service, Target};

pub use client::SlackHttp;
pub use facade::{
    ChannelsRequest, ReadRequest, SendRequest, SendResponse, Slack, SlackChannelId, SlackTarget,
    SlackUserId,
};
pub use transport::{DryRunSlackTransport, SlackTransport};

pub struct SlackService {
    http: SlackHttp,
    token: String,
    app_token: Option<String>,
}

impl SlackService {
    pub fn new(
        token: impl Into<String>,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        app_token: Option<String>,
    ) -> Self {
        let token = token.into();
        Self {
            http: SlackHttp::new(&token, scopes, config_path),
            token,
            app_token,
        }
    }

    pub fn http(&self) -> &SlackHttp {
        &self.http
    }
}

#[async_trait]
impl Service for SlackService {
    fn name(&self) -> &'static str {
        "slack"
    }

    async fn send_message(&self, _target: Target, _body: &str) -> Result<MessageId> {
        Err(ZadError::Invalid(
            "Slack uses string channel IDs; use `zad slack send --channel <ID> --body <text>`"
                .into(),
        ))
    }

    async fn read_messages(&self, _channel: ChannelId, _limit: usize) -> Result<Vec<Message>> {
        Err(ZadError::Invalid(
            "Slack uses string channel IDs; use `zad slack read --channel <ID>`".into(),
        ))
    }

    async fn listen(&self) -> Result<BoxStream<'static, Event>> {
        gateway::start_listener(self.token.clone(), self.app_token.clone()).await
    }

    async fn manage(&self, _cmd: ManageCmd) -> Result<()> {
        Err(ZadError::Invalid(
            "Slack channel management is not yet supported via the generic Service trait".into(),
        ))
    }
}
