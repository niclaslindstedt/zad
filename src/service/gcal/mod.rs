//! Google Calendar service.
//!
//! Submodules:
//!
//! - `oauth` — loopback PKCE OAuth 2.0 flow used by `zad service
//!   create gcal` to obtain a refresh token. Kept generic over
//!   `AuthUrl`/`TokenUrl`/`Scopes`/`ClientId`/`ClientSecret` so it can
//!   later move under `src/service/oauth/` once Reddit/Spotify need
//!   the same shape.
//! - `client` — hand-rolled `reqwest` wrapper over Calendar API v3 +
//!   the OpenID `userinfo` endpoint. Handles automatic access-token
//!   refresh on each run.
//! - `transport` — trait over the runtime verbs + live/dry-run impls.
//! - `permissions` — per-service policy schema composed from
//!   `src/permissions/{pattern, content, time}.rs`.
//! - `time` — RFC3339 / bare-date parsing for `--start`/`--end`.
//!
//! ## Why no `Service` trait impl?
//!
//! The shared `crate::service::Service` trait is chat-centric
//! (`send_message`/`read_messages`/`listen`/`manage`). Calendar has no
//! meaningful `send_message` or `listen`; forcing a fit would add
//! dishonest method stubs without any runtime caller using them. Per
//! `docs/services.md` §"Adding a new service" item 9, a service that
//! skips the trait and exposes its surface directly through
//! `src/cli/<name>.rs` is a supported pattern — implement the trait
//! only when a concrete caller materialises.

pub mod client;
pub mod oauth;
pub mod permissions;
pub mod time;
pub mod transport;

pub use client::GcalHttp;
pub use transport::{DryRunGcalTransport, GcalTransport};

/// Google Calendar API base URL.
pub const API_BASE: &str = "https://www.googleapis.com/calendar/v3";
/// Google's OAuth 2.0 authorization endpoint.
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google's OAuth 2.0 token endpoint (also used for refresh).
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// OpenID Connect userinfo endpoint — hit at validate-time to capture
/// the authenticated user's email for `self_email`.
pub const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
