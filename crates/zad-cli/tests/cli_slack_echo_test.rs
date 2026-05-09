//! Smoke test: when a slack project's permissions file is unsigned,
//! `zad slack send` echoes the would-be call instead of issuing it.

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
        .join("slack")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "app_id = \"A012345678\"\n\
         workspace = \"test-workspace\"\n\
         scopes = [\"chat:write\", \"channels:read\", \"channels:history\"]\n",
    )
    .unwrap();
}

fn enable_slack(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "slack"])
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
        .join("slack")
        .join("permissions.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
}

#[test]
#[serial]
fn send_with_unsigned_permissions_echoes_and_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_slack(home.path(), project.path());
    write_unsigned_local_permissions(home.path(), project.path(), "[send]\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["slack", "send", "--channel", "C1234567", "--json", "hello"])
        .assert()
        .code(3)
        .stdout(contains("\"echoed\""))
        .stdout(contains("\"kind\": \"not_trusted\""));
}
