//! InnerTube smoke tests for `YmusicHttp`.
//!
//! The Data API era of this file tested `pageToken` cursor handling
//! over `videos?myRating=like`. InnerTube delivers liked videos via
//! `POST /browse` with `browseId=FEmusic_liked_videos`, so the
//! pagination shape is different — items come back inline in the
//! first response, with optional `continuationContents` for very
//! long libraries. These tests cover the basic POST + parse path;
//! continuation walking is exercised end-to-end through
//! `get_playlist_items` against a real account.
//!
//! Each test spins up a localhost HTTP listener that pretends to be
//! both the token endpoint and InnerTube, so no network access (and
//! no keychain access) is needed.

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

async fn mock_server(items: usize) -> (String, String, Arc<RequestLog>) {
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
                let mut buf = vec![0u8; 16_384];
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
                } else if first_line.contains("/browse") {
                    innertube_liked_videos_response(items)
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
        format!("http://127.0.0.1:{port}"),
        log,
    )
}

/// Build a minimal InnerTube `/browse` response that
/// [`parse_liked_videos`] will accept. We include only the renderer
/// path the parser walks — the real responses ship thousands of
/// additional fields the parser ignores.
fn innertube_liked_videos_response(count: usize) -> String {
    let rows: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            serde_json::json!({
                "musicResponsiveListItemRenderer": {
                    "playlistItemData": { "videoId": format!("video-{i}") },
                    "flexColumns": [
                        {
                            "musicResponsiveListItemFlexColumnRenderer": {
                                "text": { "runs": [ { "text": format!("Song {i}") } ] }
                            }
                        },
                        {
                            "musicResponsiveListItemFlexColumnRenderer": {
                                "text": { "runs": [ { "text": format!("Artist {i}") } ] }
                            }
                        }
                    ]
                }
            })
        })
        .collect();
    serde_json::json!({
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [{
                                    "musicPlaylistShelfRenderer": { "contents": rows }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    })
    .to_string()
}

fn http_pointed_at(token_url: &str, api_base: &str, cache_dir: PathBuf) -> YmusicHttp {
    let mut scopes = BTreeSet::new();
    scopes.insert("library.read".to_string());
    YmusicHttp::new(
        String::new(),
        String::new(),
        "test-refresh".into(),
        scopes,
        PathBuf::new(),
    )
    .with_token_url(token_url)
    .with_api_base(api_base)
    .with_cache_dir(cache_dir)
}

#[tokio::test]
async fn list_liked_videos_returns_parsed_items() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, log) = mock_server(7).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_liked_videos(None).await.unwrap();
    assert_eq!(items.len(), 7);
    assert_eq!(items[0].id, "video-0");
    assert_eq!(items[0].snippet.as_ref().unwrap().title, "Song 0");
    assert_eq!(
        items[0].snippet.as_ref().unwrap().channel_title.as_deref(),
        Some("Artist 0")
    );

    let paths = log.snapshot();
    let browse_calls: Vec<&String> = paths.iter().filter(|p| p.contains("/browse")).collect();
    assert_eq!(browse_calls.len(), 1, "expected one /browse call: {paths:?}");
}

#[tokio::test]
async fn list_liked_videos_honors_client_side_max() {
    let tmp = TempDir::new().unwrap();
    let (token_url, api_base, _log) = mock_server(50).await;
    let http = http_pointed_at(&token_url, &api_base, tmp.path().to_path_buf());

    let items = http.list_liked_videos(Some(13)).await.unwrap();
    assert_eq!(items.len(), 13);
    assert_eq!(items[12].id, "video-12");
}
