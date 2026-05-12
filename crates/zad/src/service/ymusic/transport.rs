//! Dry-run layer for `zad ymusic`.
//!
//! Mirrors `gcal/transport.rs`: the runtime CLI holds a
//! `Box<dyn YmusicTransport>` so a `--dry-run` invocation never
//! touches the network (or the keychain). The live impl delegates to
//! [`YmusicHttp`]; the preview impl emits [`DryRunOp`] records to a
//! shared sink for every mutating verb. Reads return empty vectors in
//! preview mode by convention.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::Result;
use crate::service::ymusic::client::{
    PlaylistItem, PlaylistSummary, Privacy, SearchItem, VideoSummary, YmusicHttp,
};
use crate::service::{DryRunOp, DryRunSink};

/// Runtime surface of the YouTube Music service. One method per verb
/// reachable from `zad ymusic …`.
#[async_trait]
pub trait YmusicTransport: Send + Sync {
    async fn search(&self, query: &str, types: &[&str], limit: u32) -> Result<Vec<SearchItem>>;
    async fn list_my_playlists(&self, max: Option<u32>) -> Result<Vec<PlaylistSummary>>;
    async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary>;
    async fn get_playlist_items(
        &self,
        playlist_id: &str,
        max: Option<u32>,
    ) -> Result<Vec<PlaylistItem>>;
    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary>;
    async fn rename_playlist(&self, playlist_id: &str, new_title: &str) -> Result<()>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()>;
    async fn add_playlist_item(&self, playlist_id: &str, video_id: &str) -> Result<String>;
    async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()>;
    async fn list_liked_videos(&self, max: Option<u32>) -> Result<Vec<VideoSummary>>;
    async fn like_video(&self, video_id: &str) -> Result<()>;
    async fn unlike_video(&self, video_id: &str) -> Result<()>;
}

#[async_trait]
impl YmusicTransport for YmusicHttp {
    async fn search(&self, query: &str, types: &[&str], limit: u32) -> Result<Vec<SearchItem>> {
        YmusicHttp::search(self, query, types, limit).await
    }
    async fn list_my_playlists(&self, max: Option<u32>) -> Result<Vec<PlaylistSummary>> {
        YmusicHttp::list_my_playlists(self, max).await
    }
    async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        YmusicHttp::get_playlist(self, playlist_id).await
    }
    async fn get_playlist_items(
        &self,
        playlist_id: &str,
        max: Option<u32>,
    ) -> Result<Vec<PlaylistItem>> {
        YmusicHttp::get_playlist_items(self, playlist_id, max).await
    }
    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary> {
        YmusicHttp::create_playlist(self, title, description, privacy).await
    }
    async fn rename_playlist(&self, playlist_id: &str, new_title: &str) -> Result<()> {
        YmusicHttp::rename_playlist(self, playlist_id, new_title).await
    }
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        YmusicHttp::delete_playlist(self, playlist_id).await
    }
    async fn add_playlist_item(&self, playlist_id: &str, video_id: &str) -> Result<String> {
        YmusicHttp::add_playlist_item(self, playlist_id, video_id).await
    }
    async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()> {
        YmusicHttp::remove_playlist_item(self, playlist_item_id).await
    }
    async fn list_liked_videos(&self, max: Option<u32>) -> Result<Vec<VideoSummary>> {
        YmusicHttp::list_liked_videos(self, max).await
    }
    async fn like_video(&self, video_id: &str) -> Result<()> {
        YmusicHttp::like_video(self, video_id).await
    }
    async fn unlike_video(&self, video_id: &str) -> Result<()> {
        YmusicHttp::unlike_video(self, video_id).await
    }
}

/// Preview transport used when the caller passed `--dry-run`. Emits a
/// [`DryRunOp`] for every mutating verb and returns a stub success
/// value. Read verbs return empty vectors so `--dry-run` works even
/// without credentials.
pub struct DryRunYmusicTransport {
    sink: Arc<dyn DryRunSink>,
}

impl DryRunYmusicTransport {
    pub fn new(sink: Arc<dyn DryRunSink>) -> Self {
        Self { sink }
    }

    fn record(&self, verb: &'static str, summary: String, details: serde_json::Value) {
        self.sink.record(DryRunOp {
            service: "ymusic",
            verb,
            summary,
            details,
        });
    }
}

#[async_trait]
impl YmusicTransport for DryRunYmusicTransport {
    async fn search(&self, _q: &str, _types: &[&str], _limit: u32) -> Result<Vec<SearchItem>> {
        Ok(vec![])
    }
    async fn list_my_playlists(&self, _max: Option<u32>) -> Result<Vec<PlaylistSummary>> {
        Ok(vec![])
    }
    async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        Ok(PlaylistSummary {
            id: playlist_id.to_string(),
            snippet: None,
            content_details: None,
            status: None,
        })
    }
    async fn get_playlist_items(
        &self,
        _playlist_id: &str,
        _max: Option<u32>,
    ) -> Result<Vec<PlaylistItem>> {
        Ok(vec![])
    }
    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary> {
        self.record(
            "create_playlist",
            format!("would create playlist `{title}`"),
            json!({
                "command": "ymusic.playlists.create",
                "title": title,
                "description": description,
                "privacy": privacy.as_api_str(),
            }),
        );
        Ok(PlaylistSummary {
            id: "dry-run".into(),
            snippet: None,
            content_details: None,
            status: None,
        })
    }
    async fn rename_playlist(&self, playlist_id: &str, new_title: &str) -> Result<()> {
        self.record(
            "rename_playlist",
            format!("would rename `{playlist_id}` to `{new_title}`"),
            json!({
                "command": "ymusic.playlists.rename",
                "playlist": playlist_id,
                "new_title": new_title,
            }),
        );
        Ok(())
    }
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.record(
            "delete_playlist",
            format!("would delete playlist `{playlist_id}`"),
            json!({
                "command": "ymusic.playlists.delete",
                "playlist": playlist_id,
            }),
        );
        Ok(())
    }
    async fn add_playlist_item(&self, playlist_id: &str, video_id: &str) -> Result<String> {
        self.record(
            "add_playlist_item",
            format!("would add video `{video_id}` to `{playlist_id}`"),
            json!({
                "command": "ymusic.playlists.add",
                "playlist": playlist_id,
                "video_id": video_id,
            }),
        );
        Ok("dry-run".into())
    }
    async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()> {
        self.record(
            "remove_playlist_item",
            format!("would remove playlist item `{playlist_item_id}`"),
            json!({
                "command": "ymusic.playlists.remove",
                "playlist_item_id": playlist_item_id,
            }),
        );
        Ok(())
    }
    async fn list_liked_videos(&self, _max: Option<u32>) -> Result<Vec<VideoSummary>> {
        Ok(vec![])
    }
    async fn like_video(&self, video_id: &str) -> Result<()> {
        self.record(
            "like_video",
            format!("would like video `{video_id}`"),
            json!({
                "command": "ymusic.library.like",
                "video_id": video_id,
            }),
        );
        Ok(())
    }
    async fn unlike_video(&self, video_id: &str) -> Result<()> {
        self.record(
            "unlike_video",
            format!("would unlike video `{video_id}`"),
            json!({
                "command": "ymusic.library.unlike",
                "video_id": video_id,
            }),
        );
        Ok(())
    }
}
