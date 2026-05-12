//! YouTube Music HTTP client.
//!
//! Hand-rolled `reqwest` wrapper over YouTube Data API v3. YouTube
//! Music does not expose its own API surface; the Web/Data API
//! covers playlists, library (rated videos), and search the same way
//! Spotify Web API v1 covers Spotify. Mirrors `gcal/client.rs` and
//! `spotify/client.rs` for consistency: holds a refresh token plus
//! the OAuth client identity, lazily mints an access token on the
//! first call, and caches it for the lifetime of the process.
//!
//! ## Error mapping
//!
//! Every non-2xx HTTP status surfaces as `ZadError::Service { name:
//! "ymusic", message }`. Two cases are specialised:
//!
//! - `401` with `invalid_credentials` / `invalid_token` →
//!   "credentials revoked; re-run `zad service create ymusic`"
//! - `403`/`429` with Google's `quotaExceeded` /
//!   `rateLimitExceeded` body → "YouTube Data API quota exhausted;
//!   back off before retrying"
//!
//! ## YouTube quirks vs. Spotify
//!
//! - A playlist item is a separate resource from the video it points
//!   at. To remove an item from a playlist you need the
//!   `playlistItem.id`, not the `videoId` — the playlist read endpoint
//!   surfaces both, and the runtime CLI accepts either form (matching
//!   on the playlist side first).
//! - "Library" is YouTube's `videos?myRating=like` endpoint; albums
//!   have no analogue, so saved-album operations are not exposed.
//! - Mutating playlist endpoints take `part=snippet` and a JSON body
//!   that always carries the resource's `kind`. The helpers below
//!   wrap that boilerplate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Result, ZadError};
use crate::oauth::{self, RefreshTokenStore};
use crate::rate_limit;
use crate::service::ymusic::{API_BASE, TOKEN_URL, USERINFO_URL};
use crate::token_cache;

const SERVICE: &str = "ymusic";

/// Thin wrapper over YouTube Data API v3. Holds a refresh token and
/// mints an access token on demand.
///
/// Google's confidential-client flow does not currently rotate
/// refresh tokens, but the persist-on-rotation handling is wired up
/// the same way as Spotify so a future provider change (or a
/// per-account quirk) doesn't silently revoke the user's session.
/// When the value is unchanged, the store is never called.
#[derive(Clone)]
pub struct YmusicHttp {
    client_id: String,
    client_secret: String,
    /// Wrapped in a Mutex so a token rotation in one process can be
    /// reflected back into the in-memory state without taking `&mut
    /// self` through the whole chain.
    refresh_token: Arc<Mutex<String>>,
    /// Where to persist a rotated refresh token. `None` means "drop
    /// the rotation on the floor" — Google rarely rotates today, but
    /// the field exists so the bug Spotify hit can't recur silently.
    refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
    /// Token endpoint URL. Defaults to
    /// [`crate::service::ymusic::TOKEN_URL`]; overridable so tests
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
    /// Base URL for the YouTube Data API. Defaults to
    /// [`crate::service::ymusic::API_BASE`]; overridable so tests can
    /// point at a localhost mock.
    api_base: String,
}

impl YmusicHttp {
    /// Full-featured constructor used by runtime verbs. No persisting
    /// store — equivalent to [`Self::with_store`] with `None`. Use
    /// [`Self::with_store`] in production.
    pub fn new(
        client_id: String,
        client_secret: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        Self::with_store(
            client_id,
            client_secret,
            refresh_token,
            scopes,
            config_path,
            None,
        )
    }

