//! 1Password service — agent-first wrapper around the `op` CLI.
//!
//! The binary this module shells out to is
//! [`op`](https://developer.1password.com/docs/cli). We intentionally
//! expose only a narrow, read-oriented verb surface plus one scoped
//! write verb (`create`) so that an agent configured with a 1Password
//! Service Account token can't destroy vault state by accident.
//! Dangerous surfaces — `op user`, `op group`, `op vault create/edit/
//! delete`, `op item edit/delete/share`, `op document`,
//! `op events-api`, `op run` — are never wired through `zad`.
//!
//! Unlike `discord` / `telegram` / `gcal`, this service does **not**
//! implement [`crate::service::Service`]. That trait is shaped around
//! message / event streams (send, read, listen, manage) and does not
//! fit a secrets store. The runtime CLI in `src/cli/onepass.rs` talks
//! to [`client::OnePassClient`] directly.
//!
//! The hidden-target semantics — "anything outside the permission
//! scope is presented as if it doesn't exist" — live in
//! [`permissions`] and are enforced by
//! [`permissions::EffectivePermissions::filter_*`] before any
//! `op`-output leaves this module.

pub mod client;
pub mod permissions;

/// Unit handle for the generic lifecycle driver. Kept next to the
/// service module so `src/cli/service_onepass.rs` can refer to a type
/// named in a uniform way across services (`DiscordService`,
/// `GcalService`, `OnePassService`, …) even though we don't implement
/// the shared `Service` trait.
pub struct OnePassService;
