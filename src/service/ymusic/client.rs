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
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Result, ZadError};
use crate::oauth;
use crate::service::ymusic::{API_BASE, TOKEN_URL, USERINFO_URL};

/// Thin wrapper over YouTube Data API v3. Holds a refresh token and
/// mints an access token on demand.
#[derive(Clone)]
pub struct YmusicHttp {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    scopes: BTreeSet<String>,
    config_path: PathBuf,
    /// Cached access token for the lifetime of this process.
    cached_access: Arc<Mutex<Option<String>>>,
}

impl YmusicHttp {
    /// Full-featured constructor used by runtime verbs.
    pub fn new(
        client_id: String,
        client_secret: String,
        refresh_token: String,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            refresh_token,
            scopes,
            config_path,
            cached_access: Arc::new(Mutex::new(None)),
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
    async fn access_token(&self) -> Result<String> {
        {
            let guard = self.cached_access.lock().await;
            if let Some(t) = guard.as_ref() {
                return Ok(t.clone());
            }
        }
        let fresh = oauth::refresh_access_token(
            "ymusic",
            TOKEN_URL,
            &self.client_id,
            Some(&self.client_secret),
            &self.refresh_token,
        )
        .await?;
        let mut guard = self.cached_access.lock().await;
        *guard = Some(fresh.access_token.clone());
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
    pub async fn list_my_playlists(&self, limit: u32) -> Result<Vec<PlaylistSummary>> {
        self.require_scope("playlists.read")?;
        let limit = limit.clamp(1, 50).to_string();
        let page: PlaylistPage = self
            .get_json(
                "/playlists",
                &[
                    ("mine", "true"),
                    ("part", "snippet,contentDetails,status"),
                    ("maxResults", limit.as_str()),
                ],
            )
            .await?;
        Ok(page.items)
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
    pub async fn get_playlist_items(
        &self,
        playlist_id: &str,
        limit: u32,
    ) -> Result<Vec<PlaylistItem>> {
        self.require_scope("playlists.read")?;
        let limit = limit.clamp(1, 50).to_string();
        let page: PlaylistItemPage = self
            .get_json(
                "/playlistItems",
                &[
                    ("playlistId", playlist_id),
                    ("part", "snippet,contentDetails"),
                    ("maxResults", limit.as_str()),
                ],
            )
            .await?;
        Ok(page.items)
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
    pub async fn list_liked_videos(&self, limit: u32) -> Result<Vec<VideoSummary>> {
        self.require_scope("library.read")?;
        let limit = limit.clamp(1, 50).to_string();
        let page: VideoPage = self
            .get_json(
                "/videos",
                &[
                    ("myRating", "like"),
                    ("part", "snippet,contentDetails"),
                    ("maxResults", limit.as_str()),
                ],
            )
            .await?;
        Ok(page.items)
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
        let resp = reqwest::Client::new()
            .get(USERINFO_URL)
            .bearer_auth(&access)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp).await
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

    async fn post_empty(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<()> {
        let access = self.access_token().await?;
        let resp = reqwest::Client::new()
            .post(format!("{API_BASE}{path}"))
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
}

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "ymusic",
        message: format!("network error talking to YouTube: {e}"),
    }
}

async fn decode_response<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(map_http_error(status, &body));
    }
    resp.json::<T>().await.map_err(|e| ZadError::Service {
        name: "ymusic",
        message: format!("failed to decode YouTube response: {e}"),
    })
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
