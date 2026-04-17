use std::sync::Arc;

use serenity::all::{
    ChannelId as SerenityChannelId, CreateMessage, GetMessages, GuildId as SerenityGuildId,
    UserId as SerenityUserId,
};
use serenity::builder::CreateChannel;
use serenity::http::Http;

use crate::adapter::{ChannelId, Message, MessageId, Target, UserId};
use crate::error::Result;

/// Thin wrapper around `serenity::http::Http` that translates between the
/// adapter's domain types and serenity's.
#[derive(Clone)]
pub struct DiscordHttp {
    pub(crate) http: Arc<Http>,
}

impl DiscordHttp {
    pub fn new(token: &str) -> Self {
        Self {
            http: Arc::new(Http::new(token)),
        }
    }

    pub async fn validate_token(&self) -> Result<String> {
        let me = self.http.get_current_user().await?;
        Ok(me.name.clone())
    }

    pub async fn send(&self, target: Target, body: &str) -> Result<MessageId> {
        let channel_id = match target {
            Target::Channel(ChannelId(id)) => SerenityChannelId::new(id),
            Target::Dm(UserId(uid)) => {
                let dm = SerenityUserId::new(uid)
                    .create_dm_channel(&*self.http)
                    .await?;
                dm.id
            }
        };
        let msg = channel_id
            .send_message(&*self.http, CreateMessage::new().content(body))
            .await?;
        Ok(MessageId(msg.id.get()))
    }

    pub async fn history(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>> {
        let channel_id = SerenityChannelId::new(channel.0);
        let limit_u8: u8 = limit.min(100) as u8;
        let msgs = channel_id
            .messages(&*self.http, GetMessages::new().limit(limit_u8))
            .await?;
        Ok(msgs
            .into_iter()
            .map(|m| Message {
                id: MessageId(m.id.get()),
                channel: ChannelId(m.channel_id.get()),
                author: UserId(m.author.id.get()),
                body: m.content,
            })
            .collect())
    }

    pub async fn create_channel(&self, guild: u64, name: &str) -> Result<()> {
        SerenityGuildId::new(guild)
            .create_channel(&*self.http, CreateChannel::new(name))
            .await?;
        Ok(())
    }

    pub async fn delete_channel(&self, channel: ChannelId) -> Result<()> {
        SerenityChannelId::new(channel.0)
            .delete(&*self.http)
            .await?;
        Ok(())
    }
}
