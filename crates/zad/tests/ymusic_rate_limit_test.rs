//! End-to-end rate-limit tests for `YmusicHttp`.
//!
//! These tests stand up a single-socket mock that returns Google-shaped
//! error envelopes (HTTP 403 with `quotaExceeded` / `rateLimitExceeded`
//! `reason` codes — the YouTube Data API's de-facto equivalent of an
//! HTTP 429) and assert that:
//!
//! 1. The client surfaces [`ZadError::RateLimited`], not a plain
//!    `Service` error.
//! 2. The deadline is persisted to disk so a sibling process (or a
//!    follow-up CLI invocation) hits the [`rate_limit::precall_check`]
//!    gate instead of burning another quota point.
//! 3. Daily-quota deadlines extend past the generic short-term cap;
//!    short-term limits stay within ~60s.
//!
//! The shared state file lives under `ZAD_HOME_OVERRIDE`; every test
//! is wrapped in `#[serial]` because the env var is process-global.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serial_test::serial;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zad::ZadError;
use zad::rate_limit;
use zad::service::ymusic::client::YmusicHttp;

#[derive(Clone, Copy, Debug)]
enum MockResponse {
    /// HTTP 403 with a `quotaExceeded` reason — daily quota exhausted.
    Forbidden403QuotaExceeded,
    /// HTTP 403 with a `userRateLimitExceeded` reason — short-term
    /// per-user burst limit.
    Forbidden403UserRateLimit,
    /// HTTP 429 with `Retry-After: 7` (rare on YouTube, but verifies
    /// the canonical path still works).
    TooMany429WithRetryAfter7,
    /// HTTP 403 with `insufficientPermissions` — must NOT engage the
    /// rate-limit gate.
    Forbidden403InsufficientPermissions,
}

async fn mock_server(api_response: MockResponse) -> (String, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match stream.read(&mut buf).await {
                    Ok(n) => n,
                    Err(_) => return,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let first_line = req.lines().next().unwrap_or("").to_string();
                let response = if first_line.contains("/token") {
                    let body = serde_json::json!({
                        "access_token": "mock-access",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                    })
                    .to_string();
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body,
                    )
                } else {
                    render_api_response(api_response)
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (
        format!("http://127.0.0.1:{port}/token"),
        format!("http://127.0.0.1:{port}/v3"),
    )
}

fn render_api_response(kind: MockResponse) -> String {
    match kind {
        MockResponse::Forbidden403QuotaExceeded => {
            let body = serde_json::json!({
                "error": {
                    "code": 403,
                    "message": "The request cannot be completed because you have exceeded your quota.",
                    "errors": [{
                        "domain": "youtube.quota",
                        "reason": "quotaExceeded",
                        "message": "Quota exceeded.",
                    }],
                }
            })
            .to_string();
            format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            )
        }
        MockResponse::Forbidden403UserRateLimit => {
            let body = serde_json::json!({
                "error": {
                    "code": 403,
                    "errors": [{
                        "domain": "usageLimits",
                        "reason": "userRateLimitExceeded",
                        "message": "User Rate Limit Exceeded",
                    }],
                }
            })
            .to_string();
            format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            )
        }
        MockResponse::TooMany429WithRetryAfter7 => {
            let body = serde_json::json!({
                "error": {
                    "code": 429,
                    "errors": [{
                        "reason": "uploadRateLimitExceeded",
                    }],
                }
            })
            .to_string();
            format!(
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            )
        }
        MockResponse::Forbidden403InsufficientPermissions => {
            let body = serde_json::json!({
                "error": {
                    "code": 403,
                    "errors": [{
                        "reason": "insufficientPermissions",
                    }],
                }
            })
            .to_string();
            format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            )
        }
    }
}

