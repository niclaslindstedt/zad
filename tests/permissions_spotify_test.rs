//! Integration tests for the Spotify-specific permissions layer:
//! file loading, the global+local intersection rule, and the
//! per-function enforcement entry points. These never hit the
//! Spotify API — they drive the compiled policy directly, the same
//! way the CLI verbs do after resolving names.

use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::permissions::pattern::PatternListRaw;
use zad::service::spotify::permissions::{
    self as perms, EffectivePermissions, FunctionBlockRaw, SpotifyFunction, SpotifyPermissions,
    SpotifyPermissionsRaw,
};

fn test_key() -> SigningKey {
    zad::secrets::use_memory_backend();
    SigningKey::generate()
}

fn raw_with_playlists_write_allow(allow: Vec<&str>) -> SpotifyPermissionsRaw {
    SpotifyPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: allow.into_iter().map(String::from).collect(),
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..SpotifyPermissionsRaw::default()
    }
}

fn raw_with_playlists_write_deny(deny: Vec<&str>) -> SpotifyPermissionsRaw {
    SpotifyPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec![],
                deny: deny.into_iter().map(String::from).collect(),
            },
            ..FunctionBlockRaw::default()
        },
        ..SpotifyPermissionsRaw::default()
    }
}

fn write_raw(path: &std::path::Path, raw: &SpotifyPermissionsRaw) {
    let key = test_key();
    perms::save_file(path, raw, &key).unwrap();
}

fn load(path: &std::path::Path) -> SpotifyPermissions {
    perms::load_file(path).unwrap().unwrap()
}

fn eff(
    global: Option<SpotifyPermissions>,
    local: Option<SpotifyPermissions>,
) -> EffectivePermissions {
    EffectivePermissions { global, local }
}

// ---------------------------------------------------------------------------
// file loading + round trip
// ---------------------------------------------------------------------------

#[test]
fn absent_file_loads_as_none() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    assert!(perms::load_file(&p).unwrap().is_none());
}

#[test]
fn starter_template_round_trips_through_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = perms::starter_template();
    let key = test_key();
    perms::save_file(&p, &raw, &key).unwrap();

    let body = std::fs::read_to_string(&p).unwrap();
    assert!(body.contains("deny_words"), "body: {body}");
    assert!(body.contains("release"), "body: {body}");

    let loaded = perms::load_file(&p).unwrap().unwrap();
    assert_eq!(loaded.source, p);
}

#[test]
fn invalid_glob_surfaces_the_file_path() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = SpotifyPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec!["re:(".into()],
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..SpotifyPermissionsRaw::default()
    };
    write_raw(&p, &raw);
    let err = perms::load_file(&p).unwrap_err();
    let s = err.to_string();
    assert!(s.contains(&p.display().to_string()), "err: {s}");
    assert!(
        s.contains("invalid permissions file") || s.contains("invalid regex"),
        "err: {s}"
    );
}

// ---------------------------------------------------------------------------
// per-function enforcement
// ---------------------------------------------------------------------------

#[test]
fn write_target_is_denied_when_not_in_allow_list() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(&p, &raw_with_playlists_write_allow(vec!["zad-*"]));
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_target(SpotifyFunction::PlaylistsWrite, "marketing")
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { function, .. } => {
            assert_eq!(function, "playlists_write");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn write_target_is_allowed_when_pattern_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(&p, &raw_with_playlists_write_allow(vec!["zad-*"]));
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    effective
        .check_target(SpotifyFunction::PlaylistsWrite, "zad-test")
        .unwrap();
}

#[test]
fn deny_always_wins_over_allow() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = SpotifyPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec!["*".into()],
                deny: vec!["*release*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        ..SpotifyPermissionsRaw::default()
    };
    write_raw(&p, &raw);
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_target(SpotifyFunction::PlaylistsWrite, "v1.5-release-mix")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
    // But a non-release name is fine.
    effective
        .check_target(SpotifyFunction::PlaylistsWrite, "zad-test")
        .unwrap();
}

#[test]
fn empty_targets_list_contributes_no_constraint() {
    // A function with no `targets` configured should accept anything,
    // so an unconfigured global ∩ unconfigured local is fully permissive.
    let effective = eff(None, None);
    effective
        .check_target(SpotifyFunction::PlaylistsWrite, "anything")
        .unwrap();
    effective
        .check_target(SpotifyFunction::Search, "weird query")
        .unwrap();
}

#[test]
fn global_and_local_intersect_via_strictest_wins() {
    // Global allows zad-*; local allows scratch-*. A target must
    // pass BOTH layers, so only a target that's in both — there's no
    // such name — should pass. zad-test fails the local layer; the
    // local file has no "zad-*" entry.
    let tmp_g = tempfile::tempdir().unwrap();
    let tmp_l = tempfile::tempdir().unwrap();
    let global_p = tmp_g.path().join("permissions.toml");
    let local_p = tmp_l.path().join("permissions.toml");
    write_raw(&global_p, &raw_with_playlists_write_allow(vec!["zad-*"]));
    write_raw(&local_p, &raw_with_playlists_write_allow(vec!["scratch-*"]));
    let pg = load(&global_p);
    let pl = load(&local_p);
    let effective = eff(Some(pg), Some(pl));

    // zad-test passes global but not local -> denied.
    let err = effective
        .check_target(SpotifyFunction::PlaylistsWrite, "zad-test")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));

    // scratch-foo passes local but not global -> denied.
    let err = effective
        .check_target(SpotifyFunction::PlaylistsWrite, "scratch-foo")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
}

#[test]
fn local_deny_can_tighten_global_allow() {
    let tmp_g = tempfile::tempdir().unwrap();
    let tmp_l = tempfile::tempdir().unwrap();
    let global_p = tmp_g.path().join("permissions.toml");
    let local_p = tmp_l.path().join("permissions.toml");
    write_raw(&global_p, &raw_with_playlists_write_allow(vec!["*"]));
    write_raw(&local_p, &raw_with_playlists_write_deny(vec!["sensitive"]));
    let pg = load(&global_p);
    let pl = load(&local_p);
    let effective = eff(Some(pg), Some(pl));

    let err = effective
        .check_target(SpotifyFunction::PlaylistsWrite, "sensitive")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
    effective
        .check_target(SpotifyFunction::PlaylistsWrite, "zad-test")
        .unwrap();
}

// ---------------------------------------------------------------------------
// content rules cascade
// ---------------------------------------------------------------------------

#[test]
fn content_rules_inherit_from_top_level() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = SpotifyPermissionsRaw {
        content: zad::permissions::content::ContentRulesRaw {
            deny_words: vec!["password".into()],
            ..Default::default()
        },
        ..SpotifyPermissionsRaw::default()
    };
    write_raw(&p, &raw);
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_body(SpotifyFunction::Search, "what is the password to login")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
    // Innocent body passes.
    effective
        .check_body(SpotifyFunction::Search, "moon river")
        .unwrap();
}
