//! Spotify HTTP client.
//!
//! Hand-rolled `reqwest` wrapper over Spotify Web API v1. Mirrors
//! `gcal/client.rs` for consistency: holds a refresh token + client
//! identity, lazily mints an access token on the first call and
//! caches it for the lifetime of the process.
//!
//! Spotify is a **PKCE-only public client** for our purposes — the
//! token endpoint must not receive a `client_secret`. We pass `None`
//! into [`crate::oauth::refresh_access_token`].
//!
//! ## Error mapping
//!
//! Every non-2xx HTTP status surfaces as `ZadError::Service { name:
//! "spotify", message }`. Two cases are specialised:
//!
//! - `401` with `invalid_token` / `The access token expired` →
//!   "credentials revoked; re-run `zad service create spotify`"
//! - `429` → "Spotify rate-limited this client; back off before
//!   retrying" (the response carries a `Retry-After` header which we
//!   surface verbatim in the message)

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Result, ZadError};
use crate::oauth::{self, RefreshTokenStore};
use crate::service::spotify::{API_BASE, TOKEN_URL};

/// Thin wrapper over Spotify Web API v1. Holds a refresh token and
/// mints an access token on demand.
///
/// Spotify rotates the refresh token on every successful refresh; the
/// previous token is honoured for a short grace window, then revoked.
/// When [`Self::access_token`] sees a rotated value it persists the
/// new token via the optional [`RefreshTokenStore`] before updating
/// its in-memory copy, so the next process picks up the latest token
/// instead of replaying the stale one.
#[derive(Clone)]
pub struct SpotifyHttp {
    client_id: String,
    /// Wrapped in a Mutex so a token rotation in one process can be
    /// reflected back into the in-memory state without taking `&mut
    /// self` through the whole chain.
    refresh_token: Arc<Mutex<String>>,
    /// Where to persist a rotated refresh token. `None` means "drop
    /// the rotation on the floor" — fine for short-lived tests, fatal
    /// for long-lived deployments against rotating providers.
    refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
    /// Token endpoint URL. Defaults to
    /// [`crate::service::spotify::TOKEN_URL`]; overridable so tests
    /// can point at a localhost mock.
    token_url: String,
    scopes: BTreeSet<String>,
    config_path: PathBuf,
    /// Cached access token for the lifetime of this process. Held
    /// across the network refresh so two concurrent callers can't
    /// race two distinct rotated tokens onto the keychain.
    cached_access: Arc<Mutex<Option<String>>>,
}

impl SpotifyHttp {
    /// Full-featured constructor used by runtime verbs. No persisting
    /// store — rotated tokens are dropped on the floor; suitable only
    /// for callers that don't care or that drive rotation themselves.
    /// Use [`Self::with_store`] in production.
    pub fn new(
        client_id: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        Self::with_store(client_id, refresh_token, scopes, config_path, None)
    }

    /// Like [`Self::new`] but takes an optional [`RefreshTokenStore`]
    /// that receives every rotated refresh token. The canonical zad
    /// wiring (`Spotify::from_default_config`) supplies a
    /// [`crate::oauth::KeychainRefreshStore`] pointing at the
    /// `secrets::account("spotify", "refresh", Scope::Global)` slot.
    pub fn with_store(
        client_id: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
    ) -> Self {
        Self {
            client_id,
            refresh_token: Arc::new(Mutex::new(refresh_token)),
            refresh_token_store,
            token_url: TOKEN_URL.to_string(),
            scopes,
            config_path,
            cached_access: Arc::new(Mutex::new(None)),
        }
    }

    /// Scopeless client used by lifecycle flows (`validate`, `status
    /// check`).
    pub fn unscoped(client_id: String, refresh_token: String) -> Self {
        Self::new(client_id, refresh_token, BTreeSet::new(), PathBuf::new())
    }

    /// Override the token endpoint URL. Test-only — production code
    /// should rely on the default
    /// [`crate::service::spotify::TOKEN_URL`].
    #[doc(hidden)]
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    /// Read the in-memory refresh token. Test-only.
    #[doc(hidden)]
    pub async fn refresh_token_for_test(&self) -> String {
        self.refresh_token.lock().await.clone()
    }

    fn require_scope(&self, scope: &'static str) -> Result<()> {
        if self.scopes.contains(scope) {
            return Ok(());
        }
        Err(ZadError::ScopeDenied {
            service: "spotify",
            scope,
            config_path: self.config_path.clone(),
        })
    }

