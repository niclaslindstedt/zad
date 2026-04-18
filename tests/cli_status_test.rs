//! Integration tests for the top-level `zad status` aggregate command.
//!
//! Mirrors the constraints of `cli_service_status_test.rs`: we can't
//! reach the provider APIs from tests, so we exercise the
//! "not configured" and "configured-but-missing-credentials" branches.
//! The happy path (credentials present + ping ok) requires HTTP
//! mocking that the rest of this suite doesn't have.

use assert_cmd::Command;
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

#[test]
#[serial]
fn top_level_status_with_no_services_configured_succeeds() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["status"])
        .assert()
        // Nothing configured means nothing can fail, so ok=true / exit 0.
        // An agent that ran `zad status` at startup and got exit 0 would
        // then look at the JSON to see which services it has available.
        .success()
        .stdout(contains("discord"))
        .stdout(contains("telegram"))
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn top_level_status_json_shape() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"status\""))
        .stdout(contains("\"ok\": true"))
        .stdout(contains("\"service\": \"discord\""))
        .stdout(contains("\"service\": \"telegram\""));
}

#[test]
#[serial]
fn top_level_status_fails_when_a_configured_service_has_no_credentials() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["status", "--json"])
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
fn top_level_status_human_output_distinguishes_per_service_state() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_discord(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["status"])
        .assert()
        .failure()
        .stdout(contains("discord"))
        .stdout(contains("FAILED"))
        .stdout(contains("telegram"))
        .stdout(contains("not configured"));
}
