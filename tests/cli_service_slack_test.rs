use std::fs;

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
        .join("slack")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "app_id = \"A012345678\"\nworkspace = \"test-workspace\"\nscopes = [\"chat:write\"]\n",
    )
    .unwrap();
}

fn slugify(p: &std::path::Path) -> String {
    common::project_slug(p)
}

// ---------------------------------------------------------------------------
// create global
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn create_global_writes_flat_config_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("SLACK_BOT_TOKEN", "xoxb-fake-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "slack",
            "--app-id",
            "A012345678",
            "--bot-token-env",
            "SLACK_BOT_TOKEN",
            "--scopes",
            "chat:write,channels:history",
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
        .join("slack")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();

    assert!(!body.contains("[service.slack]"), "got:\n{body}");
    assert!(body.contains("app_id = \"A012345678\""), "got:\n{body}");
    assert!(!body.contains("xoxb-fake-token"), "token leaked:\n{body}");

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

// ---------------------------------------------------------------------------
// create local
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn create_local_writes_under_project_slug() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("SLACK_BOT_TOKEN", "xoxb-fake-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "slack",
            "--local",
            "--app-id",
            "A012345678",
            "--bot-token-env",
            "SLACK_BOT_TOKEN",
            "--scopes",
            "chat:write",
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
        .join("slack")
        .join("config.toml");
    let body = fs::read_to_string(&local_creds).unwrap();
    assert!(body.contains("scopes = [\"chat:write\"]"), "got:\n{body}");

    let global = home
        .path()
        .join(".zad")
        .join("services")
        .join("slack")
        .join("config.toml");
    assert!(
        !global.exists(),
        "global config should not be written for --local"
    );
}

// ---------------------------------------------------------------------------
// enable / disable
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn enable_writes_project_config() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "slack"])
        .assert()
        .success();

    let slug = slugify(project.path());
    let project_cfg = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    let body = fs::read_to_string(&project_cfg).unwrap();
    assert!(body.contains("slack"), "got:\n{body}");
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
        .args(["service", "enable", "slack"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "disable", "slack"])
        .assert()
        .success();

    let slug = slugify(project.path());
    let project_cfg = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    if project_cfg.exists() {
        let body = fs::read_to_string(&project_cfg).unwrap();
        assert!(
            !body.contains("[service.slack]"),
            "slack still present after disable:\n{body}"
        );
    }
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn show_prints_app_id_and_workspace() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "slack"])
        .assert()
        .success()
        .stdout(contains("A012345678"));
}

#[test]
#[serial]
fn show_json_output() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "slack", "--json"])
        .assert()
        .success()
        .stdout(contains("\"app_id\""))
        .stdout(contains("A012345678"));
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn list_includes_slack() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "list"])
        .assert()
        .success()
        .stdout(contains("slack"));
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn delete_removes_global_config() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "delete", "slack"])
        .assert()
        .success();

    let global_path = home
        .path()
        .join(".zad")
        .join("services")
        .join("slack")
        .join("config.toml");
    assert!(
        !global_path.exists(),
        "global config should be removed after delete"
    );
}

// ---------------------------------------------------------------------------
// install-URL hint after create
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn create_human_output_prints_install_url() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("SLACK_BOT_TOKEN", "xoxb-fake-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "slack",
            "--app-id",
            "A012345678",
            "--bot-token-env",
            "SLACK_BOT_TOKEN",
            "--scopes",
            "chat:write",
            "--non-interactive",
            "--no-validate",
            "--no-browser",
        ])
        .assert()
        .success()
        .stdout(contains("api.slack.com/apps/A012345678/install-on-team"));
}

#[test]
#[serial]
fn create_json_output_includes_hint_field() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("SLACK_BOT_TOKEN", "xoxb-fake-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "slack",
            "--app-id",
            "A012345678",
            "--bot-token-env",
            "SLACK_BOT_TOKEN",
            "--scopes",
            "chat:write",
            "--non-interactive",
            "--no-validate",
            "--no-browser",
            "--json",
        ])
        .assert()
        .success()
        .stdout(contains("\"hint\":"))
        .stdout(contains("api.slack.com/apps/A012345678/install-on-team"));
}

// ---------------------------------------------------------------------------
// default_channel round-trips
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn create_with_default_channel_persists_it() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("SLACK_BOT_TOKEN", "xoxb-fake-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "slack",
            "--app-id",
            "A012345678",
            "--bot-token-env",
            "SLACK_BOT_TOKEN",
            "--default-channel",
            "C1234567890",
            "--scopes",
            "chat:write",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success();

    let global_path = home
        .path()
        .join(".zad")
        .join("services")
        .join("slack")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();
    assert!(
        body.contains("default_channel = \"C1234567890\""),
        "got:\n{body}"
    );
}
