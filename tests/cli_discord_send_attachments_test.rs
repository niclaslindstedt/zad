//! CLI-level tests for `zad discord send --file`. Exercises clap
//! parsing, the oversized-file-count guard, and the dry-run payload
//! shape. Never contacts Discord — every positive case uses `--dry-run`,
//! which short-circuits before any network call.

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
        "application_id = \"1234567890\"\nscopes = [\"guilds\", \"messages.read\", \"messages.send\"]\ndefault_guild = \"999\"\n",
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

fn fixture_file(dir: &std::path::Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, contents).unwrap();
    p
}

#[test]
#[serial]
fn single_file_dry_run_emits_attachment_metadata() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    let f = fixture_file(project.path(), "report.txt", b"hello world\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "12345",
            "--dry-run",
            "--file",
        ])
        .arg(&f)
        .arg("see attached")
        .assert()
        .success()
        .stdout(contains("\"attachments\""))
        .stdout(contains("\"basename\": \"report.txt\""))
        .stdout(contains("\"bytes\": 12"));
}

#[test]
#[serial]
fn multiple_files_dry_run_records_each() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    let a = fixture_file(project.path(), "a.log", b"aaa");
    let b = fixture_file(project.path(), "b.png", &[0u8; 8]);

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "12345",
            "--dry-run",
            "--file",
        ])
        .arg(&a)
        .arg("--file")
        .arg(&b)
        .arg("two files")
        .assert()
        .success()
        .stdout(contains("\"basename\": \"a.log\""))
        .stdout(contains("\"basename\": \"b.png\""));
}

#[test]
#[serial]
fn file_only_send_allows_empty_body() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    let f = fixture_file(project.path(), "just-a-file.txt", b"hi");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "12345",
            "--dry-run",
            "--file",
        ])
        .arg(&f)
        .assert()
        .success()
        .stdout(contains("\"body\": \"\""))
        .stdout(contains("\"basename\": \"just-a-file.txt\""));
}

#[test]
#[serial]
fn over_ten_files_is_rejected_before_network() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    let mut cmd = bin();
    cmd.env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "--dry-run"]);
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
fn missing_file_surfaces_clean_error() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "12345",
            "--dry-run",
            "--file",
            "/nonexistent/nope.txt",
            "body",
        ])
        .assert()
        .failure()
        .stderr(contains("not readable"));
}

#[test]
#[serial]
fn permissions_deny_by_extension() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    // Write a local permissions file that only allows png/jpg.
    let perms_dir = project.path().join(".zad").join("projects");
    std::fs::create_dir_all(&perms_dir).ok();
    // The local project slug is derived from the project dir path. Use
    // the `permissions init --local` surface to write the file in the
    // canonical location, then hand-edit it to add the attachments
    // rule.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "init", "--local", "--force"])
        .assert()
        .success();

    // Find and rewrite the local permissions file. The project slug
    // lives under `~/.zad/projects/<slug>/services/discord/`.
    let project_services = home.path().join(".zad").join("projects");
    let mut perms_path = None;
    for entry in std::fs::read_dir(&project_services).unwrap() {
        let e = entry.unwrap();
        let candidate = e
            .path()
            .join("services")
            .join("discord")
            .join("permissions.toml");
        if candidate.exists() {
            perms_path = Some(candidate);
            break;
        }
    }
    let perms_path = perms_path.expect("local permissions file created by init");
    std::fs::write(
        &perms_path,
        r#"
[send.attachments]
extensions = { allow = ["png", "jpg"], deny = [] }
"#,
    )
    .unwrap();

    let f = fixture_file(project.path(), "secret.pdf", b"%PDF-1.4");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "send",
            "--channel",
            "12345",
            "--dry-run",
            "--file",
        ])
        .arg(&f)
        .arg("ship it")
        .assert()
        .failure()
        .stderr(contains("secret.pdf").and(contains("not in allow list")));
}
