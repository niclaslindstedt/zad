//! End-to-end tests for `zad ymusic` runtime verbs that don't need
//! a live YouTube Data API. Exercises the `permissions check`
//! subgroup, the `--dry-run` path on mutating verbs, and the scope
//! denial path on a verb whose required scope is missing.

use assert_cmd::Command;
use predicates::str::contains;
use serial_test::serial;

mod common;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

fn seed_global(home: &std::path::Path, scopes: &str) {
    let p = home
        .join(".zad")
        .join("services")
        .join("ymusic")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, format!("scopes = {scopes}\n")).unwrap();
}

fn store_keychain_creds(home: &std::path::Path) {
    // Create credentials via the CLI so the keychain entries match
    // the production naming. `--no-validate` keeps it offline.
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env("YMUSIC_REFRESH_TOKEN", "1//fake-refresh-token")
        .args([
            "service",
            "create",
            "ymusic",
            "--refresh-token-env",
            "YMUSIC_REFRESH_TOKEN",
            "--scopes",
            "search,playlists.read,playlists.write,library.read,library.write",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success();
}

fn enable_ymusic(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "ymusic"])
        .assert()
        .success();
}

#[test]
#[serial]
fn permissions_check_allows_target_when_no_policy() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path(), "[\"playlists.write\"]");
    enable_ymusic(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "ymusic",
            "permissions",
            "check",
            "--function",
            "playlists_write",
            "--target",
            "zad-test",
        ])
        .assert()
        .success()
        .stdout(contains("would be allowed"));
}

#[test]
#[serial]
fn permissions_path_prints_global_and_local_paths() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path(), "[]");
    enable_ymusic(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["ymusic", "permissions", "path"])
        .assert()
        .success()
        .stdout(common::contains_path("services/ymusic/permissions.toml"));
}

#[test]
#[serial]
fn dry_run_create_emits_preview_and_does_not_touch_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // Note: NO credentials — the dry-run transport avoids the keychain.
    seed_global(home.path(), "[\"playlists.write\"]");
    enable_ymusic(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "ymusic",
            "playlists",
            "create",
            "preview-test",
            "--dry-run",
        ])
        .assert()
        .success()
        // The dry-run sink emits the JSON payload on stdout via the
        // shared StderrTracingSink — the operation summary goes to
        // stderr's tracing layer. We just confirm the verb returned
        // success without trying to mint a token.
        ;
}

#[test]
#[serial]
fn search_without_scope_is_denied_locally() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // Project enabled, credentials present, but `search` scope is
    // missing — runtime should refuse before any network call.
    store_keychain_creds(home.path());
    // Overwrite the scopes the create flow set so `search` is gone.
    let cfg = home
        .path()
        .join(".zad")
        .join("services")
        .join("ymusic")
        .join("config.toml");
    std::fs::write(&cfg, "scopes = [\"playlists.read\"]\n").unwrap();
    enable_ymusic(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["ymusic", "search", "moon river"])
        .assert()
        .failure()
        .stderr(contains("scope"))
        .stderr(contains("search"));
}

#[test]
#[serial]
fn permissions_check_denies_target_caught_by_starter_template() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path(), "[\"playlists.write\"]");
    enable_ymusic(home.path(), project.path());

    // Initialize a global permissions file with the starter
    // template (which denies `*release*` / `*official*` for
    // playlists_write).
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["ymusic", "permissions", "init"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "ymusic",
            "permissions",
            "check",
            "--function",
            "playlists_write",
            "--target",
            "official-mix",
        ])
        .assert()
        .failure()
        .stderr(contains("playlists_write"));
}
