//! Google Calendar HTTP client.
//!
//! Hand-rolled `reqwest` wrapper over Calendar API v3 plus the OpenID
//! Connect userinfo endpoint. We deliberately avoid `google-calendar3`
//! and `yup-oauth2` — those crates drag a large, code-generation-heavy
//! dep tree, and the endpoint set zad exercises is small enough to
//! hand-roll cleanly.
//!
//! ## Access-token lifecycle
//!
//! [`GcalHttp`] always holds a *refresh* token and its OAuth client
//! identity. The first API call in the process lifetime checks the
//! cross-process access-token cache, and if it misses, acquires the
//! cross-process refresh lock and calls
//! [`crate::oauth::refresh_access_token`]. The fresh token is written
//! to both the in-memory cache and the on-disk cache so sibling
//! processes pick it up without hitting the token endpoint or the OS
//! keychain. See [`crate::token_cache`] for the rationale.
//!
//! Google's confidential-client flow does not currently rotate
//! refresh tokens, but the persist-on-rotation handling is wired up
//! the same way as Spotify/YMusic so a future provider change (or a
//! per-account quirk) doesn't silently revoke the user's session.
//! When the value is unchanged, the store is never called.
//!
//! ## Error mapping
//!
//! Every non-2xx HTTP status surfaces as `ZadError::Service { name:
//! "gcal", message }`. Two cases are specialised:
//!
//! - `401` with `invalid_credentials` → "credentials revoked; re-run
//!   `zad service create gcal`"
//! - `429` and Google's `rateLimitExceeded` body → "Google Calendar
//!   rate-limited this client; back off before retrying"

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Result, ZadError};
use crate::oauth::{self, RefreshTokenStore};
use crate::rate_limit;
use crate::service::gcal::{API_BASE, TOKEN_URL, USERINFO_URL};
use crate::token_cache;

const SERVICE: &str = "gcal";

/// Thin wrapper over Google Calendar API v3. Holds a refresh token
/// and mints an access token on demand.
#[derive(Clone)]
pub struct GcalHttp {
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
    /// [`crate::service::gcal::TOKEN_URL`]; overridable so tests can
    /// point at a localhost mock.
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
}

impl GcalHttp {
    /// Full-featured constructor used by runtime verbs. No persisting
    /// store — equivalent to [`Self::with_store`] with `None`. Use
    /// [`Self::with_store`] in production so a future provider change
    /// that does start rotating refresh tokens never silently revokes
    /// the user's session.
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
    /// wiring (`Gcal::from_default_config`) supplies a
    /// [`crate::oauth::KeychainRefreshStore`] pointing at the
    /// `secrets::account("gcal", "refresh", Scope::Global)` slot.
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
        }
    }

    /// Scopeless client used by lifecycle flows (`validate`, `status
    /// check`) that pre-date scope persistence.
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
    /// [`crate::service::gcal::TOKEN_URL`].
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
            service: "gcal",
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
            "gcal",
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

    /// GET `/users/me/calendarList`. Scope: `calendars.read`.
    pub async fn list_calendars(&self) -> Result<Vec<CalendarEntry>> {
        self.require_scope("calendars.read")?;
        let list: CalendarList = self
            .get_json("/users/me/calendarList", &[("maxResults", "250")])
            .await?;
        Ok(list.items)
    }

    /// GET `/calendars/{calendarId}`. Scope: `calendars.read`.
    pub async fn get_calendar(&self, calendar_id: &str) -> Result<Calendar> {
        self.require_scope("calendars.read")?;
        let path = format!("/calendars/{}", urlencode_path(calendar_id));
        self.get_json(&path, &[]).await
    }

    /// GET `/calendars/{calendarId}/events`. Scope: `events.read`.
    pub async fn list_events(
        &self,
        calendar_id: &str,
        params: &EventsListParams,
    ) -> Result<Vec<Event>> {
        self.require_scope("events.read")?;
        let path = format!("/calendars/{}/events", urlencode_path(calendar_id));
        let mut query: Vec<(&str, String)> = vec![("singleEvents", "true".into())];
        if let Some(t) = &params.time_min {
            query.push(("timeMin", t.clone()));
        }
        if let Some(t) = &params.time_max {
            query.push(("timeMax", t.clone()));
        }
        if let Some(q) = &params.query {
            query.push(("q", q.clone()));
        }
        if let Some(n) = params.max_results {
            query.push(("maxResults", n.to_string()));
        }
        let list: EventList = self
            .get_json(
                &path,
                &query
                    .iter()
                    .map(|(k, v)| (*k, v.as_str()))
                    .collect::<Vec<_>>(),
            )
            .await?;
        Ok(list.items)
    }

    /// GET `/calendars/{calendarId}/events/{eventId}`. Scope: `events.read`.
    pub async fn get_event(&self, calendar_id: &str, event_id: &str) -> Result<Event> {
        self.require_scope("events.read")?;
        let path = format!(
            "/calendars/{}/events/{}",
            urlencode_path(calendar_id),
            urlencode_path(event_id)
        );
        self.get_json(&path, &[]).await
    }

    /// POST `/calendars/{calendarId}/events`. Scope: `events.write`.
    pub async fn create_event(
        &self,
        calendar_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event> {
        self.require_scope("events.write")?;
        let path = format!("/calendars/{}/events", urlencode_path(calendar_id));
        let mut query: Vec<(&str, String)> = vec![];
        if let Some(s) = send_updates {
            query.push(("sendUpdates", s.into()));
        }
        self.post_json(
            &path,
            &query
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>(),
            body,
        )
        .await
    }

    /// PATCH `/calendars/{calendarId}/events/{eventId}`. Scope: `events.write`.
    pub async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        body: &serde_json::Value,
        send_updates: Option<&str>,
    ) -> Result<Event> {
        self.require_scope("events.write")?;
        let path = format!(
            "/calendars/{}/events/{}",
            urlencode_path(calendar_id),
            urlencode_path(event_id)
        );
        let mut query: Vec<(&str, String)> = vec![];
        if let Some(s) = send_updates {
            query.push(("sendUpdates", s.into()));
        }
        self.patch_json(
            &path,
            &query
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>(),
            body,
        )
        .await
    }

    /// DELETE `/calendars/{calendarId}/events/{eventId}`. Scope: `events.write`.
    pub async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        send_updates: Option<&str>,
    ) -> Result<()> {
        self.require_scope("events.write")?;
        let path = format!(
            "/calendars/{}/events/{}",
            urlencode_path(calendar_id),
            urlencode_path(event_id)
        );
        let mut query: Vec<(&str, String)> = vec![];
        if let Some(s) = send_updates {
            query.push(("sendUpdates", s.into()));
        }
        self.delete_empty(
            &path,
            &query
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>(),
        )
        .await
    }

    // -----------------------------------------------------------------
    // unscoped — called from lifecycle (pre-scopes)
    // -----------------------------------------------------------------

    /// Probe the credentials: fetch the user's primary email via the
    /// OpenID Connect `userinfo` endpoint. Used by `validate` during
    /// `zad service create gcal` and by `service status`.
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

    /// Fetch one page of calendarList (up to 1 entry) as a cheap
    /// "credentials still valid?" probe.
    pub async fn probe_calendar_list(&self) -> Result<usize> {
        let list: CalendarList = self
            .get_json("/users/me/calendarList", &[("maxResults", "1")])
            .await?;
        Ok(list.items.len())
    }

    // -----------------------------------------------------------------
    // low-level HTTP glue
    // -----------------------------------------------------------------

    /// Resolve the cache directory, preferring the override field and
    /// falling back to the standard `zad_home`-based path.
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
            .get(format!("{API_BASE}{path}"))
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
            .post(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp, cache_dir.as_deref()).await
    }

    async fn patch_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<T> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .patch(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(network_err)?;
        decode_response(resp, cache_dir.as_deref()).await
    }

    async fn delete_empty(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        let access = self.access_token().await?;
        let cache_dir = self.resolved_cache_dir();
        let resp = reqwest::Client::new()
            .delete(format!("{API_BASE}{path}"))
            .bearer_auth(&access)
            .query(query)
            .send()
            .await
            .map_err(network_err)?;
        finalize_empty(resp, cache_dir.as_deref()).await
    }
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

