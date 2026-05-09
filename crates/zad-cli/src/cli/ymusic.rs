//! `zad ymusic <verb>` — runtime surface for the YouTube Music
//! service.
//!
//! Wires together:
//! - per-verb clap args (`search`, `playlists list/show/create/
//!   rename/delete/add/remove`, `library {list,like,unlike}`, plus
//!   the mandatory `permissions` subgroup);
//! - credential + scope resolution from the effective config (local
//!   wins over global);
//! - permission gating (time window → target → content) executed
//!   **before** any network call;
//! - `--dry-run` for mutating verbs via the
//!   [`zad::service::ymusic::YmusicTransport`] indirection so
//!   previews never touch the network or the keychain.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::lifecycle::leak;
use zad::config::{self, YmusicServiceCfg};
use zad::error::{Result, ZadError};
use zad::secrets::{self, Scope};
use zad::service::default_dry_run_sink;
use zad::service::ymusic::client::{
    PlaylistItem, PlaylistSummary, Privacy, SearchItem, VideoSummary, YmusicHttp,
};
use zad::service::ymusic::permissions::{self as perms, YmusicFunction};
use zad::service::ymusic::transport::{DryRunYmusicTransport, YmusicTransport};

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
    /// Library management — the user's liked videos.
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
    /// Show one playlist's metadata and items.
    Show(PlaylistsShowArgs),
    /// Create a new playlist owned by the authenticated user.
    Create(PlaylistsCreateArgs),
    /// Rename an existing playlist.
    Rename(PlaylistsRenameArgs),
    /// Delete a playlist owned by the user.
    Delete(PlaylistsDeleteArgs),
    /// Add one or more videos to a playlist.
    Add(PlaylistsAddArgs),
    /// Remove one or more items from a playlist (by playlistItem ID
    /// or by video ID — the latter is resolved by listing the
    /// playlist first).
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
    /// the literal title of an owned playlist.
    pub playlist: Option<String>,
    /// Page size for the items listing (1..=50).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsCreateArgs {
    /// Display title for the new playlist.
    pub title: String,
    #[arg(long)]
    pub description: Option<String>,
    /// Privacy: `private` (default), `unlisted`, or `public`.
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
    pub new_title: String,
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
    /// Target playlist (ID, URL, or owned-playlist title).
    pub playlist: String,
    /// One or more video IDs (or full YouTube URLs).
    #[arg(required = true)]
    pub videos: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsRemoveArgs {
    pub playlist: String,
    /// One or more playlist-item IDs *or* video IDs. When a video ID
    /// is supplied, zad lists the playlist to find the matching
    /// item; if the same video appears multiple times, every match
    /// is removed.
    #[arg(required = true)]
    pub items: Vec<String>,
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
    /// List the authenticated user's liked videos.
    List(LibraryListArgs),
    /// Like (save) one or more videos.
    Like(LibraryMutateArgs),
    /// Unlike (unsave) one or more videos.
    Unlike(LibraryMutateArgs),
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
    pub videos: Vec<String>,
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
    /// playlist title/ID, a video ID, or a search query.
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
            LibraryAction::List(a) => run_library_list(a).await,
            LibraryAction::Like(a) => run_library_mutate(a, true).await,
            LibraryAction::Unlike(a) => run_library_mutate(a, false).await,
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
    items: Vec<SearchItemOut>,
}

#[derive(Debug, Serialize)]
struct SearchItemOut {
    kind: String,
    id: String,
    title: Option<String>,
    channel_title: Option<String>,
}

fn render_search(items: &[SearchItem]) -> SearchOutput {
    SearchOutput {
        items: items
            .iter()
            .filter_map(|i| {
                let id_block = i.id.as_ref()?;
                let (kind, id) = if let Some(v) = id_block.video_id.as_ref() {
                    ("video", v.clone())
                } else if let Some(p) = id_block.playlist_id.as_ref() {
                    ("playlist", p.clone())
                } else if let Some(c) = id_block.channel_id.as_ref() {
                    ("channel", c.clone())
                } else {
                    return None;
                };
                Some(SearchItemOut {
                    kind: kind.to_string(),
                    id,
                    title: i.snippet.as_ref().and_then(|s| s.title.clone()),
                    channel_title: i.snippet.as_ref().and_then(|s| s.channel_title.clone()),
                })
            })
            .collect(),
    }
}

