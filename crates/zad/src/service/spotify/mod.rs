//! Spotify service.
//!
//! Submodules:
//!
//! - `client` — hand-rolled `reqwest` wrapper over Spotify Web API
//!   v1. Handles automatic access-token refresh on each run, mirrors
//!   `gcal/client.rs` for consistency.
//! - `permissions` — per-service policy schema composed from
//!   `src/permissions/{pattern, content, time}.rs`.
//!
//! The OAuth 2.0 PKCE loopback flow used by `zad service create
//! spotify` lives in the shared crate-level [`crate::oauth`] module —
//! Spotify uses the **public-client** path (no `client_secret`).
//!
//! ## Why no `Service` trait impl?
//!
//! The shared `crate::service::Service` trait is chat-centric
//! (`send_message`/`read_messages`/`listen`/`manage`). Spotify has no
//! meaningful equivalent of any of those verbs; forcing a fit would
//! add dishonest method stubs. Per `docs/services.md` §"Adding a new
//! service" item 9, exposing the runtime surface directly through
//! `src/cli/spotify.rs` is a supported pattern when a service doesn't
//! map cleanly to the chat-centric trait. Same reasoning as gcal.

pub mod client;
pub mod facade;
pub mod permissions;

pub use client::SpotifyHttp;
pub use facade::{
    CreatePlaylistRequest, PlaylistsRequest, SavedTracksRequest, SearchRequest, Spotify,
    SpotifyCredentials,
};

/// Spotify Web API base URL.
pub const API_BASE: &str = "https://api.spotify.com/v1";
/// Spotify's OAuth 2.0 authorization endpoint.
pub const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
/// Spotify's OAuth 2.0 token endpoint (also used for refresh).
pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Compute the minimal set of Spotify OAuth scopes to request, given
/// the zad-level scopes the operator declared. Mirrors
/// [`crate::cli::service_gcal::google_scopes_for`]: keep the consent
/// screen as narrow as possible.
///
/// The mapping:
/// - `search` → no provider scope (search is reachable through any
///   user-auth token) — but we still need an authorized session so
///   `validate` (`GET /me`) can identify the user.
/// - `playlists.read` → `playlist-read-private`,
///   `playlist-read-collaborative`.
/// - `playlists.write` → `playlist-modify-private`,
///   `playlist-modify-public` (write implies read, so the read scopes
///   are added too).
/// - `library.read` → `user-library-read`.
/// - `library.write` → `user-library-modify` (plus
///   `user-library-read`).
pub fn spotify_scopes_for(zad_scopes: &[String]) -> Vec<String> {
    let has = |s: &str| zad_scopes.iter().any(|z| z == s);
    let mut out: Vec<String> = Vec::new();

    if has("playlists.read") || has("playlists.write") {
        out.push("playlist-read-private".into());
        out.push("playlist-read-collaborative".into());
    }
    if has("playlists.write") {
        out.push("playlist-modify-private".into());
        out.push("playlist-modify-public".into());
    }
    if has("library.read") || has("library.write") {
        out.push("user-library-read".into());
    }
    if has("library.write") {
        out.push("user-library-modify".into());
    }

    out.sort();
    out.dedup();
    out
}
