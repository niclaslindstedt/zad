//! Minimal example: send a Discord message from a Rust program by
//! depending on `zad` as a library, with no shell-out to the binary.
//!
//! Prerequisites:
//! - `zad service create discord` has been run (so a bot token lives
//!   in the OS keychain) and the project this is running from has
//!   `zad service enable discord` configured.
//! - You know a channel ID to send to. Replace `CHANNEL_ID` below.
//!
//! Run with: `cargo run --manifest-path examples/discord-library/Cargo.toml`

use zad::service::discord::{Discord, MessageBody, SendRequest};
use zad::service::{ChannelId, Target};

const CHANNEL_ID: u64 = 123_456_789_012_345_678;

#[tokio::main]
async fn main() -> zad::Result<()> {
    // Loads the same project-local-then-global config and bot token
    // the CLI uses.
    let discord = Discord::from_default_config()?;

    // Validation runs at construction — wrong-shape calls are caught
    // here, not at the network boundary.
    let req = SendRequest::new(
        Target::Channel(ChannelId(CHANNEL_ID)),
        MessageBody::text("hi from a typed Rust call"),
        vec![],
    )?;

    let resp = discord.send(req).await?;
    println!("sent message {} to channel {}", resp.message_id.0, CHANNEL_ID);

    Ok(())
}
