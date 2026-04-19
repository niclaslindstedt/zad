//! Library-level tests for the staged-commit workflow in
//! `src/permissions/staging.rs`. We drive it through Discord's
//! `PermissionsService` impl so the test surface mirrors what the CLI
//! runner uses in production.

use serial_test::serial;
use zad::error::ZadError;
use zad::permissions::mutation::{ListKind, Mutation};
use zad::permissions::signing::{self, SIGNING_ACCOUNT};
use zad::permissions::{SigningKey, staging};
use zad::service::discord::permissions::{DiscordPermissionsRaw, PermissionsService};

fn with_memory() {
    zad::secrets::use_memory_backend();
    let _ = zad::secrets::delete(SIGNING_ACCOUNT);
}

fn sample_mutation() -> Mutation {
    Mutation::AddPattern {
        function: Some("send".into()),
        target: "channel".into(),
        list: ListKind::Deny,
        value: "admin-*".into(),
    }
}

#[test]
#[serial]
fn pending_path_appends_suffix() {
    let p = std::path::Path::new("/tmp/permissions.toml");
    let pending = staging::pending_path_for(p);
    assert_eq!(
        pending,
        std::path::Path::new("/tmp/permissions.toml.pending")
    );
}

#[test]
#[serial]
fn status_reports_absent_then_present() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    let st = staging::status(&live);
    assert!(!st.live_exists);
    assert!(!st.pending_exists);

    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();
    let st = staging::status(&live);
    assert!(!st.live_exists);
    assert!(st.pending_exists);
}

#[test]
#[serial]
fn mutate_creates_pending_from_starter_when_live_absent() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();
    let pending = staging::pending_path_for(&live);
    assert!(pending.exists());

    let body = std::fs::read_to_string(&pending).unwrap();
    assert!(!body.contains("[signature]"), "pending must be unsigned");
    assert!(body.contains("admin-*"), "mutation must be reflected");
}

#[test]
#[serial]
fn mutate_is_idempotent() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();
    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();

    let body = std::fs::read_to_string(staging::pending_path_for(&live)).unwrap();
    let occurrences = body.matches("admin-*").count();
    assert_eq!(
        occurrences, 1,
        "AddPattern must be idempotent; body: {body}"
    );
}

#[test]
#[serial]
fn discard_removes_pending_only() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    // Seed live with a signed empty policy first.
    let key = SigningKey::generate();
    zad::service::discord::permissions::save_file(&live, &DiscordPermissionsRaw::default(), &key)
        .unwrap();
    assert!(live.exists());

    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();
    assert!(staging::pending_path_for(&live).exists());

    let removed = staging::discard(&live).unwrap();
    assert!(removed);
    assert!(!staging::pending_path_for(&live).exists());
    assert!(live.exists(), "discard must not touch the live file");
}

#[test]
#[serial]
fn commit_signs_and_replaces_live() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    let key = signing::load_or_create_from_keychain().unwrap();
    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();

    staging::commit::<PermissionsService>(&live, &key).unwrap();
    assert!(live.exists(), "commit must create the live file");
    assert!(
        !staging::pending_path_for(&live).exists(),
        "commit must remove the pending file"
    );

    let body = std::fs::read_to_string(&live).unwrap();
    assert!(
        body.contains("[signature]"),
        "committed file must be signed"
    );
    assert!(body.contains("admin-*"), "committed body: {body}");

    // load_file must successfully verify.
    let loaded = zad::service::discord::permissions::load_file(&live).unwrap();
    assert!(loaded.is_some());
}

#[test]
#[serial]
fn commit_without_pending_errors() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    let key = signing::load_or_create_from_keychain().unwrap();
    let err = staging::commit::<PermissionsService>(&live, &key).unwrap_err();
    assert!(
        matches!(err, ZadError::Invalid(ref msg) if msg.contains("no pending changes")),
        "got {err:?}"
    );
}

#[test]
#[serial]
fn sign_in_place_restores_signature_after_hand_edit() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    let key = signing::load_or_create_from_keychain().unwrap();
    zad::service::discord::permissions::save_file(
        &live,
        &zad::service::discord::permissions::starter_template(),
        &key,
    )
    .unwrap();

    // Simulate a hand edit that doesn't touch the signature — loading
    // would now fail (tampered payload).
    let body = std::fs::read_to_string(&live).unwrap();
    let tampered = body.replace("password", "pass-word");
    assert_ne!(
        body, tampered,
        "sentinel substitution must have matched — the starter template contains `password`"
    );
    std::fs::write(&live, &tampered).unwrap();
    assert!(
        zad::service::discord::permissions::load_file(&live).is_err(),
        "tampered load must fail before sign_in_place"
    );

    // Re-sign; load succeeds again.
    staging::sign_in_place::<PermissionsService>(&live, &key).unwrap();
    let loaded = zad::service::discord::permissions::load_file(&live).unwrap();
    assert!(
        loaded.is_some(),
        "sign_in_place must restore a valid signature"
    );
}

#[test]
#[serial]
fn diff_shows_nothing_when_no_pending() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");
    assert!(staging::diff(&live).unwrap().is_none());
}

#[test]
#[serial]
fn diff_returns_body_when_pending_exists() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");

    let key = SigningKey::generate();
    zad::service::discord::permissions::save_file(&live, &DiscordPermissionsRaw::default(), &key)
        .unwrap();

    staging::mutate_pending::<PermissionsService>(&live, &sample_mutation()).unwrap();
    let body = staging::diff(&live).unwrap().expect("pending exists");
    assert!(body.contains("admin-*"), "diff should mention the change");
}

#[test]
#[serial]
fn content_and_time_mutations_round_trip() {
    with_memory();
    let tmp = tempfile::tempdir().unwrap();
    let live = tmp.path().join("permissions.toml");
    let key = signing::load_or_create_from_keychain().unwrap();

    staging::mutate_pending::<PermissionsService>(
        &live,
        &Mutation::AddDenyWord {
            function: None,
            word: "leakme".into(),
        },
    )
    .unwrap();
    staging::mutate_pending::<PermissionsService>(
        &live,
        &Mutation::SetMaxLength {
            function: None,
            value: Some(500),
        },
    )
    .unwrap();
    staging::mutate_pending::<PermissionsService>(
        &live,
        &Mutation::SetTimeDays {
            function: None,
            days: vec!["mon".into(), "wed".into(), "fri".into()],
        },
    )
    .unwrap();

    staging::commit::<PermissionsService>(&live, &key).unwrap();
    let body = std::fs::read_to_string(&live).unwrap();
    assert!(body.contains("leakme"));
    assert!(body.contains("500"));
    assert!(body.contains("mon"));
}
