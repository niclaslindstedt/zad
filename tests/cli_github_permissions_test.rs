//! Permissions enforcement tests for `zad github`.
//!
//! Exercises the pre-network permissions gate: repo pattern allow/deny,
//! content rules, and the signed-file starter policy written by
//! `permissions init`.

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
        .join("github")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "scopes = [\"repo.read\", \"issues.read\", \"issues.write\", \
         \"pulls.read\", \"pulls.write\", \"checks.read\", \"search\"]\n\
         default_repo = \"myuser/sandbox\"\n",
    )
    .unwrap();
}

fn enable_github(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "github"])
        .assert()
        .success();
}

fn init_permissions(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["github", "permissions", "init"])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// init / show / path
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn init_writes_signed_starter_template() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    let path = home
        .path()
        .join(".zad")
        .join("services")
        .join("github")
        .join("permissions.toml");
    let body = std::fs::read_to_string(&path).unwrap();
    // Read verbs allow everything via `*`
    assert!(body.contains("issue_list"), "got:\n{body}");
    // Write verbs deny-by-default; `pr_merge` is the always-tightest one
    assert!(body.contains("pr_merge"), "got:\n{body}");
    // Deny patterns catch token fragments
    assert!(body.contains("ghp_"), "got:\n{body}");
    // Signature populated at write time
    assert!(body.contains("[signature]"), "got:\n{body}");
}

#[test]
#[serial]
fn show_reports_both_scope_files() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["github", "permissions", "show"])
        .assert()
        .success()
        .stdout(contains("global"))
        .stdout(contains("local"));
}

// ---------------------------------------------------------------------------
// starter policy denies every write verb
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn starter_policy_denies_pr_merge() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "pr",
            "merge",
            "1",
            "--repo",
            "myuser/sandbox",
            "--squash",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(contains("pr_merge"))
        .stderr(contains("permission denied"));
}

#[test]
#[serial]
fn starter_policy_denies_issue_create() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "issue",
            "create",
            "--repo",
            "myuser/sandbox",
            "--title",
            "test",
            "--body",
            "from zad",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(contains("issue_create"));
}

// ---------------------------------------------------------------------------
// permissions check subcommand
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn permissions_check_reports_deny_for_pr_merge_under_starter() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "permissions",
            "check",
            "--function",
            "pr_merge",
            "--repo",
            "myuser/sandbox",
        ])
        .assert()
        .failure()
        .stdout(contains("deny"));
}

#[test]
#[serial]
fn permissions_check_reports_allow_for_issue_list_under_starter() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "permissions",
            "check",
            "--function",
            "issue_list",
            "--repo",
            "myuser/sandbox",
        ])
        .assert()
        .success()
        .stdout(contains("allow"));
}

#[test]
#[serial]
fn permissions_check_rejects_unknown_function() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());
    init_permissions(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["github", "permissions", "check", "--function", "nonsense"])
        .assert()
        .failure()
        .stderr(contains("unknown function"));
}
