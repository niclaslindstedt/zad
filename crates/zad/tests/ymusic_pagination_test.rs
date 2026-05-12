//! Pagination regression tests for `YmusicHttp`.
//!
//! Mirrors `spotify_pagination_test` but exercises YouTube Data API
//! v3's `pageToken`/`nextPageToken` cursor over
//! `videos?myRating=like`. The pre-fix client issued a single request
//! and silently truncated at the per-page cap (50); the regression is
//! the wholesale truncation of liked-video lists for any user with a
//! larger library.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use zad::service::ymusic::client::YmusicHttp;

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
                let body = if first_line.contains("/token") {
                    serde_json::json!({
                        "access_token": "mock-access",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                    })
                    .to_string()
                } else if first_line.contains("/v3/videos") {
                    let (max_results, page_token) = parse_max_and_token(&first_line);
                    paginated_videos(total, max_results, page_token)
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
        format!("http://127.0.0.1:{port}/token"),
        format!("http://127.0.0.1:{port}/v3"),
        log,
    )
}

fn parse_max_and_token(request_line: &str) -> (u32, Option<u32>) {
    let mut max_results: u32 = 5;
    let mut page_token: Option<u32> = None;
    if let Some(qs) = request_line.split('?').nth(1)
        && let Some(end) = qs.find(' ')
    {
        for pair in qs[..end].split('&') {
            let mut kv = pair.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("maxResults"), Some(v)) => {
                    max_results = v.parse().unwrap_or(max_results);
                }
                (Some("pageToken"), Some(v)) => {
                    page_token = v.parse().ok();
                }
                _ => {}
            }
        }
    }
    (max_results, page_token)
}

fn paginated_videos(total: u32, max_results: u32, page_token: Option<u32>) -> String {
    let offset = page_token.unwrap_or(0);
    let end = (offset + max_results).min(total);
    let items: Vec<serde_json::Value> = (offset..end)
        .map(|i| {
            serde_json::json!({
                "id": format!("video-{i}"),
                "snippet": { "title": format!("Video {i}") },
            })
        })
        .collect();
    let next = if end < total {
        serde_json::Value::String(end.to_string())
    } else {
        serde_json::Value::Null
    };
    serde_json::json!({
        "items": items,
        "nextPageToken": next,
        "pageInfo": { "totalResults": total, "resultsPerPage": max_results },
    })
    .to_string()
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

#[tokio::test]
async fn list_liked_videos_walks_every_page_when_max_is_none() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, log) = mock_server(127).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_liked_videos(None).await.unwrap();
    assert_eq!(items.len(), 127);

    let paths = log.snapshot();
    let list_calls: Vec<&String> = paths.iter().filter(|p| p.contains("/v3/videos")).collect();
    assert_eq!(list_calls.len(), 3, "expected 3 pages: {paths:?}");
    assert!(
        !list_calls[0].contains("pageToken"),
        "first call sends no pageToken: {list_calls:?}"
    );
    assert!(
        list_calls[1].contains("pageToken=50"),
        "second call uses cursor: {list_calls:?}"
    );
    assert!(
        list_calls[2].contains("pageToken=100"),
        "third call uses cursor: {list_calls:?}"
    );
}

#[tokio::test]
async fn list_liked_videos_stops_at_max() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, log) = mock_server(1_000).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_liked_videos(Some(63)).await.unwrap();
    assert_eq!(items.len(), 63);

    let paths = log.snapshot();
    let list_calls: Vec<&String> = paths.iter().filter(|p| p.contains("/v3/videos")).collect();
    assert_eq!(list_calls.len(), 2, "expected 2 pages: {paths:?}");
    assert!(
        list_calls[1].contains("maxResults=13"),
        "second call narrows: {list_calls:?}"
    );
}
