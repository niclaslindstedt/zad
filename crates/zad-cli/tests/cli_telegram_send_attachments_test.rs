//! CLI-level tests for `zad telegram send --file`. Exercises clap
//! parsing, the caption vs text-length cap swap, dispatch to
//! `sendDocument` / `sendMediaGroup`, and dry-run payload shape. Never
//! contacts Telegram — every positive case uses `--dry-run`.

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
        .join("telegram")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "bot_username = \"zad_test_bot\"\nscopes = [\"messages.send\", \"messages.read\", \"chats\"]\n",
    )
    .unwrap();
}

fn enable_telegram(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .current_dir(project)
        .args(["service", "enable", "telegram"])
        .assert()
        .success();
}

fn fixture_file(dir: &std::path::Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, contents).unwrap();
    p
}

#[test]
#[serial]
fn single_file_dispatches_to_send_document() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let f = fixture_file(project.path(), "payload.txt", b"abc");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run", "--file"])
        .arg(&f)
        .arg("caption")
        .assert()
        .success()
        .stdout(contains("\"method\": \"sendDocument\""))
        .stdout(contains("\"basename\": \"payload.txt\""))
        .stdout(contains("\"bytes\": 3"));
}

#[test]
#[serial]
fn multiple_files_dispatch_to_media_group() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let a = fixture_file(project.path(), "one.log", b"a");
    let b = fixture_file(project.path(), "two.log", b"bb");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run", "--file"])
        .arg(&a)
        .arg("--file")
        .arg(&b)
        .arg("both")
        .assert()
        .success()
        .stdout(contains("\"method\": \"sendMediaGroup\""))
        .stdout(contains("one.log"))
        .stdout(contains("two.log"));
}

#[test]
#[serial]
fn no_files_still_uses_send_message() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run", "hi"])
        .assert()
        .success()
        .stdout(contains("\"method\": \"sendMessage\""));
}

#[test]
#[serial]
fn caption_cap_applies_when_attachments_present() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let f = fixture_file(project.path(), "a.txt", b"x");
    let body = "x".repeat(1025);

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run", "--file"])
        .arg(&f)
        .arg(&body)
        .assert()
        .failure()
        .stderr(contains("1025 characters").and(contains("caption cap")));
}

#[test]
#[serial]
fn text_cap_still_applies_without_attachments() {
    // Regression guard: we narrowed the cap only when attachments are
    // present. A 2000-char plain text message should still succeed —
    // the 4096 cap applies.
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let body = "x".repeat(2000);
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run", &body])
        .assert()
        .success();
}

#[test]
#[serial]
fn over_ten_files_rejected_before_network() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let mut cmd = bin();
    cmd.env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run"]);
    for i in 0..11 {
        let f = fixture_file(project.path(), &format!("f{i}.txt"), b"x");
        cmd.arg("--file").arg(&f);
    }
    cmd.arg("hi")
        .assert()
        .failure()
        .stderr(contains("11 attachments").and(contains("cap of 10")));
}

#[test]
#[serial]
fn file_only_send_allows_empty_body() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_telegram(home.path(), project.path());

    let f = fixture_file(project.path(), "solo.txt", b"z");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["telegram", "send", "--chat", "42", "--dry-run", "--file"])
        .arg(&f)
        .assert()
        .success()
        .stdout(contains("\"body\": \"\""));
}

#[test]
#[serial]
fn missing_file_surfaces_clean_error() {
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
            "42",
            "--dry-run",
            "--file",
            "/nonexistent/nope.txt",
            "body",
        ])
        .assert()
        .failure()
        .stderr(contains("not readable"));
}
