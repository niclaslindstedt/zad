//! Regression test for the `failed to decode Spotify response` error
//! that spotifai's `export` hit on playlists containing local-file
//! entries.
//!
//! Spotify's playlist endpoint surfaces user-uploaded local files
//! alongside catalog tracks. For those rows it sends `"id": null`
//! on the track, every artist, and the album — Spotify has no
//! catalog id for content the user dragged in from their hard drive.
//! Before this fix `TrackSummary::id` / `ArtistRef::id` /
//! `AlbumRef::id` were `String` (required), so the whole page
//! failed to deserialize and the entire playlist's tracks were
//! skipped.
//!
//! The fixture below is the literal JSON shape Spotify returned for
//! one of niclas's local-file rows (trimmed to the structurally
//! relevant fields).

use zad::service::spotify::client::PlaylistTrackPage;

const LOCAL_FILE_PAGE: &str = r#"{
  "items": [
    {
      "added_at": "2013-06-02T11:54:57Z",
      "is_local": true,
      "item": {
        "is_playable": true,
        "explicit": false,
        "type": "track",
        "episode": false,
        "track": true,
        "id": null,
        "name": "Depth Over Distance (Live @ KCRW)",
        "uri": "spotify:local:Ben+Howard::Depth+Over+Distance:351",
        "duration_ms": 0,
        "album": {
          "id": null,
          "name": "",
          "uri": null,
          "release_date": null,
          "artists": []
        },
        "artists": [
          {
            "id": null,
            "name": "Ben Howard",
            "uri": null
          }
        ]
      }
    }
  ]
}"#;

#[test]
fn local_file_playlist_item_decodes_with_empty_ids() {
    let page: PlaylistTrackPage =
        serde_json::from_str(LOCAL_FILE_PAGE).expect("local-file page must decode");
    assert_eq!(page.items.len(), 1, "one local-file row expected");

    let row = &page.items[0];
    let track = row.item.as_ref().expect("track payload present");
    assert!(track.id.is_empty(), "local files have no Spotify track id");
    assert_eq!(track.name, "Depth Over Distance (Live @ KCRW)");
    assert!(
        track
            .uri
            .as_deref()
            .unwrap_or("")
            .starts_with("spotify:local:"),
        "local files carry the `spotify:local:` URI scheme"
    );

    let artist = track.artists.first().expect("artist present");
    assert!(
        artist.id.is_empty(),
        "local files have no Spotify artist id"
    );
    assert_eq!(artist.name, "Ben Howard");

    let album = track.album.as_ref().expect("album shell present");
    assert!(album.id.is_empty(), "local files have no Spotify album id");
}

#[test]
fn catalog_track_still_decodes_with_real_id() {
    const CATALOG: &str = r#"{
      "items": [
        {
          "added_at": "2010-08-18T11:55:40Z",
          "is_local": false,
          "item": {
            "id": "3kB79JwXStyEAL5aNkv2Gx",
            "name": "Breathless",
            "uri": "spotify:track:3kB79JwXStyEAL5aNkv2Gx",
            "duration_ms": 240000,
            "artists": [
              { "id": "1dfeR4HaWDbWqFHLkxsg1d", "name": "The Corrs", "uri": "spotify:artist:1dfeR4HaWDbWqFHLkxsg1d" }
            ],
            "album": {
              "id": "5SDQDxIaqHd1Vt06CSxXq8",
              "name": "In Blue",
              "uri": "spotify:album:5SDQDxIaqHd1Vt06CSxXq8"
            }
          }
        }
      ]
    }"#;
    let page: PlaylistTrackPage = serde_json::from_str(CATALOG).expect("catalog page decodes");
    let track = page.items[0].item.as_ref().unwrap();
    assert_eq!(track.id, "3kB79JwXStyEAL5aNkv2Gx");
    assert_eq!(track.artists[0].id, "1dfeR4HaWDbWqFHLkxsg1d");
    assert_eq!(track.album.as_ref().unwrap().id, "5SDQDxIaqHd1Vt06CSxXq8");
}
