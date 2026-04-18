//! Runtime CLI tests for `zad telegram <verb>`. These exercise argument
//! parsing and the pre-network validation layer (project enablement,
//! credential resolution, chat parsing, scope enforcement, `--dry-run`
//! preview). They never hit the Bot API — any test that did would be
//! network-dependent and flaky.

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

fn seed_global(home: &std::path::Path) {
    seed_global_with_scopes(home, &["chats", "messages.read", "messages.send"]);
}

fn seed_global_with_scopes(home: &std::path::Path, scopes: &[&str]) {
    let p = home
        .join(".zad")
        .join("services")
        .join("telegram")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let scope_list = scopes
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(&p, format!("scopes = [{scope_list}]\n")).unwrap();
}

fn enable_telegram(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "telegram"])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

#[test]
fn help_lists_every_subcommand() {
    bin()
        .args(["telegram", "--help"])
        .assert()
        .success()
        .stdout(contains("send"))
        .stdout(contains("read"))
        .stdout(contains("chats"))
        .stdout(contains("discover"))
        .stdout(contains("directory"))
        .stdout(contains("permissions"));
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
        .args(["telegram", "send", "--chat", "12345", "hello"])
        .assert()
        .failure()
        .stderr(contains("telegram is not enabled for this project"));
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
        .args(["telegram", "read", "--chat", "12345"])
        .assert()
        .failure()
        .stderr(contains("telegram is not enabled for this project"));
}

// ---------------------------------------------------------------------------
// credential-missing guard (project enabled but no creds)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn chats_fails_when_credentials_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    // Wipe the global creds so the command has nothing to load.
    std::fs::remove_file(
        home.path()
            .join(".zad")
            .join("services")
            .join("telegram")
            .join("config.toml"),
    )
    .unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "chats"])
        .assert()
        .failure()
        .stderr(contains("no Telegram credentials found"));
}

// ---------------------------------------------------------------------------
// argument validation
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_requires_chat_or_default() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "hello"])
        .assert()
        .failure()
        .stderr(contains("no chat specified"));
}

#[test]
#[serial]
fn send_rejects_unknown_alias() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "not-a-chat", "hi"])
        .assert()
        .failure()
        .stderr(contains("is neither a chat_id nor a known directory entry"));
}

#[test]
#[serial]
fn send_requires_body_or_stdin() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "12345"])
        .assert()
        .failure()
        .stderr(contains("missing message body"));
}

#[test]
#[serial]
fn send_rejects_oversized_body() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let body = "x".repeat(4097);
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "12345", &body])
        .assert()
        .failure()
        .stderr(contains("4097 characters").and(contains("hard limit is 4096")));
}

#[test]
#[serial]
fn read_rejects_zero_limit() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "read", "--chat", "12345", "--limit", "0"])
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
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "read", "--chat", "12345", "--limit", "101"])
        .assert()
        .failure()
        .stderr(contains("between 1 and 100"));
}

// ---------------------------------------------------------------------------
// scope enforcement (runtime, before any network call)
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_denied_when_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.read", "chats"]);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "12345", "hi"])
        .assert()
        .failure()
        .stderr(
            contains("scope `messages.send` is not enabled")
                .and(contains_path("services/telegram/config.toml")),
        );
}

#[test]
#[serial]
fn read_denied_when_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.send", "chats"]);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "read", "--chat", "12345"])
        .assert()
        .failure()
        .stderr(contains("scope `messages.read` is not enabled"));
}

#[test]
#[serial]
fn chats_denied_when_chats_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.read", "messages.send"]);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "chats"])
        .assert()
        .failure()
        .stderr(contains("scope `chats` is not enabled"));
}

#[test]
#[serial]
fn discover_denied_when_chats_scope_missing() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.read", "messages.send"]);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "discover"])
        .assert()
        .failure()
        .stderr(contains("scope `chats` is not enabled"));
}

// ---------------------------------------------------------------------------
// dispatcher error surface
// ---------------------------------------------------------------------------

#[test]
fn telegram_without_subcommand_errors() {
    bin()
        .args(["telegram"])
        .assert()
        .failure()
        .stderr(contains("missing subcommand"));
}

// ---------------------------------------------------------------------------
// --dry-run (send only — reads have no side effect to preview)
// ---------------------------------------------------------------------------

#[test]
fn send_help_lists_dry_run_flag() {
    bin()
        .args(["telegram", "send", "--help"])
        .assert()
        .success()
        .stdout(contains("--dry-run"));
}

#[test]
#[serial]
fn send_dry_run_previews_without_token() {
    // Dry-run must succeed without any bot token in the keychain. The
    // memory keyring is per-process and no prior step populated it; a
    // passing exit here proves `telegram_http_for` skips `load_token`
    // when --dry-run is active.
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "telegram",
            "send",
            "--chat",
            "12345",
            "--dry-run",
            "hello world",
        ])
        .assert()
        .success()
        .stdout(contains("telegram.send"))
        .stdout(contains("\"body\": \"hello world\""))
        .stdout(contains("\"chat_id\": \"12345\""));
}

#[test]
#[serial]
fn send_dry_run_still_enforces_scope() {
    // Scope check fires before the dry-run short-circuit, so preview
    // respects the policy boundary.
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_scopes(home.path(), &["messages.read", "chats"]);
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "12345", "--dry-run", "hello"])
        .assert()
        .failure()
        .stderr(contains("scope `messages.send` is not enabled"));
}

#[test]
#[serial]
fn send_dry_run_still_rejects_oversized_body() {
    // Pre-flight validation runs regardless of --dry-run.
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let body = "x".repeat(4097);
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "12345", "--dry-run", &body])
        .assert()
        .failure()
        .stderr(contains("4097 characters").and(contains("hard limit is 4096")));
}

#[test]
#[serial]
fn send_dry_run_does_not_print_sent_line() {
    // The live-path trailing "Sent message X to …" line would be a
    // lie in dry-run mode; assert it's suppressed.
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "12345", "--dry-run", "hi"])
        .assert()
        .success()
        .stdout(contains("Sent message").not());
}

#[test]
#[serial]
fn send_dry_run_resolves_directory_alias() {
    // Dry-run must still run directory resolution so a policy check
    // against the alias fires. `team-room` -> `-100...` via the
    // directory set command; dry-run then previews with the resolved
    // numeric id.
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "telegram",
            "directory",
            "set",
            "team-room",
            "--",
            "-1001234567890",
        ])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "telegram",
            "send",
            "--chat",
            "team-room",
            "--dry-run",
            "ship it",
        ])
        .assert()
        .success()
        .stdout(contains("\"chat_id\": \"-1001234567890\""));
}
