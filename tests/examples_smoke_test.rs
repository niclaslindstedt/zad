//! OSS_SPEC §13: every example in `examples/` must parse and load via
//! the same code path the production CLI uses. A loose-TOML example
//! that silently stops parsing is worse than no example — this test
//! makes sure `make test` fails the moment an example rots.
//!
//! Example files ship **without** an embedded signature — they are
//! documentation, not authenticated policy, and we don't commit a
//! private key to sign them at rest. The smoke test re-signs a copy
//! of each example in a tempdir with a throwaway key so the
//! verify-on-load path still runs (catching schema drift and
//! pattern-compile regressions).

use std::path::PathBuf;

use zad::permissions::SigningKey;
use zad::service::discord::permissions::{self as discord_permissions, DiscordPermissionsRaw};
use zad::service::gcal::permissions::{self as gcal_permissions, GcalPermissionsRaw};
use zad::service::onepass::permissions::{self as onepass_permissions, OnePassPermissionsRaw};
use zad::service::telegram::permissions::{self as telegram_permissions, TelegramPermissionsRaw};

fn example_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(rel)
}

fn test_key() -> SigningKey {
    zad::secrets::use_memory_backend();
    SigningKey::generate()
}

#[test]
fn discord_permissions_example_loads() {
    let path = example_path("discord-permissions/permissions.toml");
    assert!(
        path.exists(),
        "example file missing at {} — did the §13 restructure get reverted?",
        path.display()
    );
    let body = std::fs::read_to_string(&path).unwrap();
    let raw: DiscordPermissionsRaw = toml::from_str(&body)
        .expect("example permissions file must parse through the production schema");

    let tmp = tempfile::tempdir().unwrap();
    let signed = tmp.path().join("permissions.toml");
    let key = test_key();
    discord_permissions::save_file(&signed, &raw, &key).unwrap();

    let loaded = discord_permissions::load_file(&signed).unwrap();
    assert!(loaded.is_some(), "signed example must verify and compile");
}

#[test]
fn onepass_permissions_example_loads() {
    let path = example_path("1pass-permissions/permissions.toml");
    assert!(
        path.exists(),
        "example file missing at {} — did the §13 restructure get reverted?",
        path.display()
    );
    let body = std::fs::read_to_string(&path).unwrap();
    let raw: OnePassPermissionsRaw = toml::from_str(&body)
        .expect("example permissions file must parse through the production schema");

    let tmp = tempfile::tempdir().unwrap();
    let signed = tmp.path().join("permissions.toml");
    let key = test_key();
    onepass_permissions::save_file(&signed, &raw, &key).unwrap();

    let loaded = onepass_permissions::load_file(&signed).unwrap();
    assert!(loaded.is_some(), "signed example must verify and compile");
}

#[test]
fn telegram_permissions_example_loads() {
    let path = example_path("telegram-permissions/permissions.toml");
    assert!(
        path.exists(),
        "example file missing at {} — did the §13 restructure get reverted?",
        path.display()
    );
    let body = std::fs::read_to_string(&path).unwrap();
    let raw: TelegramPermissionsRaw = toml::from_str(&body)
        .expect("example permissions file must parse through the production schema");

    let tmp = tempfile::tempdir().unwrap();
    let signed = tmp.path().join("permissions.toml");
    let key = test_key();
    telegram_permissions::save_file(&signed, &raw, &key).unwrap();

    let loaded = telegram_permissions::load_file(&signed).unwrap();
    assert!(loaded.is_some(), "signed example must verify and compile");
}

#[test]
fn gcal_permissions_example_loads() {
    let path = example_path("gcal-permissions/permissions.toml");
    assert!(
        path.exists(),
        "example file missing at {} — did the §13 restructure get reverted?",
        path.display()
    );
    let body = std::fs::read_to_string(&path).unwrap();
    let raw: GcalPermissionsRaw = toml::from_str(&body)
        .expect("example permissions file must parse through the production schema");

    let tmp = tempfile::tempdir().unwrap();
    let signed = tmp.path().join("permissions.toml");
    let key = test_key();
    gcal_permissions::save_file(&signed, &raw, &key).unwrap();

    let loaded = gcal_permissions::load_file(&signed).unwrap();
    assert!(loaded.is_some(), "signed example must verify and compile");
}
