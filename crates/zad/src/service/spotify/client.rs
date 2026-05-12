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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Result, ZadError};
use crate::oauth::{self, RefreshTokenStore};
use crate::rate_limit;
use crate::service::spotify::{API_BASE, TOKEN_URL};
use crate::token_cache;

const SERVICE: &str = "spotify";

/// Maximum number of URIs Spotify accepts in a single `PUT`/`DELETE
/// /me/library` call. Anything longer is chunked transparently.
const LIBRARY_BATCH: usize = 40;

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
    /// Directory used for the cross-process token cache and refresh
    /// lock. `None` means "resolve from `zad_home()` at runtime".
    /// Override via [`Self::with_cache_dir`] in tests.
    cache_service_dir: Option<PathBuf>,
    /// Base URL for the Web API. Defaults to
    /// [`crate::service::spotify::API_BASE`]; overridable so tests
    /// can point at a localhost mock.
    api_base: String,
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
            cache_service_dir: None,
            api_base: API_BASE.to_string(),
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

    /// Override the cross-process token cache directory. Test-only —
    /// production code resolves the directory from `zad_home()` at
    /// runtime. Pass a `tempfile::TempDir`-backed path to isolate tests
    /// from each other and from the real `~/.zad` state.
    #[doc(hidden)]
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_service_dir = Some(dir);
        self
    }

    /// Override the Web API base URL. Test-only — production code
    /// should rely on the default
    /// [`crate::service::spotify::API_BASE`].
    #[doc(hidden)]
    pub fn with_api_base(mut self, url: impl Into<String>) -> Self {
        self.api_base = url.into();
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

        // Resolve the cache directory once. `None` means caching is
        // unavailable; all token_cache calls are then no-ops and the
        // function falls back to the original per-process behaviour.
        let cache_dir: Option<PathBuf> = self
            .cache_service_dir
            .clone()
            .or_else(|| token_cache::service_dir(SERVICE).ok());

        // Fast path: file cache hit — skip the keychain and the token
        // endpoint. This is the common path for processes 2-N in a
        // parallel fan-out (they all start cold but only one refreshes).
        if let Some(t) = token_cache::read(cache_dir.as_deref()) {
            *cached = Some(t.clone());
            return Ok(t);
        }

        // Acquire the cross-process lock before calling the token
        // endpoint. Processes that lose the race will find a valid
        // cache entry on the re-check below and skip the network call.
        let _lock = token_cache::acquire_lock(cache_dir.as_deref()).await?;

        // Re-check after acquiring the lock: the process that held it
        // just before us has written the cache by now.
        if let Some(t) = token_cache::read(cache_dir.as_deref()) {
            *cached = Some(t.clone());
            return Ok(t);
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

        // Write to the cross-process cache so sibling processes skip
        // the token endpoint entirely. _lock is released on drop at
        // the end of this function.
        let expires_in = fresh
            .expires_in
            .unwrap_or(token_cache::DEFAULT_EXPIRES_IN_SECS);
        if let Err(e) = token_cache::write(cache_dir.as_deref(), &fresh.access_token, expires_in) {
            tracing::warn!(
                service = SERVICE,
                error = %e,
                "failed to write access-token cache; cross-process sharing disabled for this session"
            );
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
    ///
    /// Walks the `offset` cursor under the hood until either `max` is
    /// reached or the API runs out. `max == None` means "fetch
    /// everything"; `Some(n)` caps the returned `Vec` at `n` items.
    /// Single-page requests stop after one HTTP call.
    pub async fn list_my_playlists(&self, max: Option<u32>) -> Result<Vec<PlaylistSummary>> {
        self.require_scope("playlists.read")?;
        self.paged_get("/me/playlists", &[], 50, max, |page: PlaylistPage| {
            page.items
        })
        .await
    }

    /// `GET /playlists/{id}/items`. Scope: `playlists.read`.
    ///
    /// Paginates with `offset`; per-page cap is 50. `max == None`
    /// means "fetch every item in the playlist".
    pub async fn get_playlist_tracks(
        &self,
        playlist_id: &str,
        max: Option<u32>,
    ) -> Result<Vec<PlaylistTrackItem>> {
        self.require_scope("playlists.read")?;
        let path = format!("/playlists/{}/items", urlencode_path(playlist_id));
        self.paged_get(&path, &[], 50, max, |page: PlaylistTrackPage| page.items)
            .await
    }

    /// `GET /playlists/{id}`. Scope: `playlists.read`.
    pub async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        self.require_scope("playlists.read")?;
        let path = format!("/playlists/{}", urlencode_path(playlist_id));
        self.get_json(&path, &[]).await
    }

    /// `POST /me/playlists`. Scope: `playlists.write`. Targets the
    /// authenticated user; no `user_id` argument.
    pub async fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
        public: bool,
    ) -> Result<PlaylistSummary> {
        self.require_scope("playlists.write")?;
        let mut body = serde_json::json!({ "name": name, "public": public });
        if let Some(d) = description {
            body["description"] = serde_json::Value::String(d.to_string());
        }
        self.post_json("/me/playlists", &[], &body).await
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

    /// `POST /playlists/{id}/items` with a `uris` array. Scope:
    /// `playlists.write`.
    pub async fn add_playlist_tracks(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.require_scope("playlists.write")?;
        let path = format!("/playlists/{}/items", urlencode_path(playlist_id));
        let body = serde_json::json!({ "uris": uris });
        let _: serde_json::Value = self.post_json(&path, &[], &body).await?;
        Ok(())
    }

    /// `DELETE /playlists/{id}/items` with `{ items: [{ uri }] }`.
    /// Scope: `playlists.write`.
    pub async fn remove_playlist_tracks(&self, playlist_id: &str, uris: &[String]) -> Result<()> {
        self.require_scope("playlists.write")?;
        let path = format!("/playlists/{}/items", urlencode_path(playlist_id));
        let items: Vec<serde_json::Value> = uris
            .iter()
            .map(|u| serde_json::json!({ "uri": u }))
            .collect();
        let body = serde_json::json!({ "items": items });
        self.delete_with_body(&path, &[], &body).await
    }

    /// `GET /me/tracks`. Scope: `library.read`.
    ///
    /// Paginates with `offset`; per-page cap is 50. `max == None`
    /// means "fetch every liked track".
    pub async fn list_saved_tracks(&self, max: Option<u32>) -> Result<Vec<SavedTrack>> {
        self.require_scope("library.read")?;
        self.paged_get("/me/tracks", &[], 50, max, |page: SavedTrackPage| {
            page.items
        })
        .await
    }

    /// Save tracks via the unified `PUT /me/library`. Scope:
    /// `library.write`.
    pub async fn save_tracks(&self, uris: &[String]) -> Result<()> {
        self.save_to_library(uris).await
    }

    /// Unsave tracks via the unified `DELETE /me/library`. Scope:
    /// `library.write`.
    pub async fn unsave_tracks(&self, uris: &[String]) -> Result<()> {
        self.remove_from_library(uris).await
    }

    /// `GET /me/albums`. Scope: `library.read`.
    ///
    /// Paginates with `offset`; per-page cap is 50. `max == None`
    /// means "fetch every saved album".
    pub async fn list_saved_albums(&self, max: Option<u32>) -> Result<Vec<SavedAlbum>> {
        self.require_scope("library.read")?;
        self.paged_get("/me/albums", &[], 50, max, |page: SavedAlbumPage| {
            page.items
        })
        .await
    }

    /// Save albums via the unified `PUT /me/library`. Scope:
    /// `library.write`.
    pub async fn save_albums(&self, uris: &[String]) -> Result<()> {
        self.save_to_library(uris).await
    }

    /// Unsave albums via the unified `DELETE /me/library`. Scope:
    /// `library.write`.
    pub async fn unsave_albums(&self, uris: &[String]) -> Result<()> {
        self.remove_from_library(uris).await
    }

    /// `PUT /me/library?uris=<csv>`. Scope: `library.write`. Accepts
    /// any mix of `spotify:track:`, `spotify:album:`, `spotify:show:`,
    /// … URIs.
    ///
    /// Spotify caps the unified library endpoint at 40 URIs per call
    /// and expects them as a comma-separated query parameter (no
    /// request body). Slices longer than 40 are chunked transparently;
    /// the first chunk to fail aborts the rest.
    pub async fn save_to_library(&self, uris: &[String]) -> Result<()> {
        self.require_scope("library.write")?;
        for chunk in uris.chunks(LIBRARY_BATCH) {
            let joined = chunk.join(",");
            self.put_query_only("/me/library", &[("uris", joined.as_str())])
                .await?;
        }
        Ok(())
    }

    /// `DELETE /me/library?uris=<csv>`. Scope: `library.write`.
    /// Accepts any mix of `spotify:track:`, `spotify:album:`,
    /// `spotify:show:`, … URIs. Same 40-URI cap and chunking as
    /// [`Self::save_to_library`].
    pub async fn remove_from_library(&self, uris: &[String]) -> Result<()> {
        self.require_scope("library.write")?;
        for chunk in uris.chunks(LIBRARY_BATCH) {
            let joined = chunk.join(",");
            self.delete_empty("/me/library", &[("uris", joined.as_str())])
                .await?;
        }
        Ok(())
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
    // pagination helper
    // -----------------------------------------------------------------

    /// Walk Spotify's `offset` cursor over a list endpoint until either
    /// `max` is reached or the API returns a short page. `page_size` is
    /// the per-call cap (50 for most endpoints, 100 for
    /// `/playlists/{id}/items`). `extract` projects each decoded page
    /// into the items slice. The accumulator stops the moment a page
    /// comes back shorter than `page_size` — Spotify never emits empty
    /// trailing pages once the cursor is exhausted.
    async fn paged_get<P, T, F>(
        &self,
        path: &str,
        base_query: &[(&str, &str)],
        page_size: u32,
        max: Option<u32>,
        extract: F,
    ) -> Result<Vec<T>>
    where
        P: for<'de> Deserialize<'de>,
        F: Fn(P) -> Vec<T>,
    {
        // Clamp `max` against `page_size` so a single small-`max` call
        // still uses one HTTP round-trip. `page_size` is itself bounded
        // by Spotify's per-endpoint cap of 50.
        let page_size = page_size.clamp(1, 50);
        let mut out: Vec<T> = Vec::new();
        let mut offset: u32 = 0;
        loop {
            let remaining = max.map(|m| m.saturating_sub(out.len() as u32));
            if remaining == Some(0) {
                break;
            }
            let this_limit = remaining.map(|r| r.min(page_size)).unwrap_or(page_size);
            let limit_str = this_limit.to_string();
            let offset_str = offset.to_string();
            let mut query: Vec<(&str, &str)> = base_query.to_vec();
            query.push(("limit", limit_str.as_str()));
            query.push(("offset", offset_str.as_str()));
            let page: P = self.get_json(path, &query).await?;
            let mut items = extract(page);
            let received = items.len() as u32;
            out.append(&mut items);
            if received < this_limit {
                break;
            }
            offset = offset.saturating_add(received);
        }
        if let Some(m) = max {
            out.truncate(m as usize);
        }
        Ok(out)
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
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .get(format!("{}{path}", self.api_base))
            .bearer_auth(&access)
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp, cache_dir.as_deref()).await
    }

    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<T> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .post(format!("{}{path}", self.api_base))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp, cache_dir.as_deref()).await
    }

    async fn put_empty(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<()> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .put(format!("{}{path}", self.api_base))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        finalize_empty(resp, cache_dir.as_deref()).await
    }

    /// PUT without a request body — only a query string. Spotify's
    /// unified library save/unsave endpoints take their payload as a
    /// `?uris=<csv>` query parameter and reject (or ignore) a JSON
    /// body.
    async fn put_query_only(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .put(format!("{}{path}", self.api_base))
            .bearer_auth(&access)
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        finalize_empty(resp, cache_dir.as_deref()).await
    }

    async fn delete_empty(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .delete(format!("{}{path}", self.api_base))
            .bearer_auth(&access)
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        finalize_empty(resp, cache_dir.as_deref()).await
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
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .delete(format!("{}{path}", self.api_base))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        finalize_empty(resp, cache_dir.as_deref()).await
    }

    /// Resolve the cache directory, preferring the override field and
    /// falling back to the standard `zad_home`-based path.
    fn resolved_cache_dir(&self) -> Option<PathBuf> {
        self.cache_service_dir
            .clone()
            .or_else(|| token_cache::service_dir(SERVICE).ok())
    }
}

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "spotify",
        message: format!("network error talking to Spotify: {e}"),
    }
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
    cache_dir: Option<&Path>,
) -> Result<T> {
    if let Some(err) = rate_limit::check_response(SERVICE, &resp) {
        return Err(err);
    }
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 {
            token_cache::clear(cache_dir);
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(map_http_error(status, &body));
    }
    rate_limit::clear(SERVICE);
    resp.json::<T>().await.map_err(|e| ZadError::Service {
        name: "spotify",
        message: format!("failed to decode Spotify response: {e}"),
    })
}

async fn finalize_empty(resp: reqwest::Response, cache_dir: Option<&Path>) -> Result<()> {
    if let Some(err) = rate_limit::check_response(SERVICE, &resp) {
        return Err(err);
    }
    let status = resp.status();
    if status.is_success() {
        rate_limit::clear(SERVICE);
        return Ok(());
    }
    if status.as_u16() == 401 {
        token_cache::clear(cache_dir);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(map_http_error(status, &body))
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
    /// Paging reference for the playlist's contents. Non-owned
    /// playlists in Development Mode apps see metadata only and the
    /// field is absent. The `tracks` alias keeps deserialization
    /// working against extended-quota tenants that still ship the
    /// older shape.
    #[serde(default, alias = "tracks")]
    pub items: Option<ItemsRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserRef {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemsRef {
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
    /// The track or episode object. The `track` alias keeps
    /// deserialization working against extended-quota tenants that
    /// still ship the older shape.
    #[serde(default, alias = "track")]
    pub item: Option<TrackSummary>,
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