    /// Lazily fetch (and cache) an access token for the lifetime of
    /// this process. Spotify PKCE public clients send `client_id`
    /// only — no `client_secret`.
    ///
    /// The `cached_access` lock is held across the network refresh
    /// **and** across the rotated-token persist step, so two
    /// concurrent callers can't both refresh and race two distinct
    /// rotated tokens onto the keychain. The serialization is a
    /// once-per-process event (the access token then stays cached
    /// forever) so the contention cost is negligible.
    ///
    /// Exposed publicly with `#[doc(hidden)]` so tests can drive a
    /// refresh without piggy-backing on a follow-up API call (which
    /// would try to hit the real Spotify API).
    #[doc(hidden)]
    pub async fn access_token(&self) -> Result<String> {
        let mut cached = self.cached_access.lock().await;
        if let Some(t) = cached.as_ref() {
            return Ok(t.clone());
        }
        let current = self.refresh_token.lock().await.clone();
        let fresh = oauth::refresh_access_token(
            "spotify",
            &self.token_url,
            &self.client_id,
            None,
            &current,
        )
        .await?;

        // Spotify's PKCE flow rotates the refresh token on every
        // /api/token call. Persist the new value before updating the
        // in-memory copy so a crash mid-update never leaves the
        // keychain behind the in-memory state. A keychain write
        // failure surfaces — silently swallowing it would recreate
        // the original "Refresh token revoked" bug.
        if let Some(new_rt) = fresh.refresh_token.as_deref() {
            let mut rt = self.refresh_token.lock().await;
            if new_rt != rt.as_str() {
                if let Some(store) = &self.refresh_token_store {
                    store.store(new_rt)?;
                }
                *rt = new_rt.to_string();
            }
        }

        *cached = Some(fresh.access_token.clone());
        Ok(fresh.access_token)
    }

    // -----------------------------------------------------------------
    // public endpoints — scoped
    // -----------------------------------------------------------------

    /// `GET /search`. Scope: `search`. `types` is one or more of
    /// `track`, `album`, `artist`, `playlist`.
    ///
    /// Spotify quietly tightened `/search`'s `limit` cap to 10 (down
    /// from the 50 still listed in older docs); anything higher comes
    /// back as `HTTP 400 "Invalid limit"`. Other paginated endpoints
    /// (`/me/playlists`, `/me/tracks`, …) still accept 50.
    pub async fn search(&self, query: &str, types: &[&str], limit: u32) -> Result<SearchResults> {
        self.require_scope("search")?;
        let limit = limit.clamp(1, 10).to_string();
        let types_joined = types.join(",");
        self.get_json(
            "/search",
            &[
                ("q", query),
                ("type", types_joined.as_str()),
                ("limit", limit.as_str()),
            ],
        )
        .await
    }

    /// `GET /me/playlists`. Scope: `playlists.read`.
    pub async fn list_my_playlists(&self, limit: u32) -> Result<Vec<PlaylistSummary>> {
        self.require_scope("playlists.read")?;
        let limit = limit.clamp(1, 50).to_string();
        let page: PlaylistPage = self
            .get_json("/me/playlists", &[("limit", limit.as_str())])
            .await?;
        Ok(page.items)
    }

