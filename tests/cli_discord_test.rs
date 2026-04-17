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

// ---------------------------------------------------------------------------
// --help-agent
// ---------------------------------------------------------------------------

#[test]
fn help_agent_emits_parseable_json_document() {
    let out = bin()
        .args(["discord", "--help-agent"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("utf8 stdout");
    let doc: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");

    assert_eq!(doc["command"], "discord");
    assert!(doc["version"].is_string());
    assert!(doc["auth"].is_object());
    assert!(doc["preconditions"].is_array());
    assert!(doc["concepts"].is_object());
    assert!(doc["exit_codes"].is_array());

    let verbs = doc["verbs"].as_array().expect("verbs array");
    let names: Vec<&str> = verbs.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["send", "read", "channels", "join", "leave"]);

    // Every verb documents its flags, JSON command id, and at least one
    // example — an agent shouldn't need to guess invocation shape.
    for v in verbs {
        assert!(v["usage"].as_str().unwrap().starts_with("zad discord "));
        assert!(!v["examples"].as_array().unwrap().is_empty());
        assert!(
            v["json_output"]["command_id"]
                .as_str()
                .unwrap()
                .starts_with("discord.")
        );
    }

    // Spot-check that flag introspection picks up the snowflake type and
    // the `--limit` default for `read`.
    let read = &verbs[1];
    let limit_flag = read["flags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["long"] == "--limit")
        .expect("--limit flag present");
    assert_eq!(limit_flag["type"], "integer");
    assert_eq!(limit_flag["default"], "20");

    let send = &verbs[0];
    let channel_flag = send["flags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["long"] == "--channel")
        .expect("--channel flag present");
    assert_eq!(channel_flag["type"], "snowflake");
    assert_eq!(channel_flag["takes_value"], true);

    // `send` has a BODY positional; `read` has none.
    assert_eq!(send["positionals"].as_array().unwrap()[0]["name"], "BODY");
    assert!(read["positionals"].as_array().unwrap().is_empty());
}

#[test]
fn help_agent_does_not_require_a_subcommand() {
    // `zad discord` without a verb or --help-agent errors with a helpful
    // message, but `zad discord --help-agent` alone succeeds.
    bin()
        .args(["discord"])
        .assert()
        .failure()
        .stderr(contains("missing subcommand"));

    bin().args(["discord", "--help-agent"]).assert().success();
}
