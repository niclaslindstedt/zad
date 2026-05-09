use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
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
    std::fs::write(&p, "scopes = [\"chats\"]\n").unwrap();
}

#[test]
#[serial]
fn create_global_writes_flat_config_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("TELEGRAM_BOT_TOKEN", "12345:fake.token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "telegram",
            "--bot-token-env",
            "TELEGRAM_BOT_TOKEN",
            "--default-chat=-1001234567890",
            "--scopes",
            "chats,messages.send",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success()
        .stdout(contains("global"));

    let global_path = home
        .path()
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();

    assert!(!body.contains("[service.telegram]"), "got:\n{body}");
    assert!(
        body.contains("default_chat = \"-1001234567890\""),
        "got:\n{body}"
    );
    assert!(!body.contains("12345:fake.token"), "token leaked:\n{body}");

    // Nothing written to the project side.
    let slug = slugify(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    assert!(!project_path.exists());
}

#[test]
#[serial]
fn create_local_writes_under_project_slug() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("TELEGRAM_BOT_TOKEN", "12345:fake.token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "telegram",
            "--local",
            "--bot-token-env",
            "TELEGRAM_BOT_TOKEN",
            "--scopes",
            "chats",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success()
        .stdout(contains("local"));

    let slug = slugify(project.path());
    let local_creds = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("telegram")
        .join("config.toml");
    let body = fs::read_to_string(&local_creds).unwrap();
    assert!(body.contains("scopes = [\"chats\"]"), "got:\n{body}");

    let global = home
        .path()
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    assert!(!global.exists(), "--local must not touch global config");
}

#[test]
#[serial]
fn enable_uses_global_creds() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "telegram"])
        .assert()
        .success()
        .stdout(contains("enabled"))
        .stdout(contains("global"));

    let slug = slugify(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    let body = fs::read_to_string(&project_path).unwrap();
    assert!(body.contains("[service.telegram]"), "got:\n{body}");
    assert!(body.contains("enabled = true"));
}

#[test]
#[serial]
fn enable_fails_without_any_credentials() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "telegram"])
        .assert()
        .failure()
        .stderr(contains("no Telegram credentials found"));
}

#[test]
#[serial]
fn disable_removes_service_from_project_config() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "telegram"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "disable", "telegram"])
        .assert()
        .success()
        .stdout(contains("disabled"));

    let slug = slugify(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    let body = fs::read_to_string(&project_path).unwrap();
    assert!(
        !body.contains("[service.telegram]"),
        "service entry should be gone, got:\n{body}"
    );
}

#[test]
#[serial]
fn list_includes_telegram_row() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "telegram"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "list"])
        .assert()
        .success()
        .stdout(contains("telegram"))
        .stdout(contains("yes"))
        .stdout(contains("enabled"));
}

// ---------------------------------------------------------------------------
// show / delete
// ---------------------------------------------------------------------------

fn create_global(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env("TELEGRAM_BOT_TOKEN", "12345:fake.token")
        .current_dir(project)
        .args([
            "service",
            "create",
            "telegram",
            "--bot-token-env",
            "TELEGRAM_BOT_TOKEN",
            "--default-chat=-1001234567890",
            "--scopes",
            "chats,messages.send",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success();
}

#[test]
#[serial]
fn show_reports_effective_source_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "telegram"])
        .assert()
        .success()
        .stdout(contains("effective : global"))
        .stdout(contains("-1001234567890"))
        .stdout(contains("chats"))
        .stdout(contains("telegram-bot:global"))
        .stdout(predicates::str::contains("12345:fake.token").not());
}

#[test]
#[serial]
fn show_without_credentials_is_not_an_error() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "telegram"])
        .assert()
        .success()
        .stdout(contains("(none"))
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn delete_global_removes_file_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    let global_path = home
        .path()
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    assert!(global_path.exists());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "delete", "telegram"])
        .assert()
        .success()
        .stdout(contains("deleted"))
        .stdout(contains("cleared"));

    assert!(!global_path.exists(), "global config should be removed");
}

// ---------------------------------------------------------------------------
// --json output
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn json_output_for_create() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("TELEGRAM_BOT_TOKEN", "12345:fake.token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "telegram",
            "--bot-token-env",
            "TELEGRAM_BOT_TOKEN",
            "--scopes",
            "chats",
            "--non-interactive",
            "--no-validate",
            "--json",
        ])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.create.telegram\""))
        .stdout(contains("\"scope\": \"global\""))
        .stdout(predicates::str::contains("12345:fake.token").not());
}

#[test]
#[serial]
fn json_output_for_show() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "telegram", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.show.telegram\""))
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"default_chat\": \"-1001234567890\""));
}

#[test]
#[serial]
fn create_non_interactive_requires_bot_token() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "telegram",
            "--scopes",
            "chats",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--bot-token"));
}

fn slugify(p: &std::path::Path) -> String {
    common::project_slug(p)
}
