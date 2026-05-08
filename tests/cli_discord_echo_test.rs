//! Echo-mode for runtime verbs: when the permissions file isn't
//! signed (no trust-store entry) or its signature doesn't match the
//! file's bytes, the verb must NOT issue a real Discord call. Instead
//! it prints the would-be call (reusing the `--dry-run` infrastructure)
//! plus the signing reason, and exits 3.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serial_test::serial;

mod common;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

fn seed_global_creds(home: &std::path::Path) {
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
         default_guild = \"42\"\n",
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

/// Drop a permissions file at the project-local scope without signing
/// it. The trust store has no entry for this path so `verify_raw`
/// returns [`ZadError::NotTrusted`].
fn write_unsigned_local_permissions(home: &std::path::Path, project: &std::path::Path, body: &str) {
    let slug = common::project_slug(project);
    let p = home
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("permissions.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
}

/// Sign a permissions file via the library, mirroring what
/// `zad discord permissions init` does. Used by the tamper test.
fn write_signed_local_permissions(
    home: &std::path::Path,
    project: &std::path::Path,
    body: &str,
) -> std::path::PathBuf {
    use zad::permissions::SigningKey;
    use zad::permissions::signing::SIGNING_ACCOUNT;
    use zad::service::discord::permissions::{self as perms, DiscordPermissionsRaw};
    let slug = common::project_slug(project);
    let p = home
        .join(".zad")
        .join("projects")
        .join(&slug)
        .join("services")
        .join("discord")
        .join("permissions.toml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    zad::secrets::use_memory_backend();
    // SAFETY: tests are #[serial], no concurrent writers.
    unsafe {
        std::env::set_var("ZAD_HOME_OVERRIDE", home);
    }
    let key = SigningKey::generate();
    let _ = zad::secrets::delete(SIGNING_ACCOUNT);
    zad::secrets::store(SIGNING_ACCOUNT, &key.to_keychain_encoded()).unwrap();
    let raw: DiscordPermissionsRaw =
        toml::from_str(body).expect("permissions body must be valid TOML");
    perms::save_file(&p, &raw, &key).unwrap();
    p
}

// ---------------------------------------------------------------------------
// `not_trusted` — file present but no trust-store entry.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_with_unsigned_permissions_echoes_json_and_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_unsigned_local_permissions(home.path(), project.path(), "[send]\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "--json", "hello"])
        .assert()
        .code(3)
        .stdout(contains("\"echoed\""))
        .stdout(contains("\"kind\": \"not_trusted\""))
        .stdout(contains("\"target_id\": \"12345\""))
        .stdout(contains("\"command\": \"discord.send\""));
}

#[test]
#[serial]
fn send_with_unsigned_permissions_echoes_human_summary() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_unsigned_local_permissions(home.path(), project.path(), "[send]\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "hello"])
        .assert()
        .code(3)
        .stdout(contains("would have run:"))
        .stdout(contains("would send 5 chars to channel 12345"))
        .stdout(contains("reason:"))
        .stdout(contains("not trusted"));
}

// ---------------------------------------------------------------------------
// `signature_invalid` — file edited after signing.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn send_with_tampered_permissions_echoes_signature_invalid() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    // Sign a body that includes a substitutable sentinel.
    let p = write_signed_local_permissions(
        home.path(),
        project.path(),
        "[send.channels]\nallow = [\"bot-*\"]\n",
    );
    // Flip a byte in the file's canonical bytes — change the allow
    // pattern to something different. The trust-store signature for
    // this path no longer matches the file's contents.
    let body = std::fs::read_to_string(&p).unwrap();
    let tampered = body.replace("bot-*", "bot-?");
    assert_ne!(body, tampered, "sentinel substitution must have matched");
    std::fs::write(&p, &tampered).unwrap();

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "--json", "hi"])
        .assert()
        .code(3)
        .stdout(contains("\"kind\": \"signature_invalid\""))
        .stdout(contains("\"target_id\": \"12345\""));
}

// ---------------------------------------------------------------------------
// Diagnostic verbs must STILL hard-fail on signing errors — echo mode
// is scoped to runtime verbs only.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn permissions_show_with_unsigned_file_hard_fails_not_echoes() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());
    write_unsigned_local_permissions(home.path(), project.path(), "[send]\n");

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "permissions", "show"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("not trusted"));
}

// ---------------------------------------------------------------------------
// Absent permissions file → no echo. Verified through `--dry-run`,
// which doesn't need a real bot token. The dry-run JSON, not an echo
// envelope, lands on stdout, and the exit code is 0.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn dry_run_without_permissions_does_not_echo() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    seed_global_creds(home.path());
    enable_discord(home.path(), project.path());

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .current_dir(project.path())
        .args(["discord", "send", "--channel", "12345", "--dry-run", "hi"])
        .assert()
        .success()
        .stdout(contains("\"discord.send\""))
        .stdout(contains("\"echoed\"").not())
        .stdout(contains("\"kind\": \"not_trusted\"").not());
}
