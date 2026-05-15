//! YouTube Music HTTP client over InnerTube.
//!
//! Talks to the same `music.youtube.com/youtubei/v1` surface the web
//! app uses — not the Data API v3. The auth flow is still OAuth 2.0
//! against Google, but with the **TVHTML5** client credentials
//! (constants in [`super::oauth_device`]) instead of an
//! operator-supplied Desktop-app client. The refresh token is the
//! only per-user secret; everything else is shared across installs.
//!
//! ## Error mapping
//!
//! Every non-2xx HTTP status surfaces as `ZadError::Service { name:
//! "ymusic", message }`. Three cases are specialised:
//!
//! - `401` with InnerTube's `UNAUTHENTICATED` payload (or the
//!   classic `invalid_credentials` / `invalid_token` markers Google
//!   sometimes returns) → "credentials revoked; re-run `zad service
//!   create ymusic`".
//! - `429` → fed through [`rate_limit::check_response`] which records
//!   the deadline so sibling processes back off too. InnerTube's
//!   rolling limits are much looser than the Data API daily quota,
//!   so this path is rare in practice.
//! - `403` with a Google quota reason in the body
//!   (`quotaExceeded`, `dailyLimitExceeded`, `rateLimitExceeded`,
//!   `userRateLimitExceeded`) → *also* mapped to
//!   [`ZadError::RateLimited`] via [`google_quota::check_403`] and
//!   persisted to `~/.zad/state/ymusic/rate_limit.json`. InnerTube
//!   should not normally hit this branch, but the classifier is kept
//!   so a future Google policy change can't silently regress to
//!   silent quota burning.
//!
//! ## InnerTube response shape
//!
//! Every InnerTube call returns deeply-nested JSON keyed by
//! `*Renderer` types. We define a handful of `serde` projections
//! below that pick out only the fields zad's public types need, and
//! we keep every field `Option`-typed because the shapes drift
//! between Google deploys. If a verb's parser stops finding what it
//! expects, the request itself probably still succeeded — re-walk
//! the payload via the integration tests in
//! `crates/zad/tests/ymusic_*` to find the new path.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::error::{Result, ZadError};
use crate::google_quota;
use crate::oauth::{self, RefreshTokenStore};
use crate::rate_limit;
use crate::service::ymusic::oauth_device::{TVHTML5_CLIENT_ID, TVHTML5_CLIENT_SECRET};
use crate::service::ymusic::{
    API_BASE, INNERTUBE_API_KEY, TOKEN_URL, USERINFO_URL, WEB_REMIX_CLIENT_NAME,
    WEB_REMIX_CLIENT_VERSION,
};
use crate::token_cache;

const SERVICE: &str = "ymusic";

/// Thin wrapper over InnerTube. Holds the refresh token and mints an
/// access token on demand against Google's TVHTML5 client.
///
/// The constructor still accepts `client_id` / `client_secret`
/// arguments for source-compat with downstream callers, but the
/// values are ignored — every InnerTube call uses the TVHTML5
/// constants. The lifecycle path (`zad service create ymusic`) no
/// longer prompts for these fields and stores empty strings in the
/// keychain slots that historically held them.
#[derive(Clone)]
pub struct YmusicHttp {
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
    /// Cached access token for the lifetime of this process.
    cached_access: Arc<Mutex<Option<String>>>,
    /// Directory used for the cross-process token cache and refresh
    /// lock. `None` means "resolve from `zad_home()` at runtime".
    cache_service_dir: Option<PathBuf>,
    /// Base URL for InnerTube. Defaults to
    /// [`crate::service::ymusic::API_BASE`]; overridable so tests
    /// can point at a localhost mock.
    api_base: String,
}

