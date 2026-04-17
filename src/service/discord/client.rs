use std::sync::Arc;

use serenity::all::{
    ChannelId as SerenityChannelId, ChannelType, CreateMessage, GetMessages,
    GuildId as SerenityGuildId, UserId as SerenityUserId,
};
use serenity::builder::CreateChannel;
use serenity::http::Http;

use crate::error::Result;
use crate::service::{
    ChannelId, ChannelInfo, GuildInfo, MemberInfo, Message, MessageId, Target, UserId,
};

/// Thin wrapper around `serenity::http::Http` that translates between the
/// service's domain types and serenity's.
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

    /// List every channel in `guild`, including threads and voice channels.
    /// Callers can filter by [`ChannelInfo::kind`] to narrow the result.
    pub async fn list_channels(&self, guild: u64) -> Result<Vec<ChannelInfo>> {
        let channels = self.http.get_channels(SerenityGuildId::new(guild)).await?;
        let mut out: Vec<ChannelInfo> = channels
            .into_iter()
            .map(|c| ChannelInfo {
                id: ChannelId(c.id.get()),
                name: c.name,
                kind: channel_kind_name(c.kind).to_string(),
                parent: c.parent_id.map(|p| ChannelId(p.get())),
                position: c.position,
            })
            .collect();
        out.sort_by(|a, b| a.position.cmp(&b.position).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    /// Join a thread channel. Discord only allows the bot to explicitly
    /// "join" threads — guild text/voice channels are joined implicitly by
    /// having the Guild membership and the right permissions.
    pub async fn join_channel(&self, channel: ChannelId) -> Result<()> {
        self.http
            .join_thread_channel(SerenityChannelId::new(channel.0))
            .await?;
        Ok(())
    }

    pub async fn leave_channel(&self, channel: ChannelId) -> Result<()> {
        self.http
            .leave_thread_channel(SerenityChannelId::new(channel.0))
            .await?;
        Ok(())
    }

    /// List every guild the bot currently has access to. Used by the
    /// discovery surface to seed the directory.
    pub async fn list_guilds(&self) -> Result<Vec<GuildInfo>> {
        let mut out: Vec<GuildInfo> = vec![];
        let mut after: Option<serenity::all::GuildId> = None;
        // Discord caps the `limit` query at 200. Paginate so operators
        // running the bot in more than 200 guilds aren't silently
        // truncated.
        loop {
            let page = self
                .http
                .get_guilds(after.map(serenity::all::GuildPagination::After), Some(200))
                .await?;
            if page.is_empty() {
                break;
            }
            let last = page.last().map(|g| g.id);
            out.extend(page.into_iter().map(|g| GuildInfo {
                id: g.id.get(),
                name: g.name,
            }));
            match last {
                Some(id) => after = Some(id),
                None => break,
            }
        }
        Ok(out)
    }

    /// List up to `limit` members of `guild`. Requires the bot to have
    /// the `GUILD_MEMBERS` privileged intent enabled in the developer
    /// portal; otherwise Discord 4xx's and the caller should fall back
    /// to a warning rather than aborting discovery.
    pub async fn list_members(&self, guild: u64, limit: u64) -> Result<Vec<MemberInfo>> {
        let members = self
            .http
            .get_guild_members(SerenityGuildId::new(guild), Some(limit.min(1000)), None)
            .await?;
        Ok(members
            .into_iter()
            .map(|m| MemberInfo {
                id: UserId(m.user.id.get()),
                display_name: m
                    .nick
                    .clone()
                    .or_else(|| m.user.global_name.clone())
                    .unwrap_or_else(|| m.user.name.clone()),
                username: m.user.name,
            })
            .collect())
    }
}

fn channel_kind_name(kind: ChannelType) -> &'static str {
    match kind {
        ChannelType::Text => "text",
        ChannelType::Private => "dm",
        ChannelType::Voice => "voice",
        ChannelType::GroupDm => "group_dm",
        ChannelType::Category => "category",
        ChannelType::News => "news",
        ChannelType::NewsThread => "news_thread",
        ChannelType::PublicThread => "public_thread",
        ChannelType::PrivateThread => "private_thread",
        ChannelType::Stage => "stage",
        ChannelType::Directory => "directory",
        ChannelType::Forum => "forum",
        _ => "unknown",
    }
}
