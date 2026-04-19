//! Integration tests for `zad service status <svc>`.
//!
//! The `validate()` path hits real provider APIs (`GET /users/@me`,
//! `getMe`), which we can't exercise from the test harness — the
//! existing suite avoids the network the same way by passing
//! `--no-validate` to create. These tests therefore focus on the
//! code paths that don't reach the network:
//!
//! - nothing configured at either scope → `ok=false`, exit 1
//! - config present but keychain empty → `credentials_present=false`,
//!   `check.error = "credentials missing from keychain"`, exit 1
//! - local wins over global as the effective scope
//!
//! Each child process starts with a fresh `ZAD_SECRETS_MEMORY` map, so
//! a "create" + "status" across two subprocesses can never see each
//! other's keychain — which is exactly how the second case is set up.

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

fn seed_global_discord(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234567890\"\nscopes = [\"guilds\"]\n",
    )
    .unwrap();
}

fn seed_local_discord(home: &std::path::Path, project: &std::path::Path) {
    let slug = common::project_slug(project);
    let p = home
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "application_id = \"42\"\nscopes = [\"guilds\"]\n").unwrap();
}

fn seed_global_telegram(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "scopes = [\"messages.send\"]\n").unwrap();
}

// ---------------------------------------------------------------------------
// Discord: not configured
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn discord_status_not_configured_exits_nonzero() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "discord"])
        .assert()
        .failure()
        .stdout(contains("overall   : FAILED"))
        .stdout(contains("effective : (none"))
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn discord_status_json_not_configured() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "discord", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"command\": \"service.status.discord\""))
        .stdout(contains("\"service\": \"discord\""))
        .stdout(contains("\"ok\": false"))
        // `effective: null` is omitted by serde's skip_serializing_if, so
        // the key simply isn't there when nothing is configured.
        .stdout(predicates::str::contains("\"effective\"").not());
}

// ---------------------------------------------------------------------------
// Discord: config present, keychain empty
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn discord_status_reports_missing_credentials_when_keychain_empty() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "discord"])
        .assert()
        .failure()
        .stdout(contains("effective : global"))
        .stdout(contains("credentials : missing"))
        .stdout(contains("credentials missing from keychain"));
}

#[test]
#[serial]
fn discord_status_json_reports_missing_credentials() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "discord", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"service\": \"discord\""))
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"ok\": false"))
        .stdout(contains("\"credentials_present\": false"))
        .stdout(contains("\"error\": \"credentials missing from keychain\""));
}

// ---------------------------------------------------------------------------
// Discord: local wins over global
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn discord_status_prefers_local_scope_when_both_configured() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());
    seed_local_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "discord", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"effective\": \"local\""))
        // Global is configured but not the effective scope: it should
        // report `configured: true` without a `check` block.
        .stdout(contains("\"configured\": true"));
}

// ---------------------------------------------------------------------------
// Telegram: same surface, just the other service
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn telegram_status_not_configured_exits_nonzero() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "telegram", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"command\": \"service.status.telegram\""))
        .stdout(contains("\"service\": \"telegram\""))
        .stdout(contains("\"ok\": false"));
}

#[test]
#[serial]
fn telegram_status_reports_missing_credentials_when_keychain_empty() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_telegram(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "telegram", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"credentials_present\": false"))
        .stdout(contains("\"error\": \"credentials missing from keychain\""));
}