fn network_err(e: reqwest::Error) -> ZadError {
    ZadError::Service {
        name: "gcal",
        message: format!("network error talking to Google Calendar: {e}"),
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
        name: "gcal",
        message: format!("failed to decode Google Calendar response: {e}"),
    })
}

fn map_http_error(status: reqwest::StatusCode, body: &str) -> ZadError {
    let code = status.as_u16();
    let lower = body.to_ascii_lowercase();
    let message = if code == 401 || lower.contains("invalid_credentials") {
        format!(
            "Google Calendar rejected the access token (HTTP {code}); \
             the credentials may have been revoked. Re-run `zad service create gcal` to re-authorize. \
             Body: {body}"
        )
    } else if code == 429
        || lower.contains("ratelimitexceeded")
        || lower.contains("userratelimitexceeded")
    {
        format!(
            "Google Calendar rate-limited this client (HTTP {code}); \
             back off before retrying. Body: {body}"
        )
    } else {
        format!("HTTP {code}: {body}")
    };
    ZadError::Service {
        name: "gcal",
        message,
    }
}

// ---------------------------------------------------------------------------
// Percent-encode *path* components (leave `/` intact would break the call;
// we're encoding a single segment each time).
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
// Response types — kept minimal; we only project the fields the CLI
// surfaces or uses for policy decisions.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CalendarList {
    #[serde(default)]
    pub items: Vec<CalendarEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalendarEntry {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, rename = "timeZone")]
    pub time_zone: Option<String>,
    #[serde(default, rename = "accessRole")]
    pub access_role: Option<String>,
    #[serde(default)]
    pub primary: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Calendar {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, rename = "timeZone")]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EventsListParams {
    pub time_min: Option<String>,
    pub time_max: Option<String>,
    pub query: Option<String>,
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventList {
    #[serde(default)]
    pub items: Vec<Event>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Event {
    pub id: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub start: Option<EventDateTime>,
    #[serde(default)]
    pub end: Option<EventDateTime>,
    #[serde(default)]
    pub attendees: Option<Vec<EventAttendee>>,
    #[serde(default, rename = "htmlLink")]
    pub html_link: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventDateTime {
    #[serde(default, rename = "dateTime")]
    pub date_time: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default, rename = "timeZone")]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventAttendee {
    pub email: String,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "responseStatus")]
    pub response_status: Option<String>,
    #[serde(default)]
    pub optional: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "email_verified")]
    pub email_verified: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sub: Option<String>,
}
