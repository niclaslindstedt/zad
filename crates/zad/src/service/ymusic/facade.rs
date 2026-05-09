//! Typed library facade for YouTube Music. Same shape as
//! `service::gcal::facade` (full OAuth: client_id + client_secret +
//! refresh_token).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::{self, YmusicServiceCfg};
use crate::error::{Result, ZadError};
use crate::oauth::{KeychainRefreshStore, RefreshTokenStore};
use crate::secrets::{self, Scope};
use crate::service::ymusic::client::{
    PlaylistSummary, Privacy, SearchItem, VideoSummary, YmusicHttp,
};
use crate::service::ymusic::permissions::{self as perms, EffectivePermissions, YmusicFunction};

/// Full OAuth credentials for YouTube Music (same shape as gcal).
///
/// `refresh_token_store` is wired by `Ymusic::from_default_config` to
/// the canonical zad keychain slot; library users with custom storage
/// can supply their own [`RefreshTokenStore`] impl. Google does not
/// currently rotate refresh tokens, but the field is kept symmetric
/// with `SpotifyCredentials` so the bug Spotify hit can't recur
/// silently here.
#[derive(Clone)]
pub struct YmusicCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
}

impl std::fmt::Debug for YmusicCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YmusicCredentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field(
                "refresh_token_store",
                &self.refresh_token_store.as_ref().map(|_| "<store>"),
            )
            .finish()
    }
}

impl YmusicCredentials {
    /// Build credentials with no rotation store. Library callers that
    /// want zad to keep the OS keychain in sync should use
    /// [`Self::with_keychain_store`] instead.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            refresh_token: refresh_token.into(),
            refresh_token_store: None,
        }
    }

    /// Convenience constructor wiring the canonical zad
    /// [`KeychainRefreshStore`] (Global scope).
    pub fn with_keychain_store(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Self {
        let store = KeychainRefreshStore::new(secrets::account("ymusic", "refresh", Scope::Global));
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            refresh_token: refresh_token.into(),
            refresh_token_store: Some(Arc::new(store)),
        }
    }
}

/// Typed library entry point for YouTube Music.
pub struct Ymusic {
    http: YmusicHttp,
    permissions: Option<EffectivePermissions>,
}

