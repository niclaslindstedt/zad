//! `zad ymusic <verb>` — runtime surface for the YouTube Music
//! service.
//!
//! YouTube Music shares this CLI surface with Spotify: the same
//! verbs, the same arguments, and the same JSON / human output shape.
//! Spotify is the master contract; concepts that exist on both
//! providers are named the same here. The only places this surface
//! diverges from `zad spotify` are where YouTube has a genuinely
//! different concept (e.g. `--privacy` instead of `--public`, since
//! YouTube has an `unlisted` state Spotify does not have) — those
//! deviations show up as additional fields, not as renames of shared
//! concepts.
//!
//! Wires together:
//! - per-verb clap args (`search`, `playlists list/show/create/
//!   rename/delete/add/remove`, `library tracks {list,save,unsave}`,
//!   plus the mandatory `permissions` subgroup);
//! - credential + scope resolution from the effective config (local
//!   wins over global);
//! - permission gating (time window → target → content) executed
//!   **before** any network call;
//! - `--dry-run` for mutating verbs via the
//!   [`crate::service::ymusic::YmusicTransport`] indirection so
//!   previews never touch the network or the keychain.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::lifecycle::leak;
use crate::config::{self, YmusicServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::default_dry_run_sink;
use crate::service::ymusic::client::{
    PlaylistItem, PlaylistSummary, Privacy, SearchItem, VideoSummary, YmusicHttp,
};
use crate::service::ymusic::permissions::{self as perms, YmusicFunction};
use crate::service::ymusic::transport::{DryRunYmusicTransport, YmusicTransport};

// ---------------------------------------------------------------------------
// top-level args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct YmusicArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Search YouTube (videos, playlists, channels). YouTube Music
    /// shares the Data API surface — songs are videos with the
    /// `topicId` set to `Music`, but vanilla `video` queries cover
    /// most cases.
    Search(SearchArgs),
    /// Playlist management (list, show, create, rename, delete, add, remove).
    Playlists(PlaylistsArgs),
    /// Library management (saved tracks).
    Library(LibraryArgs),
    /// Inspect or scaffold the permissions policy.
    Permissions(PermissionsArgs),
}

// ---------------------------------------------------------------------------
// `zad ymusic search …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Free-text query.
    pub query: String,
    /// One or more entity types (`video`, `playlist`, `channel`).
    /// Repeatable. Defaults to `video`.
    #[arg(long = "type", value_parser = ["video", "playlist", "channel"], default_values = ["video"])]
    pub types: Vec<String>,
    /// Page size (1..=50). YouTube caps every request at 50 items.
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad ymusic playlists …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PlaylistsArgs {
    #[command(subcommand)]
    pub action: PlaylistsAction,
}

#[derive(Debug, Subcommand)]
pub enum PlaylistsAction {
    /// List the authenticated user's playlists.
    List(PlaylistsListArgs),
    /// Show one playlist's metadata and tracks.
    Show(PlaylistsShowArgs),
    /// Create a new playlist owned by the authenticated user.
    Create(PlaylistsCreateArgs),
    /// Rename an existing playlist.
    Rename(PlaylistsRenameArgs),
    /// Delete a playlist owned by the user.
    Delete(PlaylistsDeleteArgs),
    /// Add one or more tracks to a playlist.
    Add(PlaylistsAddArgs),
    /// Remove one or more tracks from a playlist.
    Remove(PlaylistsRemoveArgs),
}

