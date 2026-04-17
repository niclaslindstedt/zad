pub mod discord;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(pub u64);

#[derive(Debug, Clone)]
pub enum Target {
    Channel(ChannelId),
    Dm(UserId),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub channel: ChannelId,
    pub author: UserId,
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    MessageCreated(Message),
    MessageDeleted { channel: ChannelId, id: MessageId },
    Ready,
}

#[derive(Debug, Clone)]
pub enum ManageCmd {
    CreateChannel { guild: u64, name: String },
    DeleteChannel { channel: ChannelId },
}

#[async_trait]
pub trait Service: Send + Sync {
    fn name(&self) -> &'static str;
    async fn send_message(&self, target: Target, body: &str) -> Result<MessageId>;
    async fn read_messages(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>>;
    async fn listen(&self) -> Result<BoxStream<'static, Event>>;
    async fn manage(&self, cmd: ManageCmd) -> Result<()>;
}
