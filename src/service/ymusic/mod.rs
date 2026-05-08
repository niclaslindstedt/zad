//! YouTube Music (ymusic) service.
//!
//! YouTube Music does not expose a dedicated public API — runtime
//! verbs talk to the YouTube Data API v3 with Google OAuth, which is
//! also the entry point for the user's YouTube Music library and
//! playlists. The credential shape mirrors gcal: `client_id`,
//! `client_secret`, `refresh_token` (all in the OS keychain). The
//! OAuth 2.0 loopback flow used by `zad service create ymusic` lives
//! in the shared crate-level [`crate::oauth`] module.
//!
//! ## Submodules
//!
//! - `client` — hand-rolled `reqwest` wrapper over YouTube Data API
//!   v3. Handles automatic access-token refresh on each run.
//! - `transport` — trait over the runtime verbs + live/dry-run impls.
//! - `permissions` — per-service policy schema composed from
//!   `src/permissions/{pattern, content, time}.rs`.
//!
//! ## Why no `Service` trait impl?
//!
//! The shared `crate::service::Service` trait is chat-centric
//! (`send_message`/`read_messages`/`listen`/`manage`). YouTube Music
//! has no meaningful equivalent of any of those verbs; forcing a fit
//! would add dishonest method stubs. Same reasoning as gcal and
//! spotify — when runtime verbs land, they'll be exposed directly
//! through `src/cli/ymusic.rs`.

pub mod client;
pub mod permissions;
pub mod transport;

pub use client::YmusicHttp;
pub use transport::{DryRunYmusicTransport, YmusicTransport};

/// YouTube Data API v3 base URL. YouTube Music shares this surface —
/// playlists, library, and search are all accessed through the same
/// REST endpoints.
pub const API_BASE: &str = "https://www.googleapis.com/youtube/v3";
/// Google's OAuth 2.0 authorization endpoint.
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google's OAuth 2.0 token endpoint (also used for refresh).
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// OpenID Connect userinfo endpoint — hit at validate-time alongside
/// `channels?mine=true` to capture the authenticated user's identity.
pub const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// Compute the minimal set of Google OAuth scopes to request, given
/// the zad-level scopes the operator declared. Mirrors
/// [`crate::cli::service_gcal::google_scopes_for`]: keep the consent
/// screen as narrow as possible.
///
/// The mapping:
/// - `search` → no provider scope of its own (the YouTube Data API
///   accepts unauthenticated search, but we still need an authorized
///   session so `validate` can identify the channel).
/// - `playlists.read`, `library.read` → `youtube.readonly`.
/// - `playlists.write`, `library.write` → `youtube` (read+write
///   superset).
///
/// The OpenID Connect `openid email` scopes are always added so
/// `userinfo` can populate the authenticated email at validate time.
pub fn youtube_scopes_for(zad_scopes: &[String]) -> Vec<String> {
    let has = |s: &str| zad_scopes.iter().any(|z| z == s);
    let mut out: Vec<String> = vec!["openid".into(), "email".into()];

    if has("playlists.write") || has("library.write") {
        out.push("https://www.googleapis.com/auth/youtube".into());
    } else if has("playlists.read") || has("library.read") {
        out.push("https://www.googleapis.com/auth/youtube.readonly".into());
    }

    out.sort();
    out.dedup();
    out
}
