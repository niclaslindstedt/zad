//! Tests for the shared rate-limit module.
//!
//! Each test pins `ZAD_HOME_OVERRIDE` to a fresh tempdir so the
//! persisted state file lands in an isolated location. The module
//! reads the env at call time (not once-cell'd), so concurrent tests
//! still need `#[serial]` because two parallel tempdirs would both be
//! visible to all-current-threads.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serial_test::serial;
use zad::ZadError;
use zad::rate_limit;

fn with_home<R>(f: impl FnOnce() -> R) -> R {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: std::env::set_var is unsafe under concurrent reads; the
    // #[serial] guard above ensures no other test races us.
    unsafe {
        std::env::set_var("ZAD_HOME_OVERRIDE", tmp.path());
    }
    let out = f();
    unsafe {
        std::env::remove_var("ZAD_HOME_OVERRIDE");
    }
    drop(tmp);
    out
}

#[test]
#[serial]
fn parse_retry_after_numeric_seconds() {
    let mut h = HeaderMap::new();
    h.insert(RETRY_AFTER, HeaderValue::from_static("42"));
    let d = rate_limit::parse_retry_after(&h);
    assert_eq!(d, Duration::from_secs(42));
}

#[test]
#[serial]
fn parse_retry_after_http_date_in_future() {
    let mut h = HeaderMap::new();
    // RFC 2822 / IMF-fixdate covers this date format; jiff accepts it.
    // Use a far-future timestamp so the test is stable.
    h.insert(
        RETRY_AFTER,
        HeaderValue::from_static("Wed, 06 Nov 2099 08:49:37 GMT"),
    );
    let d = rate_limit::parse_retry_after(&h);
    // Anything well in the future and capped by MAX_WAIT_SECONDS via
    // deadline_from, but parse_retry_after itself returns the raw
    // delta. We just assert it's > 0.
    assert!(d.as_secs() > 0);
}

#[test]
#[serial]
fn parse_retry_after_missing_falls_back_to_default() {
    let h = HeaderMap::new();
    let d = rate_limit::parse_retry_after(&h);
    // Default fallback is 5s; assert it's nonzero so we never block on a
    // header-less 429 with a literal 0.
    assert!(d.as_secs() >= 1);
}

#[test]
#[serial]
fn parse_retry_after_unparseable_falls_back_to_default() {
    let mut h = HeaderMap::new();
    h.insert(RETRY_AFTER, HeaderValue::from_static("not-a-number"));
    let d = rate_limit::parse_retry_after(&h);
    assert!(d.as_secs() >= 1);
}

#[test]
#[serial]
fn write_then_read_deadline_roundtrip() {
    with_home(|| {
        let deadline = rate_limit::deadline_from(Duration::from_secs(60));
        rate_limit::write_deadline("discord", deadline).unwrap();
        let read = rate_limit::read_deadline("discord").expect("deadline persisted");
        assert_eq!(read.as_second(), deadline.as_second());
    });
}

#[test]
#[serial]
fn read_deadline_returns_none_when_in_the_past() {
    with_home(|| {
        // Write a stale deadline and confirm read_deadline cleans it up
        // and returns None.
        let past = jiff::Timestamp::now()
            .checked_sub(jiff::Span::new().seconds(10))
            .unwrap();
        rate_limit::write_deadline("spotify", past).unwrap();
        assert!(rate_limit::read_deadline("spotify").is_none());
        // File should be removed too.
        let path = rate_limit::state_path("spotify").unwrap();
        assert!(!path.exists(), "stale state file should be cleaned up");
    });
}

#[test]
#[serial]
fn clear_removes_state() {
    with_home(|| {
        let deadline = rate_limit::deadline_from(Duration::from_secs(60));
        rate_limit::write_deadline("slack", deadline).unwrap();
        assert!(rate_limit::read_deadline("slack").is_some());
        rate_limit::clear("slack");
        assert!(rate_limit::read_deadline("slack").is_none());
    });
}

#[test]
#[serial]
fn precall_check_without_state_is_noop() {
    with_home(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Even with --wait, an absent state file must return immediately.
        rt.block_on(async {
            rate_limit::precall_check("ymusic", true).await.unwrap();
            rate_limit::precall_check("ymusic", false).await.unwrap();
        });
    });
}

#[test]
#[serial]
fn precall_check_fails_fast_without_wait_when_blocked() {
    with_home(|| {
        let deadline = rate_limit::deadline_from(Duration::from_secs(60));
        rate_limit::write_deadline("telegram", deadline).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(rate_limit::precall_check("telegram", false))
            .unwrap_err();
        match err {
            ZadError::RateLimited {
                service,
                retry_after_seconds,
                ..
            } => {
                assert_eq!(service, "telegram");
                assert!(retry_after_seconds > 0);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    });
}

#[test]
#[serial]
fn handle_429_persists_state_and_emits_typed_error() {
    with_home(|| {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_static("7"));
        let err = rate_limit::handle_429("gcal", &h);
        match err {
            ZadError::RateLimited {
                service,
                retry_after_seconds,
                retry_after_utc,
            } => {
                assert_eq!(service, "gcal");
                assert!((6..=8).contains(&retry_after_seconds));
                assert!(retry_after_utc.contains('T'));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
        // State was persisted.
        let read = rate_limit::read_deadline("gcal").unwrap();
        let now = jiff::Timestamp::now().as_second();
        let delta = read.as_second() - now;
        assert!((5..=10).contains(&delta));
    });
}

#[test]
#[serial]
fn rate_limited_error_message_mentions_wait_flag() {
    let err = ZadError::RateLimited {
        service: "spotify",
        retry_after_seconds: 12,
        retry_after_utc: "2030-01-01T00:00:00Z".into(),
    };
    let s = err.to_string();
    assert!(s.contains("--wait"), "message should mention --wait: {s}");
    assert!(
        s.contains("12"),
        "message should include the wait seconds: {s}"
    );
    assert!(
        s.contains("2030-01-01T00:00:00Z"),
        "message should include the absolute deadline: {s}"
    );
}
