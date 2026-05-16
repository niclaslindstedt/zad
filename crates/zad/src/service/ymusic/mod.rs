//! YouTube Music (ymusic) service.
//!
//! Runtime verbs talk to YouTube Music's internal **InnerTube**
//! backend at `music.youtube.com/youtubei/v1`. This is the same API
//! the music.youtube.com web app uses; `ytmusicapi`-style tools talk
//! to it too. The InnerTube surface is **not metered** by the
//! YouTube Data API v3 daily quota, which the older `googleapis.com`
//! transport was bound by.
//!
//! Authentication is OAuth 2.0 device flow (RFC 8628) against
//! Google's TVHTML5 client. `zad service create ymusic` runs the
//! device flow once and stores the resulting refresh token in the OS
//! keychain. There is no client_id / client_secret to manage — the
//! TVHTML5 credentials are constants compiled into the binary (and
//! shipped with `ytmusicapi`, the music.youtube.com web app, every
//! AndroidTV install, etc.; Google does not treat them as
//! confidential).
//!
//! ## Submodules
//!
//! - `client` — hand-rolled `reqwest` wrapper over InnerTube.
//!   Handles automatic access-token refresh on each run.
//! - `oauth_device` — RFC 8628 device flow against Google's
//!   TVHTML5 client.
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
pub mod facade;
pub mod oauth_device;
pub mod permissions;
pub mod transport;

pub use client::YmusicHttp;
pub use facade::{
    AddPlaylistItemRequest, CreatePlaylistRequest, LikedRequest, PlaylistsRequest, SearchRequest,
    Ymusic, YmusicCredentials,
};
pub use transport::{DryRunYmusicTransport, YmusicTransport};

/// InnerTube base URL. YouTube Music's real backend — what the
/// `music.youtube.com` web app and `ytmusicapi` talk to. Not part of
/// the YouTube Data API v3 and not metered by the Data API daily
/// quota.
pub const API_BASE: &str = "https://music.youtube.com/youtubei/v1";
/// Google's OAuth 2.0 token endpoint, used here for the device-flow
/// poll *and* the per-call access-token refresh.
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// `clientName` value sent in every InnerTube `context.client`
/// envelope. Identifies the caller as the YouTube Music web app.
pub const WEB_REMIX_CLIENT_NAME: &str = "WEB_REMIX";

/// `clientVersion` value sent alongside [`WEB_REMIX_CLIENT_NAME`].
/// InnerTube starts rejecting old strings with a 400
/// `INVALID_ARGUMENT` once they fall too far behind the live web
/// client. The format is `1.YYYYMMDD.NN.NN`; `ytmusicapi` rebuilds
/// it as `"1." + today.strftime("%Y%m%d") + ".01.00"` on every
/// run. Bump this constant whenever a 400 sweep starts hitting
/// every `/browse` call again.
pub const WEB_REMIX_CLIENT_VERSION: &str = "1.20260516.01.00";

/// Date (UTC) on which the InnerTube surface used here was last
/// walked against `ytmusicapi`'s reference + the web client's
/// observed payloads. Bump this when the audit is repeated.
pub const API_VERIFIED_AT: &str = "2026-05-16";

/// Canonical landing page describing the InnerTube surface this
/// service targets. (Google does not document it publicly; we point
/// at the community reference instead.)
pub const API_REFERENCE_URL: &str = "https://github.com/sigma67/ytmusicapi";

/// Provider's own version tag. InnerTube has no numbered version —
/// we record the WEB_REMIX clientVersion for parity with other
/// services that surface an `api_base_version`.
pub const API_BASE_VERSION: &str = WEB_REMIX_CLIENT_VERSION;
