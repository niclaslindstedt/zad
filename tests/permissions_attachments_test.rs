//! Unit tests for the shared attachments permissions primitive. The
//! primitive is reused by every service's `[send.attachments]` block,
//! so the matrix here is service-agnostic; service-level integration
//! (layer intersection through `check_send_attachments`) is covered by
//! the Discord integration test further down.

use std::path::PathBuf;

use zad::config::directory::Directory;
use zad::permissions::attachments::{AttachmentInfo, AttachmentRules, AttachmentRulesRaw};
use zad::permissions::pattern::PatternListRaw;
use zad::service::discord::permissions::{
    DiscordPermissionsRaw, EffectivePermissions, FunctionBlockRaw, load_file, save_file,
};

fn info(name: &str, ext: &str, bytes: u64) -> AttachmentInfo {
    AttachmentInfo {
        path: PathBuf::from(name),
        basename: name.into(),
        extension: ext.into(),
        bytes,
    }
}

fn rules(raw: AttachmentRulesRaw) -> AttachmentRules {
    AttachmentRules::compile(&raw).unwrap()
}

// ---------------------------------------------------------------------------
// max_count
// ---------------------------------------------------------------------------

#[test]
fn max_count_rejects_over_cap() {
    let r = rules(AttachmentRulesRaw {
        max_count: Some(2),
        ..Default::default()
    });
    let files = vec![
        info("a.txt", "txt", 10),
        info("b.txt", "txt", 10),
        info("c.txt", "txt", 10),
    ];
    let err = r.evaluate(&files).unwrap_err();
    let msg = err.as_sentence();
    assert!(msg.contains("3"), "reason: {msg}");
    assert!(msg.contains("cap of 2"), "reason: {msg}");
}

#[test]
fn max_count_admits_at_cap() {
    let r = rules(AttachmentRulesRaw {
        max_count: Some(2),
        ..Default::default()
    });
    let files = vec![info("a.txt", "txt", 10), info("b.txt", "txt", 10)];
    r.evaluate(&files).unwrap();
}

// ---------------------------------------------------------------------------
// max_size_bytes
// ---------------------------------------------------------------------------

#[test]
fn max_size_rejects_oversized_file() {
    let r = rules(AttachmentRulesRaw {
        max_size_bytes: Some(100),
        ..Default::default()
    });
    let err = r
        .evaluate(&[info("small.txt", "txt", 50), info("big.log", "log", 200)])
        .unwrap_err();
    let msg = err.as_sentence();
    assert!(msg.contains("big.log"), "reason: {msg}");
    assert!(msg.contains("200"), "reason: {msg}");
    assert!(msg.contains("100"), "reason: {msg}");
}

// ---------------------------------------------------------------------------
// extensions allow/deny
// ---------------------------------------------------------------------------

#[test]
fn extensions_deny_blocks_match() {
    let r = rules(AttachmentRulesRaw {
        extensions: PatternListRaw {
            allow: vec![],
            deny: vec!["exe".into(), "sh".into()],
        },
        ..Default::default()
    });
    let err = r.evaluate(&[info("nasty.exe", "exe", 1)]).unwrap_err();
    let msg = err.as_sentence();
    assert!(msg.contains("nasty.exe"), "reason: {msg}");
    assert!(msg.contains("exe"), "reason: {msg}");
}

#[test]
fn extensions_allow_rejects_miss() {
    let r = rules(AttachmentRulesRaw {
        extensions: PatternListRaw {
            allow: vec!["png".into(), "jpg".into()],
            deny: vec![],
        },
        ..Default::default()
    });
    let err = r.evaluate(&[info("doc.pdf", "pdf", 1)]).unwrap_err();
    let msg = err.as_sentence();
    assert!(msg.contains("doc.pdf"), "reason: {msg}");
    assert!(msg.contains("not in allow list"), "reason: {msg}");
}

#[test]
fn extensions_allow_admits_hit() {
    let r = rules(AttachmentRulesRaw {
        extensions: PatternListRaw {
            allow: vec!["png".into()],
            deny: vec![],
        },
        ..Default::default()
    });
    r.evaluate(&[info("ok.png", "png", 1)]).unwrap();
}

// ---------------------------------------------------------------------------
// deny_filenames
// ---------------------------------------------------------------------------

#[test]
fn deny_filenames_matches_basename_glob() {
    let r = rules(AttachmentRulesRaw {
        deny_filenames: PatternListRaw {
            allow: vec![],
            deny: vec![".env*".into(), "id_rsa*".into()],
        },
        ..Default::default()
    });
    let err = r.evaluate(&[info(".env.prod", "prod", 1)]).unwrap_err();
    let msg = err.as_sentence();
    assert!(msg.contains(".env.prod"), "reason: {msg}");
    assert!(msg.contains(".env*"), "reason: {msg}");
}

