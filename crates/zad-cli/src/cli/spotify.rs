//! `zad spotify <verb>` — runtime surface for the Spotify service.
//!
//! Wires together:
//! - per-verb clap args (`search`, `playlists list/show/create/
//!   rename/delete/add/remove`, `library tracks/albums {list,save,
//!   unsave}`, plus the mandatory `permissions` subgroup);
//! - credential + scope resolution from the effective config (local
//!   wins over global);
//! - permission gating (time window → target → content) executed
//!   **before** any network call.
//!
//! No `--dry-run` for now — playlist mutations are reversible
//! (Spotify keeps a 90-day history) and search / library reads are
//! side-effect-free.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::lifecycle::leak;
use zad::config::{self, SpotifyServiceCfg};
use zad::error::{Result, ZadError};
use zad::secrets::{self, Scope};
use zad::service::spotify::client::{
    PlaylistSummary, PlaylistTrackItem, SavedAlbum, SavedTrack, SearchResults, SpotifyHttp,
};
use zad::service::spotify::permissions::{self as perms, SpotifyFunction};

// ---------------------------------------------------------------------------
// top-level args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SpotifyArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Search the Spotify catalogue (tracks, albums, artists, playlists).
    Search(SearchArgs),
    /// Playlist management (list, show, create, rename, delete, add, remove).
    Playlists(PlaylistsArgs),
    /// Library management (saved tracks and albums).
    Library(LibraryArgs),
    /// Inspect or scaffold the permissions policy.
    Permissions(PermissionsArgs),
}

// ---------------------------------------------------------------------------
// `zad spotify search …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Free-text query.
    pub query: String,
    /// One or more entity types to search across (`track`, `album`,
    /// `artist`, `playlist`). Repeatable. Defaults to `track`.
    #[arg(long = "type", value_parser = ["track", "album", "artist", "playlist"], default_values = ["track"])]
    pub types: Vec<String>,
    /// Page size (1..=50). Spotify caps every request at 50 items.
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad spotify playlists …`
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
    /// Delete (i.e. unfollow) a playlist owned by the user.
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
    /// Playlist ID, `spotify:playlist:<id>` URI, or — when previously
    /// listed — the literal name of an owned playlist.
    pub playlist: Option<String>,
    /// Page size for the tracks listing (1..=100).
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
    /// Make the playlist public (default: private).
    #[arg(long)]
    pub public: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsRenameArgs {
    pub playlist: String,
    pub new_name: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsDeleteArgs {
    pub playlist: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsAddArgs {
    /// Target playlist (ID, URI, or owned-playlist name).
    pub playlist: String,
    /// One or more track URIs (`spotify:track:<id>`) or bare track IDs.
    #[arg(required = true)]
    pub tracks: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PlaylistsRemoveArgs {
    pub playlist: String,
    #[arg(required = true)]
    pub tracks: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad spotify library …`
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct LibraryArgs {
    #[command(subcommand)]
    pub action: LibraryAction,
}

#[derive(Debug, Subcommand)]
pub enum LibraryAction {
    /// Saved-tracks operations.
    Tracks(LibraryTracksArgs),
    /// Saved-albums operations.
    Albums(LibraryAlbumsArgs),
}

#[derive(Debug, Args)]
pub struct LibraryTracksArgs {
    #[command(subcommand)]
    pub action: LibraryItemAction,
}

#[derive(Debug, Args)]
pub struct LibraryAlbumsArgs {
    #[command(subcommand)]
    pub action: LibraryItemAction,
}

#[derive(Debug, Subcommand)]
pub enum LibraryItemAction {
    /// List saved items.
    List(LibraryListArgs),
    /// Save (like) one or more items.
    Save(LibraryMutateArgs),
    /// Unsave (unlike) one or more items.
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
    /// One or more URIs (`spotify:track:<id>` or `spotify:album:<id>`)
    /// or bare IDs.
    #[arg(required = true)]
    pub items: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// `zad spotify permissions …`
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
    /// Staged-commit workflow: queue mutations in a `.pending` file and
    /// only sign on `commit`. See `cli::permissions`.
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
    /// playlist name/ID/URI, a track/album URI, or a search query.
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

pub async fn run(args: SpotifyArgs) -> Result<()> {
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
                LibraryItemAction::Save(a) => run_library_tracks_save(a).await,
                LibraryItemAction::Unsave(a) => run_library_tracks_unsave(a).await,
            },
            LibraryAction::Albums(t) => match t.action {
                LibraryItemAction::List(a) => run_library_albums_list(a).await,
                LibraryItemAction::Save(a) => run_library_albums_save(a).await,
                LibraryItemAction::Unsave(a) => run_library_albums_unsave(a).await,
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
    permissions.check_time(SpotifyFunction::Search)?;
    permissions.check_target(SpotifyFunction::Search, &args.query)?;
    permissions.check_body(SpotifyFunction::Search, &args.query)?;

    let http = http_for()?;
    let types: Vec<&str> = args.types.iter().map(|s| s.as_str()).collect();
    let results = http.search(&args.query, &types, args.limit).await?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&render_search(&results)).unwrap()
        );
        return Ok(());
    }
    print_search_human(&results);
    Ok(())
}

