use std::fs;

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
        .join("adapters")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234567890\"\nscopes = [\"guilds\"]\n",
    )
    .unwrap();
}

#[test]
#[serial]
fn create_global_writes_flat_config_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("DISCORD_BOT_TOKEN", "t.est.token")
        .current_dir(project.path())
        .args([
            "adapter",
            "create",
            "discord",
            "--application-id",
            "1234567890",
            "--bot-token-env",
            "DISCORD_BOT_TOKEN",
            "--default-guild",
            "987654321",
            "--scopes",
            "guilds,messages.send",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success()
        .stdout(contains("global"));

    let global_path = home
        .path()
        .join(".zad")
        .join("adapters")
        .join("discord")
        .join("config.toml");
    let body = fs::read_to_string(&global_path).unwrap();

    assert!(!body.contains("[adapter.discord]"), "got:\n{body}");
    assert!(
        body.contains("application_id = \"1234567890\""),
        "got:\n{body}"
    );
    assert!(
        body.contains("default_guild = \"987654321\""),
        "got:\n{body}"
    );
    assert!(!body.contains("t.est.token"), "token leaked:\n{body}");

    // Nothing written to the project side.
    let slug = slugify(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    assert!(!project_path.exists());
}

#[test]
#[serial]
fn create_local_writes_under_project_slug() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("DISCORD_BOT_TOKEN", "t.est.token")
        .current_dir(project.path())
        .args([
            "adapter",
            "create",
            "discord",
            "--local",
            "--application-id",
            "42",
            "--bot-token-env",
            "DISCORD_BOT_TOKEN",
            "--scopes",
            "guilds",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success()
        .stdout(contains("local"));

    let slug = slugify(project.path());
    let local_creds = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("adapters")
        .join("discord")
        .join("config.toml");
    let body = fs::read_to_string(&local_creds).unwrap();
    assert!(body.contains("application_id = \"42\""), "got:\n{body}");

    let global = home
        .path()
        .join(".zad")
        .join("adapters")
        .join("discord")
        .join("config.toml");
    assert!(!global.exists(), "--local must not touch global config");
}

#[test]
#[serial]
fn add_enables_adapter_using_global_creds() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .success()
        .stdout(contains("enabled"))
        .stdout(contains("global"));

    let slug = slugify(project.path());
    let project_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("config.toml");
    let body = fs::read_to_string(&project_path).unwrap();
    assert!(body.contains("[adapter.discord]"), "got:\n{body}");
    assert!(body.contains("enabled = true"));
    assert!(!body.contains("application_id"));
}

#[test]
#[serial]
fn add_prefers_local_creds_when_present() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    // Seed local creds that should win.
    let slug = slugify(project.path());
    let local = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("adapters")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    std::fs::write(&local, "application_id = \"77\"\nscopes = [\"guilds\"]\n").unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .success()
        .stdout(contains("local"));
}

#[test]
#[serial]
fn add_fails_without_any_credentials() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .failure()
        .stderr(contains("no Discord credentials found"));
}

#[test]
#[serial]
fn add_refuses_to_overwrite_without_force() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .failure()
        .stderr(contains("already configured"));

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord", "--force"])
        .assert()
        .success();
}

#[test]
#[serial]
fn create_non_interactive_requires_application_id() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("DISCORD_BOT_TOKEN", "t.est.token")
        .current_dir(project.path())
        .args([
            "adapter",
            "create",
            "discord",
            "--bot-token-env",
            "DISCORD_BOT_TOKEN",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .failure()
        .stderr(contains("--application-id"));
}

// ---------------------------------------------------------------------------
// list / show / delete
// ---------------------------------------------------------------------------

fn create_global(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env("DISCORD_BOT_TOKEN", "t.est.token")
        .current_dir(project)
        .args([
            "adapter",
            "create",
            "discord",
            "--application-id",
            "1234567890",
            "--bot-token-env",
            "DISCORD_BOT_TOKEN",
            "--scopes",
            "guilds,messages.send",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success();
}

fn create_local(home: &std::path::Path, project: &std::path::Path) {
    bin()
        .env("ZAD_HOME_OVERRIDE", home)
        .env("DISCORD_BOT_TOKEN", "t.est.token")
        .current_dir(project)
        .args([
            "adapter",
            "create",
            "discord",
            "--local",
            "--application-id",
            "42",
            "--bot-token-env",
            "DISCORD_BOT_TOKEN",
            "--scopes",
            "guilds",
            "--non-interactive",
            "--no-validate",
        ])
        .assert()
        .success();
}

#[test]
#[serial]
fn list_reports_nothing_when_empty() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "list"])
        .assert()
        .success()
        .stdout(contains("ADAPTER"))
        .stdout(contains("discord"))
        .stdout(contains("No adapters configured"));
}

