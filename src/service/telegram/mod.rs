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
//! service's verb set is deliberately smaller than what an IRC- or
//! Discord-style integration would expose.
//!
//! ## State of the integration
//!
//! Lifecycle (`zad service {create,enable,disable,show,delete}
//! telegram`) is **fully implemented** via `src/cli/service_telegram.rs`.
//! Registering a bot writes a flat `config.toml`, stores the bot token
//! in the OS keychain, and calls the Bot API's `getMe` endpoint to
//! validate before persisting anything.
//!
//! The runtime verbs (`zad telegram send`, `read`, `chats`, …) are
//! **stubbed** in `src/cli/telegram.rs`. Each handler returns
//! `ZadError::Invalid("... not yet implemented ...")` and carries a
//! block-comment describing the Bot API call it should make. The
//! `client.rs` and `transport.rs` skeletons here sketch the shapes
//! those handlers will fill in.
//!
//! The permissions layer (`permissions.rs`) and the directory cache
//! (`directory.rs`) are real so a project can author a policy and
//! hand-populate chat aliases before any runtime code lands.

pub mod client;
pub mod directory;
pub mod permissions;
pub mod transport;

pub use client::TelegramHttp;
pub use transport::{DryRunTelegramTransport, TelegramTransport};

// TODO: once runtime verbs land, add a `TelegramService` struct that
// implements the shared `crate::service::Service` trait. Deferring it
// now keeps the public surface honest — a no-op `impl Service` would
// be worse than no impl at all, because library callers would silently
// get `todo!()` panics instead of a compile-time "not wired yet".
