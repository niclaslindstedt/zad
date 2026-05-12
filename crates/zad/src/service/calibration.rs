//! Provenance for the third-party API surfaces zad wraps.
//!
//! Every service module publishes an [`ApiCalibration`] describing the
//! upstream API version it targets, the URL of the canonical reference
//! it was last verified against, and the date of that verification.
//! Callers that produce auditable artefacts (e.g. `spotifai export`)
//! can drop the calibration into their output so a future reader can
//! tell which generation of the upstream API the data was sourced
//! against without having to crack open the zad source tree.
//!
//! Constants live in each service's `mod.rs` and are wired through a
//! `const fn calibration()` on the typed facade. Bumping the
//! verification date is a manual ritual: re-walk the relevant API
//! reference pages, update the `*_API_VERIFIED_AT` constant, and ship
//! a release.

use serde::{Deserialize, Serialize};

/// Snapshot of which upstream API revision a zad service was verified
/// against. Returned by `Spotify::calibration()` /
/// `Ymusic::calibration()` (and equivalents on other services as the
/// pattern spreads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiCalibration {
    /// Short service identifier (`"spotify"`, `"ymusic"`, …) matching
    /// the slug used in `secrets::account` and `~/.zad/state/<service>`.
    pub service: &'static str,
    /// ISO 8601 date (YYYY-MM-DD, UTC) when the zad implementation was
    /// last walked against the upstream reference docs and found
    /// consistent. Updated by hand at audit time.
    pub verified_at: &'static str,
    /// URL of the canonical upstream API reference. Always a stable
    /// landing page rather than a deep link to one endpoint.
    pub reference_url: &'static str,
    /// Provider's own version tag for the API surface (`"v1"` for
    /// Spotify Web API, `"v3"` for YouTube Data API). Tracks the
    /// version segment in the base URL.
    pub api_base_version: &'static str,
}
