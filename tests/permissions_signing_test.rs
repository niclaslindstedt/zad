//! Tests for the Ed25519 signing layer that backs every service's
//! permission files. We exercise the full sign → write → load → verify
//! loop against the real Discord `*Raw` struct (so the canonical
//! serialization under test matches what production uses) and then
//! cover the failure modes: tampered payload, missing signature,
//! wrong public key, keychain pubkey mismatch.
//!
//! All cases route through the in-memory keychain backend so nothing
//! touches the operator's OS keychain.

use serial_test::serial;
use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::permissions::pattern::PatternListRaw;
use zad::permissions::signing::{self, ALGORITHM, SIGNING_ACCOUNT};
use zad::service::discord::permissions::{self as perms, DiscordPermissionsRaw, FunctionBlockRaw};

fn with_memory_secrets() {
    zad::secrets::use_memory_backend();
}

fn sample_raw() -> DiscordPermissionsRaw {
    DiscordPermissionsRaw {
        send: FunctionBlockRaw {
            channels: PatternListRaw {
                allow: vec!["bot-*".into()],
                deny: vec!["*admin*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        ..DiscordPermissionsRaw::default()
    }
}

#[test]
#[serial]
fn round_trip_sign_and_verify() {
    with_memory_secrets();
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");

    let raw = sample_raw();
    let key = SigningKey::generate();
    perms::save_file(&p, &raw, &key).unwrap();

    // File should now have a signature on disk.
    let body = std::fs::read_to_string(&p).unwrap();
    assert!(body.contains("[signature]"), "body: {body}");
    assert!(body.contains(ALGORITHM), "body: {body}");

    // load_file must successfully verify.
    let loaded = perms::load_file(&p).unwrap();
    assert!(loaded.is_some());
}

#[test]
#[serial]
fn tamper_in_payload_is_caught() {
    with_memory_secrets();
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let raw = sample_raw();
    let key = SigningKey::generate();
    perms::save_file(&p, &raw, &key).unwrap();

    // Flip a byte in the body — replace the allow pattern with a
    // different one without touching [signature].
    let body = std::fs::read_to_string(&p).unwrap();
    let tampered = body.replace("bot-*", "bot-?");
    assert_ne!(body, tampered, "sentinel substitution must have matched");
    std::fs::write(&p, &tampered).unwrap();

    let err = perms::load_file(&p).unwrap_err();
    assert!(
        matches!(err, ZadError::SignatureInvalid { .. }),
        "expected SignatureInvalid, got {err:?}"
    );
}

#[test]
#[serial]
fn missing_signature_is_caught() {
    with_memory_secrets();
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    // Write an unsigned file directly (no signing key).
    perms::save_unsigned(&p, &sample_raw()).unwrap();

    let err = perms::load_file(&p).unwrap_err();
    assert!(
        matches!(err, ZadError::SignatureMissing { .. }),
        "expected SignatureMissing, got {err:?}"
    );
}

#[test]
#[serial]
fn wrong_public_key_is_caught() {
    with_memory_secrets();
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    let key = SigningKey::generate();
    perms::save_file(&p, &sample_raw(), &key).unwrap();

    // Rewrite the embedded public_key with a fresh (valid-shape)
    // pubkey that didn't author the signature.
    let body = std::fs::read_to_string(&p).unwrap();
    let imposter = SigningKey::generate();
    let imposter_pub = imposter.public_key_b64();
    let new_body = body
        .lines()
        .map(|line| {
            if line.starts_with("public_key = ") {
                format!("public_key = \"{imposter_pub}\"")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&p, &new_body).unwrap();

    let err = perms::load_file(&p).unwrap_err();
    assert!(
        matches!(err, ZadError::SignatureInvalid { .. }),
        "expected SignatureInvalid (wrong pubkey), got {err:?}"
    );
}

#[test]
#[serial]
fn keychain_mismatch_is_caught() {
    with_memory_secrets();
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("permissions.toml");
    // Sign with ephemeral key A.
    let author = SigningKey::generate();
    perms::save_file(&p, &sample_raw(), &author).unwrap();

    // Install a DIFFERENT signing key B into the in-memory keychain.
    let operator = SigningKey::generate();
    zad::secrets::store(SIGNING_ACCOUNT, &operator.to_keychain_encoded()).unwrap();

    let err = perms::load_file(&p).unwrap_err();
    assert!(
        matches!(err, ZadError::SignatureKeyMismatch { .. }),
        "expected SignatureKeyMismatch, got {err:?}"
    );

    // Cleanup so other tests aren't affected.
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();
}

#[test]
#[serial]
fn load_or_create_from_keychain_is_idempotent() {
    with_memory_secrets();
    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let first = signing::load_or_create_from_keychain().unwrap();
    let second = signing::load_or_create_from_keychain().unwrap();
    assert_eq!(
        first.public_key_b64(),
        second.public_key_b64(),
        "the second call must return the same key"
    );

    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();
}

#[test]
#[serial]
fn signature_is_deterministic_for_same_raw_and_key() {
    // Ed25519 signatures are deterministic — this guards against an
    // accidental introduction of a non-deterministic scheme or against
    // `toml::to_string_pretty` producing unstable output for the same
    // input struct within one crate version.
    let key = SigningKey::generate();
    let raw = sample_raw();
    let sig1 = signing::sign_raw(&raw, &key).unwrap();
    let sig2 = signing::sign_raw(&raw, &key).unwrap();
    assert_eq!(sig1.value, sig2.value);
    assert_eq!(sig1.public_key, sig2.public_key);
}

#[test]
#[serial]
fn keychain_encoded_round_trip() {
    let key = SigningKey::generate();
    let encoded = key.to_keychain_encoded();
    let decoded = SigningKey::from_keychain_encoded(&encoded).unwrap();
    assert_eq!(decoded.public_key_b64(), key.public_key_b64());
    assert_eq!(decoded.fingerprint(), key.fingerprint());
}