impl Ymusic {
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope, config_path) = effective_config()?;
        let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
        let creds = load_credentials(&scope)?;
        let http = YmusicHttp::with_store(
            creds.client_id,
            creds.client_secret,
            creds.refresh_token,
            scopes,
            config_path,
            creds.refresh_token_store,
        );
        let permissions = perms::load_effective().ok();
        Ok(Self { http, permissions })
    }

    pub fn with_credentials(
        creds: YmusicCredentials,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        let http = YmusicHttp::with_store(
            creds.client_id,
            creds.client_secret,
            creds.refresh_token,
            scopes,
            config_path,
            creds.refresh_token_store,
        );
        Self {
            http,
            permissions: None,
        }
    }

    pub fn with_paths(
        creds: YmusicCredentials,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        global_permissions: Option<&Path>,
        local_permissions: Option<&Path>,
    ) -> Result<Self> {
        let http = YmusicHttp::with_store(
            creds.client_id,
            creds.client_secret,
            creds.refresh_token,
            scopes,
            config_path,
            creds.refresh_token_store,
        );
        let permissions = perms::load_from(global_permissions, local_permissions)?;
        let permissions = if permissions.any() {
            Some(permissions)
        } else {
            None
        };
        Ok(Self { http, permissions })
    }

    pub fn with_permissions(mut self, permissions: EffectivePermissions) -> Self {
        self.permissions = Some(permissions);
        self
    }

    pub async fn search(&self, req: SearchRequest) -> Result<Vec<SearchItem>> {
        if let Some(p) = &self.permissions {
            p.check_time(YmusicFunction::Search)?;
            p.check_body(YmusicFunction::Search, &req.query)?;
        }
        let kinds: Vec<&str> = req.types.iter().map(String::as_str).collect();
        self.http.search(&req.query, &kinds, req.limit).await
    }

    pub async fn playlists(&self, req: PlaylistsRequest) -> Result<Vec<PlaylistSummary>> {
        if let Some(p) = &self.permissions {
            p.check_time(YmusicFunction::PlaylistsRead)?;
        }
        self.http.list_my_playlists(req.limit).await
    }

    pub async fn create_playlist(&self, req: CreatePlaylistRequest) -> Result<PlaylistSummary> {
        if let Some(p) = &self.permissions {
            p.check_time(YmusicFunction::PlaylistsWrite)?;
            p.check_target(YmusicFunction::PlaylistsWrite, &req.title)?;
            p.check_body(YmusicFunction::PlaylistsWrite, &req.title)?;
        }
        self.http
            .create_playlist(&req.title, req.description.as_deref(), req.privacy)
            .await
    }

    pub async fn add_playlist_item(&self, req: AddPlaylistItemRequest) -> Result<String> {
        if let Some(p) = &self.permissions {
            p.check_time(YmusicFunction::PlaylistsWrite)?;
            p.check_target(YmusicFunction::PlaylistsWrite, &req.playlist_id)?;
        }
        self.http
            .add_playlist_item(&req.playlist_id, &req.video_id)
            .await
    }

    pub async fn liked(&self, req: LikedRequest) -> Result<Vec<VideoSummary>> {
        if let Some(p) = &self.permissions {
            p.check_time(YmusicFunction::LibraryRead)?;
        }
        self.http.list_liked_videos(req.limit).await
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub types: Vec<String>,
    pub limit: u32,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>, types: Vec<String>, limit: u32) -> Result<Self> {
        let query = query.into();
        if query.is_empty() {
            return Err(ZadError::Invalid("query must not be empty".into()));
        }
        if types.is_empty() {
            return Err(ZadError::Invalid(
                "at least one type required (video, playlist, channel)".into(),
            ));
        }
        if !(1..=50).contains(&limit) {
            return Err(ZadError::Invalid(format!(
                "limit must be between 1 and 50 (YouTube Data API maximum); got {limit}"
            )));
        }
        Ok(Self {
            query,
            types,
            limit,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistsRequest {
    pub limit: u32,
}

impl PlaylistsRequest {
    pub fn new(limit: u32) -> Result<Self> {
        if !(1..=50).contains(&limit) {
            return Err(ZadError::Invalid(format!(
                "limit must be between 1 and 50; got {limit}"
            )));
        }
        Ok(Self { limit })
    }
}

#[derive(Debug, Clone)]
pub struct CreatePlaylistRequest {
    pub title: String,
    pub description: Option<String>,
    pub privacy: Privacy,
}

impl CreatePlaylistRequest {
    pub fn new(
        title: impl Into<String>,
        description: Option<String>,
        privacy: Privacy,
    ) -> Result<Self> {
        let title = title.into();
        if title.is_empty() {
            return Err(ZadError::Invalid("playlist title must not be empty".into()));
        }
        Ok(Self {
            title,
            description,
            privacy,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AddPlaylistItemRequest {
    pub playlist_id: String,
    pub video_id: String,
}

impl AddPlaylistItemRequest {
    pub fn new(playlist_id: impl Into<String>, video_id: impl Into<String>) -> Result<Self> {
        let playlist_id = playlist_id.into();
        let video_id = video_id.into();
        if playlist_id.is_empty() {
            return Err(ZadError::Invalid("playlist_id must not be empty".into()));
        }
        if video_id.is_empty() {
            return Err(ZadError::Invalid("video_id must not be empty".into()));
        }
        Ok(Self {
            playlist_id,
            video_id,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LikedRequest {
    pub limit: u32,
}

impl LikedRequest {
    pub fn new(limit: u32) -> Result<Self> {
        if !(1..=50).contains(&limit) {
            return Err(ZadError::Invalid(format!(
                "limit must be between 1 and 50; got {limit}"
            )));
        }
        Ok(Self { limit })
    }
}

// ---------------------------------------------------------------------------
// Config / credential plumbing
// ---------------------------------------------------------------------------

fn effective_config() -> Result<(YmusicServiceCfg, Scope<'static>, PathBuf)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("ymusic") {
        return Err(ZadError::Invalid(format!(
            "ymusic is not enabled for this project ({}). \
             Run `zad service enable ymusic` first.",
            project_path.display()
        )));
    }
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "ymusic")?;
    if let Some(cfg) = config::load_flat::<YmusicServiceCfg>(&local_path)? {
        let leaked: &'static str = Box::leak(slug.into_boxed_str());
        return Ok((cfg, Scope::Project(leaked), local_path));
    }
    let global_path = config::path::global_service_config_path("ymusic")?;
    if let Some(cfg) = config::load_flat::<YmusicServiceCfg>(&global_path)? {
        return Ok((cfg, Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no YouTube Music credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_credentials(scope: &Scope<'_>) -> Result<YmusicCredentials> {
    let client_id = secrets::load(&secrets::account("ymusic", "client-id", scope.clone()))?.ok_or(
        ZadError::Service {
            name: "ymusic",
            message: "client-id missing from keychain; re-run `zad service create ymusic`".into(),
        },
    )?;
    let client_secret = secrets::load(&secrets::account("ymusic", "client-secret", scope.clone()))?
        .ok_or(ZadError::Service {
            name: "ymusic",
            message: "client-secret missing from keychain; re-run `zad service create ymusic`"
                .into(),
        })?;
    let refresh_account = secrets::account("ymusic", "refresh", scope.clone());
    let refresh_token = secrets::load(&refresh_account)?.ok_or(ZadError::Service {
        name: "ymusic",
        message: "refresh token missing from keychain; re-run `zad service create ymusic`".into(),
    })?;
    let refresh_token_store: Option<Arc<dyn RefreshTokenStore>> =
        Some(Arc::new(KeychainRefreshStore::new(refresh_account)));
    Ok(YmusicCredentials {
        client_id,
        client_secret,
        refresh_token,
        refresh_token_store,
    })
}
