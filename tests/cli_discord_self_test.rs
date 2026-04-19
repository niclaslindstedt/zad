//! Tests for `zad discord self {show,set,clear}` and for the `@me`
//! sigil in `zad discord send --dm @me`.
//!
//! `self set <id>` runs API validation via `DiscordHttp::get_user`, so
//! its happy path requires network access and is covered by manual E2E
//! only. CI covers `show`, `clear`, and the error branches.

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
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234\"\nscopes = [\"guilds\", \"messages.send\"]\n",
    )
    .unwrap();
}

fn seed_global_with_self(home: &std::path::Path, self_user_id: &str) {
    let p = home
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        format!(
            "application_id = \"1234\"\nscopes = [\"guilds\", \"messages.send\"]\nself_user_id = \"{self_user_id}\"\n"
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
// self show / clear
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn self_show_reports_not_configured_when_unset() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "self", "show"])
        .assert()
        .success()
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn self_clear_reverts_to_not_configured() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_self(home.path(), "555666777");
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "self", "clear"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "self", "show"])
        .assert()
        .success()
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn self_show_json_shape_when_set() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_self(home.path(), "111222333");
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "self", "show", "--json"])
        .assert()
        .success()
        .stdout(contains("\"command\": \"discord.self.show\""))
        .stdout(contains("\"self_user_id\": \"111222333\""));
}

// ---------------------------------------------------------------------------
// send --dm @me
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_at_me_resolves_self_user_id_in_dry_run() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_with_self(home.path(), "8675309");
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--dm", "@me", "--dry-run", "hi"])
        .assert()
        .success()
        .stdout(contains("8675309"));
}

#[test]
#[serial]
fn send_at_me_without_self_configured_errors_with_hint() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path()); // no self_user_id
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--dm", "@me", "--dry-run", "hi"])
        .assert()
        .failure()
        .stderr(contains("self set"));
}
