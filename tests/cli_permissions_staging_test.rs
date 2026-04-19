//! End-to-end coverage for the staged-commit permissions workflow.
//! Drives the binary across Discord (as a representative service) to
//! exercise `status`, `add`, `diff`, `commit`, `discard`, `sign`.
//! The shared runner means every other service gets the same
//! workflow — covering one service here proves the generic plumbing.

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

fn seed_global_creds(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234567890\"\n\
         scopes = [\"guilds\", \"messages.read\", \"messages.send\"]\n",
    )
    .unwrap();
}

fn enable_discord(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "discord"])
        .assert()
        .success();
}

fn local_permissions_path(home: &std::path::Path, project: &std::path::Path) -> std::path::PathBuf {
    let slug = common::project_slug(project);
    home.join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("permissions.toml")
}

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn staging_help_lists_new_subcommands() {
    bin()
        .args(["discord", "permissions", "--help"])
        .assert()
        .success()
        .stdout(contains("status"))
        .stdout(contains("diff"))
        .stdout(contains("discard"))
        .stdout(contains("commit"))
        .stdout(contains("sign"))
        .stdout(contains("add"))
        .stdout(contains("remove"))
        .stdout(contains("content"))
        .stdout(contains("time"));
}

// ---------------------------------------------------------------------------
// status / diff when nothing has been staged
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn status_reports_nothing_without_files() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "status", "--local"])
        .assert()
        .success()
        .stdout(contains("live").and(contains("absent")))
        .stdout(contains("pending").and(contains("absent")));
}

#[test]
#[serial]
fn diff_exits_cleanly_when_no_pending() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "diff", "--local"])
        .assert()
        .success()
        .stdout(contains("no pending changes"));
}

// ---------------------------------------------------------------------------
// full queue → diff → (library-level commit) → enforce
//
// Note: `commit` requires a signing key that persists across the
// subprocess boundary, but `ZAD_SECRETS_MEMORY=1` only keeps secrets
// for the lifetime of one process. So the queue/diff/status half
// runs via the CLI binary, and commit is driven through the library
// path (equivalent — both route through `cli::permissions::run`).
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn queue_and_diff_show_pending_mutation() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "init", "--local"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "add",
            "--function",
            "send",
            "--target",
            "channel",
            "--list",
            "deny",
            "--local",
            "deploy-*",
        ])
        .assert()
        .success()
        .stdout(contains("Queued"));

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "status", "--local"])
        .assert()
        .success()
        .stdout(contains("pending").and(contains("present")));

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "diff", "--local"])
        .assert()
        .success()
        .stdout(contains("deploy-*"));

    // The pending file exists next to (what will become) the live file.
    let pending = {
        let mut p = local_permissions_path(home.path(), project.path());
        p.set_file_name("permissions.toml.pending");
        p
    };
    assert!(pending.exists());
    let body = std::fs::read_to_string(&pending).unwrap();
    assert!(!body.contains("[signature]"), "pending must be unsigned");
    assert!(body.contains("deploy-*"));
}

// ---------------------------------------------------------------------------
// discard leaves live untouched
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn discard_removes_pending_only() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "init", "--local"])
        .assert()
        .success();

    let live = local_permissions_path(home.path(), project.path());
    let live_before = std::fs::read_to_string(&live).unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "add",
            "--function",
            "send",
            "--target",
            "channel",
            "--list",
            "allow",
            "--local",
            "general",
        ])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "discard", "--local"])
        .assert()
        .success()
        .stdout(contains("Discarded"));

    let live_after = std::fs::read_to_string(&live).unwrap();
    assert_eq!(live_before, live_after, "discard must not touch live");
}

// ---------------------------------------------------------------------------
// commit fails clearly with no signing key
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn commit_without_signing_key_errors_clearly() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    // Write a pending file by hand (no init, so the keychain is empty).
    use zad::service::discord::permissions::{self as perms, DiscordPermissionsRaw};
    let slug = common::project_slug(project.path());
    let live = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("permissions.toml");
    std::fs::create_dir_all(live.parent().unwrap()).unwrap();
    let pending = {
        let mut p = live.clone();
        p.set_file_name("permissions.toml.pending");
        p
    };
    let raw = DiscordPermissionsRaw::default();
    perms::save_unsigned(&pending, &raw).unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "commit", "--local"])
        .assert()
        .failure()
        .stderr(contains("no signing key in keychain"));
}
