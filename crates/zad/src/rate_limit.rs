//! Cross-process rate-limit state, shared by every service.
//!
//! When a service returns HTTP 429, the client parses `Retry-After`,
//! records a per-service deadline on disk, and returns
//! [`ZadError::RateLimited`]. The next invocation — whether from the
//! same process, a follow-up CLI run, or a sibling library caller —
//! consults this state *before* issuing any network call. That way
//! quota is not burned on a doomed request.
//!
//! The CLI's `--wait` flag turns the pre-call check from "fail fast"
//! into "sleep until the deadline, then proceed". `--wait` on a clean
//! state is a no-op, so it is always safe to add.
//!
//! State file: `~/.zad/state/<service>/rate_limit.json`. The directory
//! is created on demand; the file is opportunistically removed once
//! the deadline has passed.

use std::path::PathBuf;
use std::time::Duration;

use jiff::Timestamp;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::config::path::zad_home;
use crate::error::{Result, ZadError};

/// Hard cap on `--wait` sleep duration. Servers occasionally hand out
/// pathological `Retry-After` values; refusing to sleep more than an
/// hour keeps scripts predictable without forcing callers to invent
/// their own timeouts.
pub const MAX_WAIT_SECONDS: u64 = 3_600;

/// Default fallback when a 429 arrives with no `Retry-After` header.
/// Five seconds is the smallest interval most public APIs ever ask
/// for and is short enough that the user-facing wait is reasonable.
const DEFAULT_RETRY_AFTER_SECONDS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    /// Absolute RFC 3339 deadline.
    retry_after_utc: String,
}

/// Path to a service's rate-limit state file.
pub fn state_path(service: &str) -> Result<PathBuf> {
    Ok(zad_home()?
        .join("state")
        .join(service)
        .join("rate_limit.json"))
}