impl YmusicHttp {
    /// Full-featured constructor used by runtime verbs. No persisting
    /// store — equivalent to [`Self::with_store`] with `None`.
    ///
    /// `_client_id` and `_client_secret` are ignored — InnerTube uses
    /// TVHTML5 credentials, not a per-operator OAuth client. The
    /// parameters survive so callers built against the old Data API
    /// signature don't break at the source level.
    pub fn new(
        _client_id: String,
        _client_secret: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        Self::with_store(
            String::new(),
            String::new(),
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
        _client_id: String,
        _client_secret: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
    ) -> Self {
        Self {
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
    pub fn unscoped(_client_id: String, _client_secret: String, refresh_token: String) -> Self {
        Self::new(
            String::new(),
            String::new(),
            refresh_token,
            BTreeSet::new(),
            PathBuf::new(),
        )
    }

    /// Override the token endpoint URL. Test-only.
    #[doc(hidden)]
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    /// Override the cross-process token cache directory. Test-only.
    #[doc(hidden)]
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_service_dir = Some(dir);
        self
    }

    /// Override the InnerTube base URL. Test-only.
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
    /// this process. Refreshes against Google's TVHTML5 client
    /// (constants from [`super::oauth_device`]).
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
            TVHTML5_CLIENT_ID,
            Some(TVHTML5_CLIENT_SECRET),
            &current,
        )
        .await?;

        // Persist rotation if the provider returned a different
        // refresh token. Google rarely rotates today, but the safety
        // net is symmetric with Spotify's.
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

    /// `POST /search`. Scope: `search`. `types` is one or more of
    /// `video`, `song`, `playlist`, `channel`, `album`, `artist`.
    /// Returns at most `limit` parsed hits.
    pub async fn search(&self, query: &str, types: &[&str], limit: u32) -> Result<Vec<SearchItem>> {
        self.require_scope("search")?;
        let limit = limit.clamp(1, 50) as usize;
        let body = {
            let mut b = innertube_body();
            b["query"] = Value::String(query.to_string());
            if let Some(params) = search_params_for(types) {
                b["params"] = Value::String(params.to_string());
            }
            b
        };
        let raw: Value = self.post_innertube("/search", &body).await?;
        Ok(parse_search(&raw, limit))
    }

    /// `POST /browse` with `browseId=FEmusic_liked_playlists`. Scope:
    /// `playlists.read`. Returns user-owned playlists. The `max`
    /// argument is honoured client-side; InnerTube returns the full
    /// page in one shot for this surface.
    pub async fn list_my_playlists(&self, max: Option<u32>) -> Result<Vec<PlaylistSummary>> {
        self.require_scope("playlists.read")?;
        let body = {
            let mut b = innertube_body();
            b["browseId"] = Value::String("FEmusic_liked_playlists".to_string());
            b
        };
        let raw: Value = self.post_innertube("/browse", &body).await?;
        let mut items = parse_my_playlists(&raw);
        if let Some(m) = max {
            items.truncate(m as usize);
        }
        Ok(items)
    }

    /// `POST /browse` with `browseId=VL<playlist_id>`. Scope:
    /// `playlists.read`.
    pub async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        self.require_scope("playlists.read")?;
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{playlist_id}")
        };
        let body = {
            let mut b = innertube_body();
            b["browseId"] = Value::String(browse_id.clone());
            b
        };
        let raw: Value = self.post_innertube("/browse", &body).await?;
        parse_one_playlist(&raw, playlist_id).ok_or(ZadError::Service {
            name: "ymusic",
            message: format!("playlist `{playlist_id}` not found"),
        })
    }

    /// `POST /browse` with `browseId=VL<playlist_id>`. Scope:
    /// `playlists.read`. Walks `continuationContents` until the page
    /// stops emitting a `continuation` token (or `max` is reached).
    pub async fn get_playlist_items(
        &self,
        playlist_id: &str,
        max: Option<u32>,
    ) -> Result<Vec<PlaylistItem>> {
        self.require_scope("playlists.read")?;
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{playlist_id}")
        };
        let mut body = innertube_body();
        body["browseId"] = Value::String(browse_id);
        let raw: Value = self.post_innertube("/browse", &body).await?;
        let mut out = parse_playlist_items(&raw);
        let mut cont = next_continuation_token(&raw);
        while let Some(token) = cont {
            if let Some(m) = max
                && out.len() >= m as usize
            {
                break;
            }
            let mut body = innertube_body();
            body["continuation"] = Value::String(token.clone());
            let raw: Value =
                self.post_innertube(&format!("/browse?ctoken={token}"), &body).await?;
            out.extend(parse_playlist_items(&raw));
            cont = next_continuation_token(&raw);
        }
        if let Some(m) = max {
            out.truncate(m as usize);
        }
        Ok(out)
    }

    /// `POST /playlist/create`. Scope: `playlists.write`.
    pub async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary> {
        self.require_scope("playlists.write")?;
        let mut body = innertube_body();
        body["title"] = Value::String(title.to_string());
        if let Some(d) = description {
            body["description"] = Value::String(d.to_string());
        }
        body["privacyStatus"] = Value::String(privacy.as_innertube_str().to_string());
        let raw: Value = self.post_innertube("/playlist/create", &body).await?;
        let id = raw
            .get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(ZadError::Service {
                name: "ymusic",
                message: format!(
                    "InnerTube create_playlist returned no playlistId; body: {raw}"
                ),
            })?;
        Ok(PlaylistSummary {
            id,
            snippet: Some(PlaylistSnippet {
                title: title.to_string(),
                description: description.map(str::to_string),
                channel_id: None,
                channel_title: None,
            }),
            content_details: None,
            status: Some(PlaylistStatus {
                privacy_status: Some(privacy.as_innertube_str().to_string()),
            }),
        })
    }

    /// `POST /browse/edit_playlist` with `ACTION_SET_PLAYLIST_NAME`.
    /// Scope: `playlists.write`.
    pub async fn rename_playlist(&self, playlist_id: &str, new_title: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        let mut body = innertube_body();
        body["playlistId"] = Value::String(playlist_id.to_string());
        body["actions"] = json!([{
            "action": "ACTION_SET_PLAYLIST_NAME",
            "playlistName": new_title,
        }]);
        let raw: Value = self.post_innertube("/browse/edit_playlist", &body).await?;
        ensure_edit_succeeded(&raw, "rename_playlist")
    }

    /// `POST /playlist/delete`. Scope: `playlists.write`.
    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        let mut body = innertube_body();
        body["playlistId"] = Value::String(playlist_id.to_string());
        let _: Value = self.post_innertube("/playlist/delete", &body).await?;
        Ok(())
    }

    /// `POST /browse/edit_playlist` with `ACTION_ADD_VIDEO`. Scope:
    /// `playlists.write`. Returns InnerTube's `setVideoId` — the
    /// per-playlist-item handle that `remove_playlist_item` consumes.
    pub async fn add_playlist_item(&self, playlist_id: &str, video_id: &str) -> Result<String> {
        self.require_scope("playlists.write")?;
        let mut body = innertube_body();
        body["playlistId"] = Value::String(playlist_id.to_string());
        body["actions"] = json!([{
            "action": "ACTION_ADD_VIDEO",
            "addedVideoId": video_id,
        }]);
        let raw: Value = self.post_innertube("/browse/edit_playlist", &body).await?;
        ensure_edit_succeeded(&raw, "add_playlist_item")?;
        Ok(extract_set_video_id(&raw).unwrap_or_else(|| video_id.to_string()))
    }

    /// `POST /browse/edit_playlist` with `ACTION_REMOVE_VIDEO`. Scope:
    /// `playlists.write`. The argument is InnerTube's `setVideoId`
    /// (returned by `add_playlist_item` or by walking the playlist).
    pub async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()> {
        self.require_scope("playlists.write")?;
        // The remove action needs both `setVideoId` (per-item handle)
        // and the underlying `removedVideoId`. Older callers only
        // know the `setVideoId`; for those we send it in both slots —
        // InnerTube tolerates the duplicate.
        let mut body = innertube_body();
        body["actions"] = json!([{
            "action": "ACTION_REMOVE_VIDEO",
            "setVideoId": playlist_item_id,
            "removedVideoId": playlist_item_id,
        }]);
        let raw: Value = self.post_innertube("/browse/edit_playlist", &body).await?;
        ensure_edit_succeeded(&raw, "remove_playlist_item")
    }

    /// `POST /browse` with `browseId=FEmusic_liked_videos`. Scope:
    /// `library.read`.
    pub async fn list_liked_videos(&self, max: Option<u32>) -> Result<Vec<VideoSummary>> {
        self.require_scope("library.read")?;
        let mut body = innertube_body();
        body["browseId"] = Value::String("FEmusic_liked_videos".to_string());
        let raw: Value = self.post_innertube("/browse", &body).await?;
        let mut out = parse_liked_videos(&raw);
        if let Some(m) = max {
            out.truncate(m as usize);
        }
        Ok(out)
    }

    /// `POST /like/like`. Scope: `library.write`.
    pub async fn like_video(&self, video_id: &str) -> Result<()> {
        self.require_scope("library.write")?;
        let mut body = innertube_body();
        body["target"] = json!({"videoId": video_id});
        let _: Value = self.post_innertube("/like/like", &body).await?;
        Ok(())
    }

    /// `POST /like/removelike`. Scope: `library.write`.
    pub async fn unlike_video(&self, video_id: &str) -> Result<()> {
        self.require_scope("library.write")?;
        let mut body = innertube_body();
        body["target"] = json!({"videoId": video_id});
        let _: Value = self.post_innertube("/like/removelike", &body).await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // unscoped — called from lifecycle (pre-scopes)
    // -----------------------------------------------------------------

    /// Best-effort channel summary, derived from the JWT identity in
    /// the OAuth response. InnerTube does not expose a direct
    /// `channels?mine=true` analogue; we surface a thin record so the
    /// lifecycle banner has something to print.
    pub async fn my_channel(&self) -> Result<ChannelSummary> {
        // `FEmusic_library_landing` returns the active user's library
        // home; the response carries the channel id on the first
        // `musicTwoRowItemRenderer`. If we can't pluck it out, fall
        // back to a synthetic record so `validate` still succeeds.
        let mut body = innertube_body();
        body["browseId"] = Value::String("FEmusic_library_landing".to_string());
        let raw: Result<Value> = self.post_innertube("/browse", &body).await;
        let id = raw
            .as_ref()
            .ok()
            .and_then(|v| {
                v.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content")
                    .and_then(extract_first_browse_id)
            })
            .unwrap_or_else(|| "ymusic-user".to_string());
        Ok(ChannelSummary {
            id,
            snippet: Some(ChannelSnippet {
                title: Some("YouTube Music user".to_string()),
                description: None,
                custom_url: None,
            }),
            content_details: None,
        })
    }

    /// OpenID Connect `userinfo` — fetches the authenticated user's
    /// email so the lifecycle banner has something to show.
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
    // low-level InnerTube glue
    // -----------------------------------------------------------------

    fn resolved_cache_dir(&self) -> Option<PathBuf> {
        self.cache_service_dir
            .clone()
            .or_else(|| token_cache::service_dir(SERVICE).ok())
    }

    /// Post an InnerTube body and return the raw JSON. Every caller
    /// then runs its own `parse_*` function over the result.
    async fn post_innertube(&self, path: &str, body: &Value) -> Result<Value> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let url = if path.contains('?') {
            format!("{}{path}&key={INNERTUBE_API_KEY}", self.api_base)
        } else {
            format!("{}{path}?key={INNERTUBE_API_KEY}", self.api_base)
        };
        let resp = reqwest::Client::new()
            .post(&url)
            .bearer_auth(&access)
            .header("X-Origin", "https://music.youtube.com")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
            )
            .header("X-Goog-Visitor-Id", "")
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp, cache_dir.as_deref()).await
    }
}