    /// Like [`Self::new`] but takes an optional [`RefreshTokenStore`]
    /// that receives every rotated refresh token. The canonical zad
    /// wiring (`Ymusic::from_default_config`) supplies a
    /// [`crate::oauth::KeychainRefreshStore`] pointing at the
    /// `secrets::account("ymusic", "refresh", Scope::Global)` slot.
    pub fn with_store(
        client_id: String,
        client_secret: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
    ) -> Self {
        Self {
            client_id,
            client_secret,
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
    pub fn unscoped(client_id: String, client_secret: String, refresh_token: String) -> Self {
        Self::new(
            client_id,
            client_secret,
            refresh_token,
            BTreeSet::new(),
            PathBuf::new(),
        )
    }

    /// Override the token endpoint URL. Test-only — production code
    /// should rely on the default
    /// [`crate::service::ymusic::TOKEN_URL`].
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

    /// Override the YouTube Data API base URL. Test-only — production
    /// code should rely on the default
    /// [`crate::service::ymusic::API_BASE`].
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
            service: "ymusic",
            scope,
            config_path: self.config_path.clone(),
        })
    }

    /// Lazily fetch (and cache) an access token for the lifetime of
    /// this process.
    ///
    /// The `cached_access` lock is held across the network refresh
    /// **and** across the rotated-token persist step, so two
    /// concurrent callers can't both refresh and race two distinct
    /// rotated tokens onto the keychain. See `SpotifyHttp::access_token`
    /// for the same pattern — the bug that motivated this lives in
    /// Spotify's flow but the safety net is identical here.
    ///
    /// Exposed publicly with `#[doc(hidden)]` so tests can drive a
    /// refresh without piggy-backing on a follow-up API call.
    #[doc(hidden)]
    pub async fn access_token(&self) -> Result<String> {
        let mut cached = self.cached_access.lock().await;
        if let Some(t) = cached.as_ref() {
            return Ok(t.clone());
        }

        let cache_dir: Option<PathBuf> = self
            .cache_service_dir
            .clone()
            .or_else(|| token_cache::service_dir(SERVICE).ok());

        if let Some(t) = token_cache::read(cache_dir.as_deref()) {
            *cached = Some(t.clone());
            return Ok(t);
        }

        let _lock = token_cache::acquire_lock(cache_dir.as_deref()).await?;

        if let Some(t) = token_cache::read(cache_dir.as_deref()) {
            *cached = Some(t.clone());
            return Ok(t);
        }

        let current = self.refresh_token.lock().await.clone();
        let fresh = oauth::refresh_access_token(
            "ymusic",
            &self.token_url,
            &self.client_id,
            Some(&self.client_secret),
            &current,
        )
        .await?;

        // Persist rotation if the provider returned a different
        // refresh token. Google rarely rotates today, so this branch
        // is usually inert — but if it ever does (account flagged,
        // policy change), the rotated value lands in the keychain
        // instead of being silently dropped.
        if let Some(new_rt) = fresh.refresh_token.as_deref() {
            let mut rt = self.refresh_token.lock().await;
            if new_rt != rt.as_str() {
                if let Some(store) = &self.refresh_token_store {
                    store.store(new_rt)?;
                }
                *rt = new_rt.to_string();
            }
        }

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

    /// `GET /search?q=…&type=…`. Scope: `search`. `types` is one or
    /// more of `video`, `playlist`, `channel`. YouTube has no
    /// "artist" or "album" entity in the Data API; the closest thing
    /// to an artist is a channel.
    pub async fn search(&self, query: &str, types: &[&str], limit: u32) -> Result<Vec<SearchItem>> {
        self.require_scope("search")?;
        let limit = limit.clamp(1, 50).to_string();
        let types_joined = types.join(",");
        let page: SearchPage = self
            .get_json(
                "/search",
                &[
                    ("q", query),
                    ("type", types_joined.as_str()),
                    ("part", "snippet"),
                    ("maxResults", limit.as_str()),
                ],
            )
            .await?;
        Ok(page.items)
    }

    /// `GET /playlists?mine=true`. Scope: `playlists.read`.
    ///
    /// Walks `nextPageToken` under the hood; per-page cap is 50.
    /// `max == None` means
    /// "fetch every playlist".
    pub async fn list_my_playlists(&self, max: Option<u32>) -> Result<Vec<PlaylistSummary>> {
        self.require_scope("playlists.read")?;
        self.paged_get(
            "/playlists",
            &[("mine", "true"), ("part", "snippet,contentDetails,status")],
            max,
            |page: PlaylistPage| (page.items, page.next_page_token),
        )
        .await
    }

    /// `GET /playlists?id=<id>`. Scope: `playlists.read`.
    pub async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        self.require_scope("playlists.read")?;
        let page: PlaylistPage = self
            .get_json(
                "/playlists",
                &[
                    ("id", playlist_id),
                    ("part", "snippet,contentDetails,status"),
                ],
            )
            .await?;
        page.items.into_iter().next().ok_or(ZadError::Service {
            name: "ymusic",
            message: format!("playlist `{playlist_id}` not found"),
        })
    }

    /// `GET /playlistItems?playlistId=<id>`. Scope: `playlists.read`.
    ///
    /// Walks `nextPageToken` under the hood; per-page cap is 50.
    /// `max == None` means
    /// "fetch every item in the playlist".
    pub async fn get_playlist_items(
        &self,
        playlist_id: &str,
        max: Option<u32>,
    ) -> Result<Vec<PlaylistItem>> {
        self.require_scope("playlists.read")?;
        self.paged_get(
            "/playlistItems",
            &[
                ("playlistId", playlist_id),
                ("part", "snippet,contentDetails"),
            ],
            max,
            |page: PlaylistItemPage| (page.items, page.next_page_token),
        )
        .await
    }

    /// `POST /playlists`. Scope: `playlists.write`.
    pub async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary> {
        self.require_scope("playlists.write")?;
        let mut snippet = serde_json::json!({ "title": title });
        if let Some(d) = description {
            snippet["description"] = serde_json::Value::String(d.to_string());
        }
        let body = serde_json::json!({
            "snippet": snippet,
            "status": { "privacyStatus": privacy.as_api_str() },
        });
        self.post_json("/playlists", &[("part", "snippet,status")], &body)
            .await
    }

    /// `PUT /playlists` with `{ id, snippet: { title } }`. Scope:
    /// `playlists.write`. YouTube requires the *full* snippet on
    /// update — title is mandatory and replacing it without a body is
    /// not supported.
    pub async fn rename_playlist(&self, playlist_id: &str, new_title: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        let body = serde_json::json!({
            "id": playlist_id,
            "snippet": { "title": new_title },
        });
        self.put_empty("/playlists", &[("part", "snippet")], &body)
            .await
    }

    /// `DELETE /playlists?id=<id>`. Scope: `playlists.write`.
    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        self.delete_empty("/playlists", &[("id", playlist_id)])
            .await
    }

    /// `POST /playlistItems` with a `videoId` resource. Scope:
    /// `playlists.write`. Returns the new `playlistItem.id`.
    pub async fn add_playlist_item(&self, playlist_id: &str, video_id: &str) -> Result<String> {
        self.require_scope("playlists.write")?;
        let body = serde_json::json!({
            "snippet": {
                "playlistId": playlist_id,
                "resourceId": { "kind": "youtube#video", "videoId": video_id },
            }
        });
        let item: PlaylistItem = self
            .post_json("/playlistItems", &[("part", "snippet")], &body)
            .await?;
        Ok(item.id)
    }

    /// `DELETE /playlistItems?id=<playlist_item_id>`. Scope:
    /// `playlists.write`. The argument is the **playlist item ID**,
    /// not the video ID — runtime callers resolve a video ID by
    /// listing the playlist first.
    pub async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        self.delete_empty("/playlistItems", &[("id", playlist_item_id)])
            .await
    }

    /// `GET /videos?myRating=like`. Scope: `library.read`. The set of
    /// videos the user has liked is the closest YouTube analogue of
    /// Spotify's saved-tracks library.
    ///
    /// Walks `nextPageToken` under the hood; per-page cap is 50.
    /// `max == None` means
    /// "fetch every liked video".
    pub async fn list_liked_videos(&self, max: Option<u32>) -> Result<Vec<VideoSummary>> {
        self.require_scope("library.read")?;
        self.paged_get(
            "/videos",
            &[("myRating", "like"), ("part", "snippet,contentDetails")],
            max,
            |page: VideoPage| (page.items, page.next_page_token),
        )
        .await
    }

    /// `POST /videos/rate?id=<id>&rating=like`. Scope: `library.write`.
    pub async fn like_video(&self, video_id: &str) -> Result<()> {
        self.require_scope("library.write")?;
        self.post_empty(
            "/videos/rate",
            &[("id", video_id), ("rating", "like")],
            &serde_json::json!({}),
        )
        .await
    }

    /// `POST /videos/rate?id=<id>&rating=none`. Scope: `library.write`.
    pub async fn unlike_video(&self, video_id: &str) -> Result<()> {
        self.require_scope("library.write")?;
        self.post_empty(
            "/videos/rate",
            &[("id", video_id), ("rating", "none")],
            &serde_json::json!({}),
        )
        .await
    }

    // -----------------------------------------------------------------
    // unscoped — called from lifecycle (pre-scopes)
    // -----------------------------------------------------------------

    /// `GET /channels?mine=true`. Used by `validate` during `zad
    /// service create ymusic` and by `service status` to capture the
    /// authenticated user's YouTube channel.
    pub async fn my_channel(&self) -> Result<ChannelSummary> {
        let page: ChannelPage = self
            .get_json(
                "/channels",
                &[("mine", "true"), ("part", "snippet,contentDetails")],
            )
            .await?;
        page.items.into_iter().next().ok_or(ZadError::Service {
            name: "ymusic",
            message: "no YouTube channel is associated with this Google account; \
                 visit https://youtube.com and create a channel before running \
                 `zad service create ymusic`."
                .into(),
        })
    }

    /// OpenID Connect `userinfo` — fetches the authenticated user's
    /// email so the lifecycle banner has something to show even when
    /// the account has no YouTube channel yet.
    pub async fn userinfo(&self) -> Result<UserInfo> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .get(USERINFO_URL)
            .bearer_auth(&access)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp, cache_dir.as_deref()).await
    }

    // -----------------------------------------------------------------
    // pagination helper
    // -----------------------------------------------------------------

    /// Walk YouTube's `pageToken` cursor over a list endpoint until
    /// either `max` is reached or the API stops emitting a
    /// `nextPageToken`. YouTube Data API v3 caps every list endpoint
    /// we touch at `maxResults=50`; the helper sends 50 per call
    /// unless `max` cuts it shorter. `extract` returns
    /// `(items, next_page_token)` from each decoded page.
    async fn paged_get<P, T, F>(
        &self,
        path: &str,
        base_query: &[(&str, &str)],
        max: Option<u32>,
        extract: F,
    ) -> Result<Vec<T>>
    where
        P: for<'de> Deserialize<'de>,
        F: Fn(P) -> (Vec<T>, Option<String>),
    {
        const PAGE_SIZE: u32 = 50;
        let mut out: Vec<T> = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let remaining = max.map(|m| m.saturating_sub(out.len() as u32));
            if remaining == Some(0) {
                break;
            }
            let this_max = remaining.map(|r| r.min(PAGE_SIZE)).unwrap_or(PAGE_SIZE);
            let max_str = this_max.to_string();
            let mut query: Vec<(&str, &str)> = base_query.to_vec();
            query.push(("maxResults", max_str.as_str()));
            if let Some(tok) = page_token.as_deref() {
                query.push(("pageToken", tok));
            }
            let page: P = self.get_json(path, &query).await?;
            let (mut items, next) = extract(page);
            out.append(&mut items);
            match next {
                Some(t) if !t.is_empty() => page_token = Some(t),
                _ => break,
            }
        }
        if let Some(m) = max {
            out.truncate(m as usize);
        }
        Ok(out)
    }

    // -----------------------------------------------------------------
    // low-level HTTP glue
    // -----------------------------------------------------------------

    fn resolved_cache_dir(&self) -> Option<PathBuf> {
        self.cache_service_dir
            .clone()
            .or_else(|| token_cache::service_dir(SERVICE).ok())
    }

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

    async fn post_empty(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<()> {
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
        finalize_empty(resp, cache_dir.as_deref()).await
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
}

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "ymusic",
        message: format!("network error talking to YouTube: {e}"),
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
        name: "ymusic",
        message: format!("failed to decode YouTube response: {e}"),
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
        || lower.contains("invalid_credentials")
    {
        format!(
            "YouTube rejected the access token (HTTP {code}); the credentials may have been \
             revoked. Re-run `zad service create ymusic` to re-authorize. Body: {body}"
        )
    } else if code == 429
        || (code == 403 && (lower.contains("quotaexceeded") || lower.contains("ratelimitexceeded")))
    {
        format!(
            "YouTube Data API quota / rate limit exhausted (HTTP {code}); back off before \
             retrying. Body: {body}"
        )
    } else {
        format!("HTTP {code}: {body}")
    };
    ZadError::Service {
        name: "ymusic",
        message,
    }
}

