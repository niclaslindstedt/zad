//! Runtime CLI tests for `zad discord <verb>`. These exercise argument
//! parsing and the pre-network validation layer (project enablement,
//! credential resolution, snowflake parsing). They never hit the Discord
//! API — any test that did would be network-dependent and flaky.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serial_test::serial;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

fn seed_global(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234567890\"\nscopes = [\"guilds\"]\ndefault_guild = \"999\"\n",
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

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

#[test]
fn help_lists_every_subcommand() {
    bin()
        .args(["discord", "--help"])
        .assert()
        .success()
        .stdout(contains("send"))
        .stdout(contains("read"))
        .stdout(contains("channels"))
        .stdout(contains("join"))
        .stdout(contains("leave"));
}

// ---------------------------------------------------------------------------
// project-enablement guard
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_fails_when_project_not_enabled() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "hello"])
        .assert()
        .failure()
        .stderr(contains("discord is not enabled for this project"));
}

#[test]
#[serial]
fn read_fails_when_project_not_enabled() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "read", "--channel", "12345"])
        .assert()
        .failure()
        .stderr(contains("discord is not enabled for this project"));
}

// ---------------------------------------------------------------------------
// credential-missing guard (project enabled but no creds)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn channels_fails_when_credentials_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    // Wipe the global creds so the command has nothing to load.
    std::fs::remove_file(
        home.path()
            .join(".zad")
            .join("services")
            .join("discord")
            .join("config.toml"),
    )
    .unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "channels", "--guild", "42"])
        .assert()
        .failure()
        .stderr(contains("no Discord credentials found"));
}

// ---------------------------------------------------------------------------
// argument validation
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_requires_channel_or_dm() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "hello"])
        .assert()
        .failure()
        .stderr(contains("--channel").or(contains("--dm")));
}

#[test]
#[serial]
fn send_rejects_channel_and_dm_together() {
    bin()
        .args(["discord", "send", "--channel", "1", "--dm", "2", "hello"])
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[test]
#[serial]
fn send_rejects_non_numeric_channel() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "not-a-snowflake", "hi"])
        .assert()
        .failure()
        .stderr(contains("numeric Discord snowflake"));
}

#[test]
#[serial]
fn send_requires_body_or_stdin() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345"])
        .assert()
        .failure()
        .stderr(contains("missing message body"));
}

#[test]
#[serial]
fn read_rejects_zero_limit() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "read", "--channel", "12345", "--limit", "0"])
        .assert()
        .failure()
        .stderr(contains("--limit"));
}

#[test]
#[serial]
fn channels_needs_guild_when_no_default() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    // Seed creds without a default_guild so the CLI has nothing to fall
    // back on.
    let p = home
        .path()
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234567890\"\nscopes = [\"guilds\"]\n",
    )
    .unwrap();
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "channels"])
        .assert()
        .failure()
        .stderr(contains("no guild specified"));
}

#[test]
#[serial]
fn join_rejects_non_numeric_channel() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "join", "--channel", "not-a-snowflake"])
        .assert()
        .failure()
        .stderr(contains("numeric Discord snowflake"));
}

#[test]
#[serial]
fn leave_rejects_non_numeric_channel() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "leave", "--channel", "not-a-snowflake"])
        .assert()
        .failure()
        .stderr(contains("numeric Discord snowflake"));
}
