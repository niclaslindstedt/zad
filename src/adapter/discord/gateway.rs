use async_trait::async_trait;
use futures::stream::BoxStream;
use serenity::all::{Context, EventHandler, GatewayIntents, Ready};
use serenity::model::channel::Message as SerenityMessage;
use serenity::model::id::{ChannelId as SerenityChannelId, MessageId as SerenityMessageId};
use serenity::prelude::Client;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::adapter::{ChannelId, Event, Message, MessageId, UserId};
use crate::error::Result;

struct Forwarder {
    tx: mpsc::UnboundedSender<Event>,
}

#[async_trait]
impl EventHandler for Forwarder {
    async fn ready(&self, _ctx: Context, _data_about_bot: Ready) {
        let _ = self.tx.send(Event::Ready);
    }

    async fn message(&self, _ctx: Context, msg: SerenityMessage) {
        let ev = Event::MessageCreated(Message {
            id: MessageId(msg.id.get()),
            channel: ChannelId(msg.channel_id.get()),
            author: UserId(msg.author.id.get()),
            body: msg.content,
        });
        let _ = self.tx.send(ev);
    }

    async fn message_delete(
        &self,
        _ctx: Context,
        channel_id: SerenityChannelId,
        deleted_message_id: SerenityMessageId,
        _guild_id: Option<serenity::model::id::GuildId>,
    ) {
        let _ = self.tx.send(Event::MessageDeleted {
            channel: ChannelId(channel_id.get()),
            id: MessageId(deleted_message_id.get()),
        });
    }
}

pub async fn start_listener(token: String) -> Result<BoxStream<'static, Event>> {
    let (tx, rx) = mpsc::unbounded_channel();
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(&token, intents)
        .event_handler(Forwarder { tx })
        .await?;

    tokio::spawn(async move {
        if let Err(e) = client.start().await {
            tracing::error!(?e, "discord gateway client exited");
        }
    });

    Ok(Box::pin(UnboundedReceiverStream::new(rx)))
}
