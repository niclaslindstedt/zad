//! Same shape as `spotify_refresh_rotation_test.rs`, applied to
//! `YmusicHttp`. Google's confidential-client flow does not rotate
//! refresh tokens today, so the rotation branch is mostly inert in
//! practice — but the test is here to catch regressions if a future
//! provider change (or a per-account quirk) ever does start
//! rotating, **and** to lock in the non-rotating invariant
//! (response-without-refresh_token must not clobber the in-memory
//! token).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use zad::error::Result;
use zad::oauth::RefreshTokenStore;
use zad::service::ymusic::client::YmusicHttp;

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
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
    format!("http://127.0.0.1:{port}/token")
}

fn http_with(
    initial_refresh: &str,
    store: Option<Arc<dyn RefreshTokenStore>>,
    token_url: &str,
    cache_dir: PathBuf,
) -> YmusicHttp {
    YmusicHttp::with_store(
        "test-client".into(),
        "test-secret".into(),
        initial_refresh.into(),
        BTreeSet::new(),
        PathBuf::new(),
        store,
    )
    .with_token_url(token_url)
    .with_cache_dir(cache_dir)
}

#[tokio::test]
async fn rotated_refresh_token_is_persisted_and_remembered() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-1","refresh_token":"RT-NEW","expires_in":3600,"token_type":"Bearer"}"#
            .into(),
    )
    .await;

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(RecordingStore::default());
    let http = http_with(
        "RT-OLD",
        Some(store.clone()),
        &url,
        tmp.path().to_path_buf(),
    );

    // Drive the refresh directly so we don't follow up with a
    // real-network call to googleapis.
    let access = http.access_token().await.expect("token refresh");
    assert_eq!(access, "AT-1");

    let saved = store.saved.lock().unwrap().clone();
    assert_eq!(saved, vec!["RT-NEW".to_string()]);
    assert_eq!(http.refresh_token_for_test().await, "RT-NEW");
}

#[tokio::test]
async fn unchanged_refresh_token_does_not_call_store() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-1","refresh_token":"RT-OLD","expires_in":3600,"token_type":"Bearer"}"#
            .into(),
    )
    .await;

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(RecordingStore::default());
    let http = http_with(
        "RT-OLD",
        Some(store.clone()),
        &url,
        tmp.path().to_path_buf(),
    );

    let access = http.access_token().await.expect("token refresh");
    assert_eq!(access, "AT-1");

    assert!(store.saved.lock().unwrap().is_empty());
    assert_eq!(http.refresh_token_for_test().await, "RT-OLD");
}

/// The Google-style non-rotating case: response carries no
/// `refresh_token`. This is the steady state for Google's flow today
/// and **must not** clobber the existing token.
#[tokio::test]
async fn missing_refresh_token_preserves_existing() {
    let url = one_shot_token_server(
        r#"{"access_token":"AT-1","expires_in":3600,"token_type":"Bearer"}"#.into(),
    )
    .await;

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(RecordingStore::default());
    let http = http_with(
        "RT-OLD",
        Some(store.clone()),
        &url,
        tmp.path().to_path_buf(),
    );

    let access = http.access_token().await.expect("token refresh");
    assert_eq!(access, "AT-1");

    assert!(store.saved.lock().unwrap().is_empty());
    assert_eq!(http.refresh_token_for_test().await, "RT-OLD");
}
