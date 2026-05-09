//! Regression tests for the spotifai-reported bug where
//! `SpotifyHttp::access_token` silently dropped Spotify's rotated
//! refresh token, eventually getting it revoked.
//!
//! These tests stand up a one-shot localhost HTTP server that
//! impersonates Spotify's `/api/token` endpoint, point a `SpotifyHttp`
//! at it via the test-only `with_token_url`, and assert that:
//!
//! 1. when the refresh response carries a *different* `refresh_token`,
//!    the configured `RefreshTokenStore` is invoked exactly once with
//!    the new value and the in-memory state is updated;
//! 2. when the refresh response carries the *same* token, the store
//!    is not called (avoids redundant keychain writes);
//! 3. when the refresh response carries no `refresh_token` at all
//!    (the gcal-style non-rotating case), the in-memory copy is
//!    preserved verbatim.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use zad::error::Result;
use zad::oauth::RefreshTokenStore;
use zad::service::spotify::client::SpotifyHttp;

/// In-memory `RefreshTokenStore` that records every persist call.
#[derive(Default)]
struct RecordingStore {
    saved: Mutex<Vec<String>>,
}

impl RefreshTokenStore for RecordingStore {
    fn store(&self, refresh_token: &str) -> Result<()> {
        self.saved.lock().unwrap().push(refresh_token.to_string());
        Ok(())
    }
}

/// Stand up a one-shot HTTP/1.1 server on `127.0.0.1` that responds
/// to the next request with `body` and a 200. Returns the URL the
/// server is listening on (suitable for `SpotifyHttp::with_token_url`).
async fn one_shot_token_server(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = match listener.accept().await {
            Ok(p) => p,
            Err(_) => return,
        };
        // Drain the request — we don't need to inspect the body for
        // these tests (the rotation logic is what we care about).
        let mut buf = [0u8; 8192];
        let _ = stream.read(&mut buf).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
    format!("http://127.0.0.1:{port}/api/token")
}

fn http_with(
    initial_refresh: &str,
    store: Option<Arc<dyn RefreshTokenStore>>,
    token_url: &str,
) -> SpotifyHttp {
    SpotifyHttp::with_store(
        "test-client".into(),
        initial_refresh.into(),
        // `me()` is unscoped; no scope set required.
        BTreeSet::new(),
        PathBuf::new(),
        store,
    )
    .with_token_url(token_url)
}

/// The reported bug: Spotify rotates the refresh token, but zad-0.6.4
/// dropped the new value. With the fix in place, the rotated token
/// must be persisted via the configured store and reflected in the
/// client's in-memory copy.
#[tokio::test]
async fn rotated_refresh_token_is_persisted_and_remembered() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-1","refresh_token":"RT-NEW","expires_in":3600,"token_type":"Bearer"}"#
            .into(),
    )
    .await;

    let store = Arc::new(RecordingStore::default());
    let http = http_with("RT-OLD", Some(store.clone()), &url);

    // Drive the refresh directly so we don't follow up with a
    // real-network call to api.spotify.com.
    let access = http.access_token().await.expect("token refresh");
    assert_eq!(access, "AT-1");

    let saved = store.saved.lock().unwrap().clone();
    assert_eq!(
        saved,
        vec!["RT-NEW".to_string()],
        "rotated refresh token must be persisted via the store exactly once"
    );
    assert_eq!(
        http.refresh_token_for_test().await,
        "RT-NEW",
        "in-memory refresh token must reflect the rotated value"
    );
}

/// When the provider returns the *same* refresh token, the store
/// must not be called — there's nothing to persist, and a redundant
/// keychain write is needless.
#[tokio::test]
async fn unchanged_refresh_token_does_not_call_store() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-1","refresh_token":"RT-OLD","expires_in":3600,"token_type":"Bearer"}"#
            .into(),
    )
    .await;

    let store = Arc::new(RecordingStore::default());
    let http = http_with("RT-OLD", Some(store.clone()), &url);

    let access = http.access_token().await.expect("token refresh");
    assert_eq!(access, "AT-1");

    assert!(
        store.saved.lock().unwrap().is_empty(),
        "store must not be called when the refresh token is unchanged"
    );
    assert_eq!(http.refresh_token_for_test().await, "RT-OLD");
}

/// Regression for the gcal-style non-rotating case: when the response
/// omits `refresh_token` entirely, the existing token must survive
/// untouched. This is the invariant that protects providers like
/// Google from being clobbered by an over-eager rotation handler.
#[tokio::test]
async fn missing_refresh_token_preserves_existing() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-1","expires_in":3600,"token_type":"Bearer"}"#.into(),
    )
    .await;

    let store = Arc::new(RecordingStore::default());
    let http = http_with("RT-OLD", Some(store.clone()), &url);

    let access = http.access_token().await.expect("token refresh");
    assert_eq!(access, "AT-1");

    assert!(
        store.saved.lock().unwrap().is_empty(),
        "store must not be called when no refresh_token is returned"
    );
    assert_eq!(http.refresh_token_for_test().await, "RT-OLD");
}
