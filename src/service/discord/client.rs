use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use serenity::all::{
    ChannelId as SerenityChannelId, ChannelType, CreateMessage, GetMessages,
    GuildId as SerenityGuildId, UserId as SerenityUserId,
};
use serenity::builder::CreateChannel;
use serenity::http::Http;

use crate::error::{Result, ZadError};
use crate::service::{
    ChannelId, ChannelInfo, GuildInfo, MemberInfo, Message, MessageId, Target, UserId,
};

/// Discord's hard cap on a single message's character count. Discord
/// counts codepoints, not bytes — hence `chars().count()` at the
/// validation site.
pub const DISCORD_MAX_MESSAGE_LEN: usize = 2000;

/// Discord REST error code for "Unknown Channel". The JSON body uses a
/// numeric code independent of the HTTP status; this is the canonical
/// way to tell "channel snowflake the bot can't resolve" apart from
/// other 404/403s.
const DISCORD_CODE_UNKNOWN_CHANNEL: isize = 10003;

/// Thin wrapper around `serenity::http::Http` that translates between the
/// service's domain types and serenity's, and enforces the declared
/// `scopes` locally before any network call.
#[derive(Clone)]
pub struct DiscordHttp {
    pub(crate) http: Arc<Http>,
    scopes: BTreeSet<String>,
    config_path: PathBuf,
}

impl DiscordHttp {
    /// Construct a client with a declared scope set. Every subsequent
    /// method checks its required scope against this set before hitting
    /// the network. `config_path` is only used to make the error message
    /// actionable — no I/O happens against it here.
    pub fn new(token: &str, scopes: BTreeSet<String>, config_path: PathBuf) -> Self {
        Self {
            http: Arc::new(Http::new(token)),
            scopes,
            config_path,
        }
    }

    /// Construct a client with no declared scopes. Only safe for code
    /// paths that call [`Self::validate_token`] and nothing else (the
    /// `service create discord` flow validates a token *before* scopes
    /// are persisted).
    pub fn unscoped(token: &str) -> Self {
        Self::new(token, BTreeSet::new(), PathBuf::new())
    }

    fn require_scope(&self, scope: &'static str) -> Result<()> {
        if self.scopes.contains(scope) {
            return Ok(());
        }
        Err(ZadError::ScopeDenied {
            service: "discord",
            scope,
            config_path: self.config_path.clone(),
        })
    }

    pub async fn validate_token(&self) -> Result<String> {
        // No scope check: this is called during `service create discord`
        // before scopes are persisted.
        let me = self.http.get_current_user().await?;
        Ok(me.name.clone())
    }

    /// Fetch a human user's display name by snowflake. Used to validate
    /// the ID supplied for `--self-user` / `zad discord self set <id>`
    /// before persisting it.
    ///
    /// No scope check: this runs before scopes are persisted during
    /// `service create`, and on the `self set` path the call is an
    /// identity ping with no side-effects.
    pub async fn get_user(&self, id: u64) -> Result<String> {
        let user = self
            .http
            .get_user(SerenityUserId::new(id))
            .await
            .map_err(|e| map_http(e, HttpCtx::User(id)))?;
        Ok(user.name.clone())
    }

    pub async fn send(&self, target: Target, body: &str) -> Result<MessageId> {
        self.require_scope("messages.send")?;
        let len = body.chars().count();
        if len > DISCORD_MAX_MESSAGE_LEN {
            return Err(ZadError::Invalid(format!(
                "message body is {len} characters; Discord's hard limit is {DISCORD_MAX_MESSAGE_LEN}"
            )));
        }
        let (channel_id, channel_id_raw) = match target {
            Target::Channel(ChannelId(id)) => (SerenityChannelId::new(id), id),
            Target::Dm(UserId(uid)) => {
                let dm = SerenityUserId::new(uid)
                    .create_dm_channel(&*self.http)
                    .await
                    .map_err(|e| map_http(e, HttpCtx::User(uid)))?;
                let id = dm.id.get();
                (dm.id, id)
            }
        };
        let msg = channel_id
            .send_message(&*self.http, CreateMessage::new().content(body))
            .await
            .map_err(|e| map_http(e, HttpCtx::Channel(channel_id_raw)))?;
        Ok(MessageId(msg.id.get()))
    }