fn print_search_human(items: &[SearchItem]) {
    if items.is_empty() {
        println!("No results.");
        return;
    }
    for i in items {
        let Some(id_block) = i.id.as_ref() else {
            continue;
        };
        let (kind, id) = if let Some(v) = id_block.video_id.as_ref() {
            ("video   ", v.as_str())
        } else if let Some(p) = id_block.playlist_id.as_ref() {
            ("playlist", p.as_str())
        } else if let Some(c) = id_block.channel_id.as_ref() {
            ("channel ", c.as_str())
        } else {
            continue;
        };
        let title = i
            .snippet
            .as_ref()
            .and_then(|s| s.title.as_deref())
            .unwrap_or("(no title)");
        let channel = i
            .snippet
            .as_ref()
            .and_then(|s| s.channel_title.as_deref())
            .unwrap_or("?");
        println!("  {kind} {id:24}  {title} [{channel}]");
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
            let title = p.snippet.as_ref().map(|s| s.title.as_str()).unwrap_or("");
            permissions
                .check_target(YmusicFunction::PlaylistsRead, title)
                .is_ok()
        })
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
        return Ok(());
    }
    if filtered.is_empty() {
        println!("No playlists visible (or all filtered by permissions).");
        return Ok(());
    }
    for p in &filtered {
        let title = p
            .snippet
            .as_ref()
            .map(|s| s.title.as_str())
            .unwrap_or("(no title)");
        let total = p
            .content_details
            .as_ref()
            .and_then(|c| c.item_count)
            .unwrap_or(0);
        let privacy = p
            .status
            .as_ref()
            .and_then(|s| s.privacy_status.as_deref())
            .unwrap_or("?");
        println!("  {:36}  {title} ({total} items, {privacy})", p.id);
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

    if args.json {
        let out = serde_json::json!({ "playlist": summary, "items": items });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("id          : {}", summary.id);
    if let Some(s) = &summary.snippet {
        println!("title       : {}", s.title);
        if let Some(d) = &s.description {
            if !d.is_empty() {
                println!("description : {d}");
            }
        }
    }
    if let Some(s) = &summary.status {
        if let Some(p) = &s.privacy_status {
            println!("privacy     : {p}");
        }
    }
    println!("items       :");
    for item in &items {
        print_playlist_item(item);
    }
    Ok(())
}

fn print_playlist_item(item: &PlaylistItem) {
    let video_id = item
        .content_details
        .as_ref()
        .and_then(|c| c.video_id.as_deref())
        .or_else(|| {
            item.snippet
                .as_ref()
                .and_then(|s| s.resource_id.as_ref())
                .and_then(|r| r.video_id.as_deref())
        })
        .unwrap_or("?");
    let title = item
        .snippet
        .as_ref()
        .and_then(|s| s.title.as_deref())
        .unwrap_or("(no title)");
    let owner = item
        .snippet
        .as_ref()
        .and_then(|s| s.video_owner_channel_title.as_deref())
        .unwrap_or("?");
    println!("  item={} video={video_id}  {title} [{owner}]", item.id);
}

async fn run_playlists_create(args: PlaylistsCreateArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.title)?;
    permissions.check_body(YmusicFunction::PlaylistsWrite, &args.title)?;
    if let Some(d) = &args.description {
        permissions.check_body(YmusicFunction::PlaylistsWrite, d)?;
    }

    let privacy = parse_privacy(&args.privacy)?;
    let transport = transport_for(args.dry_run)?;
    let summary = transport
        .create_playlist(&args.title, args.description.as_deref(), privacy)
        .await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    } else {
        let title = summary
            .snippet
            .as_ref()
            .map(|s| s.title.as_str())
            .unwrap_or(args.title.as_str());
        println!("Created playlist `{title}` (id={})", summary.id);
    }
    Ok(())
}