#[derive(Debug, Args)]
pub struct PlaylistsListArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsShowArgs {
    /// Playlist ID, full YouTube URL, or — when previously listed —
    /// the literal name of an owned playlist.
    pub playlist: Option<String>,
    /// Page size for the tracks listing (1..=50).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsCreateArgs {
    /// Display name for the new playlist.
    pub name: String,
    #[arg(long)]
    pub description: Option<String>,
    /// Privacy: `private` (default), `unlisted`, or `public`. YouTube
    /// has an `unlisted` state Spotify does not, so this verb takes a
    /// three-valued enum instead of Spotify's `--public` boolean.
    #[arg(long, value_parser = ["private", "unlisted", "public"], default_value = "private")]
    pub privacy: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsRenameArgs {
    pub playlist: String,
    pub new_name: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsDeleteArgs {
    pub playlist: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsAddArgs {
    /// Target playlist (ID, URL, or owned-playlist name).
    pub playlist: String,
    /// One or more video IDs (or full YouTube URLs).
    #[arg(required = true)]
    pub tracks: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsRemoveArgs {
    pub playlist: String,
    /// One or more video IDs *or* playlistItem IDs. When a video ID
    /// is supplied, zad lists the playlist to find the matching
    /// item; if the same video appears multiple times, every match
    /// is removed.
    #[arg(required = true)]
    pub tracks: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad ymusic library …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct LibraryArgs {
    #[command(subcommand)]
    pub action: LibraryAction,
}

#[derive(Debug, Subcommand)]
pub enum LibraryAction {
    /// Saved-tracks operations (mirrors `spotify library tracks`).
    /// YouTube has no album library, so the `albums` sub-namespace
    /// from `spotify library` is intentionally absent here.
    Tracks(LibraryTracksArgs),
}

#[derive(Debug, Args)]
pub struct LibraryTracksArgs {
    #[command(subcommand)]
    pub action: LibraryItemAction,
}

#[derive(Debug, Subcommand)]
pub enum LibraryItemAction {
    /// List saved tracks.
    List(LibraryListArgs),
    /// Save (like) one or more tracks.
    Save(LibraryMutateArgs),
    /// Unsave (unlike) one or more tracks.
    Unsave(LibraryMutateArgs),
}

#[derive(Debug, Args)]
pub struct LibraryListArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LibraryMutateArgs {
    /// One or more video IDs (or full YouTube URLs).
    #[arg(required = true)]
    pub tracks: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad ymusic permissions …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum PermissionsAction {
    /// Print the effective policy (both file paths + bodies).
    Show(PermissionsShowArgs),
    /// Print the two candidate file paths, one per line.
    Path(PermissionsPathArgs),
    /// Write a starter policy to the selected scope.
    Init(PermissionsInitArgs),
    /// Dry-run a permissions check without hitting the network.
    Check(PermissionsCheckArgs),
    /// Staged-commit workflow: queue mutations in a `.pending` file
    /// and only sign on `commit`. See `cli::permissions`.
    #[command(flatten)]
    Staging(crate::cli::permissions::StagingAction),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsPathArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsInitArgs {
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsCheckArgs {
    /// Function name: `search`, `playlists_read`, `playlists_write`,
    /// `library_read`, or `library_write`.
    #[arg(long)]
    pub function: String,
    /// Target to check against the function's `targets` list — a
    /// playlist name/ID, a video ID, or a search query.
    #[arg(long)]
    pub target: Option<String>,
    /// Body text to evaluate against the function's content rules
    /// (e.g. a search query, a playlist description).
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

pub async fn run(args: YmusicArgs) -> Result<()> {
    match args.action {
        Action::Search(a) => run_search(a).await,
        Action::Playlists(a) => match a.action {
            PlaylistsAction::List(a) => run_playlists_list(a).await,
            PlaylistsAction::Show(a) => run_playlists_show(a).await,
            PlaylistsAction::Create(a) => run_playlists_create(a).await,
            PlaylistsAction::Rename(a) => run_playlists_rename(a).await,
            PlaylistsAction::Delete(a) => run_playlists_delete(a).await,
            PlaylistsAction::Add(a) => run_playlists_add(a).await,
            PlaylistsAction::Remove(a) => run_playlists_remove(a).await,
        },
        Action::Library(a) => match a.action {
            LibraryAction::Tracks(t) => match t.action {
                LibraryItemAction::List(a) => run_library_tracks_list(a).await,
                LibraryItemAction::Save(a) => run_library_tracks_mutate(a, true).await,
                LibraryItemAction::Unsave(a) => run_library_tracks_mutate(a, false).await,
            },
        },
        Action::Permissions(a) => run_permissions(a),
    }
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

async fn run_search(args: SearchArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::Search)?;
    permissions.check_target(YmusicFunction::Search, &args.query)?;
    permissions.check_body(YmusicFunction::Search, &args.query)?;

    let transport = transport_for(false)?;
    let types: Vec<&str> = args.types.iter().map(|s| s.as_str()).collect();
    let items = transport.search(&args.query, &types, args.limit).await?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&render_search(&items)).unwrap()
        );
        return Ok(());
    }
    print_search_human(&items);
    Ok(())
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    videos: Option<Vec<VideoSearchOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    playlists: Option<Vec<PlaylistSearchOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channels: Option<Vec<ChannelSearchOut>>,
}

#[derive(Debug, Serialize)]
struct VideoSearchOut {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
struct PlaylistSearchOut {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChannelSearchOut {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

fn render_search(items: &[SearchItem]) -> SearchOutput {
    let mut videos: Vec<VideoSearchOut> = Vec::new();
    let mut playlists: Vec<PlaylistSearchOut> = Vec::new();
    let mut channels: Vec<ChannelSearchOut> = Vec::new();
    for i in items {
        let Some(id_block) = i.id.as_ref() else {
            continue;
        };
        let snippet = i.snippet.as_ref();
        let name = snippet
            .and_then(|s| s.title.clone())
            .unwrap_or_else(|| "(no title)".to_string());
        if let Some(v) = id_block.video_id.as_ref() {
            videos.push(VideoSearchOut {
                id: v.clone(),
                name,
                channel: snippet.and_then(|s| s.channel_title.clone()),
            });
        } else if let Some(p) = id_block.playlist_id.as_ref() {
            playlists.push(PlaylistSearchOut {
                id: p.clone(),
                name,
                owner: snippet.and_then(|s| s.channel_title.clone()),
            });
        } else if let Some(c) = id_block.channel_id.as_ref() {
            channels.push(ChannelSearchOut {
                id: c.clone(),
                name,
                description: snippet.and_then(|s| s.description.clone()),
            });
        }
    }
    SearchOutput {
        videos: if videos.is_empty() {
            None
        } else {
            Some(videos)
        },
        playlists: if playlists.is_empty() {
            None
        } else {
            Some(playlists)
        },
        channels: if channels.is_empty() {
            None
        } else {
            Some(channels)
        },
    }
}

fn print_search_human(items: &[SearchItem]) {
    let out = render_search(items);
    if out.videos.is_none() && out.playlists.is_none() && out.channels.is_none() {
        println!("No results.");
        return;
    }
    if let Some(vs) = &out.videos {
        println!("Videos");
        for v in vs {
            let channel = v.channel.as_deref().unwrap_or("");
            println!("  {:25}  {} [{channel}]", v.id, v.name);
        }
    }
    if let Some(ps) = &out.playlists {
        println!("Playlists");
        for p in ps {
            let owner = p.owner.as_deref().unwrap_or("?");
            println!("  {:25}  {} (by {owner})", p.id, p.name);
        }
    }
    if let Some(cs) = &out.channels {
        println!("Channels");
        for c in cs {
            println!("  {:25}  {}", c.id, c.name);
        }
    }
}

// ---------------------------------------------------------------------------
// playlists
// ---------------------------------------------------------------------------

async fn run_playlists_list(args: PlaylistsListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsRead)?;

    let transport = transport_for(false)?;
    let items = transport.list_my_playlists(args.limit).await?;
    let filtered: Vec<&PlaylistSummary> = items
        .iter()
        .filter(|p| {
            let name = p.snippet.as_ref().map(|s| s.title.as_str()).unwrap_or("");
            permissions
                .check_target(YmusicFunction::PlaylistsRead, name)
                .is_ok()
        })
        .collect();

    let out: Vec<PlaylistSummaryOut> = filtered.iter().map(|p| render_playlist(p)).collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    if out.is_empty() {
        println!("No playlists visible (or all filtered by permissions).");
        return Ok(());
    }
    for p in &out {
        let total = p.tracks.as_ref().and_then(|t| t.total).unwrap_or(0);
        let owner = p
            .owner
            .as_ref()
            .map(|o| o.display_name.as_deref().unwrap_or(o.id.as_str()))
            .unwrap_or("?");
        println!("  {:25}  {} ({total} tracks, by {owner})", p.id, p.name);
    }
    Ok(())
}

async fn run_playlists_show(args: PlaylistsShowArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsRead)?;

    let (cfg, _label, _scope, _path) = effective_config()?;
    let raw = playlist_target(args.playlist.as_deref(), cfg.default_playlist.as_deref())?;
    permissions.check_target(YmusicFunction::PlaylistsRead, &raw)?;
    let resolved = strip_playlist_url(&raw);

    let transport = transport_for(false)?;
    let summary = transport.get_playlist(&resolved).await?;
    let items = transport.get_playlist_items(&resolved, args.limit).await?;
    let summary_out = render_playlist(&summary);
    let tracks_out: Vec<PlaylistTrackItemOut> = items.iter().map(render_playlist_track).collect();

    if args.json {
        let out = serde_json::json!({ "playlist": summary_out, "tracks": tracks_out });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("id          : {}", summary_out.id);
    println!("name        : {}", summary_out.name);
    if let Some(d) = &summary_out.description {
        if !d.is_empty() {
            println!("description : {d}");
        }
    }
    if let Some(o) = &summary_out.owner {
        println!(
            "owner       : {}",
            o.display_name.as_deref().unwrap_or(o.id.as_str())
        );
    }
    if let Some(p) = &summary_out.privacy {
        println!("privacy     : {p}");
    }
    println!("tracks      :");
    for t in &tracks_out {
        print_playlist_track_human(t);
    }
    Ok(())
}

fn print_playlist_track_human(item: &PlaylistTrackItemOut) {
    let Some(track) = &item.track else {
        return;
    };
    let channel = track.channel.as_deref().unwrap_or("");
    println!("  {:25}  {} — {channel}", track.id, track.name);
}

async fn run_playlists_create(args: PlaylistsCreateArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.name)?;
    permissions.check_body(YmusicFunction::PlaylistsWrite, &args.name)?;
    if let Some(d) = &args.description {
        permissions.check_body(YmusicFunction::PlaylistsWrite, d)?;
    }

    let privacy = parse_privacy(&args.privacy)?;
    let transport = transport_for(args.dry_run)?;
    let summary = transport
        .create_playlist(&args.name, args.description.as_deref(), privacy)
        .await?;

    if args.dry_run {
        return Ok(());
    }
    let out = render_playlist(&summary);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Created playlist `{}` (id={})", out.name, out.id);
    }
    Ok(())
}

async fn run_playlists_rename(args: PlaylistsRenameArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.playlist)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.new_name)?;
    permissions.check_body(YmusicFunction::PlaylistsWrite, &args.new_name)?;

    let resolved = strip_playlist_url(&args.playlist);
    let transport = transport_for(args.dry_run)?;
    transport.rename_playlist(&resolved, &args.new_name).await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({ "id": resolved, "new_name": args.new_name });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Renamed `{resolved}` → `{}`", args.new_name);
    }
    Ok(())
}

async fn run_playlists_delete(args: PlaylistsDeleteArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.playlist)?;

    let resolved = strip_playlist_url(&args.playlist);
    let transport = transport_for(args.dry_run)?;
    transport.delete_playlist(&resolved).await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({ "id": resolved, "deleted": true });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Deleted playlist `{resolved}`");
    }
    Ok(())
}

async fn run_playlists_add(args: PlaylistsAddArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.playlist)?;
    for v in &args.tracks {
        permissions.check_target(YmusicFunction::PlaylistsWrite, v)?;
    }

