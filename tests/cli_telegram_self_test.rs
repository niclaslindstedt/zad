//! Tests for `zad telegram self {show,set,clear,capture}` and for the
//! `@me` sigil in `zad telegram send --chat @me`.
//!
//! Follows the same no-HTTP-mocking discipline as the rest of the suite:
//! the `capture` happy path (which polls `getUpdates`) is covered only
//! by manual E2E. CI exercises `show/set/clear` (pure config I/O) plus
//! the error branches that never reach the network.

use assert_cmd::Command;
use predicates::str::contains;
use serial_test::serial;

mod common;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

fn seed_global(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "scopes = [\"chats\", \"messages.read\", \"messages.send\"]\n",
    )
    .unwrap();
}

fn seed_global_with_self(home: &std::path::Path, self_chat_id: i64) {
    let p = home
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        format!(
            "scopes = [\"chats\", \"messages.read\", \"messages.send\"]\nself_chat_id = {self_chat_id}\n"
        ),
    )
    .unwrap();
}

fn enable_telegram(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "telegram"])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// self show / set / clear
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn self_show_reports_not_configured_when_unset() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "self", "show"])
        .assert()
        .success()
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn self_set_then_show_roundtrips() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "self", "set", "12345"])
        .assert()
        .success()
        .stdout(contains("12345"));

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "self", "show"])
        .assert()
        .success()
        .stdout(contains("12345"));
}

#[test]
#[serial]
fn self_clear_reverts_to_not_configured() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_self(home.path(), 777);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "self", "clear"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "self", "show"])
        .assert()
        .success()
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn self_show_json_shape() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_self(home.path(), -1001234567890);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "self", "show", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"telegram.self.show\""))
        .stdout(contains("\"self_chat_id\": -1001234567890"));
}

// ---------------------------------------------------------------------------
// send --chat @me
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_at_me_resolves_self_chat_id_in_dry_run() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_self(home.path(), 8675309);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "@me", "--dry-run", "hi"])
        .assert()
        .success()
        .stdout(contains("\"chat_id\": \"8675309\""));
}

#[test]
#[serial]
fn send_at_me_without_self_configured_errors_with_hint() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path()); // no self_chat_id
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "@me", "--dry-run", "hi"])
        .assert()
        .failure()
        .stderr(contains("self capture"));
}