#[derive(Debug, Serialize)]
struct SearchOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    tracks: Option<Vec<TrackOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    albums: Option<Vec<AlbumOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artists: Option<Vec<ArtistOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    playlists: Option<Vec<PlaylistOut>>,
}

#[derive(Debug, Serialize)]
struct TrackOut {
    id: String,
    name: String,
    uri: Option<String>,
    artists: Vec<String>,
    album: Option<String>,
    explicit: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AlbumOut {
    id: String,
    name: String,
    uri: Option<String>,
    release_date: Option<String>,
    artists: Vec<String>,
    total_tracks: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ArtistOut {
    id: String,
    name: String,
    uri: Option<String>,
    genres: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PlaylistOut {
    id: String,
    name: String,
    uri: Option<String>,
    owner: Option<String>,
    public: Option<bool>,
    tracks: Option<u32>,
}

fn render_search(r: &SearchResults) -> SearchOutput {
    SearchOutput {
        tracks: r.tracks.as_ref().map(|p| {
            p.items
                .iter()
                .map(|t| TrackOut {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    uri: t.uri.clone(),
                    artists: t.artists.iter().map(|a| a.name.clone()).collect(),
                    album: t.album.as_ref().map(|a| a.name.clone()),
                    explicit: t.explicit,
                })
                .collect()
        }),
        albums: r.albums.as_ref().map(|p| {
            p.items
                .iter()
                .map(|a| AlbumOut {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    uri: a.uri.clone(),
                    release_date: a.release_date.clone(),
                    artists: a.artists.iter().map(|x| x.name.clone()).collect(),
                    total_tracks: a.total_tracks,
                })
                .collect()
        }),
        artists: r.artists.as_ref().map(|p| {
            p.items
                .iter()
                .map(|a| ArtistOut {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    uri: a.uri.clone(),
                    genres: a.genres.clone(),
                })
                .collect()
        }),
        playlists: r.playlists.as_ref().map(|p| {
            p.items
                .iter()
                .map(|pl| PlaylistOut {
                    id: pl.id.clone(),
                    name: pl.name.clone(),
                    uri: pl.uri.clone(),
                    owner: pl.owner.as_ref().map(|o| o.id.clone()),
                    public: pl.public,
                    tracks: pl.items.as_ref().and_then(|t| t.total),
                })
                .collect()
        }),
    }
}

fn print_search_human(r: &SearchResults) {
    if let Some(p) = &r.tracks {
        if !p.items.is_empty() {
            println!("Tracks");
            for t in &p.items {
                let artists = t
                    .artists
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let album = t.album.as_ref().map(|a| a.name.as_str()).unwrap_or("");
                println!("  {:25}  {} — {} [{album}]", t.id, t.name, artists);
            }
        }
    }
    if let Some(p) = &r.albums {
        if !p.items.is_empty() {
            println!("Albums");
            for a in &p.items {
                let artists = a
                    .artists
                    .iter()
                    .map(|x| x.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let date = a.release_date.as_deref().unwrap_or("?");
                println!("  {:25}  {} — {} ({date})", a.id, a.name, artists);
            }
        }
    }
    if let Some(p) = &r.artists {
        if !p.items.is_empty() {
            println!("Artists");
            for a in &p.items {
                println!("  {:25}  {}", a.id, a.name);
            }
        }
    }
    if let Some(p) = &r.playlists {
        if !p.items.is_empty() {
            println!("Playlists");
            for pl in &p.items {
                let owner = pl.owner.as_ref().map(|o| o.id.as_str()).unwrap_or("?");
                println!("  {:25}  {} (by {owner})", pl.id, pl.name);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// playlists
// ---------------------------------------------------------------------------

async fn run_playlists_list(args: PlaylistsListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::PlaylistsRead)?;

    let http = http_for()?;
    let items = http.list_my_playlists(args.limit).await?;
    let filtered: Vec<&PlaylistSummary> = items
        .iter()
        .filter(|p| {
            permissions
                .check_target(SpotifyFunction::PlaylistsRead, &p.name)
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
        let total = p.items.as_ref().and_then(|t| t.total).unwrap_or(0);
        let owner = p.owner.as_ref().map(|o| o.id.as_str()).unwrap_or("?");
        println!("  {:25}  {} ({total} tracks, by {owner})", p.id, p.name);
    }
    Ok(())
}

async fn run_playlists_show(args: PlaylistsShowArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::PlaylistsRead)?;

    let (cfg, _label, _scope, _path) = effective_config()?;
    let raw = playlist_target(args.playlist.as_deref(), cfg.default_playlist.as_deref())?;
    permissions.check_target(SpotifyFunction::PlaylistsRead, &raw)?;
    let resolved = strip_playlist_uri(&raw);

    let http = http_for()?;
    let summary = http.get_playlist(&resolved).await?;
    let tracks = http.get_playlist_tracks(&resolved, args.limit).await?;

    if args.json {
        let out = serde_json::json!({ "playlist": summary, "tracks": tracks });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("id          : {}", summary.id);
    println!("name        : {}", summary.name);
    if let Some(d) = &summary.description {
        if !d.is_empty() {
            println!("description : {d}");
        }
    }
    if let Some(o) = &summary.owner {
        println!("owner       : {}", o.id);
    }
    println!("tracks      :");
    for t in &tracks {
        print_playlist_track(t);
    }
    Ok(())
}

fn print_playlist_track(entry: &PlaylistTrackItem) {
    let Some(track) = &entry.item else {
        return;
    };
    let artists = track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let album = track.album.as_ref().map(|a| a.name.as_str()).unwrap_or("");
    println!("  {:25}  {} — {} [{album}]", track.id, track.name, artists);
}

async fn run_playlists_create(args: PlaylistsCreateArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::PlaylistsWrite)?;
    permissions.check_target(SpotifyFunction::PlaylistsWrite, &args.name)?;
    if let Some(d) = &args.description {
        permissions.check_body(SpotifyFunction::PlaylistsWrite, d)?;
    }

    let http = http_for()?;
    let summary = http
        .create_playlist(&args.name, args.description.as_deref(), args.public)
        .await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    } else {
        println!("Created playlist `{}` (id={})", summary.name, summary.id);
    }
    Ok(())
}

async fn run_playlists_rename(args: PlaylistsRenameArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::PlaylistsWrite)?;
    permissions.check_target(SpotifyFunction::PlaylistsWrite, &args.playlist)?;
    permissions.check_target(SpotifyFunction::PlaylistsWrite, &args.new_name)?;

    let resolved = strip_playlist_uri(&args.playlist);
    let http = http_for()?;
    http.rename_playlist(&resolved, &args.new_name).await?;

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
    permissions.check_time(SpotifyFunction::PlaylistsWrite)?;
    permissions.check_target(SpotifyFunction::PlaylistsWrite, &args.playlist)?;

    let resolved = strip_playlist_uri(&args.playlist);
    let http = http_for()?;
    http.unfollow_playlist(&resolved).await?;

    if args.json {
        let out = serde_json::json!({ "id": resolved, "unfollowed": true });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Unfollowed (deleted) playlist `{resolved}`");
    }
    Ok(())
}

async fn run_playlists_add(args: PlaylistsAddArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::PlaylistsWrite)?;
    permissions.check_target(SpotifyFunction::PlaylistsWrite, &args.playlist)?;
    for t in &args.tracks {
        permissions.check_target(SpotifyFunction::PlaylistsWrite, t)?;
    }

    let resolved = strip_playlist_uri(&args.playlist);
    let uris = normalize_track_uris(&args.tracks);
    let http = http_for()?;
    http.add_playlist_tracks(&resolved, &uris).await?;

    if args.json {
        let out = serde_json::json!({ "id": resolved, "added": uris });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Added {} track(s) to `{resolved}`", uris.len());
    }
    Ok(())
}

async fn run_playlists_remove(args: PlaylistsRemoveArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::PlaylistsWrite)?;
    permissions.check_target(SpotifyFunction::PlaylistsWrite, &args.playlist)?;
    for t in &args.tracks {
        permissions.check_target(SpotifyFunction::PlaylistsWrite, t)?;
    }

    let resolved = strip_playlist_uri(&args.playlist);
    let uris = normalize_track_uris(&args.tracks);
    let http = http_for()?;
    http.remove_playlist_tracks(&resolved, &uris).await?;

    if args.json {
        let out = serde_json::json!({ "id": resolved, "removed": uris });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("Removed {} track(s) from `{resolved}`", uris.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// library
// ---------------------------------------------------------------------------

async fn run_library_tracks_list(args: LibraryListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::LibraryRead)?;

    let http = http_for()?;
    let items = http.list_saved_tracks(args.limit).await?;
    let filtered: Vec<&SavedTrack> = items
        .iter()
        .filter(|s| {
            s.track
                .uri
                .as_deref()
                .map(|u| {
                    permissions
                        .check_target(SpotifyFunction::LibraryRead, u)
                        .is_ok()
                })
                .unwrap_or(true)
        })
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
        return Ok(());
    }
    if filtered.is_empty() {
        println!("No saved tracks (or all filtered by permissions).");
        return Ok(());
    }
    for s in &filtered {
        let artists = s
            .track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {:25}  {} — {}", s.track.id, s.track.name, artists);
    }
    Ok(())
}

async fn run_library_tracks_save(args: LibraryMutateArgs) -> Result<()> {
    library_mutate_tracks(args, true).await
}

async fn run_library_tracks_unsave(args: LibraryMutateArgs) -> Result<()> {
    library_mutate_tracks(args, false).await
}

async fn library_mutate_tracks(args: LibraryMutateArgs, save: bool) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::LibraryWrite)?;
    for u in &args.items {
        permissions.check_target(SpotifyFunction::LibraryWrite, u)?;
    }

    let uris = normalize_uris(&args.items, "track");
    let http = http_for()?;
    if save {
        http.save_tracks(&uris).await?;
    } else {
        http.unsave_tracks(&uris).await?;
    }

    let verb = if save { "saved" } else { "unsaved" };
    if args.json {
        let out = serde_json::json!({ verb: uris });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{verb} {} track(s)", uris.len());
    }
    Ok(())
}

async fn run_library_albums_list(args: LibraryListArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::LibraryRead)?;

    let http = http_for()?;
    let items = http.list_saved_albums(args.limit).await?;
    let filtered: Vec<&SavedAlbum> = items
        .iter()
        .filter(|s| {
            s.album
                .uri
                .as_deref()
                .map(|u| {
                    permissions
                        .check_target(SpotifyFunction::LibraryRead, u)
                        .is_ok()
                })
                .unwrap_or(true)
        })
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
        return Ok(());
    }
    if filtered.is_empty() {
        println!("No saved albums (or all filtered by permissions).");
        return Ok(());
    }
    for s in &filtered {
        let artists = s
            .album
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let date = s.album.release_date.as_deref().unwrap_or("?");
        println!(
            "  {:25}  {} — {} ({date})",
            s.album.id, s.album.name, artists
        );
    }
    Ok(())
}

