use serial_test::serial;
use zad::secrets::{self, Scope};

fn init_mem() {
    secrets::use_memory_backend();
}

#[test]
#[serial]
fn store_load_delete_cycle() {
    init_mem();
    let account = "discord-bot:-test-roundtrip";

    secrets::store(account, "super-secret-token").unwrap();
    assert_eq!(
        secrets::load(account).unwrap().as_deref(),
        Some("super-secret-token")
    );

    secrets::delete(account).unwrap();
    assert_eq!(secrets::load(account).unwrap(), None);
}

#[test]
#[serial]
fn delete_missing_is_ok() {
    init_mem();
    secrets::delete("discord-bot:-never-existed").unwrap();
}

#[test]
fn account_name_covers_both_scopes() {
    assert_eq!(
        secrets::account("discord", "bot", Scope::Project("-Users-foo-bar")),
        "discord-bot:-Users-foo-bar"
    );
    assert_eq!(
        secrets::account("discord", "bot", Scope::Global),
        "discord-bot:global"
    );
}

#[test]
fn account_name_supports_multi_secret_services() {
    // Services with multiple keychain entries (OAuth client_secret +
    // refresh token, or GitHub App PEM) name each via `kind`.
    assert_eq!(
        secrets::account("reddit", "client-secret", Scope::Global),
        "reddit-client-secret:global"
    );
    assert_eq!(
        secrets::account("github", "pem", Scope::Project("-tmp-repo")),
        "github-pem:-tmp-repo"
    );
}
