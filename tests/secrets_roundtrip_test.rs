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
        secrets::discord_bot_account(Scope::Project("-Users-foo-bar")),
        "discord-bot:-Users-foo-bar"
    );
    assert_eq!(
        secrets::discord_bot_account(Scope::Global),
        "discord-bot:global"
    );
}
