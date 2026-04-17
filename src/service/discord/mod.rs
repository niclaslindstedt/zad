pub mod client;
pub mod gateway;
pub mod permissions;
pub mod transport;

use std::collections::BTreeSet;
use std::path::PathBuf;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::error::Result;
use crate::service::{ChannelId, Event, ManageCmd, Message, MessageId, Service, Target};

pub use client::DiscordHttp;
pub use transport::{DiscordTransport, DryRunDiscordTransport};

pub struct DiscordService {
    http: DiscordHttp,
    token: String,
}

impl DiscordService {
    /// Construct a service from a bot token and its declared scope set.
    /// `config_path` is referenced only in scope-violation error
    /// messages so the caller can find and edit the right file; no I/O
    /// happens against it here. Does not validate the token — call
    /// [`DiscordHttp::validate_token`] via [`DiscordService::http`] if
    /// you need eager validation.
    pub fn new(token: impl Into<String>, scopes: BTreeSet<String>, config_path: PathBuf) -> Self {
        let token = token.into();
        Self {
            http: DiscordHttp::new(&token, scopes, config_path),
            token,
        }
    }

    pub fn http(&self) -> &DiscordHttp {
        &self.http
    }
}

#[async_trait]
impl Service for DiscordService {
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
