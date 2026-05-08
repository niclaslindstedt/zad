//! Integration tests for the YouTube Music permissions layer: file
//! loading, the global+local intersection rule, and the per-function
//! enforcement entry points. These never hit the Data API — they
//! drive the compiled policy directly, the same way the CLI verbs do
//! after resolving names. Mirrors `permissions_spotify_test.rs`.

use serial_test::serial;
use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::permissions::pattern::PatternListRaw;
use zad::service::ymusic::permissions::{
    self as perms, EffectivePermissions, FunctionBlockRaw, YmusicFunction, YmusicPermissions,
    YmusicPermissionsRaw,
};

mod common;

fn test_key() -> SigningKey {
    common::ensure_signing_env()
}

fn raw_with_playlists_write_allow(allow: Vec<&str>) -> YmusicPermissionsRaw {
    YmusicPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: allow.into_iter().map(String::from).collect(),
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..YmusicPermissionsRaw::default()
    }
}

fn raw_with_playlists_write_deny(deny: Vec<&str>) -> YmusicPermissionsRaw {
    YmusicPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec![],
                deny: deny.into_iter().map(String::from).collect(),
            },
            ..FunctionBlockRaw::default()
        },
        ..YmusicPermissionsRaw::default()
    }
}

fn write_raw(path: &std::path::Path, raw: &YmusicPermissionsRaw) {
    let key = test_key();
    perms::save_file(path, raw, &key).unwrap();
}

fn load(path: &std::path::Path) -> YmusicPermissions {
    perms::load_file(path).unwrap().unwrap()
}

fn eff(
    global: Option<YmusicPermissions>,
    local: Option<YmusicPermissions>,
) -> EffectivePermissions {
    EffectivePermissions { global, local }
}

// ---------------------------------------------------------------------------
// file loading + round trip
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn absent_file_loads_as_none() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    assert!(perms::load_file(&p).unwrap().is_none());
}

#[test]
#[serial]
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
#[serial]
fn invalid_glob_surfaces_the_file_path() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = YmusicPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec!["re:(".into()],
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..YmusicPermissionsRaw::default()
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
#[serial]
fn write_target_is_denied_when_not_in_allow_list() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(&p, &raw_with_playlists_write_allow(vec!["zad-*"]));
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_target(YmusicFunction::PlaylistsWrite, "marketing")
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { function, .. } => {
            assert_eq!(function, "playlists_write");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
#[serial]
fn write_target_is_allowed_when_pattern_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(&p, &raw_with_playlists_write_allow(vec!["zad-*"]));
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    effective
        .check_target(YmusicFunction::PlaylistsWrite, "zad-test")
        .unwrap();
}

#[test]
#[serial]
fn deny_always_wins_over_allow() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = YmusicPermissionsRaw {
        playlists_write: FunctionBlockRaw {
            targets: PatternListRaw {
                allow: vec!["*".into()],
                deny: vec!["*release*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        ..YmusicPermissionsRaw::default()
    };
    write_raw(&p, &raw);
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_target(YmusicFunction::PlaylistsWrite, "v1.5-release-mix")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
    effective
        .check_target(YmusicFunction::PlaylistsWrite, "zad-test")
        .unwrap();
}

#[test]
#[serial]
fn empty_targets_list_contributes_no_constraint() {
    let effective = eff(None, None);
    effective
        .check_target(YmusicFunction::PlaylistsWrite, "anything")
        .unwrap();
    effective
        .check_target(YmusicFunction::Search, "weird query")
        .unwrap();
}

#[test]
#[serial]
fn global_and_local_intersect_via_strictest_wins() {
    let tmp_g = tempfile::tempdir().unwrap();
    let tmp_l = tempfile::tempdir().unwrap();
    let global_p = tmp_g.path().join("permissions.toml");
    let local_p = tmp_l.path().join("permissions.toml");
    write_raw(&global_p, &raw_with_playlists_write_allow(vec!["zad-*"]));
    write_raw(&local_p, &raw_with_playlists_write_allow(vec!["scratch-*"]));
    let pg = load(&global_p);
    let pl = load(&local_p);
    let effective = eff(Some(pg), Some(pl));

    let err = effective
        .check_target(YmusicFunction::PlaylistsWrite, "zad-test")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));

    let err = effective
        .check_target(YmusicFunction::PlaylistsWrite, "scratch-foo")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
}

#[test]
#[serial]
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
        .check_target(YmusicFunction::PlaylistsWrite, "sensitive")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
    effective
        .check_target(YmusicFunction::PlaylistsWrite, "zad-test")
        .unwrap();
}

// ---------------------------------------------------------------------------
// content rules cascade
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn content_rules_inherit_from_top_level() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = YmusicPermissionsRaw {
        content: zad::permissions::content::ContentRulesRaw {
            deny_words: vec!["password".into()],
            ..Default::default()
        },
        ..YmusicPermissionsRaw::default()
    };
    write_raw(&p, &raw);
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_body(YmusicFunction::Search, "what is the password to login")
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
    effective
        .check_body(YmusicFunction::Search, "moon river")
        .unwrap();
}
