//! Typed library facade for Spotify. OAuth PKCE — public client; no
//! `client_secret`. Same shape as the other facades: three
//! constructors (`from_default_config`, `with_credentials`,
//! `with_paths`), validating `*Request` types per verb, automatic
//! permission enforcement.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::{self, SpotifyServiceCfg};
use crate::error::{Result, ZadError};
use crate::oauth::{KeychainRefreshStore, RefreshTokenStore};
use crate::secrets::{self, Scope};
use crate::service::spotify::client::{PlaylistSummary, SavedTrack, SearchResults, SpotifyHttp};
use crate::service::spotify::permissions::{self as perms, EffectivePermissions, SpotifyFunction};

/// Public-client OAuth PKCE credentials for Spotify. Only a
/// `client_id` and a long-lived `refresh_token` — Spotify's PKCE flow
/// doesn't issue or accept a `client_secret`.
///
/// Spotify rotates the refresh token on every `/api/token` call;
/// `refresh_token_store` decides where the rotated value gets
/// persisted. `Spotify::from_default_config` wires this to a
/// [`KeychainRefreshStore`] pointing at the canonical zad slot. Set
/// it to `None` only if you intend to manage rotation yourself.
#[derive(Clone)]
pub struct SpotifyCredentials {
    pub client_id: String,
    pub refresh_token: String,
    pub refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
}

impl std::fmt::Debug for SpotifyCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotifyCredentials")
            .field("client_id", &self.client_id)
            .field("refresh_token", &"<redacted>")
            .field(
                "refresh_token_store",
                &self.refresh_token_store.as_ref().map(|_| "<store>"),
            )
            .finish()
    }
}

impl SpotifyCredentials {
    /// Build credentials with no rotation store. Library callers that
    /// want zad to keep the OS keychain in sync should use
    /// [`Self::with_keychain_store`] instead.
    pub fn new(client_id: impl Into<String>, refresh_token: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            refresh_token: refresh_token.into(),
            refresh_token_store: None,
        }
    }

    /// Convenience constructor wiring the canonical zad
    /// [`KeychainRefreshStore`] (Global scope). Equivalent to what
    /// [`Spotify::from_default_config`] does internally.
    pub fn with_keychain_store(
        client_id: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Self {
        let store =
            KeychainRefreshStore::new(secrets::account("spotify", "refresh", Scope::Global));
        Self {
            client_id: client_id.into(),
            refresh_token: refresh_token.into(),
            refresh_token_store: Some(Arc::new(store)),
        }
    }
}

/// Typed library entry point for Spotify.
pub struct Spotify {
    http: SpotifyHttp,
    permissions: Option<EffectivePermissions>,
}

