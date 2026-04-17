//! Tests for the offline surface of `zad discord discover` / `directory`:
//! loading the file, manually mapping entries, and watching the runtime
//! verbs (`send`, `read`, `join`, `leave`, `channels`) resolve names the
//! directory knows about. None of these tests hit the network — the
//! `discover` happy path needs a bot token and is exercised by hand.

use assert_cmd::Command;
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

fn directory_path(home: &std::path::Path, project: &std::path::Path) -> std::path::PathBuf {
    // On macOS tempfile hands out paths under `/var/`, a symlink to
    // `/private/var/`, and `getcwd(3)` inside the spawned child resolves
    // the symlink — so the slug must match the canonical form. Windows
    // canonicalization yields `\\?\`-prefixed paths the child won't
    // return, so we skip canonicalization there.
    let effective = if cfg!(target_os = "macos") {
        std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf())
    } else {
        project.to_path_buf()
    };
    let slug: String = effective
        .to_str()
        .unwrap()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect();
    home.join(".zad")
        .join("projects")
        .join(slug)
        .join("services")
        .join("discord")
        .join("directory.toml")
}

// ---------------------------------------------------------------------------
// project-enablement guard
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn directory_requires_project_enablement() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory"])
        .assert()
        .failure()
        .stderr(contains("discord is not enabled for this project"));
}

// ---------------------------------------------------------------------------
// list on an empty directory
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn directory_list_on_empty_hints_at_discover() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory"])
        .assert()
        .success()
        .stdout(contains("(empty)"))
        .stdout(contains("zad discord discover"));
}

// ---------------------------------------------------------------------------
// manual mapping
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn directory_set_persists_each_kind() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    for (kind, name, id) in [
        ("guild", "main-server", "111"),
        ("channel", "general", "222"),
        ("channel", "main-server/deploys", "333"),
        ("user", "alice", "444"),
    ] {
        bin()
            .env("ZAD_HOME_OVERRIDE", home.path())
            .current_dir(project.path())
            .args(["discord", "directory", "set", kind, name, id])
            .assert()
            .success();
    }

    let body = std::fs::read_to_string(directory_path(home.path(), project.path())).unwrap();
    assert!(body.contains("main-server"));
    assert!(body.contains("general"));
    assert!(body.contains("main-server/deploys"));
    assert!(body.contains("alice"));

    // List reports them back.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory"])
        .assert()
        .success()
        .stdout(contains("main-server"))
        .stdout(contains("general"))
        .stdout(contains("alice"));
}

#[test]
#[serial]
fn directory_set_rejects_non_numeric_id() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args([
            "discord",
            "directory",
            "set",
            "channel",
            "general",
            "notasnowflake",
        ])
        .assert()
        .failure()
        .stderr(contains("numeric Discord snowflake"));
}

#[test]
#[serial]
fn directory_remove_is_idempotent() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    // Seed one entry, remove it, remove again — the second call must be
    // a friendly no-op so agent scripts don't have to pre-check.
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory", "set", "channel", "general", "222"])
        .assert()
        .success();
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory", "remove", "channel", "general"])
        .assert()
        .success()
        .stdout(contains("Removed"));
    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory", "remove", "channel", "general"])
        .assert()
        .success()
        .stdout(contains("No channel entry named"));
}

#[test]
#[serial]
fn directory_clear_requires_force() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory", "clear"])
        .assert()
        .failure()
        .stderr(contains("--force"));
}

// ---------------------------------------------------------------------------
// resolver — runtime verbs accept names from the directory
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_resolves_channel_name_from_directory() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory", "set", "channel", "general", "1111"])
        .assert()
        .success();

    // Wipe credentials so we fail *after* resolution — proving the name
    // was accepted. If the resolver rejected it, we'd see the "neither
    // numeric … directory entry" error instead.
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
        .args(["discord", "send", "--channel", "general", "hi"])
        .assert()
        .failure()
        .stderr(contains("no Discord credentials found"));
}

#[test]
#[serial]
fn send_hints_at_discover_when_name_is_unknown() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "#ghost", "hi"])
        .assert()
        .failure()
        .stderr(contains("zad discord discover"))
        .stderr(contains("zad discord directory set channel ghost"));
}

#[test]
#[serial]
fn dm_strips_at_prefix_before_lookup() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "directory", "set", "user", "alice", "4444"])
        .assert()
        .success();

    // Same trick: remove creds to prove the resolver passed.
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
        .args(["discord", "send", "--dm", "@alice", "hi"])
        .assert()
        .failure()
        .stderr(contains("no Discord credentials found"));
}
