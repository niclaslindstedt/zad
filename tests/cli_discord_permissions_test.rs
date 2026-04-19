//! CLI surface for `zad discord permissions` and the enforcement path
//! that runs at the top of every runtime verb. These drive the binary
//! and a tempdir-rooted `~/.zad/`; none of them hit Discord.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serial_test::serial;

mod common;
use common::contains_path;

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
         scopes = [\"guilds\", \"messages.read\", \"messages.send\"]\n\
         default_guild = \"999\"\n",
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

fn write_local_permissions(home: &std::path::Path, project: &std::path::Path, body: &str) {
    use zad::permissions::SigningKey;
    use zad::service::discord::permissions::{self as perms, DiscordPermissionsRaw};
    let slug = common::project_slug(project);
    let p = home
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("permissions.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    // Parse the literal body and sign it so the CLI subprocess (which
    // verifies every permission file on load) sees a valid signature.
    // The subprocess runs with `ZAD_SECRETS_MEMORY=1` and no stored
    // signing key, so the keychain cross-check is skipped and the
    // embedded pubkey is authoritative.
    let raw: DiscordPermissionsRaw =
        toml::from_str(body).expect("write_local_permissions: body must be valid TOML");
    let key = SigningKey::generate();
    perms::save_file(&p, &raw, &key).unwrap();
}

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

#[test]
fn help_lists_permissions_subcommand() {
    bin()
        .args(["discord", "--help"])
        .assert()
        .success()
        .stdout(contains("permissions"));
}

#[test]
fn permissions_help_lists_actions() {
    bin()
        .args(["discord", "permissions", "--help"])
        .assert()
        .success()
        .stdout(contains("show"))
        .stdout(contains("init"))
        .stdout(contains("path"))
        .stdout(contains("check"));
}

// ---------------------------------------------------------------------------
// path + show
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn path_prints_both_candidate_locations() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "path"])
        .assert()
        .success()
        .stdout(contains_path("services/discord/permissions.toml"))
        .stdout(contains_path("projects/").and(contains_path("services/discord/permissions.toml")));
}

#[test]
#[serial]
fn show_reports_no_restrictions_when_no_files_exist() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "show"])
        .assert()
        .success()
        .stdout(contains("not present"));
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn init_local_writes_a_permissions_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "init", "--local"])
        .assert()
        .success()
        .stdout(contains_path("services/discord/permissions.toml"));

    let slug = common::project_slug(project.path());
    let p = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("permissions.toml");
    assert!(p.exists());
    let body = std::fs::read_to_string(&p).unwrap();
    assert!(body.contains("deny_words"), "body: {body}");
}

#[test]
#[serial]
fn init_refuses_to_overwrite_without_force() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_local_permissions(home.path(), project.path(), "# existing\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "init", "--local"])
        .assert()
        .failure()
        .stderr(contains("--force"));
}

// ---------------------------------------------------------------------------
// check (dry-run)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn check_reports_allow_when_no_rules_configured() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "check",
            "--function",
            "send",
            "--channel",
            "general",
        ])
        .assert()
        .success()
        .stdout(contains("allow"));
}

#[test]
#[serial]
fn check_reports_deny_with_rule_location_when_channel_matches_deny_list() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_local_permissions(
        home.path(),
        project.path(),
        "[send]\nchannels.deny = [\"*admin*\"]\n",
    );

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "check",
            "--function",
            "send",
            "--channel",
            "server-admin",
        ])
        .assert()
        .failure()
        .stdout(contains("deny"))
        .stdout(contains_path("services/discord/permissions.toml"));
}

#[test]
#[serial]
fn check_with_body_fires_on_denied_word() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_local_permissions(
        home.path(),
        project.path(),
        "[content]\ndeny_words = [\"password\"]\n",
    );

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "check",
            "--function",
            "send",
            "--channel",
            "general",
            "--body",
            "my password is hunter2",
        ])
        .assert()
        .failure()
        .stdout(contains("deny"))
        .stdout(contains("password"));
}

// ---------------------------------------------------------------------------
// real verbs get blocked
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_is_blocked_before_any_network_when_channel_is_denied() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_local_permissions(
        home.path(),
        project.path(),
        "[send]\nchannels.allow = [\"general\"]\n",
    );

    // Attempting to send to `marketing` — which is not numeric and not
    // in the directory — would normally fail with a name-resolution
    // error. Use a numeric ID so we know the permission layer, not the
    // directory layer, is what's refusing.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "111222333444555666",
            "hello",
        ])
        .assert()
        .failure()
        .stderr(contains("permission denied"))
        .stderr(contains("send"));
}

#[test]
#[serial]
fn send_is_blocked_when_body_matches_deny_word_before_hitting_network() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_local_permissions(
        home.path(),
        project.path(),
        "[content]\ndeny_words = [\"api_key\"]\n\n[send]\nchannels.allow = [\"*\"]\n",
    );

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "111222333444555666",
            "api_key=hunter2",
        ])
        .assert()
        .failure()
        .stderr(contains("permission denied"))
        .stderr(contains("api_key"));
}

#[test]
#[serial]
fn invalid_permissions_file_surfaces_its_path_in_the_error() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    // Unbalanced regex — parse-time failure.
    write_local_permissions(
        home.path(),
        project.path(),
        "[send]\nchannels.allow = [\"re:(\"]\n",
    );

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "show"])
        .assert()
        .failure()
        .stderr(contains_path("services/discord/permissions.toml"));
}