    pub async fn history(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>> {
        self.require_scope("messages.read")?;
        let channel_id = SerenityChannelId::new(channel.0);
        let limit_u8: u8 = limit.min(100) as u8;
        let msgs = channel_id
            .messages(&*self.http, GetMessages::new().limit(limit_u8))
            .await
            .map_err(|e| map_http(e, HttpCtx::Channel(channel.0)))?;
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
        self.require_scope("channels.manage")?;
        SerenityGuildId::new(guild)
            .create_channel(&*self.http, CreateChannel::new(name))
            .await?;
        Ok(())
    }

    pub async fn delete_channel(&self, channel: ChannelId) -> Result<()> {
        self.require_scope("channels.manage")?;
        SerenityChannelId::new(channel.0)
            .delete(&*self.http)
            .await
            .map_err(|e| map_http(e, HttpCtx::Channel(channel.0)))?;
        Ok(())
    }

    /// List every channel in `guild`, including threads and voice channels.
    /// Callers can filter by [`ChannelInfo::kind`] to narrow the result.
    pub async fn list_channels(&self, guild: u64) -> Result<Vec<ChannelInfo>> {
        self.require_scope("guilds")?;
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
        self.require_scope("guilds")?;
        self.http
            .join_thread_channel(SerenityChannelId::new(channel.0))
            .await
            .map_err(|e| map_http(e, HttpCtx::Channel(channel.0)))?;
        Ok(())
    }

    pub async fn leave_channel(&self, channel: ChannelId) -> Result<()> {
        self.require_scope("guilds")?;
        self.http
            .leave_thread_channel(SerenityChannelId::new(channel.0))
            .await
            .map_err(|e| map_http(e, HttpCtx::Channel(channel.0)))?;
        Ok(())
    }

    /// List every guild the bot currently has access to. Used by the
    /// discovery surface to seed the directory.
    pub async fn list_guilds(&self) -> Result<Vec<GuildInfo>> {
        self.require_scope("guilds")?;
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
        self.require_scope("guilds")?;
        let members = self
            .http
            .get_guild_members(SerenityGuildId::new(guild), Some(limit.min(1000)), None)
            .await
            .map_err(|e| map_http(e, HttpCtx::GuildMembers))?;
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

/// What the caller was trying to do when the HTTP error fired. Used by
/// [`classify_http`] / [`map_http`] to decide which typed variant (if
/// any) is more actionable than the generic `Discord(String)` wrapper.
#[derive(Debug, Clone, Copy)]
pub enum HttpCtx {
    /// A channel-addressed call (`send`, `history`, `delete_channel`, …).
    /// 404 or Discord code 10003 → the snowflake is wrong or invisible.
    Channel(u64),
    /// A DM create (`send --dm`). Same treatment as `Channel` for
    /// unknown-target diagnostics; the id is the user snowflake, not a
    /// channel, but the message is clearer if we say so.
    User(u64),
    /// `GET /guilds/{id}/members`. A 403 here almost always means the
    /// `GUILD_MEMBERS` privileged intent isn't enabled.
    GuildMembers,
}

/// Pure status-code/discord-code → typed-variant mapping. Exposed so
/// tests can exercise the decision table without constructing
/// `serenity::Error` (which is `#[non_exhaustive]` and not portably
/// forgeable from outside the crate). Returns `None` when the
/// status/code combination doesn't warrant a typed variant and the
/// caller should fall back to the string wrapper.
pub fn classify_http(ctx: HttpCtx, status: u16, code: isize) -> Option<ZadError> {
    match ctx {
        HttpCtx::Channel(id) | HttpCtx::User(id) => {
            if status == 404 || code == DISCORD_CODE_UNKNOWN_CHANNEL {
                Some(ZadError::DiscordChannelNotFound { id })
            } else {
                None
            }
        }
        HttpCtx::GuildMembers => {
            if status == 403 {
                Some(ZadError::DiscordPrivilegedIntent {
                    intent: "GUILD_MEMBERS",
                })
            } else {
                None
            }
        }
    }
}

/// Map a `serenity::Error` into a typed `ZadError` when we can, falling
/// back to the generic wrapper otherwise. Centralized here so a future
/// serenity upgrade that reshapes the error enum has one place to fix.
pub(crate) fn map_http(err: serenity::Error, ctx: HttpCtx) -> ZadError {
    if let serenity::Error::Http(ref http_err) = err
        && let serenity::http::HttpError::UnsuccessfulRequest(resp) = http_err
        && let Some(mapped) = classify_http(ctx, resp.status_code.as_u16(), resp.error.code)
    {
        return mapped;
    }
    ZadError::Service {
        name: "discord",
        message: err.to_string(),
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
