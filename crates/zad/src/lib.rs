//! zad — typed Rust library for connecting AI agents to external services
//! (Discord, Slack, Google Calendar, Spotify, Telegram, YouTube Music,
//! 1Password) via scoped service configurations.
//!
//! The CLI binary lives in the sibling `zad-cli` crate and is a thin
//! wrapper over this library. Rust callers depend on `zad` directly,
//! pinning a version in `Cargo.toml` (`zad = "0.6"`), and call the
//! per-service facades to get typed inputs and typed responses without
//! shelling out to the binary.
//!
//! ## Quick start
//!
//! ```no_run
//! use zad::service::discord::{Discord, MessageBody, SendRequest};
//! use zad::service::{ChannelId, Target};
//!
//! # async fn doc() -> zad::Result<()> {
//! let discord = Discord::from_default_config()?;
//! let req = SendRequest::new(
//!     Target::Channel(ChannelId(123_456_789_012_345_678)),
//!     MessageBody::text("hi from a typed call"),
//!     vec![],
//! )?;
//! let resp = discord.send(req).await?;
//! println!("sent message {}", resp.message_id.0);
//! # Ok(())
//! # }
//! ```

// `ZadError` aggregates third-party error types (`keyring::Error`,
// `toml::de::Error`) that are individually over clippy's default
// 128-byte `result_large_err` threshold. Boxing every one for a
// library that returns Result a handful of times trades clarity for
// nothing measurable.
#![allow(clippy::result_large_err)]

pub mod config;
pub mod error;
pub mod logging;
pub mod oauth;
pub mod permissions;
pub mod rate_limit;
pub mod secrets;
pub mod service;

// Curated re-exports for the common path. Callers can also reach into
// the modules directly for less common operations.
pub use error::{Result, ZadError};
pub use service::{ChannelId, ChannelInfo, Message, MessageId, Target, UserId};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