    let resolved = strip_playlist_url(&args.playlist);
    let video_ids: Vec<String> = args.tracks.iter().map(|v| extract_video_id(v)).collect();
    let transport = transport_for(args.dry_run)?;
    let mut item_ids: Vec<String> = Vec::with_capacity(video_ids.len());
    for vid in &video_ids {
        let item_id = transport.add_playlist_item(&resolved, vid).await?;
        item_ids.push(item_id);
    }

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({
            "id": resolved,
            "added": video_ids,
            "item_ids": item_ids,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Added {} track(s) to `{resolved}`", video_ids.len());
    }
    Ok(())
}

async fn run_playlists_remove(args: PlaylistsRemoveArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.playlist)?;
    for v in &args.tracks {
        permissions.check_target(YmusicFunction::PlaylistsWrite, v)?;
    }

    let resolved = strip_playlist_url(&args.playlist);
    let transport = transport_for(args.dry_run)?;

    // Resolve video IDs to playlistItem IDs by listing the playlist
    // once. Anything that already looks like a playlistItem ID
    // is just attempted as-is and we let the API surface a 404 if
    // it's not a real item.
    let listing: Option<Vec<PlaylistItem>> = if args.tracks.iter().any(|s| is_likely_video_id(s)) {
        Some(transport.get_playlist_items(&resolved, 50).await?)
    } else {
        None
    };

