//! Runtime CLI tests for `zad github <verb>`. These exercise the
//! pre-subprocess validation layer: project enablement, scope
//! enforcement, repo resolution, and the dry-run preview. They never
//! actually shell out to `gh`.

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
    seed_global_with(
        home,
        "scopes = [\"repo.read\", \"issues.read\", \"issues.write\", \
         \"pulls.read\", \"pulls.write\", \"checks.read\", \"search\"]\n\
         default_repo = \"myuser/sandbox\"\n",
    );
}

fn seed_global_with(home: &std::path::Path, body: &str) {
    let p = home
        .join(".zad")
        .join("services")
        .join("github")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
}

fn enable_github(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "github"])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

#[test]
fn help_lists_every_subcommand() {
    bin()
        .args(["github", "--help"])
        .assert()
        .success()
        .stdout(contains("issue"))
        .stdout(contains("pr"))
        .stdout(contains("repo"))
        .stdout(contains("file"))
        .stdout(contains("code"))
        .stdout(contains("run"))
        .stdout(contains("permissions"));
}

#[test]
fn issue_help_lists_every_subcommand() {
    bin()
        .args(["github", "issue", "--help"])
        .assert()
        .success()
        .stdout(contains("list"))
        .stdout(contains("view"))
        .stdout(contains("create"))
        .stdout(contains("comment"))
        .stdout(contains("close"));
}

#[test]
fn pr_help_lists_every_subcommand() {
    bin()
        .args(["github", "pr", "--help"])
        .assert()
        .success()
        .stdout(contains("list"))
        .stdout(contains("view"))
        .stdout(contains("diff"))
        .stdout(contains("create"))
        .stdout(contains("comment"))
        .stdout(contains("review"))
        .stdout(contains("merge"))
        .stdout(contains("checks"));
}

// ---------------------------------------------------------------------------
// project-enablement guard
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn issue_list_fails_when_project_not_enabled() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["github", "issue", "list", "--repo", "myuser/sandbox"])
        .assert()
        .failure()
        .stderr(contains("github is not enabled for this project"));
}

#[test]
#[serial]
fn pr_merge_fails_when_project_not_enabled() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

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
        .stderr(contains("github is not enabled for this project"));
}

// ---------------------------------------------------------------------------
// scope enforcement
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn issue_create_fails_without_issues_write_scope() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // Only read scopes — writes should be rejected.
    seed_global_with(
        home.path(),
        "scopes = [\"repo.read\", \"issues.read\"]\n\
         default_repo = \"myuser/sandbox\"\n",
    );
    enable_github(home.path(), project.path());

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
            "Hello",
            "--body",
            "from zad",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(contains("issues.write"));
}

#[test]
#[serial]
fn code_search_fails_without_search_scope() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with(
        home.path(),
        "scopes = [\"repo.read\"]\ndefault_repo = \"myuser/sandbox\"\n",
    );
    enable_github(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["github", "code", "search", "fn main"])
        .assert()
        .failure()
        .stderr(contains("search"));
}

// ---------------------------------------------------------------------------
// repo resolution
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn issue_list_fails_when_no_repo_and_no_default() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with(home.path(), "scopes = [\"repo.read\", \"issues.read\"]\n");
    enable_github(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["github", "issue", "list"])
        .assert()
        .failure()
        .stderr(contains("no repo specified"));
}

// ---------------------------------------------------------------------------
// dry-run preview
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn pr_merge_dry_run_emits_preview_without_gh() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());

    // No permissions.toml → no permission constraints → scope is the
    // only gate. Dry-run short-circuits the keychain and gh spawn, so
    // this test runs even without `gh` installed.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "pr",
            "merge",
            "42",
            "--repo",
            "myuser/sandbox",
            "--squash",
            "--delete-branch",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(contains("\"command\": \"github.pr.merge\""))
        .stdout(contains("\"number\": \"42\""))
        .stdout(contains("\"mode\": \"squash\""))
        .stdout(contains("\"delete_branch\": true"));
}

#[test]
#[serial]
fn issue_comment_dry_run_previews_body() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "issue",
            "comment",
            "7",
            "--repo",
            "myuser/sandbox",
            "--body",
            "triaging",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(contains("\"command\": \"github.issue.comment\""))
        .stdout(contains("\"body\": \"triaging\""));
}

#[test]
#[serial]
fn pr_review_requires_a_mode_flag() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "github",
            "pr",
            "review",
            "1",
            "--repo",
            "myuser/sandbox",
            "--body",
            "lgtm",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(contains("--approve"));
}

#[test]
#[serial]
fn pr_merge_requires_a_mode_flag() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_github(home.path(), project.path());

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
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(contains("--squash"));
}
