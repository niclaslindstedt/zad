//! Integration tests for `zad service status` — both the aggregate form
//! (no `--service` flag) and the single-service form
//! (`--service <name>`).
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
//! - aggregate form fans out across every service and reports each row
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
// Single service: discord — not configured
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn discord_status_not_configured_exits_nonzero() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "--service", "discord"])
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
        .args(["service", "status", "--service", "discord", "--json"])
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
// Single service: discord — config present, keychain empty
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
        .args(["service", "status", "--service", "discord"])
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
        .args(["service", "status", "--service", "discord", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"service\": \"discord\""))
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"ok\": false"))
        .stdout(contains("\"credentials_present\": false"))
        .stdout(contains("\"error\": \"credentials missing from keychain\""));
}

// ---------------------------------------------------------------------------
// Single service: discord — local wins over global
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
        .args(["service", "status", "--service", "discord", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"effective\": \"local\""))
        // Global is configured but not the effective scope: it should
        // report `configured: true` without a `check` block.
        .stdout(contains("\"configured\": true"));
}

// ---------------------------------------------------------------------------
// Single service: telegram — same surface, just the other service
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn telegram_status_not_configured_exits_nonzero() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "--service", "telegram", "--json"])
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
        .args(["service", "status", "--service", "telegram", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"credentials_present\": false"))
        .stdout(contains("\"error\": \"credentials missing from keychain\""));
}

// ---------------------------------------------------------------------------
// Unknown --service value → clap rejects with usage error (exit 2)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn unknown_service_value_is_rejected_by_clap() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "--service", "bogus"])
        .assert()
        .failure()
        // clap's PossibleValuesParser enumerates valid services in the
        // error message — assert both known services appear so an
        // agent reading stderr can correct the call.
        .stderr(contains("discord"))
        .stderr(contains("telegram"));
}

// ---------------------------------------------------------------------------
// Aggregate: no --service filter
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn aggregate_status_with_no_services_configured_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status"])
        .assert()
        // Nothing configured means nothing can fail, so ok=true / exit 0.
        // An agent that ran `zad service status` at startup and got
        // exit 0 would then look at the JSON to see which services it
        // has available.
        .success()
        .stdout(contains("discord"))
        .stdout(contains("telegram"))
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn aggregate_status_json_shape() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.status\""))
        .stdout(contains("\"ok\": true"))
        .stdout(contains("\"service\": \"discord\""))
        .stdout(contains("\"service\": \"telegram\""));
}

#[test]
#[serial]
fn aggregate_status_fails_when_a_configured_service_has_no_credentials() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status", "--json"])
        .assert()
        .failure()
        .stdout(contains("\"ok\": false"))
        .stdout(contains("\"service\": \"discord\""))
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"credentials_present\": false"))
        // Telegram isn't configured at all so it contributes a
        // non-failing row (effective omitted).
        .stdout(contains("\"service\": \"telegram\""));
}

#[test]
#[serial]
fn aggregate_status_human_output_distinguishes_per_service_state() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "status"])
        .assert()
        .failure()
        .stdout(contains("discord"))
        .stdout(contains("FAILED"))
        .stdout(contains("telegram"))
        .stdout(contains("not configured"));
}
