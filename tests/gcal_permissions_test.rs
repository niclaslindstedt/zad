//! Unit tests for the gcal permissions schema: compile / load, alias
//! matching, numeric-cap intersection, hard-coded reminder cap,
//! starter template round-trip.

use std::path::PathBuf;

use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::service::gcal::permissions::{
    self, EffectivePermissions, FunctionBlockRaw, GcalFunction, GcalPermissionsRaw,
    HARD_REMINDER_MINUTES_CAP,
};

fn test_key() -> SigningKey {
    zad::secrets::use_memory_backend();
    SigningKey::generate()
}

fn tempfile_with(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("permissions.toml");
    // Parse + re-sign so verify_raw succeeds. Empty bodies are legal.
    let raw: GcalPermissionsRaw = toml::from_str(contents).expect("test body must be valid TOML");
    let key = test_key();
    permissions::save_file(&path, &raw, &key).unwrap();
    (dir, path)
}

fn compile_effective(
    global_body: &str,
    local_body: Option<&str>,
) -> (Vec<tempfile::TempDir>, EffectivePermissions) {
    let mut keepalive = vec![];
    let (gdir, gpath) = tempfile_with(global_body);
    keepalive.push(gdir);
    let global = permissions::load_file(&gpath).unwrap();
    let local = match local_body {
        Some(body) => {
            let (ldir, lpath) = tempfile_with(body);
            keepalive.push(ldir);
            permissions::load_file(&lpath).unwrap()
        }
        None => None,
    };
    (keepalive, EffectivePermissions { global, local })
}

#[test]
fn empty_file_admits_everything() {
    let (_k, eff) = compile_effective("", None);
    assert!(
        eff.check_calendar(GcalFunction::CreateEvent, "primary", "primary")
            .is_ok()
    );
    assert!(
        eff.check_attendee(GcalFunction::CreateEvent, "alice@example.com", None)
            .is_ok()
    );
}

#[test]
fn calendar_allow_restricts_to_primary() {
    let global = r#"
[create_event]
calendars.allow = ["primary"]
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.check_calendar(GcalFunction::CreateEvent, "primary", "primary")
            .is_ok()
    );
    let err = eff
        .check_calendar(
            GcalFunction::CreateEvent,
            "other@example.com",
            "other@example.com",
        )
        .unwrap_err();
    matches!(err, ZadError::PermissionDenied { .. });
}

#[test]
fn calendar_deny_beats_allow() {
    let global = r#"
[create_event]
calendars.allow = ["*"]
calendars.deny  = ["*personal*"]
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.check_calendar(
            GcalFunction::CreateEvent,
            "work@example.com",
            "work@example.com"
        )
        .is_ok()
    );
    assert!(
        eff.check_calendar(
            GcalFunction::CreateEvent,
            "my-personal@example.com",
            "my-personal@example.com"
        )
        .is_err()
    );
}

#[test]
fn attendee_allow_matches_glob_and_self_email() {
    let global = r#"
[create_event]
attendees.allow = ["*@mycompany.com", "@me"]
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.check_attendee(GcalFunction::CreateEvent, "bob@mycompany.com", None)
            .is_ok()
    );
    assert!(
        eff.check_attendee(GcalFunction::CreateEvent, "bob@other.com", None)
            .is_err()
    );
    // @me resolves against self_email when the alias list is checked.
    assert!(
        eff.check_attendee(
            GcalFunction::CreateEvent,
            "@me",
            Some("alice@mycompany.com")
        )
        .is_ok()
    );
}

#[test]
fn numeric_caps_intersect_strictly_across_layers() {
    let global = r#"
[create_event]
max_future_days = 30
max_attendees   = 50
"#;
    let local = r#"
[create_event]
max_future_days = 7
max_attendees   = 10
"#;
    let (_k, eff) = compile_effective(global, Some(local));
    // 5 days / 3 attendees — passes both.
    assert!(
        eff.check_event_caps(GcalFunction::CreateEvent, Some(5), Some(60 * 24), Some(3))
            .is_ok()
    );
    // 10 days — fails local's 7-day cap (strictest wins).
    assert!(
        eff.check_event_caps(GcalFunction::CreateEvent, Some(10), Some(60 * 24), Some(3))
            .is_err()
    );
    // 20 attendees — fails local's 10 cap even though it passes global.
    assert!(
        eff.check_event_caps(GcalFunction::CreateEvent, Some(3), Some(60 * 24), Some(20))
            .is_err()
    );
}

#[test]
fn min_notice_minutes_denies_events_too_soon() {
    let global = r#"
[create_event]
min_notice_minutes = 60
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.check_event_caps(GcalFunction::CreateEvent, Some(1), Some(30), None)
            .is_err()
    );
    assert!(
        eff.check_event_caps(GcalFunction::CreateEvent, Some(1), Some(120), None)
            .is_ok()
    );
}

#[test]
fn send_updates_allowed_restricts_value_set() {
    let global = r#"
[create_event]
send_updates_allowed = { allow = ["none", "external"] }
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.check_send_updates(GcalFunction::CreateEvent, "none")
            .is_ok()
    );
    assert!(
        eff.check_send_updates(GcalFunction::CreateEvent, "external")
            .is_ok()
    );
    assert!(
        eff.check_send_updates(GcalFunction::CreateEvent, "all")
            .is_err()
    );
}

#[test]
fn reminder_cap_is_enforced_even_without_config() {
    let (_k, eff) = compile_effective("", None);
    // Well below the cap.
    assert!(
        eff.check_reminder_minutes(GcalFunction::CreateEvent, 60)
            .is_ok()
    );
    // Exceeds the hard-coded cap.
    let err = eff
        .check_reminder_minutes(GcalFunction::CreateEvent, HARD_REMINDER_MINUTES_CAP + 1)
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { reason, .. } => {
            assert!(reason.contains("built-in safety cap"), "got: {reason}");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn block_shared_calendars_surfaces_source_path() {
    let global = r#"
[create_event]
block_shared_calendars = true
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.block_shared_calendars(GcalFunction::CreateEvent)
            .is_some()
    );
    assert!(
        eff.block_shared_calendars(GcalFunction::ListEvents)
            .is_none()
    );
}

#[test]
fn delete_event_default_deny_pattern_works() {
    // Matches the starter template's default-deny on delete.
    let global = r#"
[delete_event]
calendars.allow = []
calendars.deny  = ["*"]
"#;
    let (_k, eff) = compile_effective(global, None);
    assert!(
        eff.check_calendar(GcalFunction::DeleteEvent, "primary", "primary")
            .is_err()
    );
}

#[test]
fn starter_template_round_trips_through_toml() {
    let raw = permissions::starter_template();
    let body = toml::to_string_pretty(&raw).unwrap();
    let parsed: GcalPermissionsRaw = toml::from_str(&body).unwrap();
    assert_eq!(raw, parsed);
}

#[test]
fn function_parse_error_names_valid_verbs() {
    let err = GcalFunction::parse("nope").unwrap_err();
    match err {
        ZadError::Invalid(msg) => {
            assert!(msg.contains("create_event"), "got: {msg}");
            assert!(msg.contains("invite"), "got: {msg}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn function_block_raw_default_is_empty() {
    let b = FunctionBlockRaw::default();
    assert!(b.calendars.allow.is_empty() && b.calendars.deny.is_empty());
    assert!(b.max_future_days.is_none());
}