    let mut removed: Vec<String> = Vec::new();
    let mut item_ids: Vec<String> = Vec::new();
    for raw in &args.tracks {
        let candidate = extract_video_id(raw);
        if is_likely_video_id(raw) {
            // Map a video ID to every matching playlistItem ID.
            if let Some(list) = listing.as_ref() {
                let matches: Vec<&PlaylistItem> = list
                    .iter()
                    .filter(|it| {
                        it.content_details
                            .as_ref()
                            .and_then(|c| c.video_id.as_deref())
                            == Some(candidate.as_str())
                            || it
                                .snippet
                                .as_ref()
                                .and_then(|s| s.resource_id.as_ref())
                                .and_then(|r| r.video_id.as_deref())
                                == Some(candidate.as_str())
                    })
                    .collect();
                if matches.is_empty() {
                    return Err(ZadError::Service {
                        name: "ymusic",
                        message: format!("video `{candidate}` is not in playlist `{resolved}`"),
                    });
                }
                for m in matches {
                    transport.remove_playlist_item(&m.id).await?;
                    item_ids.push(m.id.clone());
                    removed.push(candidate.clone());
                }
            }
        } else {
            // Treat raw as an opaque playlistItem ID.
            transport.remove_playlist_item(raw).await?;
            item_ids.push(raw.clone());
            removed.push(raw.clone());
        }
    }

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({
            "id": resolved,
            "removed": removed,
            "item_ids": item_ids,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Removed {} track(s) from `{resolved}`", removed.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// library
// ---------------------------------------------------------------------------

async fn run_library_tracks_list(args: LibraryListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::LibraryRead)?;

    let transport = transport_for(false)?;
    let items = transport.list_liked_videos(args.limit).await?;
    let filtered: Vec<&VideoSummary> = items
        .iter()
        .filter(|v| {
            permissions
                .check_target(YmusicFunction::LibraryRead, &v.id)
                .is_ok()
        })
        .collect();

    let out: Vec<SavedTrackOut> = filtered
        .iter()
        .map(|v| SavedTrackOut {
            added_at: None,
            track: render_video(v),
        })
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    if out.is_empty() {
        println!("No saved tracks (or all filtered by permissions).");
        return Ok(());
    }
    for s in &out {
        let channel = s.track.channel.as_deref().unwrap_or("");
        println!("  {:25}  {} — {channel}", s.track.id, s.track.name);
    }
    Ok(())
}

async fn run_library_tracks_mutate(args: LibraryMutateArgs, save: bool) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::LibraryWrite)?;
    for v in &args.tracks {
        permissions.check_target(YmusicFunction::LibraryWrite, v)?;
    }