async fn run_library_albums_save(args: LibraryMutateArgs) -> Result<()> {
    library_mutate_albums(args, true).await
}

async fn run_library_albums_unsave(args: LibraryMutateArgs) -> Result<()> {
    library_mutate_albums(args, false).await
}

async fn library_mutate_albums(args: LibraryMutateArgs, save: bool) -> Result<()> {
    let permissions = perms::load_effective()?;
    permissions.check_time(SpotifyFunction::LibraryWrite)?;
    for u in &args.items {
        permissions.check_target(SpotifyFunction::LibraryWrite, u)?;
    }

    let uris = normalize_uris(&args.items, "album");
    let http = http_for()?;
    if save {
        http.save_albums(&uris).await?;
    } else {
        http.unsave_albums(&uris).await?;
    }

    let verb = if save { "saved" } else { "unsaved" };
    if args.json {
        let out = serde_json::json!({ verb: uris });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("{verb} {} album(s)", uris.len());
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
    // Force compile so syntax errors surface immediately.
    let _ = perms::load_effective()?;

    if args.json {
        let out = PermissionsShowOut {
            command: "spotify.permissions.show",
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
    println!("Spotify permissions");
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
            "command": "spotify.permissions.path",
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
            command: "spotify.permissions.init",
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
            "command": "spotify.permissions.check",
            "function": args.function,
            "ok": true,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("✓ would be allowed by current spotify permissions");
    }
    Ok(())
}

fn parse_function(name: &str) -> Result<SpotifyFunction> {
    Ok(match name {
        "search" => SpotifyFunction::Search,
        "playlists_read" => SpotifyFunction::PlaylistsRead,
        "playlists_write" => SpotifyFunction::PlaylistsWrite,
        "library_read" => SpotifyFunction::LibraryRead,
        "library_write" => SpotifyFunction::LibraryWrite,
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

pub(crate) fn effective_config()
-> Result<(SpotifyServiceCfg, &'static str, Scope<'static>, PathBuf)> {
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "spotify")?;
    let global_path = config::path::global_service_config_path("spotify")?;

    let project_cfg = config::load()?;
    if !project_cfg.has_service("spotify") {
        return Err(ZadError::Invalid(format!(
            "spotify is not enabled for this project ({}). \
             Run `zad service enable spotify` first.",
            config::path::project_config_path()?.display()
        )));
    }

    if let Some(cfg) = config::load_flat::<SpotifyServiceCfg>(&local_path)? {
        let slug_leaked = leak(slug);
        return Ok((cfg, "local", Scope::Project(slug_leaked), local_path));
    }
    if let Some(cfg) = config::load_flat::<SpotifyServiceCfg>(&global_path)? {
        return Ok((cfg, "global", Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no spotify credentials found.\n  looked in:\n    {}\n    {}\n  \
         Run `zad service create spotify`.",
        local_path.display(),
        global_path.display()
    )))
}

fn http_for() -> Result<SpotifyHttp> {
    let (cfg, _label, scope, path) = effective_config()?;
    let client_id = secrets::load(&secrets::account("spotify", "client-id", scope.clone()))?
        .ok_or(ZadError::Service {
            name: "spotify",
            message: "client-id missing from keychain; re-run `zad service create spotify`".into(),
        })?;
    let refresh_token = secrets::load(&secrets::account("spotify", "refresh", scope))?.ok_or(
        ZadError::Service {
            name: "spotify",
            message: "refresh token missing from keychain; re-run `zad service create spotify`"
                .into(),
        },
    )?;
    let scope_set: BTreeSet<String> = cfg.scopes.iter().cloned().collect();
    Ok(SpotifyHttp::new(client_id, refresh_token, scope_set, path))
}

/// Resolve `--playlist <raw>` against `default_playlist` fallback.
/// Returns the raw string the user typed (or the configured default);
/// callers strip any `spotify:playlist:` prefix before hitting the
/// API via [`strip_playlist_uri`].
fn playlist_target(flag: Option<&str>, default: Option<&str>) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if let Some(v) = default {
        return Ok(v.to_string());
    }
    Err(ZadError::MissingRequired(
        "--playlist (or set `default_playlist` in the spotify config)",
    ))
}

/// Strip `spotify:playlist:` (or `spotify:track:` / `spotify:album:`)
/// prefixes from a URI, returning the bare ID. Bare IDs pass through.
fn strip_playlist_uri(s: &str) -> String {
    s.strip_prefix("spotify:playlist:")
        .or_else(|| s.strip_prefix("spotify:track:"))
        .or_else(|| s.strip_prefix("spotify:album:"))
        .unwrap_or(s)
        .to_string()
}

/// Normalise a list of user-supplied track refs into URIs the API
/// accepts on the playlist add / remove endpoints. Bare IDs get the
/// `spotify:track:` prefix; full URIs pass through.
fn normalize_track_uris(items: &[String]) -> Vec<String> {
    normalize_uris(items, "track")
}

/// Normalise user-supplied refs to fully-qualified `spotify:<kind>:<id>`
/// URIs — the form `PUT/DELETE /me/library`, `POST /playlists/{id}/items`,
/// and `DELETE /playlists/{id}/items` all expect after February 2026.
/// `kind` is `"track"`, `"album"`, `"show"`, …; it is only used to add
/// the prefix to bare IDs. Anything already starting with `spotify:` is
/// passed through verbatim.
fn normalize_uris(items: &[String], kind: &str) -> Vec<String> {
    items
        .iter()
        .map(|s| {
            if s.starts_with("spotify:") {
                s.clone()
            } else {
                format!("spotify:{kind}:{s}")
            }
        })
        .collect()
}
