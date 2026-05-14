//! Google API rate-limit / quota classification, shared by the
//! Google-backed services (YouTube Music, Google Calendar).
//!
//! Google APIs don't follow the canonical "HTTP 429 + `Retry-After`"
//! pattern that Spotify, Discord, and Slack do. Instead, quota
//! exhaustion is signalled by HTTP **403** with a `reason` code in
//! the JSON error envelope, and the `Retry-After` header is rarely
//! sent. The reasons fall into two semantic buckets:
//!
//! - `quotaExceeded` / `dailyLimitExceeded` — per-project per-day
//!   budget, resets at midnight **Pacific Time** per the YouTube Data
//!   API quota docs.
//! - `rateLimitExceeded` / `userRateLimitExceeded` — short-term
//!   burst limit (roughly 60 requests per 100 seconds per user on
//!   YouTube Data API).
//!
//! Without classifying these correctly, parallel `zad ymusic`
//! invocations each burn another quota point on a doomed call —
//! every invalid YouTube call still costs at least 1 unit per the
//! Data API quota calculator. By promoting these 403s to the same
//! [`ZadError::RateLimited`] / persisted-deadline state machine
//! that 429-shaped providers use, the next process consults
//! `~/.zad/state/<service>/rate_limit.json` *before* sending and
//! fails fast (or, with `--wait`, blocks until the window passes).
//!
//! The generic state-machine primitives live in [`crate::rate_limit`];
//! this module is the Google-specific classifier layered on top.

use std::time::Duration;

use jiff::Timestamp;
use reqwest::header::HeaderMap;

use crate::error::ZadError;
use crate::rate_limit::{
    MAX_DAILY_WAIT_SECONDS, MAX_WAIT_SECONDS, deadline_from_max, parse_retry_after,
    rate_limited_error, write_deadline,
};

/// Default fallback for Google's short-term `rateLimitExceeded` /
/// `userRateLimitExceeded` 403s. The YouTube Data API's per-user
/// budget is roughly 60 requests per 100 seconds, so sleeping for a
/// minute clears the typical sliding window without making the wait
/// painful in scripts. Honor the provider's `Retry-After` first when
/// it's present.
const DEFAULT_SHORT_TERM_SECONDS: u64 = 60;

/// Kind of quota a Google API hit. Determines which deadline strategy
/// to use when persisting the rate-limit window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCategory {
    /// Per-user / per-100-seconds style burst limit. Recovers in tens
    /// of seconds. Honors `Retry-After` when present, otherwise falls
    /// back to a 60-second deadline.
    ShortTerm,
    /// Per-project per-day quota. Recovers at the daily reset
    /// boundary (midnight Pacific Time for YouTube). Deadlines are
    /// capped at [`MAX_DAILY_WAIT_SECONDS`] in case the timezone
    /// computation produces an absurd value.
    Daily,
}

/// Inspect a Google API JSON error body and classify it as a
/// rate-limit error if it carries one of the well-known reason codes.
/// Returns `None` if the body isn't a Google quota error.
///
/// Most Google APIs (including the YouTube Data API v3) wrap errors
/// in the shape:
///
/// ```json
/// { "error": { "code": 403, "message": "...",
///              "errors": [ { "reason": "quotaExceeded", ... } ] } }
/// ```
///
/// We match on `reason` via lowercased substring rather than parsing
/// the JSON: the body wording is localised and may change, and
/// substring matching also catches the gRPC-mapped variants Google
/// occasionally emits.
pub fn classify(body: &str) -> Option<QuotaCategory> {
    let lower = body.to_ascii_lowercase();
    // Daily reasons first — `dailyLimitExceeded` is a subset of
    // `quotaExceeded` semantics but Google sometimes emits the more
    // specific form. Either way: the quota resets at the daily
    // boundary, not in seconds.
    if lower.contains("dailylimitexceeded") || lower.contains("quotaexceeded") {
        return Some(QuotaCategory::Daily);
    }
    if lower.contains("userratelimitexceeded") || lower.contains("ratelimitexceeded") {
        return Some(QuotaCategory::ShortTerm);
    }
    None
}

/// Compute the next midnight in `America/Los_Angeles` (Pacific Time),
/// where the YouTube Data API resets its per-project daily quota.
///
/// Uses jiff's tz database so daylight-saving transitions are handled
/// correctly. On the (very rare) path where the tz database is
/// unavailable we fall back to "now + 24h" so callers always get a
/// usable deadline.
pub fn next_pacific_midnight() -> Timestamp {
    fn compute() -> Option<Timestamp> {
        let tz = jiff::tz::TimeZone::get("America/Los_Angeles").ok()?;
        let now_pt = Timestamp::now().to_zoned(tz);
        let tomorrow = now_pt.date().tomorrow().ok()?;
        let midnight = tomorrow
            .at(0, 0, 0, 0)
            .to_zoned(now_pt.time_zone().clone())
            .ok()?;
        Some(midnight.timestamp())
    }
    compute().unwrap_or_else(|| {
        let now = Timestamp::now();
        now.checked_add(jiff::Span::new().hours(24)).unwrap_or(now)
    })
}

/// Turn a classified Google quota error into a persisted
/// [`ZadError::RateLimited`]. Daily quotas deadline at the next
/// Pacific-midnight reset (capped at [`MAX_DAILY_WAIT_SECONDS`]);
/// short-term limits honor `Retry-After` if present, otherwise fall
/// back to [`DEFAULT_SHORT_TERM_SECONDS`]. Either way the deadline is
/// written to `~/.zad/state/<service>/rate_limit.json` so every
/// sibling process hits the pre-call gate instead of burning more
/// quota.
pub fn handle(service: &'static str, category: QuotaCategory, headers: &HeaderMap) -> ZadError {
    let deadline = match category {
        QuotaCategory::Daily => {
            let target = next_pacific_midnight();
            let now = Timestamp::now();
            let secs = target.as_second().saturating_sub(now.as_second()).max(0) as u64;
            deadline_from_max(Duration::from_secs(secs), MAX_DAILY_WAIT_SECONDS)
        }
        QuotaCategory::ShortTerm => {
            let raw = headers
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let duration = if raw.is_some() {
                parse_retry_after(headers)
            } else {
                Duration::from_secs(DEFAULT_SHORT_TERM_SECONDS)
            };
            deadline_from_max(duration, MAX_WAIT_SECONDS)
        }
    };
    if let Err(e) = write_deadline(service, deadline) {
        tracing::warn!(
            service = service,
            error = %e,
            "failed to persist Google quota deadline",
        );
    }
    rate_limited_error(service, deadline)
}

/// Composed helper: if `status` is 403 and `body` is a Google quota
/// error, persist the deadline and return the typed error. Callers
/// who already know the response is non-2xx (and have read the body
/// to inspect for the quota reasons) use this to handle the
/// classification, persistence, and error construction in one call.
pub fn check_403(
    service: &'static str,
    status: reqwest::StatusCode,
    body: &str,
    headers: &HeaderMap,
) -> Option<ZadError> {
    if status.as_u16() != 403 {
        return None;
    }
    let category = classify(body)?;
    Some(handle(service, category, headers))
}
