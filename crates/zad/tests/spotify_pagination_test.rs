//! Pagination regression tests for `SpotifyHttp`.
//!
//! The four list endpoints (`list_my_playlists`, `list_saved_tracks`,
//! `list_saved_albums`, `get_playlist_tracks`) walk Spotify's
//! `offset`/`limit` cursor under the hood. Before this work each one
//! issued a single request and silently truncated at the per-page cap;
//! the regression is the wholesale truncation of `spotifai export` for
//! any user with more than 50 liked tracks or playlists.
//!
//! We stand up a localhost HTTP server that impersonates both
//! `accounts.spotify.com/api/token` (one-shot OAuth refresh) and
//! `api.spotify.com/v1/me/tracks` (multi-page list), point a
//! `SpotifyHttp` at it via the test-only `with_token_url` and
//! `with_api_base` hooks, and assert that:
//!
//! 1. `list_saved_tracks(None)` walks the entire cursor and returns
//!    every item.
//! 2. `list_saved_tracks(Some(N))` stops early when `N` is satisfied.
//! 3. The `offset` query parameter increments page-over-page.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use zad::service::spotify::client::SpotifyHttp;

/// In-memory record of every request the mock server received.
#[derive(Default)]
struct RequestLog {
    paths: Mutex<Vec<String>>,
}

impl RequestLog {
    fn push(&self, path: String) {
        self.paths.lock().unwrap().push(path);
    }

    fn snapshot(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

/// Stand up a long-running HTTP/1.1 server on `127.0.0.1` that serves
/// both the OAuth token endpoint and a paginated `/v1/me/tracks`
/// endpoint sized to `total` items. Returns `(token_url, api_base,
/// log)`.
async fn mock_server(total: u32) -> (String, String, Arc<RequestLog>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(RequestLog::default());
    let log_clone = log.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let log = log_clone.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = match stream.read(&mut buf).await {
                    Ok(n) => n,
                    Err(_) => return,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let first_line = req.lines().next().unwrap_or("").to_string();
                log.push(first_line.clone());
                let body = if first_line.contains("/api/token") {
                    // OAuth refresh — return a static access token.
                    serde_json::json!({
                        "access_token": "mock-access",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                    })
                    .to_string()
                } else if first_line.contains("/v1/me/tracks") {
                    let (limit, offset) = parse_limit_offset(&first_line);
                    paginated_saved_tracks(total, limit, offset)
                } else {
                    "{}".to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (
        format!("http://127.0.0.1:{port}/api/token"),
        format!("http://127.0.0.1:{port}/v1"),
        log,
    )
}

fn parse_limit_offset(request_line: &str) -> (u32, u32) {
    // first_line: "GET /v1/me/tracks?limit=50&offset=0 HTTP/1.1"
    let mut limit = 20;
    let mut offset = 0;
    if let Some(qs) = request_line.split('?').nth(1)
        && let Some(end) = qs.find(' ')
    {
        for pair in qs[..end].split('&') {
            let mut kv = pair.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("limit"), Some(v)) => limit = v.parse().unwrap_or(limit),
                (Some("offset"), Some(v)) => offset = v.parse().unwrap_or(offset),
                _ => {}
            }
        }
    }
    (limit, offset)
}

fn paginated_saved_tracks(total: u32, limit: u32, offset: u32) -> String {
    let end = (offset + limit).min(total);
    let items: Vec<serde_json::Value> = (offset..end)
        .map(|i| {
            serde_json::json!({
                "added_at": "2026-01-01T00:00:00Z",
                "track": {
                    "id": format!("track-{i}"),
                    "name": format!("Track {i}"),
                    "uri": format!("spotify:track:track-{i}"),
                    "artists": [],
                },
            })
        })
        .collect();
    serde_json::json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
        "next": if end < total { serde_json::Value::String("next".into()) } else { serde_json::Value::Null },
    })
    .to_string()
}

fn http_pointed_at(token_url: &str, api_base: &str, cache_dir: PathBuf) -> SpotifyHttp {
    let mut scopes = BTreeSet::new();
    scopes.insert("library.read".to_string());
    SpotifyHttp::new(
        "test-client".into(),
        "test-refresh".into(),
        scopes,
        PathBuf::new(),
    )
    .with_token_url(token_url)
    .with_api_base(api_base)
    .with_cache_dir(cache_dir)
}

#[tokio::test]
async fn list_saved_tracks_walks_every_page_when_max_is_none() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, log) = mock_server(133).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_saved_tracks(None).await.unwrap();
    assert_eq!(items.len(), 133);

    let paths = log.snapshot();
    let list_calls: Vec<&String> = paths
        .iter()
        .filter(|p| p.contains("/v1/me/tracks"))
        .collect();
    assert_eq!(
        list_calls.len(),
        3,
        "expected 3 pages (50+50+33): {paths:?}"
    );
    assert!(
        list_calls[0].contains("offset=0"),
        "first page: {list_calls:?}"
    );
    assert!(
        list_calls[1].contains("offset=50"),
        "second page: {list_calls:?}"
    );
    assert!(
        list_calls[2].contains("offset=100"),
        "third page: {list_calls:?}"
    );
}

#[tokio::test]
async fn list_saved_tracks_stops_at_max_when_capped() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, log) = mock_server(1_000).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_saved_tracks(Some(75)).await.unwrap();
    assert_eq!(items.len(), 75);

    let paths = log.snapshot();
    let list_calls: Vec<&String> = paths
        .iter()
        .filter(|p| p.contains("/v1/me/tracks"))
        .collect();
    assert_eq!(list_calls.len(), 2, "expected 2 pages (50+25): {paths:?}");
    assert!(
        list_calls[1].contains("limit=25"),
        "second page narrows: {list_calls:?}"
    );
}

#[tokio::test]
async fn list_saved_tracks_one_page_when_corpus_smaller_than_page() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, log) = mock_server(7).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_saved_tracks(None).await.unwrap();
    assert_eq!(items.len(), 7);

    let paths = log.snapshot();
    let list_calls: Vec<&String> = paths
        .iter()
        .filter(|p| p.contains("/v1/me/tracks"))
        .collect();
    assert_eq!(list_calls.len(), 1, "expected 1 page: {paths:?}");
}
