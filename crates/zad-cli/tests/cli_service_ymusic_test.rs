//! End-to-end tests for `zad service {create, enable, disable, show,
//! delete} ymusic`. Modelled on `cli_service_gcal_test.rs` — same
//! Google OAuth Desktop-app credential shape (`client_id`,
//! `client_secret`, `refresh_token`).

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
        .join("ymusic")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        r#"scopes = ["search", "playlists.read"]
"#,
    )
    .unwrap();
}

fn create_global(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env(
            "YMUSIC_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("YMUSIC_CLIENT_SECRET", "test-client-secret")
        .env("YMUSIC_REFRESH_TOKEN", "1//fake-refresh-token")
        .current_dir(project)
        .args([
            "service",
            "create",
            "ymusic",
            "--client-id-env",
            "YMUSIC_CLIENT_ID",
            "--client-secret-env",
            "YMUSIC_CLIENT_SECRET",
            "--refresh-token-env",
            "YMUSIC_REFRESH_TOKEN",
            "--scopes",
            "search,playlists.read,playlists.write",
            "--default-playlist=PLxFakePlaylistId",
            "--self-channel=UCxxxxxxxxxxxxxxxxxxxxxxxx",
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
            "YMUSIC_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("YMUSIC_CLIENT_SECRET", "test-client-secret")
        .env("YMUSIC_REFRESH_TOKEN", "1//fake-refresh-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "ymusic",
            "--client-id-env",
            "YMUSIC_CLIENT_ID",
            "--client-secret-env",
            "YMUSIC_CLIENT_SECRET",
            "--refresh-token-env",
            "YMUSIC_REFRESH_TOKEN",
            "--scopes",
            "search,playlists.read,playlists.write",
            "--default-playlist=PLxFake",
            "--self-channel=UCabc",
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
        .join("ymusic")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();

    assert!(!body.contains("[service.ymusic]"), "got:\n{body}");
    assert!(
        body.contains("default_playlist = \"PLxFake\""),
        "got:\n{body}"
    );
    assert!(body.contains("self_channel_id = \"UCabc\""), "got:\n{body}");
    assert!(body.contains("scopes ="), "got:\n{body}");

    // Secrets must never leak into the TOML.
    assert!(
        !body.contains("test-client-id.apps.googleusercontent.com"),
        "client_id leaked:\n{body}"
    );
    assert!(
        !body.contains("test-client-secret"),
        "client_secret leaked:\n{body}"
    );
    assert!(
        !body.contains("fake-refresh-token"),
        "refresh token leaked:\n{body}"
    );

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
        .env(
            "YMUSIC_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("YMUSIC_CLIENT_SECRET", "test-client-secret")
        .env("YMUSIC_REFRESH_TOKEN", "1//fake-refresh-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "ymusic",
            "--local",
            "--client-id-env",
            "YMUSIC_CLIENT_ID",
            "--client-secret-env",
            "YMUSIC_CLIENT_SECRET",
            "--refresh-token-env",
            "YMUSIC_REFRESH_TOKEN",
            "--scopes",
            "search",
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
        .join("ymusic")
        .join("config.toml");
    let body = fs::read_to_string(&local_creds).unwrap();
    assert!(body.contains("scopes = [\"search\"]"), "got:\n{body}");

    let global = home
        .path()
        .join(".zad")
        .join("services")
        .join("ymusic")
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
        .args(["service", "enable", "ymusic"])
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
    assert!(body.contains("[service.ymusic]"), "got:\n{body}");
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
        .args(["service", "enable", "ymusic"])
        .assert()
        .failure()
        .stderr(contains("no YouTube Music credentials found"));
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
        .args(["service", "enable", "ymusic"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "disable", "ymusic"])
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
        !body.contains("[service.ymusic]"),
        "service entry should be gone, got:\n{body}"
    );
}

#[test]
#[serial]
fn list_includes_ymusic_row() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "ymusic"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "list"])
        .assert()
        .success()
        .stdout(contains("ymusic"))
        .stdout(contains("yes"))
        .stdout(contains("enabled"));
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
        .args(["service", "show", "ymusic"])
        .assert()
        .success()
        .stdout(contains("effective : global"))
        .stdout(contains("PLxFakePlaylistId"))
        .stdout(contains("playlists.read"))
        .stdout(contains("ymusic-client-id:global"))
        .stdout(contains("ymusic-client-secret:global"))
        .stdout(contains("ymusic-refresh:global"))
        .stdout(predicates::str::contains("test-client-secret").not())
        .stdout(predicates::str::contains("fake-refresh-token").not());
}

#[test]
#[serial]
fn show_without_credentials_is_not_an_error() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "ymusic"])
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
        .join("ymusic")
        .join("config.toml");
    assert!(global_path.exists());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "delete", "ymusic"])
        .assert()
        .success()
        .stdout(contains("deleted"))
        .stdout(contains("cleared"));

    assert!(!global_path.exists(), "global config should be removed");
}

#[test]
#[serial]
fn json_output_for_create() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env(
            "YMUSIC_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("YMUSIC_CLIENT_SECRET", "test-client-secret")
        .env("YMUSIC_REFRESH_TOKEN", "1//fake-refresh-token")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "ymusic",
            "--client-id-env",
            "YMUSIC_CLIENT_ID",
            "--client-secret-env",
            "YMUSIC_CLIENT_SECRET",
            "--refresh-token-env",
            "YMUSIC_REFRESH_TOKEN",
            "--scopes",
            "search",
            "--non-interactive",
            "--no-validate",
            "--json",
        ])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.create.ymusic\""))
        .stdout(contains("\"scope\": \"global\""))
        .stdout(predicates::str::contains("fake-refresh-token").not());
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
        .args(["service", "show", "ymusic", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.show.ymusic\""))
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"default_playlist\": \"PLxFakePlaylistId\""));
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
            "ymusic",
            "--scopes",
            "search",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--client-id"));
}

#[test]
#[serial]
fn create_non_interactive_requires_client_secret() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env(
            "YMUSIC_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "ymusic",
            "--client-id-env",
            "YMUSIC_CLIENT_ID",
            "--scopes",
            "search",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--client-secret"));
}

#[test]
#[serial]
fn create_non_interactive_requires_refresh_token() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env(
            "YMUSIC_CLIENT_ID",
            "test-client-id.apps.googleusercontent.com",
        )
        .env("YMUSIC_CLIENT_SECRET", "test-client-secret")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "ymusic",
            "--client-id-env",
            "YMUSIC_CLIENT_ID",
            "--client-secret-env",
            "YMUSIC_CLIENT_SECRET",
            "--scopes",
            "search",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--refresh-token"));
}

fn slugify(p: &std::path::Path) -> String {
    common::project_slug(p)
}
