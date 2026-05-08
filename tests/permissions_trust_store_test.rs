//! Tests for the per-machine trust store at
//! `~/.zad/signing/trusted.toml`. The trust store is the new home for
//! permission-file authorization (signatures no longer live inside the
//! permission files themselves) and is the primary tamper-detection
//! surface — these tests pin its non-negotiables: self-signature
//! verification, keychain cross-check, and symlink refusal.

use serial_test::serial;
use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::permissions::signing::SIGNING_ACCOUNT;
use zad::permissions::trust::{TrustEntry, TrustStore, trust_store_path};

mod common;

fn fresh_test_env() -> tempfile::TempDir {
    zad::secrets::use_memory_backend();
    let _ = zad::secrets::delete(SIGNING_ACCOUNT);
    let home = tempfile::tempdir().unwrap();
    // SAFETY: tests are #[serial], no concurrent writers.
    unsafe {
        std::env::set_var("ZAD_HOME_OVERRIDE", home.path());
    }
    home
}

fn install_keychain_key(key: &SigningKey) {
    let _ = zad::secrets::delete(SIGNING_ACCOUNT);
    zad::secrets::store(SIGNING_ACCOUNT, &key.to_keychain_encoded()).unwrap();
}

fn sample_entry(path: &str) -> TrustEntry {
    TrustEntry {
        path: path.into(),
        algorithm: "ed25519".into(),
        public_key: "pk".into(),
        signed_at: "2026-05-08T00:00:00Z".into(),
        value: "sig".into(),
    }
}

#[test]
#[serial]
fn empty_load_returns_empty_store_when_file_absent() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);
    let store = TrustStore::load().unwrap();
    assert!(store.entries.is_empty());
}

#[test]
#[serial]
fn save_then_load_round_trips_entries() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    let mut store = TrustStore::default();
    store.upsert(sample_entry("/abs/a/permissions.toml"));
    store.upsert(sample_entry("/abs/b/permissions.toml"));
    store.save(&key).unwrap();

    let loaded = TrustStore::load().unwrap();
    assert_eq!(loaded.entries.len(), 2);
    assert!(loaded.entries.contains_key("/abs/a/permissions.toml"));
    assert!(loaded.entries.contains_key("/abs/b/permissions.toml"));
}

#[test]
#[serial]
fn upsert_replaces_existing_entry_for_same_path() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    let mut store = TrustStore::default();
    let mut e1 = sample_entry("/abs/x/permissions.toml");
    e1.public_key = "first".into();
    store.upsert(e1);

    let mut e2 = sample_entry("/abs/x/permissions.toml");
    e2.public_key = "second".into();
    store.upsert(e2);

    assert_eq!(store.entries.len(), 1);
    assert_eq!(
        store.entries["/abs/x/permissions.toml"].public_key,
        "second"
    );
}

#[test]
#[serial]
fn save_writes_self_signature_block() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    TrustStore::default().save(&key).unwrap();

    let body = std::fs::read_to_string(trust_store_path().unwrap()).unwrap();
    assert!(body.contains("[signature]"), "body: {body}");
    assert!(body.contains("ed25519"), "body: {body}");
}

#[test]
#[serial]
fn tampering_with_an_entry_breaks_self_signature() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    let mut store = TrustStore::default();
    store.upsert(sample_entry("/abs/path/permissions.toml"));
    store.save(&key).unwrap();

    // Hand-edit the entry's value field. The trust store self-signature
    // must catch this.
    let body = std::fs::read_to_string(trust_store_path().unwrap()).unwrap();
    // Flip the entry's path field. Targeted substitution that doesn't
    // hit "signed_at" or "[signature]".
    let tampered = body.replace(
        "/abs/path/permissions.toml",
        "/abs/attacker/permissions.toml",
    );
    assert_ne!(body, tampered, "sentinel substitution must have matched");
    std::fs::write(trust_store_path().unwrap(), &tampered).unwrap();

    let err = TrustStore::load().unwrap_err();
    assert!(
        matches!(err, ZadError::TrustStoreTampered { .. }),
        "expected TrustStoreTampered, got {err:?}"
    );
}

#[test]
#[serial]
fn rewriting_the_store_with_an_attacker_key_is_caught() {
    let _home = fresh_test_env();
    let owner = SigningKey::generate();
    install_keychain_key(&owner);

    // Owner signs an empty store.
    TrustStore::default().save(&owner).unwrap();

    // Attacker generates their own keypair, builds a store, signs it
    // with their key, and overwrites the file. The keychain still
    // holds the owner's key.
    let attacker = SigningKey::generate();
    let mut malicious = TrustStore::default();
    malicious.upsert(sample_entry("/abs/malicious/permissions.toml"));
    // Point the trust store path env-var trick: save_at uses an
    // explicit path so we can write the malicious bytes via the
    // attacker's key.
    malicious
        .save_at(&trust_store_path().unwrap(), &attacker)
        .unwrap();

    let err = TrustStore::load().unwrap_err();
    assert!(
        matches!(err, ZadError::TrustStoreTampered { .. }),
        "expected TrustStoreTampered after attacker signature, got {err:?}"
    );
}

#[cfg(unix)]
#[test]
#[serial]
fn symlink_at_trust_store_path_is_refused() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    // Point trusted.toml at a "real" file living elsewhere.
    let real = _home.path().join("real-trust.toml");
    TrustStore::default().save_at(&real, &key).unwrap();

    let store_path = trust_store_path().unwrap();
    std::fs::create_dir_all(store_path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &store_path).unwrap();

    let err = TrustStore::load().unwrap_err();
    match err {
        ZadError::TrustStoreTampered { reason, .. } => {
            assert!(reason.contains("symlink"), "reason: {reason}");
        }
        other => panic!("expected TrustStoreTampered (symlink), got {other:?}"),
    }
}

#[test]
#[serial]
fn save_uses_atomic_replace() {
    let _home = fresh_test_env();
    let key = SigningKey::generate();
    install_keychain_key(&key);

    // Two consecutive saves; both must produce a valid self-signed
    // store. Catches a regression where save isn't atomic and leaves
    // the file half-written.
    let mut store = TrustStore::default();
    store.upsert(sample_entry("/abs/a/permissions.toml"));
    store.save(&key).unwrap();
    store.upsert(sample_entry("/abs/b/permissions.toml"));
    store.save(&key).unwrap();

    let loaded = TrustStore::load().unwrap();
    assert_eq!(loaded.entries.len(), 2);
}

// ensure unused common helper is not flagged
#[test]
#[serial]
fn ensure_signing_env_helper_is_idempotent() {
    let k1 = common::ensure_signing_env();
    let k2 = common::ensure_signing_env();
    assert_eq!(k1.public_key_b64(), k2.public_key_b64());
}