// ---------------------------------------------------------------------------
// InnerTube context envelope
// ---------------------------------------------------------------------------

/// Every InnerTube body starts with the same `context.client` block.
fn innertube_body() -> Value {
    json!({
        "context": {
            "client": {
                "clientName": WEB_REMIX_CLIENT_NAME,
                "clientVersion": WEB_REMIX_CLIENT_VERSION,
                "hl": "en",
                "gl": "US",
            },
            "user": {},
        },
    })
}

/// Map a Data-API-style type filter to an InnerTube `params` value
/// for `/search`. `None` means "no filter — return whatever
/// InnerTube ranks highest" (matches the Web Music search default).
///
/// The opaque `params` strings below are protobuf payloads that
/// `music.youtube.com` ships in its bundle. They are widely
/// catalogued in the ytmusicapi reference; the values here are the
/// ones the web app sends when a user filters by category.
fn search_params_for(types: &[&str]) -> Option<&'static str> {
    let primary = types.first()?.to_ascii_lowercase();
    Some(match primary.as_str() {
        "video" | "videos" => "EgWKAQIQAWoOEAMQBBAJEAoQBRAVEBM%3D",
        "song" | "songs" | "track" => "EgWKAQIIAWoOEAMQBBAJEAoQBRAVEBM%3D",
        "playlist" | "playlists" => "EgWKAQIoAWoOEAMQBBAJEAoQBRAVEBM%3D",
        "channel" | "channels" | "artist" | "artists" => {
            "EgWKAQIgAWoOEAMQBBAJEAoQBRAVEBM%3D"
        }
        "album" | "albums" => "EgWKAQIYAWoOEAMQBBAJEAoQBRAVEBM%3D",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Response parsers — pluck minimal projections out of InnerTube JSON.
// ---------------------------------------------------------------------------

fn parse_search(raw: &Value, limit: usize) -> Vec<SearchItem> {
    let mut out: Vec<SearchItem> = Vec::with_capacity(limit);
    // `contents.tabbedSearchResultsRenderer.tabs[0].tabRenderer.content
    //  .sectionListRenderer.contents[].musicShelfRenderer.contents[]
    //  .musicResponsiveListItemRenderer`
    let sections = raw
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array);
    let Some(sections) = sections else {
        return out;
    };
    for section in sections {
        let shelf = section
            .pointer("/musicShelfRenderer/contents")
            .and_then(Value::as_array);
        let Some(rows) = shelf else { continue };
        for row in rows {
            if out.len() >= limit {
                return out;
            }
            let r = match row.pointer("/musicResponsiveListItemRenderer") {
                Some(v) => v,
                None => continue,
            };
            let video_id = r
                .pointer("/playlistItemData/videoId")
                .and_then(Value::as_str)
                .or_else(|| {
                    r.pointer(
                        "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
                    )
                    .and_then(Value::as_str)
                })
                .map(str::to_string);
            let title = first_run_text(r.pointer("/flexColumns/0"));
            let channel_title = first_run_text(r.pointer("/flexColumns/1"));
            out.push(SearchItem {
                id: Some(SearchItemId {
                    kind: video_id.as_ref().map(|_| "youtube#video".to_string()),
                    video_id,
                    playlist_id: None,
                    channel_id: None,
                }),
                snippet: Some(SearchItemSnippet {
                    title,
                    channel_title,
                    description: None,
                }),
            });
        }
    }
    out
}

fn parse_my_playlists(raw: &Value) -> Vec<PlaylistSummary> {
    let mut out: Vec<PlaylistSummary> = Vec::new();
    let items = raw
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/itemSectionRenderer/contents/0/gridRenderer/items")
        .and_then(Value::as_array);
    let Some(items) = items else {
        return out;
    };
    for item in items {
        let r = match item.pointer("/musicTwoRowItemRenderer") {
            Some(v) => v,
            None => continue,
        };
        let title = first_run_text(r.pointer("/title")).unwrap_or_default();
        // Skip the "New playlist" tile YouTube Music prepends.
        if title.eq_ignore_ascii_case("new playlist") {
            continue;
        }
        let id = r
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .map(|s| s.trim_start_matches("VL").to_string());
        let Some(id) = id else { continue };
        let description = first_run_text(r.pointer("/subtitle"));
        out.push(PlaylistSummary {
            id,
            snippet: Some(PlaylistSnippet {
                title,
                description,
                channel_id: None,
                channel_title: None,
            }),
            content_details: None,
            status: None,
        });
    }
    out
}

fn parse_one_playlist(raw: &Value, original_id: &str) -> Option<PlaylistSummary> {
    let header = raw.pointer("/header/musicDetailHeaderRenderer")?;
    let title = first_run_text(header.get("title")).unwrap_or_default();
    let description = first_run_text(header.get("description"));
    Some(PlaylistSummary {
        id: original_id.trim_start_matches("VL").to_string(),
        snippet: Some(PlaylistSnippet {
            title,
            description,
            channel_id: None,
            channel_title: None,
        }),
        content_details: None,
        status: None,
    })
}

fn parse_playlist_items(raw: &Value) -> Vec<PlaylistItem> {
    let mut out: Vec<PlaylistItem> = Vec::new();
    let rows = raw
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents")
        .or_else(|| raw.pointer("/continuationContents/musicPlaylistShelfContinuation/contents"))
        .and_then(Value::as_array);
    let Some(rows) = rows else {
        return out;
    };
    for row in rows {
        let r = match row.pointer("/musicResponsiveListItemRenderer") {
            Some(v) => v,
            None => continue,
        };
        let video_id = r
            .pointer("/playlistItemData/videoId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let set_video_id = r
            .pointer("/playlistItemData/playlistSetVideoId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let title = first_run_text(r.pointer("/flexColumns/0"));
        let channel_title = first_run_text(r.pointer("/flexColumns/1"));
        let id = set_video_id.clone().or_else(|| video_id.clone()).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        out.push(PlaylistItem {
            id,
            snippet: Some(PlaylistItemSnippet {
                title,
                video_owner_channel_title: channel_title,
                resource_id: video_id.clone().map(|v| ResourceId {
                    kind: Some("youtube#video".to_string()),
                    video_id: Some(v),
                }),
            }),
            content_details: video_id.map(|v| PlaylistItemContentDetails {
                video_id: Some(v),
            }),
        });
    }
    out
}

fn parse_liked_videos(raw: &Value) -> Vec<VideoSummary> {
    let mut out: Vec<VideoSummary> = Vec::new();
    let rows = raw
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents")
        .and_then(Value::as_array);
    let Some(rows) = rows else {
        return out;
    };
    for row in rows {
        let r = match row.pointer("/musicResponsiveListItemRenderer") {
            Some(v) => v,
            None => continue,
        };
        let video_id = r
            .pointer("/playlistItemData/videoId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(id) = video_id else { continue };
        let title = first_run_text(r.pointer("/flexColumns/0")).unwrap_or_default();
        let channel_title = first_run_text(r.pointer("/flexColumns/1"));
        out.push(VideoSummary {
            id,
            snippet: Some(VideoSnippet {
                title,
                channel_title,
                channel_id: None,
                description: None,
            }),
            content_details: None,
        });
    }
    out
}

fn next_continuation_token(raw: &Value) -> Option<String> {
    raw.pointer("/continuationContents/musicPlaylistShelfContinuation/continuations/0/nextContinuationData/continuation")
        .or_else(|| raw.pointer("/continuationContents/musicShelfContinuation/continuations/0/nextContinuationData/continuation"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_set_video_id(raw: &Value) -> Option<String> {
    raw.pointer("/playlistEditResults/0/playlistEditVideoAddedResultData/setVideoId")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_first_browse_id(node: &Value) -> Option<String> {
    if let Some(v) = node.get("browseId").and_then(Value::as_str) {
        return Some(v.to_string());
    }
    match node {
        Value::Array(arr) => arr.iter().find_map(extract_first_browse_id),
        Value::Object(map) => map.values().find_map(extract_first_browse_id),
        _ => None,
    }
}

fn ensure_edit_succeeded(raw: &Value, op: &'static str) -> Result<()> {
    let status = raw
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("STATUS_UNKNOWN");
    if status == "STATUS_SUCCEEDED" {
        return Ok(());
    }
    Err(ZadError::Service {
        name: "ymusic",
        message: format!("InnerTube `{op}` returned status `{status}`; body: {raw}"),
    })
}

/// Walk a `flexColumns[n]` block and return the first run's text.
/// InnerTube renders all human-visible strings as `runs: [{text: …}]`
/// arrays; the first run carries the canonical value while subsequent
/// runs add styling fragments we don't need.
fn first_run_text(node: Option<&Value>) -> Option<String> {
    let node = node?;
    if let Some(s) = node
        .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
        .and_then(Value::as_str)
    {
        return Some(s.to_string());
    }
    if let Some(s) = node.pointer("/runs/0/text").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    None
}

// ---------------------------------------------------------------------------
// HTTP plumbing
// ---------------------------------------------------------------------------

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "ymusic",
        message: format!("network error talking to YouTube Music: {e}"),
    }
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
    cache_dir: Option<&Path>,
) -> Result<T> {
    // Fast path for the canonical RFC 6585 status. Google's
    // `uploadRateLimitExceeded` is the only 429 the Data API
    // currently emits, but the shared helper keeps every service on
    // the same contract.
    if let Some(err) = rate_limit::check_response(SERVICE, &resp) {
        return Err(err);
    }
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 {
            token_cache::clear(cache_dir);
        }
        // Snapshot headers before consuming the body — the 403-quota
        // classifier reads `Retry-After` when Google ships one.
        let headers = resp.headers().clone();
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_error(status, &body, &headers));
    }
    rate_limit::clear(SERVICE);
    resp.json::<T>().await.map_err(|e| ZadError::Service {
        name: "ymusic",
        message: format!("failed to decode YouTube Music response: {e}"),
    })
}

/// Map a non-2xx InnerTube response to a typed error.
///
/// On 403 we inspect the JSON body for one of Google's quota reasons
/// and promote it to [`ZadError::RateLimited`], persisting the
/// deadline so every sibling process is gated by
/// [`rate_limit::precall_check`] until the window passes. InnerTube
/// rarely returns 403 quota responses (that was the Data API's
/// failure mode), but keeping the classifier is cheap insurance.
fn classify_error(
    status: reqwest::StatusCode,
    body: &str,
    headers: &reqwest::header::HeaderMap,
) -> ZadError {
    if let Some(err) = google_quota::check_403(SERVICE, status, body, headers) {
        return err;
    }
    map_http_error(status, body)
}

fn map_http_error(status: reqwest::StatusCode, body: &str) -> ZadError {
    let code = status.as_u16();
    let lower = body.to_ascii_lowercase();
    let message = if code == 401
        || lower.contains("unauthenticated")
        || lower.contains("invalid_token")
        || lower.contains("invalid_credentials")
    {
        format!(
            "YouTube Music rejected the access token (HTTP {code}); the credentials may have \
             been revoked. Re-run `zad service create ymusic` to re-authorize. Body: {body}"
        )
    } else if code == 429 {
        format!(
            "YouTube Music rate-limited this call (HTTP {code}); back off before retrying. \
             Body: {body}"
        )
    } else if code == 403 {
        // 403 without a known quota reason — usually scope / consent /
        // ownership. Point the user at the most common remediation.
        format!(
            "YouTube rejected the request (HTTP {code}); the OAuth grant may be missing a scope \
             or the target resource is owned by a different channel. Re-run \
             `zad service create ymusic` if you recently changed scopes. Body: {body}"
        )
    } else if (500..=599).contains(&code) {
        // 5xx is typically transient — surface a hint so operators
        // know a re-run is reasonable.
        format!(
            "YouTube returned a server error (HTTP {code}); this is typically transient — \
             retry the same command. Body: {body}"
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

/// Privacy setting for a YouTube Music playlist. InnerTube accepts
/// the uppercase strings below verbatim in the `privacyStatus` field
/// of `playlist/create`. We surface the lowercase form via
/// [`Privacy::as_api_str`] for parity with the older Data API call
/// sites that pass the value through verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privacy {
    Private,
    Unlisted,
    Public,
}

impl Privacy {
    /// Lowercase form, accepted by the older Data API. Kept for
    /// source-compat with callers that pass the string back into
    /// places that need it (e.g. dry-run rendering).
    pub fn as_api_str(self) -> &'static str {
        match self {
            Privacy::Private => "private",
            Privacy::Unlisted => "unlisted",
            Privacy::Public => "public",
        }
    }

    /// Uppercase form expected by InnerTube `playlist/create`.
    pub fn as_innertube_str(self) -> &'static str {
        match self {
            Privacy::Private => "PRIVATE",
            Privacy::Unlisted => "UNLISTED",
            Privacy::Public => "PUBLIC",
        }
    }
}

// ---------------------------------------------------------------------------
// Public response types — shape-compatible with the old Data API
// projections so the facade and CLI keep compiling unchanged.
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

/// One row of `/search` output. The `id` block carries one of
/// `videoId` / `playlistId` / `channelId`; only the matching field
/// for the requested `type` is populated.
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
