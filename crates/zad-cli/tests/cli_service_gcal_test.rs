//! End-to-end tests for `zad service {create, enable, disable, show,
//! delete} gcal`. Modelled on `cli_service_telegram_test.rs` — the
//! differences are (a) three keychain entries per scope instead of
//! one and (b) non-interactive `create` needs three of the OAuth
//! fields up front.

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
        .join("gcal")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        r#"scopes = ["calendars.read", "events.read"]
"#,
    )
    .unwrap();
}

fn create_global(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env(
            "GCAL_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("GCAL_CLIENT_SECRET", "test-client-secret")
        .env("GCAL_REFRESH_TOKEN", "1//fake-refresh-token")
        .current_dir(project)
        .args([
            "service",
            "create",
            "gcal",
            "--client-id-env",
            "GCAL_CLIENT_ID",
            "--client-secret-env",
            "GCAL_CLIENT_SECRET",
            "--refresh-token-env",
            "GCAL_REFRESH_TOKEN",
            "--scopes",
            "calendars.read,events.read,events.write",
            "--default-calendar",
            "primary",
            "--self-email",
            "alice@example.com",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success();
}

#[test]
#[serial]
fn create_global_writes_flat_config_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env(
            "GCAL_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("GCAL_CLIENT_SECRET", "test-client-secret")
        .env("GCAL_REFRESH_TOKEN", "1//fake-refresh-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "gcal",
            "--client-id-env",
            "GCAL_CLIENT_ID",
            "--client-secret-env",
            "GCAL_CLIENT_SECRET",
            "--refresh-token-env",
            "GCAL_REFRESH_TOKEN",
            "--scopes",
            "calendars.read,events.write",
            "--default-calendar",
            "primary",
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
        .join("gcal")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();
    assert!(!body.contains("[service.gcal]"), "got:\n{body}");
    assert!(
        body.contains("default_calendar = \"primary\""),
        "got:\n{body}"
    );
    // No secrets should ever land in the TOML.
    assert!(
        !body.contains("test-client-secret"),
        "secret leaked:\n{body}"
    );
    assert!(
        !body.contains("1//fake-refresh-token"),
        "refresh leaked:\n{body}"
    );
    assert!(
        !body.contains("test-client-id.apps.googleusercontent.com"),
        "client id leaked:\n{body}"
    );
}

#[test]
#[serial]
fn create_local_writes_under_project_slug() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("GCAL_CLIENT_ID", "cid")
        .env("GCAL_CLIENT_SECRET", "csecret")
        .env("GCAL_REFRESH_TOKEN", "rtok")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "gcal",
            "--local",
            "--client-id-env",
            "GCAL_CLIENT_ID",
            "--client-secret-env",
            "GCAL_CLIENT_SECRET",
            "--refresh-token-env",
            "GCAL_REFRESH_TOKEN",
            "--scopes",
            "calendars.read",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success()
        .stdout(contains("local"));

    let slug = common::project_slug(project.path());
    let local_creds = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("gcal")
        .join("config.toml");
    let body = fs::read_to_string(&local_creds).unwrap();
    assert!(
        body.contains("scopes = [\"calendars.read\"]"),
        "got:\n{body}"
    );

    let global = home
        .path()
        .join(".zad")
        .join("services")
        .join("gcal")
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
        .args(["service", "enable", "gcal"])
        .assert()
        .success()
        .stdout(contains("enabled"))
        .stdout(contains("global"));

    let slug = common::project_slug(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    let body = fs::read_to_string(&project_path).unwrap();
    assert!(body.contains("[service.gcal]"), "got:\n{body}");
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
        .args(["service", "enable", "gcal"])
        .assert()
        .failure()
        .stderr(contains("no Google Calendar credentials found"));
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
        .args(["service", "enable", "gcal"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "disable", "gcal"])
        .assert()
        .success()
        .stdout(contains("disabled"));

    let slug = common::project_slug(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    let body = fs::read_to_string(&project_path).unwrap();
    assert!(
        !body.contains("[service.gcal]"),
        "service entry should be gone, got:\n{body}"
    );
}

#[test]
#[serial]
fn list_includes_gcal_row() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "gcal"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "list"])
        .assert()
        .success()
        .stdout(contains("gcal"));
}

#[test]
#[serial]
fn show_reports_effective_source_and_three_keychain_entries() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "gcal"])
        .assert()
        .success()
        .stdout(contains("effective : global"))
        .stdout(contains("alice@example.com"))
        .stdout(contains("gcal-client-id:global"))
        .stdout(contains("gcal-client-secret:global"))
        .stdout(contains("gcal-refresh:global"))
        .stdout(predicates::str::contains("test-client-secret").not())
        .stdout(predicates::str::contains("1//fake-refresh-token").not());
}

#[test]
#[serial]
fn delete_global_removes_file_and_all_keychain_entries() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    let global_path = home
        .path()
        .join(".zad")
        .join("services")
        .join("gcal")
        .join("config.toml");
    assert!(global_path.exists());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "delete", "gcal"])
        .assert()
        .success()
        .stdout(contains("deleted"))
        .stdout(contains("cleared"));

    assert!(!global_path.exists(), "global config should be removed");
}

#[test]
#[serial]
fn create_non_interactive_requires_client_id() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "gcal",
            "--scopes",
            "calendars.read",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--client-id"));
}

#[test]
#[serial]
fn create_non_interactive_requires_refresh_token_when_no_browser() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("GCAL_CLIENT_ID", "cid")
        .env("GCAL_CLIENT_SECRET", "csecret")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "gcal",
            "--client-id-env",
            "GCAL_CLIENT_ID",
            "--client-secret-env",
            "GCAL_CLIENT_SECRET",
            "--scopes",
            "calendars.read",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--refresh-token"));
}

#[test]
#[serial]
fn json_output_for_create() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("GCAL_CLIENT_ID", "cid")
        .env("GCAL_CLIENT_SECRET", "csecret")
        .env("GCAL_REFRESH_TOKEN", "rtok")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "gcal",
            "--client-id-env",
            "GCAL_CLIENT_ID",
            "--client-secret-env",
            "GCAL_CLIENT_SECRET",
            "--refresh-token-env",
            "GCAL_REFRESH_TOKEN",
            "--scopes",
            "calendars.read",
            "--non-interactive",
            "--no-validate",
            "--json",
        ])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.create.gcal\""))
        .stdout(contains("\"scope\": \"global\""))
        .stdout(predicates::str::contains("csecret").not())
        .stdout(predicates::str::contains("rtok").not());
}