#[test]
#[serial]
fn list_reports_global_and_project_state() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "list"])
        .assert()
        .success()
        .stdout(contains("discord"))
        .stdout(contains("yes"))
        .stdout(contains("enabled"));
}

#[test]
#[serial]
fn show_reports_effective_source_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "show", "discord"])
        .assert()
        .success()
        .stdout(contains("effective : global"))
        .stdout(contains("1234567890"))
        .stdout(contains("guilds"))
        .stdout(contains("discord-bot:global"))
        .stdout(predicates::str::contains("t.est.token").not());
    // Note: the "stored" vs "missing" token state is not asserted here
    // because each child process gets its own `ZAD_SECRETS_MEMORY` map
    // and the `create` and `show` invocations run in separate processes.
}

#[test]
#[serial]
fn show_without_credentials_is_not_an_error() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "show", "discord"])
        .assert()
        .success()
        .stdout(contains("(none"))
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn show_prefers_local_over_global() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());
    create_local(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "show", "discord"])
        .assert()
        .success()
        .stdout(contains("effective : local"));
}

#[test]
#[serial]
fn delete_global_removes_file_and_keychain() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    let global_path = home
        .path()
        .join(".zad")
        .join("adapters")
        .join("discord")
        .join("config.toml");
    assert!(global_path.exists());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "delete", "discord"])
        .assert()
        .success()
        .stdout(contains("deleted"))
        .stdout(contains("cleared"));

    assert!(!global_path.exists(), "global config should be removed");

    // Subsequent show reflects absence.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "show", "discord"])
        .assert()
        .success()
        .stdout(contains("not configured"));
}

#[test]
#[serial]
fn delete_local_leaves_global_intact() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());
    create_local(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "delete", "discord", "--local"])
        .assert()
        .success()
        .stdout(contains("local"));

    let slug = slugify(project.path());
    let local_path = home
        .path()
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("adapters")
        .join("discord")
        .join("config.toml");
    let global_path = home
        .path()
        .join(".zad")
        .join("adapters")
        .join("discord")
        .join("config.toml");

    assert!(!local_path.exists(), "local config should be removed");
    assert!(global_path.exists(), "global config must not be touched");
}

#[test]
#[serial]
fn delete_missing_without_force_errors() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "delete", "discord"])
        .assert()
        .failure()
        .stderr(contains("no discord credentials"));

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "delete", "discord", "--force"])
        .assert()
        .success();
}

#[test]
#[serial]
fn delete_warns_when_project_still_references_adapter() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    create_global(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "add", "discord"])
        .assert()
        .success();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["adapter", "delete", "discord"])
        .assert()
        .success()
        .stdout(contains("warning"))
        .stdout(contains("still references"));
}

fn slugify(p: &std::path::Path) -> String {
    // On macOS tempfile hands out paths under `/var/`, a symlink to
    // `/private/var/`, and `getcwd(3)` inside the spawned child
    // resolves the symlink — so the slug must match the canonical form.
    // On Windows `std::fs::canonicalize` returns a `\\?\`-prefixed
    // extended-length path that (a) the child's `current_dir()` does
    // *not* return, and (b) slugifies to a filename with `?` in it,
    // which Windows rejects. So canonicalize macOS only.
    let effective = if cfg!(target_os = "macos") {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    } else {
        p.to_path_buf()
    };
    effective
        .to_str()
        .unwrap()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            _ => c,
        })
        .collect()
}
