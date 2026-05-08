//! Coverage for the `ZAD_PERMISSIONS_PATH` and `ZAD_PERMISSIONS_ROOT`
//! env vars: pin the *local* permissions file to a fixed location so
//! the same policy applies regardless of which directory `zad` runs
//! from. Global resolution is unaffected.

use assert_cmd::Command;
use predicates::str::contains;
use serial_test::serial;

mod common;
use common::contains_path;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

fn seed_discord_creds(home: &std::path::Path) {
    let p = home
        .join(".zad")
        .join("services")
        .join("discord")
        .join("config.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        "application_id = \"1234567890\"\n\
         scopes = [\"guilds\", \"messages.read\", \"messages.send\"]\n\
         default_guild = \"999\"\n",
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

fn write_signed_permissions(path: &std::path::Path, body: &str) {
    use zad::permissions::SigningKey;
    use zad::service::discord::permissions::{self as perms, DiscordPermissionsRaw};
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let raw: DiscordPermissionsRaw =
        toml::from_str(body).expect("write_signed_permissions: body must be valid TOML");
    let key = SigningKey::generate();
    perms::save_file(path, &raw, &key).unwrap();
}

#[test]
#[serial]
fn permissions_path_env_var_pins_local_file_across_directories() {
    let home = tempfile::tempdir().unwrap();
    let pinned_dir = tempfile::tempdir().unwrap();
    let project_a = tempfile::tempdir().unwrap();
    let project_b = tempfile::tempdir().unwrap();
    seed_discord_creds(home.path());
    // `service enable` is cwd-aware, but the pinned permissions file
    // does not need to live under any particular project tree.
    enable_discord(home.path(), project_a.path());
    enable_discord(home.path(), project_b.path());

    let pinned = pinned_dir.path().join("my-permissions.toml");
    write_signed_permissions(&pinned, "[send]\nchannels.deny = [\"*admin*\"]\n");

    // Same env var, different cwds: both should refuse the denied
    // channel because the local layer is pinned to `pinned`.
    for cwd in [project_a.path(), project_b.path()] {
        bin()
            .env("ZAD_HOME_OVERRIDE", home.path())
            .env("ZAD_PERMISSIONS_PATH", &pinned)
            .current_dir(cwd)
            .args([
                "discord",
                "permissions",
                "check",
                "--function",
                "send",
                "--channel",
                "server-admin",
            ])
            .assert()
            .failure()
            .stdout(contains("deny"));
    }
}

#[test]
#[serial]
fn permissions_path_env_var_is_reported_by_path_subcommand() {
    let home = tempfile::tempdir().unwrap();
    let pinned_dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_discord_creds(home.path());
    enable_discord(home.path(), project.path());

    let pinned = pinned_dir.path().join("custom.toml");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("ZAD_PERMISSIONS_PATH", &pinned)
        .current_dir(project.path())
        .args(["discord", "permissions", "path"])
        .assert()
        .success()
        .stdout(contains_path("custom.toml"));
}

#[test]
#[serial]
fn permissions_root_env_var_appends_service_subpath() {
    let home = tempfile::tempdir().unwrap();
    let root_dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_discord_creds(home.path());
    enable_discord(home.path(), project.path());

    // `<root>/discord/permissions.toml` — the convention the env var
    // promises.
    let pinned = root_dir.path().join("discord").join("permissions.toml");
    write_signed_permissions(&pinned, "[send]\nchannels.deny = [\"*admin*\"]\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("ZAD_PERMISSIONS_ROOT", root_dir.path())
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "check",
            "--function",
            "send",
            "--channel",
            "server-admin",
        ])
        .assert()
        .failure()
        .stdout(contains("deny"));
}

#[test]
#[serial]
fn permissions_path_takes_precedence_over_root() {
    let home = tempfile::tempdir().unwrap();
    let root_dir = tempfile::tempdir().unwrap();
    let path_dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_discord_creds(home.path());
    enable_discord(home.path(), project.path());

    // _ROOT location: would *allow* (no rules).
    let root_file = root_dir.path().join("discord").join("permissions.toml");
    write_signed_permissions(&root_file, "");

    // _PATH location: denies. _PATH must win.
    let path_file = path_dir.path().join("strict.toml");
    write_signed_permissions(&path_file, "[send]\nchannels.deny = [\"*admin*\"]\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .env("ZAD_PERMISSIONS_ROOT", root_dir.path())
        .env("ZAD_PERMISSIONS_PATH", &path_file)
        .current_dir(project.path())
        .args([
            "discord",
            "permissions",
            "check",
            "--function",
            "send",
            "--channel",
            "server-admin",
        ])
        .assert()
        .failure()
        .stdout(contains("deny"));
}