    /// `GET /playlists/{id}/tracks`. Scope: `playlists.read`.
    pub async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        limit: u32,
    ) -> Result<Vec<PlaylistTrackItem>> {
        self.require_scope("playlists.read")?;
        let limit = limit.clamp(1, 100).to_string();
        let path = format!("/playlists/{}/tracks", urlencode_path(playlist_id));
        let page: PlaylistTrackPage = self.get_json(&path, &[("limit", limit.as_str())]).await?;
        Ok(page.items)
    }

    /// `GET /playlists/{id}`. Scope: `playlists.read`.
    pub async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        self.require_scope("playlists.read")?;
        let path = format!("/playlists/{}", urlencode_path(playlist_id));
        self.get_json(&path, &[]).await
    }

    /// `POST /users/{user_id}/playlists`. Scope: `playlists.write`.
    pub async fn create_playlist(
        &self,
        user_id: &str,
        name: &str,
        description: Option<&str>,
        public: bool,
    ) -> Result<PlaylistSummary> {
        self.require_scope("playlists.write")?;
        let path = format!("/users/{}/playlists", urlencode_path(user_id));
        let mut body = serde_json::json!({ "name": name, "public": public });
        if let Some(d) = description {
            body["description"] = serde_json::Value::String(d.to_string());
        }
        self.post_json(&path, &[], &body).await
    }

    /// `PUT /playlists/{id}` with `{ "name": <new> }`. Scope:
    /// `playlists.write`.
    pub async fn rename_playlist(&self, playlist_id: &str, new_name: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        let path = format!("/playlists/{}", urlencode_path(playlist_id));
        let body = serde_json::json!({ "name": new_name });
        self.put_empty(&path, &[], &body).await
    }

    /// `DELETE /playlists/{id}/followers` — Spotify's "delete
    /// playlist" is really an unfollow. Scope: `playlists.write`.
    pub async fn unfollow_playlist(&self, playlist_id: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        let path = format!("/playlists/{}/followers", urlencode_path(playlist_id));
        self.delete_empty(&path, &[]).await
    }

    /// `POST /playlists/{id}/tracks` with a `uris` array. Scope:
    /// `playlists.write`.
    pub async fn add_playlist_tracks(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.require_scope("playlists.write")?;
        let path = format!("/playlists/{}/tracks", urlencode_path(playlist_id));
        let body = serde_json::json!({ "uris": uris });
        let _: serde_json::Value = self.post_json(&path, &[], &body).await?;
        Ok(())
    }

    /// `DELETE /playlists/{id}/tracks` with `{ tracks: [{ uri }] }`.
    /// Scope: `playlists.write`.
    pub async fn remove_playlist_tracks(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.require_scope("playlists.write")?;
        let path = format!("/playlists/{}/tracks", urlencode_path(playlist_id));
        let tracks: Vec<serde_json::Value> = uris
            .iter()
            .map(|u| serde_json::json!({ "uri": u }))
            .collect();
        let body = serde_json::json!({ "tracks": tracks });
        self.delete_with_body(&path, &[], &body).await
    }

    /// `GET /me/tracks`. Scope: `library.read`.
    pub async fn list_saved_tracks(&self, limit: u32) -> Result<Vec<SavedTrack>> {
        self.require_scope("library.read")?;
        let limit = limit.clamp(1, 50).to_string();
        let page: SavedTrackPage = self
            .get_json("/me/tracks", &[("limit", limit.as_str())])
            .await?;
        Ok(page.items)
    }

    /// `PUT /me/tracks?ids=…`. Scope: `library.write`.
    pub async fn save_tracks(&self, ids: &[String]) -> Result<()> {
        self.require_scope("library.write")?;
        let joined = ids.join(",");
        self.put_empty(
            "/me/tracks",
            &[("ids", joined.as_str())],
            &serde_json::json!({}),
        )
        .await
    }

    /// `DELETE /me/tracks?ids=…`. Scope: `library.write`.
    pub async fn unsave_tracks(&self, ids: &[String]) -> Result<()> {
        self.require_scope("library.write")?;
        let joined = ids.join(",");
        self.delete_empty("/me/tracks", &[("ids", joined.as_str())])
            .await
    }

    /// `GET /me/albums`. Scope: `library.read`.
    pub async fn list_saved_albums(&self, limit: u32) -> Result<Vec<SavedAlbum>> {
        self.require_scope("library.read")?;
        let limit = limit.clamp(1, 50).to_string();
        let page: SavedAlbumPage = self
            .get_json("/me/albums", &[("limit", limit.as_str())])
            .await?;
        Ok(page.items)
    }

    /// `PUT /me/albums?ids=…`. Scope: `library.write`.
    pub async fn save_albums(&self, ids: &[String]) -> Result<()> {
        self.require_scope("library.write")?;
        let joined = ids.join(",");
        self.put_empty(
            "/me/albums",
            &[("ids", joined.as_str())],
            &serde_json::json!({}),
        )
        .await
    }

    /// `DELETE /me/albums?ids=…`. Scope: `library.write`.
    pub async fn unsave_albums(&self, ids: &[String]) -> Result<()> {
        self.require_scope("library.write")?;
        let joined = ids.join(",");
        self.delete_empty("/me/albums", &[("ids", joined.as_str())])
            .await
    }

    // -----------------------------------------------------------------
    // unscoped — called from lifecycle (pre-scopes)
    // -----------------------------------------------------------------

    /// `GET /me` — fetches the authenticated user's profile. Used by
    /// `validate` during `zad service create spotify` and by
    /// `service status`.
    pub async fn me(&self) -> Result<UserProfile> {
        self.get_json("/me", &[]).await
    }

    // -----------------------------------------------------------------
    // low-level HTTP glue
    // -----------------------------------------------------------------

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let access = self.access_token().await?;
        let resp = reqwest::Client::new()
            .get(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp).await
    }

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<T> {
        let access = self.access_token().await?;
        let resp = reqwest::Client::new()
            .post(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp).await
    }

    async fn put_empty(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<()> {
        let access = self.access_token().await?;
        let resp = reqwest::Client::new()
            .put(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(map_http_error(status, &body))
    }

    async fn delete_empty(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        let access = self.access_token().await?;
        let resp = reqwest::Client::new()
            .delete(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(map_http_error(status, &body))
    }

    /// Spotify's "remove tracks from playlist" is a `DELETE` with a
    /// JSON body — `reqwest`'s `.delete()` accepts `.json(body)` so
    /// we route through a separate helper to avoid foot-guns.
    async fn delete_with_body(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<()> {
        let access = self.access_token().await?;
        let resp = reqwest::Client::new()
            .delete(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        Err(map_http_error(status, &body))
    }
}

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "spotify",
        message: format!("network error talking to Spotify: {e}"),
    }
}

async fn decode_response<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(map_http_error(status, &body));
    }
    resp.json::<T>().await.map_err(|e| ZadError::Service {
        name: "spotify",
        message: format!("failed to decode Spotify response: {e}"),
    })
}

