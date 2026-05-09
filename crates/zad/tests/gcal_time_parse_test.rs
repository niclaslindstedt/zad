//! Boundary-case tests for the `--start` / `--end` RFC3339 + bare-date
//! parser. All pure; no network.

use zad::service::gcal::time::{EventTime, parse_event_time};

#[test]
fn rfc3339_utc_is_datetime() {
    let t = parse_event_time("2026-05-01T15:30:00Z").unwrap();
    assert!(matches!(t, EventTime::DateTime(_)));
    assert!(!t.is_all_day());
}

#[test]
fn rfc3339_with_offset_is_datetime() {
    let t = parse_event_time("2026-05-01T15:30:00-07:00").unwrap();
    assert!(matches!(t, EventTime::DateTime(_)));
}

#[test]
fn bare_date_is_all_day() {
    let t = parse_event_time("2026-04-19").unwrap();
    assert!(matches!(t, EventTime::Date(ref s) if s == "2026-04-19"));
    assert!(t.is_all_day());
}

#[test]
fn all_day_json_uses_date_field() {
    let t = parse_event_time("2026-04-19").unwrap();
    let v = t.to_api_json(None);
    assert_eq!(v.get("date").and_then(|x| x.as_str()), Some("2026-04-19"));
    assert!(v.get("dateTime").is_none());
}

#[test]
fn datetime_json_includes_optional_timezone() {
    let t = parse_event_time("2026-05-01T15:30:00Z").unwrap();
    let v = t.to_api_json(Some("America/Los_Angeles"));
    assert!(v.get("dateTime").is_some());
    assert_eq!(
        v.get("timeZone").and_then(|x| x.as_str()),
        Some("America/Los_Angeles")
    );
}

#[test]
fn empty_input_errors() {
    let err = parse_event_time("   ").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("empty"), "got: {msg}");
}

#[test]
fn invalid_rfc3339_errors_with_rfc3339_message() {
    // Contains `T` so we take the datetime path; gibberish after it.
    let err = parse_event_time("2026-04-19Tgarbage").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("RFC3339"), "got: {msg}");
}

#[test]
fn invalid_date_errors_with_date_message() {
    let err = parse_event_time("2026-04").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("YYYY-MM-DD"), "got: {msg}");
}
