//! Slack Socket Mode gateway — real-time event stream via WebSocket.
//!
//! Requires an App-Level Token (`xapp-...`) in addition to the bot token.
//! When no app-level token is configured, [`start_listener`] returns an
//! empty stream with a tracing warning rather than an error, so the service
//! starts cleanly even without Socket Mode configured.

use futures::stream::BoxStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::error::Result;
use crate::service::{ChannelId, Event, Message, MessageId, UserId};

pub async fn start_listener(
    bot_token: String,
    app_token: Option<String>,
) -> Result<BoxStream<'static, Event>> {
    let Some(app_tok) = app_token else {
        tracing::warn!(
            "slack gateway: no app-level token configured; \
             Socket Mode events are disabled. \
             Run `zad service create slack` and provide --app-token, \
             or set one later via `zad service show slack`."
        );
        return Ok(Box::pin(tokio_stream::empty()));
    };

    let ws_url = crate::service::slack::client::SlackHttp::connections_open(&app_tok).await?;

    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        if let Err(e) = run_socket_mode(ws_url, bot_token, tx).await {
            tracing::error!(?e, "slack socket mode client exited");
        }
    });

    Ok(Box::pin(UnboundedReceiverStream::new(rx)))
}

async fn run_socket_mode(
    ws_url: String,
    _bot_token: String,
    tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let (ws_stream, _) =
        connect_async(&ws_url)
            .await
            .map_err(|e| crate::error::ZadError::Service {
                name: "slack",
                message: format!("socket mode connect: {e}"),
            })?;

    let (mut write, mut read) = ws_stream.split();
    let _ = tx.send(Event::Ready);

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(?e, "slack socket mode: read error");
                break;
            }
        };

        let text = match msg {
            WsMessage::Text(t) => t,
            WsMessage::Close(_) => break,
            WsMessage::Ping(data) => {
                let _ = write.send(WsMessage::Pong(data)).await;
                continue;
            }
            _ => continue,
        };

        let val: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(?e, "slack socket mode: invalid JSON");
                continue;
            }
        };

        // Acknowledge every envelope immediately.
        if let Some(env_id) = val.get("envelope_id").and_then(|v| v.as_str()) {
            let ack = serde_json::json!({ "envelope_id": env_id }).to_string();
            let _ = write.send(WsMessage::Text(ack)).await;
        }

        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "hello" => {}
            "events_api" => {
                let payload = match val.get("payload") {
                    Some(p) => p,
                    None => continue,
                };
                let event = match payload.get("event") {
                    Some(e) => e,
                    None => continue,
                };
                if let Some(ev) = translate_event(event) {
                    let _ = tx.send(ev);
                }
            }
            "disconnect" => {
                tracing::info!("slack socket mode: disconnect requested");
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

fn translate_event(event: &serde_json::Value) -> Option<Event> {
    let ev_type = event.get("type").and_then(|v| v.as_str())?;
    match ev_type {
        "message" => {
            let channel = event.get("channel").and_then(|v| v.as_str())?;
            let user = event.get("user").and_then(|v| v.as_str())?;
            let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let ts = event.get("ts").and_then(|v| v.as_str()).unwrap_or("0");
            Some(Event::MessageCreated(Message {
                id: MessageId(ts_to_u64(ts)),
                channel: ChannelId(slack_id_hash(channel)),
                author: UserId(slack_id_hash(user)),
                body: text.to_string(),
            }))
        }
        "message_deleted" => {
            let channel = event.get("channel").and_then(|v| v.as_str())?;
            let previous = event.get("previous_message")?;
            let ts = previous.get("ts").and_then(|v| v.as_str()).unwrap_or("0");
            Some(Event::MessageDeleted {
                channel: ChannelId(slack_id_hash(channel)),
                id: MessageId(ts_to_u64(ts)),
            })
        }
        _ => None,
    }
}

/// Convert a Slack `ts` string like `"1512085950.000216"` to a u64 by
/// taking the integer part (seconds since epoch). Lossy but deterministic
/// enough for the `Event` domain type.
fn ts_to_u64(ts: &str) -> u64 {
    ts.split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Stable hash of a Slack string ID (e.g. `"C1234567890"`) to a u64.
/// Used only for the generic `Service` trait / domain `ChannelId`/`UserId`
/// newtypes — the real Slack CLI layer always uses the original string ID.
pub fn slack_id_hash(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}
