//! Telegram service.
//!
//! Submodules:
//!
//! - `client` — HTTP wrapper around Telegram's Bot API.
//! - `transport` — trait over the runtime verbs + live/dry-run impls.
//! - `directory` — project-local name → chat_id cache.
//! - `permissions` — per-service policy layered on top of the declared
//!   scopes.
//!
//! Telegram's Bot API exposes a flat `chat_id` address space covering
//! private chats, groups, supergroups, and channels. Bots are added to
//! chats by a human admin rather than joining on their own, so this
//! service's verb set is deliberately smaller than what, say, an IRC
//! integration would expose — there are no `join` / `leave` verbs and
//! no separate user/channel/guild axes in the permissions schema.
//!
//! Lifecycle (`zad service {create,enable,disable,show,delete}
//! telegram`) is wired via `src/cli/service_telegram.rs`; runtime
//! verbs (`zad telegram {send,read,chats,discover,directory,
//! permissions}`) live in `src/cli/telegram.rs` and call through the
//! [`TelegramTransport`] trait so `--dry-run` can swap in a preview
//! impl without touching the keychain.

pub mod client;
pub mod directory;
pub mod facade;
pub mod permissions;
pub mod transport;

pub use client::TelegramHttp;
pub use facade::{ChatId, ChatsRequest, ReadRequest, SendRequest, SendResponse, Telegram};
pub use transport::{DryRunTelegramTransport, TelegramTransport};
