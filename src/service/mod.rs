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

/// Lightweight descriptor for a channel returned by a service's listing
/// endpoint. `kind` is a free-form, service-specific label (e.g. `"text"`,
/// `"voice"`, `"public_thread"` for Discord).
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: ChannelId,
    pub name: String,
    pub kind: String,
    pub parent: Option<ChannelId>,
    pub position: u16,
}

/// Minimal guild (server) descriptor used by the discovery surface.
#[derive(Debug, Clone)]
pub struct GuildInfo {
    pub id: u64,
    pub name: String,
}

/// Minimal member descriptor used by the discovery surface. `display_name`
/// is the name the directory should index under (server nickname if set,
/// else the user's global display name, else the raw username).
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
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
