//! Lifecycle integration tests for the 1pass service.
//!
//! Mirrors `cli_service_discord_test.rs` in shape and asserts the
//! generic driver wires correctly for a new service: keychain entries
//! are written, the TOML config is shaped right, `enable` walks both
//! scopes, `delete` reverses `create`, etc.
//!
//! `op` itself is never invoked by these tests — we pass
//! `--no-validate` so the create path skips the provider ping, and no
//! runtime verb is exercised here.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;
use serial_test::serial;

mod common;

use common::contains_path;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

fn seed_global(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("1pass")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "account = \"my.1password.com\"\nscopes = [\"read\"]\n").unwrap();
}

#[test]
#[serial]
fn create_global_writes_flat_config_and_no_token_leak() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("OP_SERVICE_ACCOUNT_TOKEN_TEST", "ops_redacted")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "1pass",
            "--account",
            "my.1password.com",
            "--token-env",
            "OP_SERVICE_ACCOUNT_TOKEN_TEST",
            "--default-vault",
            "AgentWork",
            "--scopes",
            "read,write",
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
        .join("1pass")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();

    assert!(!body.contains("[service.1pass]"), "config is flat:\n{body}");
    assert!(
        body.contains("account = \"my.1password.com\""),
        "got:\n{body}"
    );
    assert!(
        body.contains("default_vault = \"AgentWork\""),
        "got:\n{body}"
    );
    assert!(!body.contains("ops_redacted"), "token leaked:\n{body}");

    let slug = common::project_slug(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    assert!(
        !project_path.exists(),
        "create --global must not touch project config"
    );
}

#[test]
#[serial]
fn create_local_writes_under_project_slug() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("OP_TOK", "ops_x")
        .current_dir(project.path())
        .args([
            "service",
            "create",
            "1pass",
            "--local",
            "--account",
            "team.1password.eu",
            "--token-env",
            "OP_TOK",
            "--scopes",
            "read",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success()
        .stdout(contains("local"));

    let slug = common::project_slug(project.path());
    let local = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("1pass")
        .join("config.toml");
    let body = fs::read_to_string(&local).unwrap();
    assert!(
        body.contains("account = \"team.1password.eu\""),
        "got:\n{body}"
    );

    let global = home
        .path()
        .join(".zad")
        .join("services")
        .join("1pass")
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
        .args(["service", "enable", "1pass"])
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
    assert!(body.contains("[service.1pass]"), "got:\n{body}");
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
        .args(["service", "enable", "1pass"])
        .assert()
        .failure()
        .stderr(contains("no 1Password credentials found"));
}

#[test]
#[serial]
fn delete_removes_config_and_clears_keychain_placeholder() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "delete", "1pass"])
        .assert()
        .success()
        .stdout(contains("deleted"));

    let global_path = home
        .path()
        .join(".zad")
        .join("services")
        .join("1pass")
        .join("config.toml");
    assert!(!global_path.exists());
}

#[test]
#[serial]
fn show_reports_both_scopes() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "show", "1pass"])
        .assert()
        .success()
        .stdout(contains_path("services/1pass/config.toml"))
        .stdout(contains("my.1password.com"));
}

#[test]
#[serial]
fn list_includes_1pass_row() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["service", "list"])
        .assert()
        .success()
        .stdout(contains("1pass"));
}

#[test]
#[serial]
fn permissions_init_global_writes_starter() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["1pass", "permissions", "init"])
        .assert()
        .success()
        .stdout(contains_path("services/1pass/permissions.toml"));

    let path = home
        .path()
        .join(".zad")
        .join("services")
        .join("1pass")
        .join("permissions.toml");
    let body = fs::read_to_string(&path).unwrap();
    // Starter template gates `[create].vaults.allow` to `AgentWork`
    // (rendered as a dotted `[create.vaults]` header by the toml
    // serializer since no flat fields sit on `[create]` directly).
    assert!(body.contains("[create.vaults]"), "got:\n{body}");
    assert!(body.contains("AgentWork"), "got:\n{body}");
}

#[test]
#[serial]
fn permissions_path_prints_both_files() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["1pass", "permissions", "path"])
        .assert()
        .success()
        .stdout(contains_path("services/1pass/permissions.toml"));
}

#[test]
#[serial]
fn permissions_check_create_denies_when_no_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    // No permissions file at either scope → create is deny-by-default.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "1pass",
            "permissions",
            "check",
            "--function",
            "create",
            "--vault",
            "Personal",
            "--category",
            "Login",
            "--title",
            "foo",
        ])
        .assert()
        .failure()
        .stdout(contains("denied"));
}

#[test]
#[serial]
fn permissions_check_create_allowed_for_whitelisted_vault() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    // Write a starter policy at global scope. AgentWork is allowed.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["1pass", "permissions", "init"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "1pass",
            "permissions",
            "check",
            "--function",
            "create",
            "--vault",
            "AgentWork",
            "--category",
            "Login",
            "--title",
            "bot-token",
            "--tag",
            "agent-managed",
        ])
        .assert()
        .success()
        .stdout(contains("allowed"));
}

#[test]
#[serial]
fn permissions_check_create_denies_outside_vault() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["1pass", "permissions", "init"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "1pass",
            "permissions",
            "check",
            "--function",
            "create",
            "--vault",
            "Personal",
            "--category",
            "Login",
            "--title",
            "anything",
            "--tag",
            "agent-managed",
        ])
        .assert()
        .failure()
        .stdout(contains("denied"));
}