// ---------------------------------------------------------------------------
// privacy enum
// ---------------------------------------------------------------------------

/// Privacy setting for a YouTube playlist. The Data API accepts the
/// three string values below verbatim; we use a typed wrapper so the
/// CLI can validate user input without leaking the on-the-wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    Private,
    Unlisted,
    Public,
}

impl Privacy {
    pub fn as_api_str(self) -> &'static str {
        match self {
            Privacy::Private => "private",
            Privacy::Unlisted => "unlisted",
            Privacy::Public => "public",
        }
    }
}

// ---------------------------------------------------------------------------
// Response types — minimal projections of the YouTube Data API objects.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserInfo {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sub: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelSummary {
    pub id: String,
    #[serde(default)]
    pub snippet: Option<ChannelSnippet>,
    #[serde(rename = "contentDetails", default)]
    pub content_details: Option<ChannelContentDetails>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelSnippet {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "customUrl", default)]
    pub custom_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelContentDetails {
    #[serde(rename = "relatedPlaylists", default)]
    pub related_playlists: Option<RelatedPlaylists>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelatedPlaylists {
    #[serde(default)]
    pub likes: Option<String>,
    #[serde(default)]
    pub uploads: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelPage {
    #[serde(default)]
    pub items: Vec<ChannelSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistSummary {
    pub id: String,
    #[serde(default)]
    pub snippet: Option<PlaylistSnippet>,
    #[serde(rename = "contentDetails", default)]
    pub content_details: Option<PlaylistContentDetails>,
    #[serde(default)]
    pub status: Option<PlaylistStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistSnippet {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "channelId", default)]
    pub channel_id: Option<String>,
    #[serde(rename = "channelTitle", default)]
    pub channel_title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistContentDetails {
    #[serde(rename = "itemCount", default)]
    pub item_count: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistStatus {
    #[serde(rename = "privacyStatus", default)]
    pub privacy_status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistPage {
    #[serde(default)]
    pub items: Vec<PlaylistSummary>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistItem {
    pub id: String,
    #[serde(default)]
    pub snippet: Option<PlaylistItemSnippet>,
    #[serde(rename = "contentDetails", default)]
    pub content_details: Option<PlaylistItemContentDetails>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistItemSnippet {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "videoOwnerChannelTitle", default)]
    pub video_owner_channel_title: Option<String>,
    #[serde(rename = "resourceId", default)]
    pub resource_id: Option<ResourceId>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaylistItemContentDetails {
    #[serde(rename = "videoId", default)]
    pub video_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceId {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(rename = "videoId", default)]
    pub video_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistItemPage {
    #[serde(default)]
    pub items: Vec<PlaylistItem>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoSummary {
    pub id: String,
    #[serde(default)]
    pub snippet: Option<VideoSnippet>,
    #[serde(rename = "contentDetails", default)]
    pub content_details: Option<VideoContentDetails>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoSnippet {
    pub title: String,
    #[serde(rename = "channelTitle", default)]
    pub channel_title: Option<String>,
    #[serde(rename = "channelId", default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoContentDetails {
    #[serde(default)]
    pub duration: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoPage {
    #[serde(default)]
    pub items: Vec<VideoSummary>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
}

/// One row of `/search` output. The `id` block carries one of
/// `videoId` / `playlistId` / `channelId`; only the matching field
/// for the requested `type` is populated. We keep all three optional
/// so a single struct covers every search-result variant without
/// `#[serde(untagged)]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchItem {
    #[serde(default)]
    pub id: Option<SearchItemId>,
    #[serde(default)]
    pub snippet: Option<SearchItemSnippet>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchItemId {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(rename = "videoId", default)]
    pub video_id: Option<String>,
    #[serde(rename = "playlistId", default)]
    pub playlist_id: Option<String>,
    #[serde(rename = "channelId", default)]
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchItemSnippet {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "channelTitle", default)]
    pub channel_title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchPage {
    #[serde(default)]
    pub items: Vec<SearchItem>,
}