// ---------------------------------------------------------------------------
// merge semantics (in isolation, without the service layer)
// ---------------------------------------------------------------------------

#[test]
fn merge_picks_min_cap() {
    let a = rules(AttachmentRulesRaw {
        max_count: Some(5),
        ..Default::default()
    });
    let b = rules(AttachmentRulesRaw {
        max_count: Some(3),
        ..Default::default()
    });
    let merged = a.merge(b);
    assert_eq!(merged.max_count, Some(3));
}

#[test]
fn merge_unions_deny_lists() {
    let a = rules(AttachmentRulesRaw {
        deny_filenames: PatternListRaw {
            allow: vec![],
            deny: vec!["*.pem".into()],
        },
        ..Default::default()
    });
    let b = rules(AttachmentRulesRaw {
        deny_filenames: PatternListRaw {
            allow: vec![],
            deny: vec![".env*".into()],
        },
        ..Default::default()
    });
    let merged = a.merge(b);
    // Both patterns apply: hits on either should deny.
    let err = merged.evaluate(&[info(".env", "", 1)]).unwrap_err();
    assert!(err.as_sentence().contains(".env*"));
    let err = merged.evaluate(&[info("key.pem", "pem", 1)]).unwrap_err();
    assert!(err.as_sentence().contains("*.pem"));
}

// ---------------------------------------------------------------------------
// service-level layering through Discord's check_send_attachments.
// Mirrors the shape existing Discord/Telegram permission integration
// tests use — global and local files both narrow.
// ---------------------------------------------------------------------------

fn test_key() -> zad::permissions::SigningKey {
    zad::secrets::use_memory_backend();
    zad::permissions::SigningKey::generate()
}

fn write_perms(path: &std::path::Path, raw: &DiscordPermissionsRaw) {
    let key = test_key();
    save_file(path, raw, &key).unwrap();
}

fn eff_with(
    global: Option<&std::path::Path>,
    local: Option<&std::path::Path>,
) -> EffectivePermissions {
    EffectivePermissions {
        global: global.map(|p| load_file(p).unwrap().unwrap()),
        local: local.map(|p| load_file(p).unwrap().unwrap()),
    }
}

fn raw_with_attachments(attachments: AttachmentRulesRaw) -> DiscordPermissionsRaw {
    DiscordPermissionsRaw {
        send: FunctionBlockRaw {
            attachments,
            ..FunctionBlockRaw::default()
        },
        ..DiscordPermissionsRaw::default()
    }
}

#[test]
fn layered_both_files_must_admit() {
    let tmp = tempfile::tempdir().unwrap();
    let g = tmp.path().join("g.toml");
    let l = tmp.path().join("l.toml");
    write_perms(
        &g,
        &raw_with_attachments(AttachmentRulesRaw {
            max_count: Some(5),
            ..Default::default()
        }),
    );
    write_perms(
        &l,
        &raw_with_attachments(AttachmentRulesRaw {
            extensions: PatternListRaw {
                allow: vec!["png".into()],
                deny: vec![],
            },
            ..Default::default()
        }),
    );

    let eff = eff_with(Some(&g), Some(&l));
    // Global admits (≤5), but local denies pdf — the denial must
    // surface the local file path.
    let err = eff
        .check_send_attachments(&[info("doc.pdf", "pdf", 1)])
        .unwrap_err();
    match err {
        zad::error::ZadError::PermissionDenied {
            function,
            config_path,
            ..
        } => {
            assert_eq!(function, "send");
            assert_eq!(config_path, l);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn layered_global_deny_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let g = tmp.path().join("g.toml");
    let l = tmp.path().join("l.toml");
    write_perms(
        &g,
        &raw_with_attachments(AttachmentRulesRaw {
            max_count: Some(1),
            ..Default::default()
        }),
    );
    // Local is permissive. Global's cap must still apply.
    write_perms(&l, &raw_with_attachments(AttachmentRulesRaw::default()));
    let eff = eff_with(Some(&g), Some(&l));

    let err = eff
        .check_send_attachments(&[info("a.txt", "txt", 1), info("b.txt", "txt", 1)])
        .unwrap_err();
    match err {
        zad::error::ZadError::PermissionDenied { config_path, .. } => {
            assert_eq!(config_path, g);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn layered_missing_files_admit() {
    let eff = EffectivePermissions::default();
    eff.check_send_attachments(&[info("anything.bin", "bin", 9999)])
        .unwrap();
}

// Silence the unused-import warning for the directory helper — keeps
// the file ready for future tests that need to resolve names.
#[allow(dead_code)]
fn _unused_directory() -> Directory {
    Directory::default()
}
