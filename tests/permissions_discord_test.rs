//! Integration tests for the Discord-specific permissions layer: file
//! loading, the global+local intersection rule, and the per-function
//! enforcement entry points. These never hit Discord — they drive the
//! compiled policy directly, the same way the CLI verbs do after
//! resolving names.

use std::path::PathBuf;

use zad::config::directory::Directory;
use zad::error::ZadError;
use zad::permissions::content::ContentRulesRaw;
use zad::permissions::pattern::PatternListRaw;
use zad::service::discord::permissions::{
    self as perms, DiscordFunction, DiscordPermissionsRaw, EffectivePermissions, FunctionBlockRaw,
};

fn raw_with_send_allow(allow: Vec<&str>) -> DiscordPermissionsRaw {
    DiscordPermissionsRaw {
        send: FunctionBlockRaw {
            channels: PatternListRaw {
                allow: allow.into_iter().map(String::from).collect(),
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..DiscordPermissionsRaw::default()
    }
}

fn write_raw(path: &std::path::Path, raw: &DiscordPermissionsRaw) {
    perms::save_file(path, raw).unwrap();
}

fn load(path: &std::path::Path) -> perms::DiscordPermissions {
    perms::load_file(path).unwrap().unwrap()
}

fn eff(
    global: Option<perms::DiscordPermissions>,
    local: Option<perms::DiscordPermissions>,
) -> EffectivePermissions {
    EffectivePermissions { global, local }
}

fn empty_directory() -> Directory {
    Directory::default()
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
    perms::save_file(&p, &raw).unwrap();

    let body = std::fs::read_to_string(&p).unwrap();
    assert!(body.contains("deny_words"), "body: {body}");
    assert!(body.contains("admin"), "body: {body}");

    let loaded = perms::load_file(&p).unwrap().unwrap();
    assert_eq!(loaded.source, p);
}

#[test]
fn invalid_glob_surfaces_the_file_path() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    std::fs::write(&p, "[send]\nchannels.allow = [\"re:(\"]\n").unwrap();
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
fn send_channel_is_denied_when_not_in_allow_list() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(&p, &raw_with_send_allow(vec!["general"]));
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_send_channel("marketing", 111, &empty_directory())
        .unwrap_err();
    match err {
        ZadError::PermissionDenied {
            function,
            reason,
            config_path,
        } => {
            assert_eq!(function, "send");
            assert_eq!(config_path, p);
            assert!(reason.contains("marketing"), "reason: {reason}");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    // `general` is allowed.
    assert!(
        effective
            .check_send_channel("general", 222, &empty_directory())
            .is_ok()
    );
}

#[test]
fn deny_pattern_fires_on_reverse_lookup_name_even_when_agent_passed_id() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(
        &p,
        &DiscordPermissionsRaw {
            send: FunctionBlockRaw {
                channels: PatternListRaw {
                    allow: vec![],
                    deny: vec!["*admin*".into()],
                },
                ..FunctionBlockRaw::default()
            },
            ..DiscordPermissionsRaw::default()
        },
    );
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let mut dir = Directory::default();
    dir.channels
        .insert("server-admin".into(), "999000000000".into());

    // The agent passed the snowflake — the deny still fires because
    // `server-admin` is a reverse-lookup alias for that ID.
    let err = effective
        .check_send_channel("999000000000", 999_000_000_000, &dir)
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
}

#[test]
fn body_deny_words_block_send() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(
        &p,
        &DiscordPermissionsRaw {
            content: ContentRulesRaw {
                deny_words: vec!["api_key".into()],
                ..Default::default()
            },
            ..DiscordPermissionsRaw::default()
        },
    );
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    assert!(effective.check_send_body("hello").is_ok());
    let err = effective
        .check_send_body("leaked api_key=hunter2")
        .unwrap_err();
    if let ZadError::PermissionDenied { reason, .. } = err {
        assert!(reason.contains("api_key"), "reason: {reason}");
    } else {
        panic!("unexpected: {err:?}");
    }
}

#[test]
fn per_function_content_rules_narrow_the_defaults() {
    // Top-level content rule allows everything, but `send` adds a
    // deny_word that should still fire on sends.
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(
        &p,
        &DiscordPermissionsRaw {
            send: FunctionBlockRaw {
                content: ContentRulesRaw {
                    deny_words: vec!["confidential".into()],
                    ..Default::default()
                },
                ..FunctionBlockRaw::default()
            },
            ..DiscordPermissionsRaw::default()
        },
    );
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    assert!(effective.check_send_body("all good").is_ok());
    assert!(effective.check_send_body("this is CONFIDENTIAL").is_err());
}

// ---------------------------------------------------------------------------
// global ∩ local
// ---------------------------------------------------------------------------

#[test]
fn global_and_local_both_must_admit() {
    let tmp = tempfile::tempdir().unwrap();
    let g = tmp.path().join("global.toml");
    let l = tmp.path().join("local.toml");
    write_raw(&g, &raw_with_send_allow(vec!["general", "bot-*"]));
    write_raw(&l, &raw_with_send_allow(vec!["bot-ci"]));
    let effective = eff(Some(load(&g)), Some(load(&l)));

    // `general`: allowed by global, denied by local → overall denied.
    let err = effective
        .check_send_channel("general", 1, &empty_directory())
        .unwrap_err();
    if let ZadError::PermissionDenied { config_path, .. } = err {
        assert_eq!(config_path, l, "local should be the rejecting file");
    } else {
        panic!("unexpected: {err:?}");
    }

    // `bot-ci`: allowed by both.
    assert!(
        effective
            .check_send_channel("bot-ci", 2, &empty_directory())
            .is_ok()
    );

    // `bot-foo`: allowed by global, rejected by local's tighter list.
    assert!(
        effective
            .check_send_channel("bot-foo", 3, &empty_directory())
            .is_err()
    );
}

#[test]
fn missing_file_contributes_no_restrictions() {
    let tmp = tempfile::tempdir().unwrap();
    let l = tmp.path().join("local.toml");
    write_raw(&l, &raw_with_send_allow(vec!["general"]));
    // global: None
    let effective = eff(None, Some(load(&l)));

    assert!(
        effective
            .check_send_channel("general", 1, &empty_directory())
            .is_ok()
    );
    // Local still enforces.
    assert!(
        effective
            .check_send_channel("other", 2, &empty_directory())
            .is_err()
    );
}

#[test]
fn no_files_present_means_no_permission_restrictions() {
    let effective = eff(None, None);
    assert!(!effective.any());
    assert!(
        effective
            .check_send_channel("anywhere", 42, &empty_directory())
            .is_ok()
    );
    assert!(effective.check_send_body("anything").is_ok());
    assert!(effective.check_time(DiscordFunction::Send).is_ok());
}

// ---------------------------------------------------------------------------
// path helpers
// ---------------------------------------------------------------------------

#[test]
fn path_helpers_point_under_the_expected_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    // Use the env-var override rather than `set_home_override` to avoid
    // the per-process OnceLock — other tests in the same binary may
    // have already set it.
    unsafe {
        std::env::set_var("ZAD_HOME_OVERRIDE", tmp.path());
    }

    let g: PathBuf = perms::global_path().unwrap();
    let g_s = g.display().to_string();
    let g_slash = g_s.replace('\\', "/");
    assert!(
        g_slash.ends_with("services/discord/permissions.toml"),
        "{g_s}"
    );

    let l: PathBuf = perms::local_path_for("some-slug").unwrap();
    let l_s = l.display().to_string();
    let l_slash = l_s.replace('\\', "/");
    assert!(
        l_slash.ends_with("projects/some-slug/services/discord/permissions.toml"),
        "{l_s}"
    );

    unsafe {
        std::env::remove_var("ZAD_HOME_OVERRIDE");
    }
}
