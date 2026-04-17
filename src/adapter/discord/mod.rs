pub mod client;
pub mod gateway;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::adapter::{Adapter, ChannelId, Event, ManageCmd, Message, MessageId, Target};
use crate::error::Result;

pub use client::DiscordHttp;

pub struct DiscordAdapter {
    http: DiscordHttp,
    token: String,
}

impl DiscordAdapter {
    /// Construct an adapter from a bot token. Does not validate the token —
    /// call [`DiscordHttp::validate_token`] via [`DiscordAdapter::http`] if
    /// you need eager validation.
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        Self {
            http: DiscordHttp::new(&token),
            token,
        }
    }

    pub fn http(&self) -> &DiscordHttp {
        &self.http
    }
}

#[async_trait]
impl Adapter for DiscordAdapter {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn send_message(&self, target: Target, body: &str) -> Result<MessageId> {
        self.http.send(target, body).await
    }

    async fn read_messages(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>> {
        self.http.history(channel, limit).await
    }

    async fn listen(&self) -> Result<BoxStream<'static, Event>> {
        gateway::start_listener(self.token.clone()).await
    }

    async fn manage(&self, cmd: ManageCmd) -> Result<()> {
        match cmd {
            ManageCmd::CreateChannel { guild, name } => {
                self.http.create_channel(guild, &name).await
            }
            ManageCmd::DeleteChannel { channel } => self.http.delete_channel(channel).await,
        }
    }
}
