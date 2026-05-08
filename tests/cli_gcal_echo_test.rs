//! Smoke test: when a gcal project's permissions file is unsigned,
//! `zad gcal calendars list` echoes the would-be call instead of
//! issuing it.
//!
//! Calendars list is the cheapest verb: it doesn't need any OAuth
//! plumbing under dry-run because the dry-run transport returns an
//! empty list. The echo path forces dry-run mode and renders the
//! signing reason.

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
        .join("gcal")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "scopes = [\"calendars.read\", \"events.read\"]\n").unwrap();
}

fn enable_gcal(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "gcal"])
        .assert()
        .success();
}

fn write_unsigned_local_permissions(home: &std::path::Path, project: &std::path::Path, body: &str) {
    let slug = common::project_slug(project);
    let p = home
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("gcal")
        .join("permissions.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
}

#[test]
#[serial]
fn calendars_list_with_unsigned_permissions_echoes_and_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_gcal(home.path(), project.path());
    write_unsigned_local_permissions(home.path(), project.path(), "");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["gcal", "calendars", "list", "--json"])
        .assert()
        .code(3)
        .stdout(contains("\"kind\": \"not_trusted\""));
}
