//! Typed library facade for Google Calendar.
//!
//! Same shape as `service::discord::facade` but adapted for OAuth
//! authentication. Three constructors:
//! - [`Gcal::from_default_config`] — CLI-equivalent; reads default
//!   paths and the keychain.
//! - [`Gcal::with_credentials`] — explicit OAuth credentials, no env
//!   reads, no permission enforcement.
//! - [`Gcal::with_paths`] — fully explicit OAuth credentials plus
//!   explicit `permissions.toml` paths. Recommended for library code.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::{self, GcalServiceCfg};
use crate::error::{Result, ZadError};
use crate::oauth::{KeychainRefreshStore, RefreshTokenStore};
use crate::secrets::{self, Scope};
use crate::service::gcal::client::{CalendarEntry, Event, EventsListParams, GcalHttp};
use crate::service::gcal::permissions::{self as perms, EffectivePermissions, GcalFunction};

/// OAuth 2.0 credentials for Google Calendar. Use one of these — the
/// library never reads them from the OS keychain on its own; the
/// caller hands them in.
///
/// `refresh_token_store` is wired by `Gcal::from_default_config` to
/// the canonical zad keychain slot; library users with custom storage
/// can supply their own [`RefreshTokenStore`] impl. Google does not
/// currently rotate refresh tokens, but the field is kept symmetric
/// with `SpotifyCredentials`/`YmusicCredentials` so the bug Spotify hit
/// can't recur silently here.
#[derive(Clone)]
pub struct GcalCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub refresh_token_store: Option<Arc<dyn RefreshTokenStore>>,
}

impl std::fmt::Debug for GcalCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcalCredentials")
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

impl GcalCredentials {
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
        let store = KeychainRefreshStore::new(secrets::account("gcal", "refresh", Scope::Global));
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            refresh_token: refresh_token.into(),
            refresh_token_store: Some(Arc::new(store)),
        }
    }
}

/// Typed library entry point for Google Calendar.
pub struct Gcal {
    http: GcalHttp,
    permissions: Option<EffectivePermissions>,
}

