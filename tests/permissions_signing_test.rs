//! Tests for the Ed25519 signing layer that backs every service's
//! permission files.
//!
//! Permission files are unsigned on disk; their authorization lives
//! in `~/.zad/signing/trusted.toml`, signed by the keychain key. Each
//! test installs a signing key into the in-memory keychain backend
//! before exercising the round trip — verification refuses to proceed
//! without one.

use serial_test::serial;
use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::permissions::pattern::PatternListRaw;
use zad::permissions::signing::{self, ALGORITHM, SIGNING_ACCOUNT};
use zad::service::discord::permissions::{self as perms, DiscordPermissionsRaw, FunctionBlockRaw};

fn fresh_test_env() -> tempfile::TempDir {
    zad::secrets::use_memory_backend();
    let _ = zad::secrets::delete(SIGNING_ACCOUNT);
    let home = tempfile::tempdir().unwrap();
    // Use the env var (not set_home_override) — the latter is OnceLock,
    // so it would freeze the home dir to whatever the first test set
    // it to. Env vars can be reset per #[serial] test.
    // SAFETY: tests are #[serial], so no concurrent writers.
    unsafe {
        std::env::set_var("ZAD_HOME_OVERRIDE", home.path());
    }
    home
}

fn install_keychain_key(key: &SigningKey) {
    let _ = zad::secrets::delete(SIGNING_ACCOUNT);
    zad::secrets::store(SIGNING_ACCOUNT, &key.to_keychain_encoded()).unwrap();
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
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    let p = _home.path().join("permissions.toml");
    let raw = sample_raw();
    perms::save_file(&p, &raw, &key).unwrap();

    // The permissions file body itself does NOT carry a signature
    // anymore — authorization lives in the trust store.
    let body = std::fs::read_to_string(&p).unwrap();
    assert!(
        !body.contains("[signature]"),
        "permissions file must not embed a signature: {body}"
    );

    // load_file must successfully verify against the trust store.
    let loaded = perms::load_file(&p).unwrap();
    assert!(loaded.is_some());
}

#[test]
#[serial]
fn tamper_in_payload_is_caught() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    let p = _home.path().join("permissions.toml");
    perms::save_file(&p, &sample_raw(), &key).unwrap();

    // Flip a byte in the body — replace the allow pattern with a
    // different one. The trust store's signature for this path no
    // longer matches the file's bytes.
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
fn no_trust_entry_fails_closed() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);
    // Initialize trust store as empty.
    zad::permissions::TrustStore::default().save(&key).unwrap();

    let p = _home.path().join("permissions.toml");
    // Write the body unsigned, but DON'T sign-to-trust-store.
    perms::save_unsigned(&p, &sample_raw()).unwrap();

    let err = perms::load_file(&p).unwrap_err();
    assert!(
        matches!(err, ZadError::NotTrusted { .. }),
        "expected NotTrusted, got {err:?}"
    );
}

#[test]
#[serial]
fn keychain_empty_falls_back_to_self_signature_in_memory_mode() {
    let _home = fresh_test_env();
    // In memory mode the OS keychain has no real gate to enforce, so
    // verification falls back to the trust store's own self-signature
    // when the keychain is empty. This keeps tests workable; the
    // `SigningKeyMissing` hard-fail still applies in production.
    let key = SigningKey::generate();
    install_keychain_key(&key);
    let p = _home.path().join("permissions.toml");
    perms::save_file(&p, &sample_raw(), &key).unwrap();

    zad::secrets::delete(SIGNING_ACCOUNT).unwrap();

    let loaded = perms::load_file(&p).unwrap();
    assert!(
        loaded.is_some(),
        "memory-mode load must succeed without a keychain key"
    );
}

#[test]
#[serial]
fn keychain_mismatch_is_caught() {
    let _home = fresh_test_env();
    let author = SigningKey::generate();
    install_keychain_key(&author);

    let p = _home.path().join("permissions.toml");
    perms::save_file(&p, &sample_raw(), &author).unwrap();

    // Rotate the keychain to a DIFFERENT key — old trust entries are
    // signed by the previous key, new keychain holds a different one.
    let operator = SigningKey::generate();
    install_keychain_key(&operator);

    let err = perms::load_file(&p).unwrap_err();
    // The trust store itself was signed by `author` (the previous
    // keychain key), so its self-signature now fails to verify against
    // the rotated keychain key — surfacing as TrustStoreTampered.
    assert!(
        matches!(err, ZadError::TrustStoreTampered { .. }),
        "expected TrustStoreTampered after keychain rotation, got {err:?}"
    );
}

#[test]
#[serial]
fn load_or_create_from_keychain_is_idempotent() {
    fresh_test_env();
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
    let sig1 = signing::sign_unsigned(&raw, &key).unwrap();
    let sig2 = signing::sign_unsigned(&raw, &key).unwrap();
    assert_eq!(sig1.value, sig2.value);
    assert_eq!(sig1.public_key, sig2.public_key);
    assert_eq!(sig1.algorithm, ALGORITHM);
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
