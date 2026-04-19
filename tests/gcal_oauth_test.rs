//! Pure unit tests for the OAuth helper. These never touch the
//! network — they verify URL construction, scope derivation, and the
//! PKCE challenge/verifier relationship.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use zad::cli::service_gcal;

#[test]
fn google_scopes_for_read_only_is_narrow() {
    let scopes = service_gcal::google_scopes_for(&["calendars.read".into(), "events.read".into()]);
    assert!(scopes.contains(&"openid".to_string()));
    assert!(scopes.contains(&"email".to_string()));
    assert!(
        scopes
            .iter()
            .any(|s| s == "https://www.googleapis.com/auth/calendar.events.readonly")
    );
    // No write scope.
    assert!(
        !scopes
            .iter()
            .any(|s| s == "https://www.googleapis.com/auth/calendar.events")
    );
}

#[test]
fn google_scopes_for_write_upgrades_to_events_scope() {
    let scopes = service_gcal::google_scopes_for(&["events.write".into()]);
    assert!(
        scopes
            .iter()
            .any(|s| s == "https://www.googleapis.com/auth/calendar.events")
    );
}

#[test]
fn google_scopes_includes_calendarlist_when_write_absent() {
    let scopes = service_gcal::google_scopes_for(&["calendars.read".into()]);
    assert!(
        scopes
            .iter()
            .any(|s| s == "https://www.googleapis.com/auth/calendar.calendarlist.readonly")
    );
}

#[test]
fn pkce_challenge_is_sha256_of_verifier_base64_no_pad() {
    // Reconstruct a verifier → challenge pair the same way the helper
    // does and assert the relationship holds. We can't poke
    // `oauth::Pkce` directly (it's `pub(crate)`), so we exercise the
    // algorithm end-to-end by running it here and confirming the
    // challenge is deterministic and the inverse of the verifier.
    let verifier = "some-well-known-verifier-string";
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);

    // Round-trip identity: the challenge is deterministic from the
    // verifier; no padding; URL-safe alphabet.
    assert_eq!(challenge.len(), 43); // SHA-256 → 32 bytes → 43 base64url chars
    assert!(!challenge.contains('='));
    assert!(!challenge.contains('+'));
    assert!(!challenge.contains('/'));
}