impl Gcal {
    /// CLI-equivalent: load project-or-global config + OAuth
    /// client_id/client_secret/refresh_token from the keychain +
    /// effective permissions from default paths. **Honors
    /// `ZAD_HOME_OVERRIDE` and friends.**
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope, config_path) = effective_config()?;
        let scopes: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
        let creds = load_credentials(&scope)?;
        let http = GcalHttp::with_store(
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

    /// Explicit OAuth credentials + scope set + config path. Reads no
    /// env vars; no on-disk permission enforcement (layer back on with
    /// [`Gcal::with_permissions`]).
    pub fn with_credentials(
        creds: GcalCredentials,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
    ) -> Self {
        let http = GcalHttp::with_store(
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

    /// Fully explicit, env-free constructor. Recommended for library
    /// code. Pass `None` for both permission paths to skip on-disk
    /// policy enforcement.
    pub fn with_paths(
        creds: GcalCredentials,
        scopes: BTreeSet<String>,
        config_path: PathBuf,
        global_permissions: Option<&Path>,
        local_permissions: Option<&Path>,
    ) -> Result<Self> {
        let http = GcalHttp::with_store(
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

    /// List the user's calendars.
    pub async fn calendars(&self, _req: CalendarsRequest) -> Result<Vec<CalendarEntry>> {
        if let Some(p) = &self.permissions {
            p.check_time(GcalFunction::ListCalendars)?;
        }
        self.http.list_calendars().await
    }

    /// List events on a calendar.
    pub async fn events(&self, req: EventsRequest) -> Result<Vec<Event>> {
        if let Some(p) = &self.permissions {
            p.check_time(GcalFunction::ListEvents)?;
            p.check_calendar(GcalFunction::ListEvents, &req.calendar_id, &req.calendar_id)?;
        }
        self.http.list_events(&req.calendar_id, &req.params).await
    }

    /// Create an event on a calendar.
    pub async fn create_event(&self, req: CreateEventRequest) -> Result<Event> {
        if let Some(p) = &self.permissions {
            p.check_time(GcalFunction::CreateEvent)?;
            p.check_calendar(
                GcalFunction::CreateEvent,
                &req.calendar_id,
                &req.calendar_id,
            )?;
            if let Some(summary) = req.body.get("summary").and_then(|v| v.as_str()) {
                p.check_body(GcalFunction::CreateEvent, summary)?;
            }
        }
        self.http
            .create_event(&req.calendar_id, &req.body, req.send_updates.as_deref())
            .await
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CalendarsRequest;

impl CalendarsRequest {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone)]
pub struct EventsRequest {
    pub calendar_id: String,
    pub params: EventsListParams,
}

impl EventsRequest {
    pub fn new(calendar_id: impl Into<String>, params: EventsListParams) -> Result<Self> {
        let calendar_id = calendar_id.into();
        if calendar_id.is_empty() {
            return Err(ZadError::Invalid("calendar_id must not be empty".into()));
        }
        Ok(Self {
            calendar_id,
            params,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CreateEventRequest {
    pub calendar_id: String,
    pub body: serde_json::Value,
    pub send_updates: Option<String>,
}

impl CreateEventRequest {
    /// `body` should be a Google Calendar
    /// [Event resource](https://developers.google.com/calendar/api/v3/reference/events)
    /// JSON object — at minimum `summary`, `start`, `end`. Validation
    /// here is shallow (non-empty calendar id, non-null body); full
    /// schema validation is left to the API.
    pub fn new(
        calendar_id: impl Into<String>,
        body: serde_json::Value,
        send_updates: Option<String>,
    ) -> Result<Self> {
        let calendar_id = calendar_id.into();
        if calendar_id.is_empty() {
            return Err(ZadError::Invalid("calendar_id must not be empty".into()));
        }
        if !body.is_object() {
            return Err(ZadError::Invalid("event body must be a JSON object".into()));
        }
        if body.get("summary").is_none() && body.get("start").is_none() && body.get("end").is_none()
        {
            return Err(ZadError::Invalid(
                "event body must include at least one of summary/start/end".into(),
            ));
        }
        Ok(Self {
            calendar_id,
            body,
            send_updates,
        })
    }
}

// ---------------------------------------------------------------------------
// Config / credential plumbing — mirrors `cli/gcal.rs`.
// ---------------------------------------------------------------------------

fn effective_config() -> Result<(GcalServiceCfg, Scope<'static>, PathBuf)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("gcal") {
        return Err(ZadError::Invalid(format!(
            "gcal is not enabled for this project ({}). \
             Run `zad service enable gcal` first.",
            project_path.display()
        )));
    }
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "gcal")?;
    if let Some(cfg) = config::load_flat::<GcalServiceCfg>(&local_path)? {
        let leaked: &'static str = Box::leak(slug.into_boxed_str());
        return Ok((cfg, Scope::Project(leaked), local_path));
    }
    let global_path = config::path::global_service_config_path("gcal")?;
    if let Some(cfg) = config::load_flat::<GcalServiceCfg>(&global_path)? {
        return Ok((cfg, Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no gcal credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

fn load_credentials(scope: &Scope<'_>) -> Result<GcalCredentials> {
    let client_id = secrets::load(&secrets::account("gcal", "client-id", scope.clone()))?.ok_or(
        ZadError::Service {
            name: "gcal",
            message: "client-id missing from keychain; re-run `zad service create gcal`".into(),
        },
    )?;
    let client_secret = secrets::load(&secrets::account("gcal", "client-secret", scope.clone()))?
        .ok_or(ZadError::Service {
        name: "gcal",
        message: "client-secret missing from keychain; re-run `zad service create gcal`".into(),
    })?;
    let refresh_account = secrets::account("gcal", "refresh", scope.clone());
    let refresh_token = secrets::load(&refresh_account)?.ok_or(ZadError::Service {
        name: "gcal",
        message: "refresh token missing from keychain; re-run `zad service create gcal`".into(),
    })?;
    let refresh_token_store: Option<Arc<dyn RefreshTokenStore>> =
        Some(Arc::new(KeychainRefreshStore::new(refresh_account)));
    Ok(GcalCredentials {
        client_id,
        client_secret,
        refresh_token,
        refresh_token_store,
    })
}
