//! Dry-run layer for `zad ymusic`.
//!
//! Mirrors `gcal/transport.rs`: the runtime CLI holds a
//! `Box<dyn YmusicTransport>` so a `--dry-run` invocation never
//! touches the network (or the keychain). The live impl delegates to
//! [`YmusicHttp`]; the preview impl emits [`DryRunOp`] records to a
//! shared sink for every mutating verb. Reads return empty vectors in
//! preview mode by convention.
//!
//! Naming follows the Spotify-master contract used at the CLI layer
//! (`name` / `new_name` / `track`) rather than YouTube's wire-level
//! vocabulary (`title` / `video`). The thin wrappers below feed the
//! same values into [`YmusicHttp`], which still uses the Data API
//! field names on the wire.

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
    async fn list_my_playlists(&self, limit: u32) -> Result<Vec<PlaylistSummary>>;
    async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary>;
    async fn get_playlist_items(&self, playlist_id: &str, limit: u32) -> Result<Vec<PlaylistItem>>;
    async fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary>;
    async fn rename_playlist(&self, playlist_id: &str, new_name: &str) -> Result<()>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()>;
    async fn add_playlist_item(&self, playlist_id: &str, track_id: &str) -> Result<String>;
    async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()>;
    async fn list_liked_videos(&self, limit: u32) -> Result<Vec<VideoSummary>>;
    async fn like_video(&self, track_id: &str) -> Result<()>;
    async fn unlike_video(&self, track_id: &str) -> Result<()>;
}

#[async_trait]
impl YmusicTransport for YmusicHttp {
    async fn search(&self, query: &str, types: &[&str], limit: u32) -> Result<Vec<SearchItem>> {
        YmusicHttp::search(self, query, types, limit).await
    }
    async fn list_my_playlists(&self, limit: u32) -> Result<Vec<PlaylistSummary>> {
        YmusicHttp::list_my_playlists(self, limit).await
    }
    async fn get_playlist(&self, playlist_id: &str) -> Result<PlaylistSummary> {
        YmusicHttp::get_playlist(self, playlist_id).await
    }
    async fn get_playlist_items(&self, playlist_id: &str, limit: u32) -> Result<Vec<PlaylistItem>> {
        YmusicHttp::get_playlist_items(self, playlist_id, limit).await
    }
    async fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary> {
        YmusicHttp::create_playlist(self, name, description, privacy).await
    }
    async fn rename_playlist(&self, playlist_id: &str, new_name: &str) -> Result<()> {
        YmusicHttp::rename_playlist(self, playlist_id, new_name).await
    }
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        YmusicHttp::delete_playlist(self, playlist_id).await
    }
    async fn add_playlist_item(&self, playlist_id: &str, track_id: &str) -> Result<String> {
        YmusicHttp::add_playlist_item(self, playlist_id, track_id).await
    }
    async fn remove_playlist_item(&self, playlist_item_id: &str) -> Result<()> {
        YmusicHttp::remove_playlist_item(self, playlist_item_id).await
    }
    async fn list_liked_videos(&self, limit: u32) -> Result<Vec<VideoSummary>> {
        YmusicHttp::list_liked_videos(self, limit).await
    }
    async fn like_video(&self, track_id: &str) -> Result<()> {
        YmusicHttp::like_video(self, track_id).await
    }
    async fn unlike_video(&self, track_id: &str) -> Result<()> {
        YmusicHttp::unlike_video(self, track_id).await
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
    async fn list_my_playlists(&self, _limit: u32) -> Result<Vec<PlaylistSummary>> {
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
        _limit: u32,
    ) -> Result<Vec<PlaylistItem>> {
        Ok(vec![])
    }
    async fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
        privacy: Privacy,
    ) -> Result<PlaylistSummary> {
        self.record(
            "create_playlist",
            format!("would create playlist `{name}`"),
            json!({
                "command": "ymusic.playlists.create",
                "name": name,
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
    async fn rename_playlist(&self, playlist_id: &str, new_name: &str) -> Result<()> {
        self.record(
            "rename_playlist",
            format!("would rename `{playlist_id}` to `{new_name}`"),
            json!({
                "command": "ymusic.playlists.rename",
                "playlist": playlist_id,
                "new_name": new_name,
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
    async fn add_playlist_item(&self, playlist_id: &str, track_id: &str) -> Result<String> {
        self.record(
            "add_playlist_item",
            format!("would add track `{track_id}` to `{playlist_id}`"),
            json!({
                "command": "ymusic.playlists.add",
                "playlist": playlist_id,
                "track_id": track_id,
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
                "item_id": playlist_item_id,
            }),
        );
        Ok(())
    }
    async fn list_liked_videos(&self, _limit: u32) -> Result<Vec<VideoSummary>> {
        Ok(vec![])
    }
    async fn like_video(&self, track_id: &str) -> Result<()> {
        self.record(
            "like_video",
            format!("would save track `{track_id}`"),
            json!({
                "command": "ymusic.library.tracks.save",
                "track_id": track_id,
            }),
        );
        Ok(())
    }
    async fn unlike_video(&self, track_id: &str) -> Result<()> {
        self.record(
            "unlike_video",
            format!("would unsave track `{track_id}`"),
            json!({
                "command": "ymusic.library.tracks.unsave",
                "track_id": track_id,
            }),
        );
        Ok(())
    }
}