impl Spotify {
    /// CLI-equivalent: project-or-global config + OAuth client_id +
    /// refresh_token from keychain + permissions from default paths.
    /// **Honors `ZAD_HOME_OVERRIDE` and friends.**
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope, config_path) = effective_config()?;
        let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
        let creds = load_credentials(&scope)?;
        let http = SpotifyHttp::with_store(
            creds.client_id,
            creds.refresh_token,
            scopes,
            config_path,
            creds.refresh_token_store,
        );
        let permissions = perms::load_effective().ok();
        Ok(Self { http, permissions })
    }

    /// Explicit OAuth credentials + scopes + config path. Reads no
    /// env vars; no on-disk permission enforcement.
    pub fn with_credentials(
        creds: SpotifyCredentials,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        let http = SpotifyHttp::with_store(
            creds.client_id,
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

    /// Fully explicit, env-free constructor. Recommended for library code.
    pub fn with_paths(
        creds: SpotifyCredentials,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        global_permissions: Option<&Path>,
        local_permissions: Option<&Path>,
    ) -> Result<Self> {
        let http = SpotifyHttp::with_store(
            creds.client_id,
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

    pub async fn search(&self, req: SearchRequest) -> Result<SearchResults> {
        if let Some(p) = &self.permissions {
            p.check_time(SpotifyFunction::Search)?;
            p.check_body(SpotifyFunction::Search, &req.query)?;
        }
        let kinds: Vec<&str> = req.types.iter().map(String::as_str).collect();
        self.http.search(&req.query, &kinds, req.limit).await
    }

    pub async fn playlists(&self, req: PlaylistsRequest) -> Result<Vec<PlaylistSummary>> {
        if let Some(p) = &self.permissions {
            p.check_time(SpotifyFunction::PlaylistsRead)?;
        }
        self.http.list_my_playlists(req.limit).await
    }

    pub async fn create_playlist(&self, req: CreatePlaylistRequest) -> Result<PlaylistSummary> {
        if let Some(p) = &self.permissions {
            p.check_time(SpotifyFunction::PlaylistsWrite)?;
            p.check_target(SpotifyFunction::PlaylistsWrite, &req.name)?;
            p.check_body(SpotifyFunction::PlaylistsWrite, &req.name)?;
        }
        self.http
            .create_playlist(&req.name, req.description.as_deref(), req.public)
            .await
    }

    pub async fn saved_tracks(&self, req: SavedTracksRequest) -> Result<Vec<SavedTrack>> {
        if let Some(p) = &self.permissions {
            p.check_time(SpotifyFunction::LibraryRead)?;
        }
        self.http.list_saved_tracks(req.limit).await
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
    /// Spotify caps `/search`'s `limit` at 10 (tightened from the
    /// previously documented 50; values > 10 now return HTTP 400
    /// "Invalid limit"). Validates non-empty query and at least one
    /// item type (`track`, `artist`, `album`, `playlist`).
    pub fn new(query: impl Into<String>, types: Vec<String>, limit: u32) -> Result<Self> {
        let query = query.into();
        if query.is_empty() {
            return Err(ZadError::Invalid("query must not be empty".into()));
        }
        if types.is_empty() {
            return Err(ZadError::Invalid(
                "at least one type required (track, artist, album, playlist)".into(),
            ));
        }
        if !(1..=10).contains(&limit) {
            return Err(ZadError::Invalid(format!(
                "limit must be between 1 and 10 (Spotify /search maximum); got {limit}"
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
    pub name: String,
    pub description: Option<String>,
    pub public: bool,
}

impl CreatePlaylistRequest {
    pub fn new(name: impl Into<String>, description: Option<String>, public: bool) -> Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(ZadError::Invalid("playlist name must not be empty".into()));
        }
        Ok(Self {
            name,
            description,
            public,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SavedTracksRequest {
    pub limit: u32,
}

impl SavedTracksRequest {
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

fn effective_config() -> Result<(SpotifyServiceCfg, Scope<'static>, PathBuf)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("spotify") {
        return Err(ZadError::Invalid(format!(
            "spotify is not enabled for this project ({}). \
             Run `zad service enable spotify` first.",
            project_path.display()
        )));
    }
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "spotify")?;
    if let Some(cfg) = config::load_flat::<SpotifyServiceCfg>(&local_path)? {
        let leaked: &'static str = Box::leak(slug.into_boxed_str());
        return Ok((cfg, Scope::Project(leaked), local_path));
    }
    let global_path = config::path::global_service_config_path("spotify")?;
    if let Some(cfg) = config::load_flat::<SpotifyServiceCfg>(&global_path)? {
        return Ok((cfg, Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no Spotify credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_credentials(scope: &Scope<'_>) -> Result<SpotifyCredentials> {
    let client_id = secrets::load(&secrets::account("spotify", "client-id", scope.clone()))?
        .ok_or(ZadError::Service {
            name: "spotify",
            message: "client-id missing from keychain; re-run `zad service create spotify`".into(),
        })?;
    let refresh_account = secrets::account("spotify", "refresh", scope.clone());
    let refresh_token = secrets::load(&refresh_account)?.ok_or(ZadError::Service {
        name: "spotify",
        message: "refresh token missing from keychain; re-run `zad service create spotify`".into(),
    })?;
    // Default-config consumers always want rotated tokens persisted
    // back into the same keychain slot they were loaded from. Library
    // users with custom storage go through `with_credentials` /
    // `with_paths` and supply their own `refresh_token_store`.
    let refresh_token_store: Option<Arc<dyn RefreshTokenStore>> =
        Some(Arc::new(KeychainRefreshStore::new(refresh_account)));
    Ok(SpotifyCredentials {
        client_id,
        refresh_token,
        refresh_token_store,
    })
}