fn map_http_error(status: reqwest::StatusCode, body: &str) -> ZadError {
    let code = status.as_u16();
    let lower = body.to_ascii_lowercase();
    let message = if code == 401
        || lower.contains("invalid_token")
        || lower.contains("the access token expired")
    {
        format!(
            "Spotify rejected the access token (HTTP {code}); the credentials may have been \
             revoked. Re-run `zad service create spotify` to re-authorize. Body: {body}"
        )
    } else if code == 429 {
        format!(
            "Spotify rate-limited this client (HTTP {code}); back off before retrying. \
             Body: {body}"
        )
    } else {
        format!("HTTP {code}: {body}")
    };
    ZadError::Service {
        name: "spotify",
        message,
    }
}

// ---------------------------------------------------------------------------
// Percent-encode path components.
// ---------------------------------------------------------------------------

fn urlencode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Response types — minimal projections of the Spotify API objects.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct UserProfile {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub public: Option<bool>,
    #[serde(default)]
    pub collaborative: Option<bool>,
    #[serde(default)]
    pub owner: Option<UserRef>,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub tracks: Option<TracksRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserRef {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracksRef {
    #[serde(default)]
    pub total: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistPage {
    #[serde(default)]
    pub items: Vec<PlaylistSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistTrackItem {
    #[serde(default)]
    pub track: Option<TrackSummary>,
    #[serde(default)]
    pub added_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistTrackPage {
    #[serde(default)]
    pub items: Vec<PlaylistTrackItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    #[serde(default)]
    pub album: Option<AlbumRef>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub explicit: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlbumRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlbumSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub artists: Vec<ArtistRef>,
    #[serde(default)]
    pub total_tracks: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
}

/// Top-level `/search` response. Each requested type maps to a paged
/// section; sections for types we didn't ask for arrive as `None`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SearchResults {
    #[serde(default)]
    pub tracks: Option<TrackSearchPage>,
    #[serde(default)]
    pub albums: Option<AlbumSearchPage>,
    #[serde(default)]
    pub artists: Option<ArtistSearchPage>,
    #[serde(default)]
    pub playlists: Option<PlaylistSearchPage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackSearchPage {
    #[serde(default)]
    pub items: Vec<TrackSummary>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct AlbumSearchPage {
    #[serde(default)]
    pub items: Vec<AlbumSummary>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct ArtistSearchPage {
    #[serde(default)]
    pub items: Vec<ArtistSummary>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistSearchPage {
    #[serde(default)]
    pub items: Vec<PlaylistSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SavedTrack {
    #[serde(default)]
    pub added_at: Option<String>,
    pub track: TrackSummary,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedTrackPage {
    #[serde(default)]
    pub items: Vec<SavedTrack>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SavedAlbum {
    #[serde(default)]
    pub added_at: Option<String>,
    pub album: AlbumSummary,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedAlbumPage {
    #[serde(default)]
    pub items: Vec<SavedAlbum>,
}
