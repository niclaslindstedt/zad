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
    seed_global_with_scopes(home, &["guilds", "messages.read", "messages.send"]);
}

fn seed_global_with_scopes(home: &std::path::Path, scopes: &[&str]) {
    let p = home
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let scope_list = scopes
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        &p,
        format!(
            "application_id = \"1234567890\"\nscopes = [{scope_list}]\ndefault_guild = \"999\"\n"
        ),
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
        .stderr(contains(
            "is neither a numeric snowflake nor a known directory entry",
        ));
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
fn read_rejects_limit_above_100() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "read", "--channel", "12345", "--limit", "101"])
        .assert()
        .failure()
        .stderr(contains("between 1 and 100"));
}

#[test]
#[serial]
fn send_rejects_oversized_body() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    let body = "x".repeat(2001);
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", &body])
        .assert()
        .failure()
        // The oversized-body error must fire before any keychain or
        // network access — `2001 characters` only appears in the local
        // pre-validation message.
        .stderr(contains("2001 characters").and(contains("hard limit is 2000")));
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
        .stderr(contains(
            "is neither a numeric snowflake nor a known directory entry",
        ));
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
        .stderr(contains(
            "is neither a numeric snowflake nor a known directory entry",
        ));
}

// ---------------------------------------------------------------------------
// scope enforcement (runtime, before any network call)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_denied_when_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.read", "guilds"]);
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "hi"])
        .assert()
        .failure()
        .stderr(
            contains("scope `messages.send` is not enabled")
                .and(contains("services/discord/config.toml")),
        );
}

#[test]
#[serial]
fn read_denied_when_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.send", "guilds"]);
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "read", "--channel", "12345"])
        .assert()
        .failure()
        .stderr(contains("scope `messages.read` is not enabled"));
}

#[test]
#[serial]
fn channels_denied_when_guilds_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.read", "messages.send"]);
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "channels", "--guild", "42"])
        .assert()
        .failure()
        .stderr(contains("scope `guilds` is not enabled"));
}

// ---------------------------------------------------------------------------
// dispatcher error surface
// ---------------------------------------------------------------------------

#[test]
fn discord_without_subcommand_errors() {
    bin()
        .args(["discord"])
        .assert()
        .failure()
        .stderr(contains("missing subcommand"));
}