    let ids: Vec<String> = args.tracks.iter().map(|v| extract_video_id(v)).collect();
    let transport = transport_for(args.dry_run)?;
    for id in &ids {
        if save {
            transport.like_video(id).await?;
        } else {
            transport.unlike_video(id).await?;
        }
    }

    if args.dry_run {
        return Ok(());
    }
    let verb = if save { "saved" } else { "unsaved" };
    if args.json {
        let out = serde_json::json!({ verb: ids });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{verb} {} track(s)", ids.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// permissions subgroup — show / path / init / check
// ---------------------------------------------------------------------------

fn run_permissions(args: PermissionsArgs) -> Result<()> {
    match args.action {
        None => run_permissions_show(PermissionsShowArgs { json: args.json }),
        Some(PermissionsAction::Show(a)) => run_permissions_show(a),
        Some(PermissionsAction::Path(a)) => run_permissions_path(a),
        Some(PermissionsAction::Init(a)) => run_permissions_init(a),
        Some(PermissionsAction::Check(a)) => run_permissions_check(a),
        Some(PermissionsAction::Staging(a)) => {
            crate::cli::permissions::run::<perms::PermissionsService>(a)
        }
    }
}

#[derive(Debug, Serialize)]
struct PermissionsScopeOut {
    path: String,
    present: bool,
}

#[derive(Debug, Serialize)]
struct PermissionsShowOut {
    command: &'static str,
    global: PermissionsScopeOut,
    local: PermissionsScopeOut,
}

fn run_permissions_show(args: PermissionsShowArgs) -> Result<()> {
    let global_path = perms::global_path()?;
    let local_path = perms::local_path_current()?;
    let _ = perms::load_effective()?;

    if args.json {
        let out = PermissionsShowOut {
            command: "ymusic.permissions.show",
            global: PermissionsScopeOut {
                path: global_path.display().to_string(),
                present: global_path.exists(),
            },
            local: PermissionsScopeOut {
                path: local_path.display().to_string(),
                present: local_path.exists(),
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("YouTube Music permissions");
    print_scope_block("global", &global_path);
    print_scope_block("local", &local_path);
    Ok(())
}

fn print_scope_block(label: &str, path: &Path) {
    println!();
    println!("  [{label}] {}", path.display());
    if !path.exists() {
        println!("    (no file at this scope)");
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(body) => {
            for line in body.lines() {
                println!("    {line}");
            }
        }
        Err(e) => println!("    (could not read: {e})"),
    }
}

fn run_permissions_path(args: PermissionsPathArgs) -> Result<()> {
    let global_path = perms::global_path()?;
    let local_path = perms::local_path_current()?;
    if args.json {
        let out = serde_json::json!({
            "command": "ymusic.permissions.path",
            "global": global_path.display().to_string(),
            "local": local_path.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{}", global_path.display());
        println!("{}", local_path.display());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsInitOutput {
    command: &'static str,
    scope: &'static str,
    path: String,
    written: bool,
}

fn run_permissions_init(args: PermissionsInitArgs) -> Result<()> {
    let (path, scope) = if args.local {
        (perms::local_path_current()?, "local")
    } else {
        (perms::global_path()?, "global")
    };
    if path.exists() && !args.force {
        return Err(ZadError::Invalid(format!(
            "permissions file already exists at {}. Pass --force to overwrite.",
            path.display()
        )));
    }
    let template = perms::starter_template();
    let key = crate::permissions::signing::load_or_create_from_keychain()?;
    crate::permissions::signing::write_public_key_cache(&key)?;
    perms::save_file(&path, &template, &key)?;
    if args.json {
        let out = PermissionsInitOutput {
            command: "ymusic.permissions.init",
            scope,
            path: path.display().to_string(),
            written: true,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Wrote starter permissions ({scope}): {}", path.display());
        println!("Signed with key {}.", key.fingerprint());
        println!("Review it; the defaults deny `*release*`/`*official*` playlists.");
    }
    Ok(())
}

fn run_permissions_check(args: PermissionsCheckArgs) -> Result<()> {
    let f = parse_function(&args.function)?;
    let permissions = perms::load_effective()?;

    permissions.check_time(f)?;
    if let Some(t) = &args.target {
        permissions.check_target(f, t)?;
    }
    if let Some(b) = &args.body {
        permissions.check_body(f, b)?;
    }

    if args.json {
        let out = serde_json::json!({
            "command": "ymusic.permissions.check",
            "function": args.function,
            "ok": true,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("✓ would be allowed by current ymusic permissions");
    }
    Ok(())
}

fn parse_function(name: &str) -> Result<YmusicFunction> {
    Ok(match name {
        "search" => YmusicFunction::Search,
        "playlists_read" => YmusicFunction::PlaylistsRead,
        "playlists_write" => YmusicFunction::PlaylistsWrite,
        "library_read" => YmusicFunction::LibraryRead,
        "library_write" => YmusicFunction::LibraryWrite,
        other => {
            return Err(ZadError::Invalid(format!(
                "unknown function `{other}`; expected one of: search, playlists_read, \
                 playlists_write, library_read, library_write"
            )));
        }
    })
}

// ---------------------------------------------------------------------------
// output normalisation — flatten YouTube's nested API shapes to the
// Spotify-shaped contract that `zad spotify` emits. Spotify is the
// master contract; YouTube-only concepts surface as additional
// fields (`privacy`, `item_id`).
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PlaylistSummaryOut {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<UserRefOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tracks: Option<TracksRefOut>,
    /// YouTube has an `unlisted` state Spotify does not — surface it
    /// as an extra field rather than overloading `public`.
    #[serde(skip_serializing_if = "Option::is_none")]
    privacy: Option<String>,
}

#[derive(Debug, Serialize)]
struct UserRefOut {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct TracksRefOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PlaylistTrackItemOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    track: Option<TrackOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    added_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct TrackOut {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    /// YouTube playlist entries have an item_id distinct from the
    /// video id; Spotify has no such concept. Set on tracks coming
    /// from a playlist, omitted on free-floating tracks (search,
    /// library list).
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct SavedTrackOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    added_at: Option<String>,
    track: TrackOut,
}

fn render_playlist(p: &PlaylistSummary) -> PlaylistSummaryOut {
    let snippet = p.snippet.as_ref();
    let name = snippet
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "(no title)".to_string());
    let description = snippet.and_then(|s| s.description.clone());
    let owner = snippet.and_then(|s| {
        s.channel_id.as_ref().map(|cid| UserRefOut {
            id: cid.clone(),
            display_name: s.channel_title.clone(),
        })
    });
    let total = p.content_details.as_ref().and_then(|c| c.item_count);
    let privacy = p.status.as_ref().and_then(|s| s.privacy_status.clone());
    let public = privacy.as_deref().and_then(|s| match s {
        "public" => Some(true),
        "private" => Some(false),
        _ => None,
    });
    PlaylistSummaryOut {
        id: p.id.clone(),
        name,
        description,
        public,
        owner,
        tracks: Some(TracksRefOut { total }),
        privacy,
    }
}

fn render_playlist_track(item: &PlaylistItem) -> PlaylistTrackItemOut {
    let snippet = item.snippet.as_ref();
    let video_id = item
        .content_details
        .as_ref()
        .and_then(|c| c.video_id.clone())
        .or_else(|| {
            snippet
                .and_then(|s| s.resource_id.as_ref())
                .and_then(|r| r.video_id.clone())
        })
        .unwrap_or_else(|| "?".to_string());
    let name = snippet
        .and_then(|s| s.title.clone())
        .unwrap_or_else(|| "(no title)".to_string());
    let channel = snippet.and_then(|s| s.video_owner_channel_title.clone());
    PlaylistTrackItemOut {
        track: Some(TrackOut {
            id: video_id,
            name,
            channel,
            item_id: Some(item.id.clone()),
        }),
        added_at: None,
    }
}

fn render_video(v: &VideoSummary) -> TrackOut {
    let snippet = v.snippet.as_ref();
    let name = snippet
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "(no title)".to_string());
    let channel = snippet.and_then(|s| s.channel_title.clone());
    TrackOut {
        id: v.id.clone(),
        name,
        channel,
        item_id: None,
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

pub(crate) fn effective_config() -> Result<(YmusicServiceCfg, &'static str, Scope<'static>, PathBuf)>
{
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "ymusic")?;
    let global_path = config::path::global_service_config_path("ymusic")?;

    let project_cfg = config::load()?;
    if !project_cfg.has_service("ymusic") {
        return Err(ZadError::Invalid(format!(
            "ymusic is not enabled for this project ({}). \
             Run `zad service enable ymusic` first.",
            config::path::project_config_path()?.display()
        )));
    }

    if let Some(cfg) = config::load_flat::<YmusicServiceCfg>(&local_path)? {
        let slug_leaked = leak(slug);
        return Ok((cfg, "local", Scope::Project(slug_leaked), local_path));
    }
    if let Some(cfg) = config::load_flat::<YmusicServiceCfg>(&global_path)? {
        return Ok((cfg, "global", Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no ymusic credentials found.\n  looked in:\n    {}\n    {}\n  \
         Run `zad service create ymusic`.",
        local_path.display(),
        global_path.display()
    )))
}

/// Live `YmusicHttp` for the effective scope. Reads three keychain
/// entries; any one missing surfaces a clear `re-run create` error.
fn http_for() -> Result<YmusicHttp> {
    let (cfg, _label, scope, path) = effective_config()?;
    let client_id = secrets::load(&secrets::account("ymusic", "client-id", scope.clone()))?.ok_or(
        ZadError::Service {
            name: "ymusic",
            message: "client-id missing from keychain; re-run `zad service create ymusic`".into(),
        },
    )?;
    let client_secret = secrets::load(&secrets::account("ymusic", "client-secret", scope.clone()))?
        .ok_or(ZadError::Service {
            name: "ymusic",
            message: "client-secret missing from keychain; re-run `zad service create ymusic`"
                .into(),
        })?;
    let refresh_token =
        secrets::load(&secrets::account("ymusic", "refresh", scope))?.ok_or(ZadError::Service {
            name: "ymusic",
            message: "refresh token missing from keychain; re-run `zad service create ymusic`"
                .into(),
        })?;
    let scope_set: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    Ok(YmusicHttp::new(
        client_id,
        client_secret,
        refresh_token,
        scope_set,
        path,
    ))
}

/// Build a transport: `--dry-run` returns the preview impl,
/// otherwise the live HTTP client.
fn transport_for(dry_run: bool) -> Result<Box<dyn YmusicTransport>> {
    if dry_run {
        Ok(Box::new(DryRunYmusicTransport::new(default_dry_run_sink())))
    } else {
        Ok(Box::new(http_for()?))
    }
}

/// Resolve `--playlist <raw>` against `default_playlist` fallback.
fn playlist_target(flag: Option<&str>, default: Option<&str>) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if let Some(v) = default {
        return Ok(v.to_string());
    }
    Err(ZadError::MissingRequired(
        "--playlist (or set `default_playlist` in the ymusic config)",
    ))
}

/// Strip a YouTube playlist URL down to its raw `PL…` ID. Bare IDs
/// pass through. Accepts the common `https://music.youtube.com/
/// playlist?list=PL…` and `https://www.youtube.com/playlist?list=PL…`
/// forms.
fn strip_playlist_url(s: &str) -> String {
    if let Some(idx) = s.find("list=") {
        let rest = &s[idx + 5..];
        let end = rest.find(['&', '#']).unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    s.to_string()
}

/// Pull a video ID out of a YouTube URL or pass a bare ID through.
/// Recognised forms: `youtube.com/watch?v=…`, `music.youtube.com/
/// watch?v=…`, `youtu.be/…`. Anything else is returned as-is.
fn extract_video_id(s: &str) -> String {
    if let Some(idx) = s.find("v=") {
        let rest = &s[idx + 2..];
        let end = rest.find(['&', '#']).unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    if let Some(rest) = s.strip_prefix("https://youtu.be/") {
        let end = rest.find(['?', '&', '#', '/']).unwrap_or(rest.len());
        return rest[..end].to_string();
    }
    s.to_string()
}

/// Heuristic for "does this look like a YouTube video ID rather than
/// a playlistItem ID?". Video IDs are exactly 11 chars from the
/// `[A-Za-z0-9_-]` alphabet; playlistItem IDs are much longer
/// base64-ish strings.
fn is_likely_video_id(s: &str) -> bool {
    let id = extract_video_id(s);
    id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn parse_privacy(s: &str) -> Result<Privacy> {
    Ok(match s {
        "private" => Privacy::Private,
        "unlisted" => Privacy::Unlisted,
        "public" => Privacy::Public,
        other => {
            return Err(ZadError::Invalid(format!(
                "unknown privacy `{other}`; expected one of: private, unlisted, public"
            )));
        }
    })
}
