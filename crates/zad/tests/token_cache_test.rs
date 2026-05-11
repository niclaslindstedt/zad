//! Tests for the cross-process access-token cache and refresh lock.
//!
//! Each test uses an isolated `tempfile::TempDir` as the service
//! directory so tests can run in parallel without interfering with
//! each other or with the real `~/.zad` state.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use zad::error::Result;
use zad::oauth::RefreshTokenStore;
use zad::service::spotify::client::SpotifyHttp;
use zad::token_cache;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Spin up a one-shot HTTP/1.1 server that serves `body` to the next
/// request. Returns the token-endpoint URL.
async fn one_shot_token_server(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = match listener.accept().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let mut buf = [0u8; 8192];
        let _ = stream.read(&mut buf).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
    format!("http://127.0.0.1:{port}/api/token")
}

#[derive(Default)]
struct RecordingStore {
    saved: Mutex<Vec<String>>,
}

impl RefreshTokenStore for RecordingStore {
    fn store(&self, rt: &str) -> Result<()> {
        self.saved.lock().unwrap().push(rt.to_string());
        Ok(())
    }
}

fn spotify_with_cache(token_url: &str, cache_dir: PathBuf) -> SpotifyHttp {
    use std::collections::BTreeSet;
    SpotifyHttp::with_store(
        "test-client".into(),
        "RT-INITIAL".into(),
        BTreeSet::new(),
        PathBuf::new(),
        None,
    )
    .with_token_url(token_url)
    .with_cache_dir(cache_dir)
}

// ---------------------------------------------------------------------------
// token_cache unit tests
// ---------------------------------------------------------------------------

#[test]
fn read_returns_none_on_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(token_cache::read(Some(tmp.path())).is_none());
}

#[test]
fn write_then_read_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    token_cache::write(Some(tmp.path()), "ACCESS-1", 3600).unwrap();
    let got = token_cache::read(Some(tmp.path()));
    assert_eq!(got.as_deref(), Some("ACCESS-1"));
}

#[test]
fn clear_removes_cached_token() {
    let tmp = tempfile::tempdir().unwrap();
    token_cache::write(Some(tmp.path()), "ACCESS-1", 3600).unwrap();
    token_cache::clear(Some(tmp.path()));
    assert!(token_cache::read(Some(tmp.path())).is_none());
}

#[test]
fn read_returns_none_when_service_dir_is_none() {
    assert!(token_cache::read(None).is_none());
}

#[test]
fn write_is_noop_when_service_dir_is_none() {
    token_cache::write(None, "ACCESS-1", 3600).unwrap();
    // No panic; nothing written.
}

#[tokio::test]
async fn acquire_lock_returns_none_when_service_dir_is_none() {
    let guard = token_cache::acquire_lock(None).await.unwrap();
    assert!(guard.is_none());
}

#[tokio::test]
async fn lock_is_exclusive_within_process() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Acquire the lock on task A.
    let _lock_a = token_cache::acquire_lock(Some(dir)).await.unwrap().unwrap();

    // Attempting to acquire it on task B should time out quickly.
    // We lower the effective timeout by using a very short LOCK_WAIT
    // ceiling — we can't do that from outside the module, so instead
    // we just assert that trying to acquire from a second concurrent
    // task fails within a reasonable wall-clock window.
    let dir_buf = dir.to_path_buf();
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        token_cache::acquire_lock(Some(&dir_buf)),
    )
    .await;

    // The future either timed out (still running, lock held) or
    // returned an error (timed out internally). Either way, task B
    // must not have acquired the lock while task A holds it.
    match result {
        Err(_elapsed) => { /* tokio timeout — lock still held, correct */ }
        Ok(Err(_)) => { /* token_cache internal timeout — also correct */ }
        Ok(Ok(Some(_))) => panic!("second task acquired the lock while first task holds it"),
        Ok(Ok(None)) => unreachable!("service_dir is Some"),
    }
}

// ---------------------------------------------------------------------------
// integration: second process reads from cache (simulated via second client)
// ---------------------------------------------------------------------------

/// The core fan-out scenario: two `SpotifyHttp` instances sharing a
/// cache directory (simulating two processes with the same service
/// dir). The first one performs the token refresh; the second one
/// must pick up the cached token without hitting the token endpoint.
#[tokio::test]
async fn second_client_uses_file_cache_without_refreshing() {
    // The mock server only handles ONE request. If the second client
    // also calls the token endpoint, the test would deadlock waiting
    // for a second server that never arrives.
    let url = one_shot_token_server(
        r#"{"access_token":"AT-CACHED","expires_in":3600,"token_type":"Bearer"}"#.into(),
    )
    .await;

    let tmp = tempfile::tempdir().unwrap();

    // Client A — triggers the actual refresh.
    let client_a = spotify_with_cache(&url, tmp.path().to_path_buf());
    let token_a = client_a.access_token().await.expect("client A refresh");
    assert_eq!(token_a, "AT-CACHED");

    // Client B — different instance, same cache dir, token server is
    // now gone. Must return AT-CACHED from the file cache.
    let client_b = spotify_with_cache("http://127.0.0.1:0/unreachable", tmp.path().to_path_buf());
    let token_b = client_b.access_token().await.expect("client B cache hit");
    assert_eq!(token_b, "AT-CACHED", "client B must use the file cache");
}

/// When the store is called for refresh-token rotation AND the access
/// token is written to the file cache, the cache survives a fresh
/// client instantiation (the "next process" scenario).
#[tokio::test]
async fn rotated_token_and_cache_survive_fresh_client() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-2","refresh_token":"RT-2","expires_in":3600,"token_type":"Bearer"}"#
            .into(),
    )
    .await;

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(RecordingStore::default());

    let client = {
        use std::collections::BTreeSet;
        SpotifyHttp::with_store(
            "test-client".into(),
            "RT-1".into(),
            BTreeSet::new(),
            PathBuf::new(),
            Some(store.clone()),
        )
        .with_token_url(&url)
        .with_cache_dir(tmp.path().to_path_buf())
    };

    let token = client.access_token().await.expect("refresh");
    assert_eq!(token, "AT-2");
    assert_eq!(store.saved.lock().unwrap().as_slice(), ["RT-2"]);

    // Fresh client — simulates a new process starting with the same
    // service dir. Token endpoint is gone; must serve from cache.
    let next = spotify_with_cache("http://127.0.0.1:0/unreachable", tmp.path().to_path_buf());
    let cached = next
        .access_token()
        .await
        .expect("cache hit on next process");
    assert_eq!(cached, "AT-2");
}