async fn run_playlists_rename(args: PlaylistsRenameArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.playlist)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.new_title)?;
    permissions.check_body(YmusicFunction::PlaylistsWrite, &args.new_title)?;

    let resolved = strip_playlist_url(&args.playlist);
    let transport = transport_for(args.dry_run)?;
    transport
        .rename_playlist(&resolved, &args.new_title)
        .await?;

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({ "id": resolved, "new_title": args.new_title });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Renamed `{resolved}` → `{}`", args.new_title);
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
    for v in &args.videos {
        permissions.check_target(YmusicFunction::PlaylistsWrite, v)?;
    }

    let resolved = strip_playlist_url(&args.playlist);
    let video_ids: Vec<String> = args.videos.iter().map(|v| extract_video_id(v)).collect();
    let transport = transport_for(args.dry_run)?;
    let mut added: Vec<String> = Vec::with_capacity(video_ids.len());
    for vid in &video_ids {
        let item_id = transport.add_playlist_item(&resolved, vid).await?;
        added.push(item_id);
    }

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({ "id": resolved, "added": added });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Added {} video(s) to `{resolved}`", added.len());
    }
    Ok(())
}

async fn run_playlists_remove(args: PlaylistsRemoveArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::PlaylistsWrite)?;
    permissions.check_target(YmusicFunction::PlaylistsWrite, &args.playlist)?;
    for v in &args.items {
        permissions.check_target(YmusicFunction::PlaylistsWrite, v)?;
    }

    let resolved = strip_playlist_url(&args.playlist);
    let transport = transport_for(args.dry_run)?;

    // Resolve video IDs to playlistItem IDs by listing the playlist
    // once. Anything that already looks like a playlistItem ID
    // (length > 24, starts with `PL` or `UE` is not the rule —
    // YouTube uses opaque IDs) is just attempted as-is and we let
    // the API surface a 404 if it's not a real item.
    let listing: Option<Vec<PlaylistItem>> = if args.items.iter().any(|s| is_likely_video_id(s)) {
        Some(transport.get_playlist_items(&resolved, 50).await?)
    } else {
        None
    };

    let mut removed: Vec<String> = Vec::new();
    for raw in &args.items {
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
                    removed.push(m.id.clone());
                }
            }
        } else {
            // Treat raw as an opaque playlistItem ID.
            transport.remove_playlist_item(raw).await?;
            removed.push(raw.clone());
        }
    }

    if args.dry_run {
        return Ok(());
    }
    if args.json {
        let out = serde_json::json!({ "id": resolved, "removed": removed });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Removed {} item(s) from `{resolved}`", removed.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// library
// ---------------------------------------------------------------------------

async fn run_library_list(args: LibraryListArgs) -> Result<()> {
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

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
        return Ok(());
    }
    if filtered.is_empty() {
        println!("No liked videos (or all filtered by permissions).");
        return Ok(());
    }
    for v in &filtered {
        let title = v
            .snippet
            .as_ref()
            .map(|s| s.title.as_str())
            .unwrap_or("(no title)");
        let channel = v
            .snippet
            .as_ref()
            .and_then(|s| s.channel_title.as_deref())
            .unwrap_or("?");
        println!("  {:24}  {title} [{channel}]", v.id);
    }
    Ok(())
}

async fn run_library_mutate(args: LibraryMutateArgs, like: bool) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(YmusicFunction::LibraryWrite)?;
    for v in &args.videos {
        permissions.check_target(YmusicFunction::LibraryWrite, v)?;
    }

    let ids: Vec<String> = args.videos.iter().map(|v| extract_video_id(v)).collect();
    let transport = transport_for(args.dry_run)?;
    for id in &ids {
        if like {
            transport.like_video(id).await?;
        } else {
            transport.unlike_video(id).await?;
        }
    }

    if args.dry_run {
        return Ok(());
    }
    let verb = if like { "liked" } else { "unliked" };
    if args.json {
        let out = serde_json::json!({ verb: ids });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{verb} {} video(s)", ids.len());
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
    let key = zad::permissions::signing::load_or_create_from_keychain()?;
    zad::permissions::signing::write_public_key_cache(&key)?;
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
