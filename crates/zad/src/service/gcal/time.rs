//! RFC3339 and bare-date parsing for `--start` / `--end`.
//!
//! Calendar event boundaries accept three user-facing forms:
//!
//! - RFC3339 with `Z` (`2026-04-19T15:30:00Z`)
//! - RFC3339 with offset (`2026-04-19T15:30:00-07:00`)
//! - Bare ISO date (`2026-04-19`) → all-day event; Google expects the
//!   `start.date` / `end.date` fields instead of `start.dateTime`.
//!
//! Returned as [`EventTime`] so the HTTP client can emit the right
//! Calendar API field shape.

use jiff::{Timestamp, civil::Date};

use crate::error::{Result, ZadError};

/// Parsed event boundary. The two variants map 1:1 to Calendar API's
/// `EventDateTime` — `dateTime + timeZone` vs. `date`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventTime {
    /// Full timestamp. Stored as an RFC3339 string (always with offset
    /// preserved) because Google's API happily accepts either form.
    DateTime(String),
    /// All-day event. Stored as `YYYY-MM-DD`.
    Date(String),
}

impl EventTime {
    /// Render as a Calendar API `EventDateTime` JSON object, picking
    /// `dateTime` or `date` based on the variant.
    pub fn to_api_json(&self, tz: Option<&str>) -> serde_json::Value {
        match self {
            EventTime::DateTime(s) => {
                let mut obj = serde_json::json!({ "dateTime": s });
                if let Some(z) = tz {
                    obj["timeZone"] = serde_json::Value::String(z.to_string());
                }
                obj
            }
            EventTime::Date(s) => serde_json::json!({ "date": s }),
        }
    }

    /// `true` iff this is a bare-date (all-day) boundary.
    pub fn is_all_day(&self) -> bool {
        matches!(self, EventTime::Date(_))
    }

    /// As an RFC3339 string the caller can embed into an error message
    /// or a `timeMin`/`timeMax` query. All-day dates are promoted to
    /// midnight UTC.
    pub fn as_rfc3339(&self) -> String {
        match self {
            EventTime::DateTime(s) => s.clone(),
            EventTime::Date(d) => format!("{d}T00:00:00Z"),
        }
    }
}

/// Parse any of the three user-facing forms. The order of attempts
/// matters: bare date must be tried last because the full RFC3339
/// forms are also "parsable" as invalid bare dates up to the `T`.
pub fn parse_event_time(raw: &str) -> Result<EventTime> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(ZadError::Invalid(
            "event time must not be empty; expected RFC3339 or YYYY-MM-DD".into(),
        ));
    }

    // Bare date — prefer this match when there's no `T` separator, so
    // an invalid RFC3339 string like `2026-04-19Tbad` still fails with
    // a useful RFC3339 error rather than a confusing date error.
    if !s.contains('T') {
        return s
            .parse::<Date>()
            .map(|d| EventTime::Date(d.to_string()))
            .map_err(|e| {
                ZadError::Invalid(format!("invalid date `{raw}`: expected YYYY-MM-DD ({e})"))
            });
    }

    // `Timestamp` parses RFC3339 with either `Z` or an offset —
    // `Zoned` would also demand an IANA `[Zone]` suffix, which
    // humans don't paste.
    match s.parse::<Timestamp>() {
        Ok(ts) => Ok(EventTime::DateTime(ts.to_string())),
        Err(e) => Err(ZadError::Invalid(format!(
            "invalid RFC3339 timestamp `{raw}`: {e}"
        ))),
    }
}

/// Minutes from `now` to `start`. Negative if `start` is in the past.
pub fn minutes_from_now(start: &EventTime) -> Option<i64> {
    let ts = as_timestamp(start)?;
    let now = Timestamp::now();
    Some((ts.as_second() - now.as_second()) / 60)
}

/// Days from `now` to `start`. Negative if `start` is in the past.
pub fn days_from_now(start: &EventTime) -> Option<i64> {
    minutes_from_now(start).map(|m| m / (60 * 24))
}

fn as_timestamp(t: &EventTime) -> Option<Timestamp> {
    t.as_rfc3339().parse::<Timestamp>().ok()
}