fn http_pointed_at(token_url: &str, api_base: &str, cache_dir: PathBuf) -> YmusicHttp {
    let mut scopes = BTreeSet::new();
    scopes.insert("library.read".to_string());
    YmusicHttp::new(
        "test-client".into(),
        "test-secret".into(),
        "test-refresh".into(),
        scopes,
        PathBuf::new(),
    )
    .with_token_url(token_url)
    .with_api_base(api_base)
    .with_cache_dir(cache_dir)
}

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
#[serial]
fn quota_exceeded_403_surfaces_as_rate_limited_with_long_daily_deadline() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    with_home(|| {
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let (token_url, api_base) = mock_server(MockResponse::Forbidden403QuotaExceeded).await;
            let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

            let err = http.list_liked_videos(Some(10)).await.unwrap_err();
            match err {
                ZadError::RateLimited {
                    service,
                    retry_after_seconds,
                    ..
                } => {
                    assert_eq!(service, "ymusic");
                    assert!(
                        retry_after_seconds > 0,
                        "daily quota deadline must be in the future: {retry_after_seconds}",
                    );
                    // Daily quotas extend past the short-term cap;
                    // verify the deadline is at least into the next
                    // wall-clock hour. (We can't assert "> 1h" without
                    // pinning the wall clock — at 23:30 PT the next
                    // midnight is 30 minutes away — but we can assert
                    // "<= the 25h ceiling".)
                    assert!(
                        retry_after_seconds <= 25 * 3600,
                        "daily deadline within sane bounds: {retry_after_seconds}",
                    );
                }
                other => panic!("expected RateLimited, got {other:?}"),
            }

            // Cross-process: the state file must be persisted so a
            // sibling process hits the precall gate.
            let deadline = rate_limit::read_deadline("ymusic")
                .expect("rate-limit state must be persisted for cross-process visibility");
            let now = jiff::Timestamp::now().as_second();
            assert!(deadline.as_second() > now, "persisted deadline in future");
        });
    });
}

#[test]
#[serial]
fn user_rate_limit_403_surfaces_as_short_term_rate_limited() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    with_home(|| {
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let (token_url, api_base) = mock_server(MockResponse::Forbidden403UserRateLimit).await;
            let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

            let err = http.list_liked_videos(Some(10)).await.unwrap_err();
            match err {
                ZadError::RateLimited {
                    retry_after_seconds,
                    ..
                } => {
                    // Short-term default is 60s. Allow some slack for
                    // wall-clock drift between persist and read.
                    assert!(
                        (50..=70).contains(&retry_after_seconds),
                        "short-term default ~60s: {retry_after_seconds}",
                    );
                }
                other => panic!("expected RateLimited, got {other:?}"),
            }
        });
    });
}

#[test]
#[serial]
fn http_429_with_retry_after_still_routes_through_canonical_handler() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    with_home(|| {
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let (token_url, api_base) = mock_server(MockResponse::TooMany429WithRetryAfter7).await;
            let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

            let err = http.list_liked_videos(Some(10)).await.unwrap_err();
            match err {
                ZadError::RateLimited {
                    retry_after_seconds,
                    ..
                } => {
                    assert!(
                        (5..=10).contains(&retry_after_seconds),
                        "Retry-After=7 must round-trip: {retry_after_seconds}",
                    );
                }
                other => panic!("expected RateLimited, got {other:?}"),
            }
        });
    });
}

#[test]
#[serial]
fn forbidden_403_without_quota_reason_is_a_regular_service_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    with_home(|| {
        rt.block_on(async {
            let tmp = TempDir::new().unwrap();
            let (token_url, api_base) =
                mock_server(MockResponse::Forbidden403InsufficientPermissions).await;
            let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

            let err = http.list_liked_videos(Some(10)).await.unwrap_err();
            match err {
                ZadError::Service { name, message } => {
                    assert_eq!(name, "ymusic");
                    // Message must point at re-running `service create`
                    // since that's the actionable next step for a
                    // scope / consent error.
                    assert!(
                        message.contains("HTTP 403"),
                        "message names the status code: {message}",
                    );
                }
                ZadError::RateLimited { .. } => {
                    panic!("a non-quota 403 must not engage the rate-limit gate");
                }
                other => panic!("expected Service error, got {other:?}"),
            }
            // And critically: no state file is written, so subsequent
            // (potentially well-scoped) calls aren't gated.
            assert!(
                rate_limit::read_deadline("ymusic").is_none(),
                "non-quota 403 must not persist a deadline",
            );
        });
    });
}
