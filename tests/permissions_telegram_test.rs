//! Integration tests for the Telegram-specific permissions layer:
//! file loading, the global+local intersection rule, and the
//! per-function enforcement entry points. These never hit the Bot
//! API — they drive the compiled policy directly, the same way the
//! CLI verbs will after resolving names.

use zad::error::ZadError;
use zad::permissions::content::ContentRulesRaw;
use zad::permissions::pattern::PatternListRaw;
use zad::service::telegram::directory::Directory;
use zad::service::telegram::permissions::{
    self as perms, EffectivePermissions, FunctionBlockRaw, TelegramPermissions,
    TelegramPermissionsRaw,
};

fn raw_with_send_allow(allow: Vec<&str>) -> TelegramPermissionsRaw {
    TelegramPermissionsRaw {
        send: FunctionBlockRaw {
            chats: PatternListRaw {
                allow: allow.into_iter().map(String::from).collect(),
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..TelegramPermissionsRaw::default()
    }
}

fn write_raw(path: &std::path::Path, raw: &TelegramPermissionsRaw) {
    perms::save_file(path, raw).unwrap();
}

fn load(path: &std::path::Path) -> TelegramPermissions {
    perms::load_file(path).unwrap().unwrap()
}

fn eff(
    global: Option<TelegramPermissions>,
    local: Option<TelegramPermissions>,
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
    std::fs::write(&p, "[send]\nchats.allow = [\"re:(\"]\n").unwrap();
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
fn send_chat_is_denied_when_not_in_allow_list() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(&p, &raw_with_send_allow(vec!["team-room"]));
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let err = effective
        .check_send_chat("marketing", 111, &empty_directory())
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

    // `team-room` is allowed.
    assert!(
        effective
            .check_send_chat("team-room", 222, &empty_directory())
            .is_ok()
    );
}

#[test]
fn deny_pattern_fires_on_reverse_lookup_name_even_when_agent_passed_id() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(
        &p,
        &TelegramPermissionsRaw {
            send: FunctionBlockRaw {
                chats: PatternListRaw {
                    allow: vec![],
                    deny: vec!["*admin*".into()],
                },
                ..FunctionBlockRaw::default()
            },
            ..TelegramPermissionsRaw::default()
        },
    );
    let pol = load(&p);
    let effective = eff(None, Some(pol));

    let mut dir = Directory::default();
    dir.chats
        .insert("ops-admin".into(), "-1001234567890".into());

    // The agent passed the numeric chat_id — the deny still fires
    // because `ops-admin` is a reverse-lookup alias for that ID.
    let err = effective
        .check_send_chat("-1001234567890", -1001234567890, &dir)
        .unwrap_err();
    assert!(matches!(err, ZadError::PermissionDenied { .. }));
}

#[test]
fn body_deny_words_block_send() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    write_raw(
        &p,
        &TelegramPermissionsRaw {
            content: ContentRulesRaw {
                deny_words: vec!["api_key".into()],
                ..Default::default()
            },
            ..TelegramPermissionsRaw::default()
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

// ---------------------------------------------------------------------------
// global ∩ local
// ---------------------------------------------------------------------------

#[test]
fn global_and_local_both_must_admit() {
    let tmp = tempfile::tempdir().unwrap();
    let g = tmp.path().join("global.toml");
    let l = tmp.path().join("local.toml");
    write_raw(&g, &raw_with_send_allow(vec!["team-room", "bot-*"]));
    write_raw(&l, &raw_with_send_allow(vec!["bot-ci"]));
    let effective = eff(Some(load(&g)), Some(load(&l)));

    // `team-room`: allowed by global, denied by local → overall denied.
    let err = effective
        .check_send_chat("team-room", 1, &empty_directory())
        .unwrap_err();
    if let ZadError::PermissionDenied { config_path, .. } = err {
        assert_eq!(config_path, l, "local should be the rejecting file");
    } else {
        panic!("unexpected: {err:?}");
    }

    // `bot-ci`: allowed by both.
    assert!(
        effective
            .check_send_chat("bot-ci", 2, &empty_directory())
            .is_ok()
    );

    // `bot-foo`: allowed by global, rejected by local's tighter list.
    assert!(
        effective
            .check_send_chat("bot-foo", 3, &empty_directory())
            .is_err()
    );
}

#[test]
fn no_files_present_means_no_permission_restrictions() {
    let effective = eff(None, None);
    assert!(!effective.any());
    assert!(
        effective
            .check_send_chat("anywhere", 42, &empty_directory())
            .is_ok()
    );
    assert!(effective.check_send_body("anything").is_ok());
}
