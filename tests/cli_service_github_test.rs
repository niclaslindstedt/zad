//! Lifecycle tests for `zad service {create,enable,disable,show,delete}
//! github`. Mirrors `cli_service_telegram_test.rs`: `--no-validate`
//! keeps the tests network-free.

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
        .join("github")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "scopes = [\"repo.read\"]\n").unwrap();
}

#[test]
#[serial]
fn create_global_writes_flat_config_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("GITHUB_PAT", "ghp_fake_token_for_tests")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "github",
            "--pat-env",
            "GITHUB_PAT",
            "--default-repo",
            "myuser/sandbox",
            "--scopes",
            "repo.read,issues.read",
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
        .join("github")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();

    assert!(!body.contains("[service.github]"), "got:\n{body}");
    assert!(
        body.contains("default_repo = \"myuser/sandbox\""),
        "got:\n{body}"
    );
    assert!(
        !body.contains("ghp_fake_token_for_tests"),
        "token leaked:\n{body}"
    );
}

#[test]
#[serial]
fn create_local_writes_under_project_slug() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("GITHUB_PAT", "ghp_fake_token_for_tests")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "github",
            "--local",
            "--pat-env",
            "GITHUB_PAT",
            "--scopes",
            "repo.read",
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
        .join("github")
        .join("config.toml");
    let body = fs::read_to_string(&local_creds).unwrap();
    assert!(body.contains("scopes = [\"repo.read\"]"), "got:\n{body}");

    let global = home
        .path()
        .join(".zad")
        .join("services")
        .join("github")
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
        .args(["service", "enable", "github"])
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
    assert!(body.contains("[service.github]"), "got:\n{body}");
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
        .args(["service", "enable", "github"])
        .assert()
        .failure()
        .stderr(contains("no GitHub credentials found"));
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
        .args(["service", "enable", "github"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "disable", "github"])
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
        !body.contains("[service.github]"),
        "service entry should be gone, got:\n{body}"
    );
}

#[test]
#[serial]
fn list_includes_github_row() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "enable", "github"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "list"])
        .assert()
        .success()
        .stdout(contains("github"))
        .stdout(contains("enabled"));
}

fn create_global(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env("GITHUB_PAT", "ghp_fake_token_for_tests")
        .current_dir(project)
        .args([
            "service",
            "create",
            "github",
            "--pat-env",
            "GITHUB_PAT",
            "--default-repo",
            "myuser/sandbox",
            "--scopes",
            "repo.read,issues.read",
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
        .args(["service", "show", "github"])
        .assert()
        .success()
        .stdout(contains("effective : global"))
        .stdout(contains("myuser/sandbox"))
        .stdout(contains("repo.read"))
        .stdout(contains("github-pat:global"))
        .stdout(predicates::str::contains("ghp_fake_token_for_tests").not());
}

#[test]
#[serial]
fn show_without_credentials_is_not_an_error() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "github"])
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
        .join("github")
        .join("config.toml");
    assert!(global_path.exists());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "delete", "github"])
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
        .env("GITHUB_PAT", "ghp_fake_token_for_tests")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "github",
            "--pat-env",
            "GITHUB_PAT",
            "--scopes",
            "repo.read",
            "--non-interactive",
            "--no-validate",
            "--json",
        ])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.create.github\""))
        .stdout(contains("\"scope\": \"global\""))
        .stdout(predicates::str::contains("ghp_fake_token_for_tests").not());
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
        .args(["service", "show", "github", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"service.show.github\""))
        .stdout(contains("\"effective\": \"global\""))
        .stdout(contains("\"default_repo\": \"myuser/sandbox\""));
}

#[test]
#[serial]
fn create_non_interactive_requires_pat() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "github",
            "--scopes",
            "repo.read",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--pat"));
}

#[test]
fn create_rejects_repo_missing_slash() {
    bin()
        .env("GITHUB_PAT", "ghp_fake")
        .args([
            "service",
            "create",
            "github",
            "--pat-env",
            "GITHUB_PAT",
            "--default-repo",
            "not-a-valid-repo-name",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("owner/name"));
}