/// Read the persisted deadline for `service`, if any. Returns `None`
/// when no state file exists, when the file is unparseable (treated
/// as absent rather than fatal — a corrupt cache should never block a
/// fresh call), or when the deadline is already in the past (the file
/// is opportunistically removed in that case).
pub fn read_deadline(service: &str) -> Option<Timestamp> {
    let path = state_path(service).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    let state: PersistedState = serde_json::from_slice(&bytes).ok()?;
    let ts: Timestamp = state.retry_after_utc.parse().ok()?;
    if ts <= Timestamp::now() {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(ts)
}

/// Persist a deadline for `service`. The parent directory is created
/// on demand. I/O errors are propagated so the caller can decide
/// whether to surface them; in practice the 429 path already returns
/// an error, so any write failure here is logged via `tracing` by the
/// caller and otherwise non-fatal.
pub fn write_deadline(service: &str, deadline: Timestamp) -> Result<()> {
    let path = state_path(service)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let state = PersistedState {
        retry_after_utc: deadline.to_string(),
    };
    let json = serde_json::to_vec(&state).map_err(|e| ZadError::Io {
        path: path.clone(),
        source: std::io::Error::other(e),
    })?;
    std::fs::write(&path, json).map_err(|e| ZadError::Io {
        path: path.clone(),
        source: e,
    })?;
    Ok(())
}

/// Remove any persisted deadline for `service`. Called after a
/// successful request to ensure stale state never lingers.
pub fn clear(service: &str) {
    if let Ok(path) = state_path(service) {
        let _ = std::fs::remove_file(path);
    }
}

/// Parse a `Retry-After` header value, which may be either a
/// non-negative integer of delta-seconds or an HTTP-date. Falls back
/// to [`DEFAULT_RETRY_AFTER_SECONDS`] if the header is absent or
/// unparseable so callers always get a usable deadline.
pub fn parse_retry_after(headers: &HeaderMap) -> Duration {
    let raw = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(value) = raw else {
        return Duration::from_secs(DEFAULT_RETRY_AFTER_SECONDS);
    };
    if let Ok(secs) = value.parse::<u64>() {
        return Duration::from_secs(secs);
    }
    if let Ok(ts) = parse_http_date(value) {
        let delta = ts.as_second().saturating_sub(Timestamp::now().as_second());
        if delta > 0 {
            return Duration::from_secs(delta as u64);
        }
        return Duration::from_secs(0);
    }
    Duration::from_secs(DEFAULT_RETRY_AFTER_SECONDS)
}

/// Parse an HTTP-date (RFC 7231 §7.1.1.1). Only the preferred IMF-fixdate
/// form (`Sun, 06 Nov 1994 08:49:37 GMT`) is required by the spec and is
/// what every major service uses; we accept that and reject anything
/// else, falling back to the default seconds in the caller.
fn parse_http_date(s: &str) -> std::result::Result<Timestamp, jiff::Error> {
    // jiff understands RFC 2822, which is a superset of IMF-fixdate.
    let zoned = jiff::fmt::rfc2822::parse(s)?;
    Ok(zoned.timestamp())
}

/// Convert a `Retry-After` duration into an absolute deadline.
pub fn deadline_from(duration: Duration) -> Timestamp {
    let now = Timestamp::now();
    let secs = duration.as_secs().min(MAX_WAIT_SECONDS) as i64;
    now.checked_add(jiff::Span::new().seconds(secs))
        .unwrap_or(now)
}

/// Build a [`ZadError::RateLimited`] from an absolute deadline.
pub fn rate_limited_error(service: &'static str, deadline: Timestamp) -> ZadError {
    let now = Timestamp::now();
    let secs = deadline.as_second().saturating_sub(now.as_second()).max(0) as u64;
    ZadError::RateLimited {
        service,
        retry_after_seconds: secs,
        retry_after_utc: deadline.to_string(),
    }
}

/// Pre-call gate: consult persisted state. If we are still inside a
/// wait window:
/// - with `wait = true`, sleep until the deadline and return `Ok(())`.
/// - with `wait = false`, return `Err(ZadError::RateLimited { .. })`
///   so the caller can fail fast without spending a request.
///
/// When no deadline is recorded (or it has already passed), this is a
/// no-op regardless of `wait`. That keeps `--wait` safe to leave on
/// permanently in scripts.
pub async fn precall_check(service: &'static str, wait: bool) -> Result<()> {
    let Some(deadline) = read_deadline(service) else {
        return Ok(());
    };
    if !wait {
        return Err(rate_limited_error(service, deadline));
    }
    let now = Timestamp::now();
    let secs = deadline.as_second().saturating_sub(now.as_second()).max(0) as u64;
    let capped = secs.min(MAX_WAIT_SECONDS);
    tracing::info!(
        service = service,
        wait_seconds = capped,
        "rate-limit wait window active; sleeping"
    );
    tokio::time::sleep(Duration::from_secs(capped)).await;
    clear(service);
    Ok(())
}

/// Convert a 429 response (already detected by the caller) into a
/// [`ZadError::RateLimited`] and persist the deadline so subsequent
/// calls hit the pre-call gate. Centralized so every service follows
/// the same rules.
pub fn handle_429(service: &'static str, headers: &HeaderMap) -> ZadError {
    let duration = parse_retry_after(headers);
    let deadline = deadline_from(duration);
    if let Err(e) = write_deadline(service, deadline) {
        tracing::warn!(service = service, error = %e, "failed to persist rate-limit state");
    }
    rate_limited_error(service, deadline)
}

/// If `resp` is a 429, persist the deadline and return a
/// [`ZadError::RateLimited`]; otherwise return `None`. Call this
/// **before** consuming the response body so the `Retry-After`
/// header is still available.
///
/// Inspecting the status and headers here does not drop `resp`; the
/// caller continues with `.text()` / `.json()` for non-429 statuses.
pub fn check_response(service: &'static str, resp: &reqwest::Response) -> Option<ZadError> {
    if resp.status().as_u16() == 429 {
        Some(handle_429(service, resp.headers()))
    } else {
        None
    }
}
