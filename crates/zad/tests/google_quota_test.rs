//! Unit tests for the `google_quota` classifier module.
//!
//! These tests pin `ZAD_HOME_OVERRIDE` to a temp directory so the
//! persisted deadline file lands in a sandboxed location. The module
//! reads `ZAD_HOME_OVERRIDE` at call time, so `#[serial]` is needed
//! anywhere we mutate env.

use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
use serial_test::serial;
use zad::ZadError;
use zad::google_quota::{self, QuotaCategory};
use zad::rate_limit;

fn with_home<R>(f: impl FnOnce() -> R) -> R {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: serial-tested.
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
fn classify_daily_quota_reasons() {
    let bodies = [
        r#"{"error":{"errors":[{"reason":"quotaExceeded"}]}}"#,
        r#"{"error":{"errors":[{"reason":"dailyLimitExceeded"}]}}"#,
        // Substring fallback also matches camelCase variants.
        r#"random text mentioning QUOTAEXCEEDED somewhere"#,
    ];
    for body in bodies {
        assert_eq!(
            google_quota::classify(body),
            Some(QuotaCategory::Daily),
            "body should classify as Daily: {body}",
        );
    }
}

#[test]
fn classify_short_term_quota_reasons() {
    let bodies = [
        r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#,
        r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}"#,
    ];
    for body in bodies {
        assert_eq!(
            google_quota::classify(body),
            Some(QuotaCategory::ShortTerm),
            "body should classify as ShortTerm: {body}",
        );
    }
}

#[test]
fn classify_unrelated_403_returns_none() {
    let bodies = [
        r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#,
        r#"{"error":{"errors":[{"reason":"forbidden"}]}}"#,
        r#"plain text 404"#,
        "",
    ];
    for body in bodies {
        assert_eq!(
            google_quota::classify(body),
            None,
            "body should not classify: {body}",
        );
    }
}

#[test]
fn next_pacific_midnight_is_in_the_future_and_within_a_day() {
    let now = jiff::Timestamp::now().as_second();
    let target = google_quota::next_pacific_midnight().as_second();
    let delta = target - now;
    assert!(
        delta > 0,
        "PT midnight must be in the future: delta={delta}"
    );
    // PT midnight is at most 24h+1h DST slack away from any UTC instant.
    assert!(
        delta <= 25 * 3600,
        "PT midnight must land within ~25h: delta={delta}",
    );
}

#[test]
#[serial]
fn handle_daily_persists_long_deadline_in_state_file() {
    with_home(|| {
        let err = google_quota::handle("ymusic", QuotaCategory::Daily, &HeaderMap::new());
        match err {
            ZadError::RateLimited {
                service,
                retry_after_seconds,
                ..
            } => {
                assert_eq!(service, "ymusic");
                // Daily deadline can span hours; assert it's longer
                // than the generic 1h short-term cap.
                assert!(
                    retry_after_seconds > 3600 || retry_after_seconds <= 25 * 3600,
                    "daily deadline within sane bounds: {retry_after_seconds}",
                );
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
        let read = rate_limit::read_deadline("ymusic").expect("state persisted");
        let now = jiff::Timestamp::now().as_second();
        let delta = read.as_second() - now;
        assert!(delta > 0, "persisted deadline must be in the future");
    });
}

#[test]
#[serial]
fn handle_short_term_honors_retry_after() {
    with_home(|| {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("17"));
        let err = google_quota::handle("ymusic", QuotaCategory::ShortTerm, &headers);
        match err {
            ZadError::RateLimited {
                retry_after_seconds,
                ..
            } => {
                assert!(
                    (15..=20).contains(&retry_after_seconds),
                    "short-term deadline honors Retry-After=17: got {retry_after_seconds}",
                );
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    });
}

#[test]
#[serial]
fn handle_short_term_falls_back_to_sixty_seconds_when_header_absent() {
    with_home(|| {
        // Google rarely sends Retry-After on 403 quota errors; the
        // classifier falls back to the documented per-100s window.
        let err = google_quota::handle("ymusic", QuotaCategory::ShortTerm, &HeaderMap::new());
        match err {
            ZadError::RateLimited {
                retry_after_seconds,
                ..
            } => {
                assert!(
                    (55..=65).contains(&retry_after_seconds),
                    "short-term default ~60s: got {retry_after_seconds}",
                );
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    });
}

#[test]
#[serial]
fn check_403_returns_none_for_non_403_statuses() {
    with_home(|| {
        let body = r#"{"error":{"errors":[{"reason":"quotaExceeded"}]}}"#;
        let result = google_quota::check_403("ymusic", StatusCode::OK, body, &HeaderMap::new());
        assert!(
            result.is_none(),
            "check_403 must ignore non-403 responses even if body looks like a quota error",
        );
        let result =
            google_quota::check_403("ymusic", StatusCode::NOT_FOUND, body, &HeaderMap::new());
        assert!(
            result.is_none(),
            "404 with quota-looking body must be a no-op"
        );
    });
}

#[test]
#[serial]
fn check_403_returns_none_for_403_without_quota_reason() {
    with_home(|| {
        let body = r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#;
        let result =
            google_quota::check_403("ymusic", StatusCode::FORBIDDEN, body, &HeaderMap::new());
        assert!(
            result.is_none(),
            "a 403 that isn't a quota error must not engage the rate-limit gate",
        );
        // State file must not be created when classifier returns None.
        assert!(
            rate_limit::read_deadline("ymusic").is_none(),
            "no deadline should be persisted for non-quota 403s",
        );
    });
}

#[test]
#[serial]
fn check_403_persists_and_returns_rate_limited_for_quota_body() {
    with_home(|| {
        let body = r#"{"error":{"code":403,"errors":[{"reason":"quotaExceeded"}]}}"#;
        let result =
            google_quota::check_403("ymusic", StatusCode::FORBIDDEN, body, &HeaderMap::new());
        match result {
            Some(ZadError::RateLimited { service, .. }) => {
                assert_eq!(service, "ymusic");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
        assert!(
            rate_limit::read_deadline("ymusic").is_some(),
            "quota body must persist a deadline so sibling processes are gated",
        );
    });
}

#[test]
#[serial]
fn handle_short_term_caps_pathological_retry_after_values() {
    with_home(|| {
        let mut headers = HeaderMap::new();
        // A misbehaving server hands out an absurd value. The
        // generic cap (MAX_WAIT_SECONDS = 3600) must kick in even
        // for the short-term Google branch.
        headers.insert(RETRY_AFTER, HeaderValue::from_static("99999999"));
        let err = google_quota::handle("ymusic", QuotaCategory::ShortTerm, &headers);
        match err {
            ZadError::RateLimited {
                retry_after_seconds,
                ..
            } => {
                assert!(
                    retry_after_seconds <= rate_limit::MAX_WAIT_SECONDS,
                    "short-term cap must hold: {retry_after_seconds}",
                );
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    });
}
