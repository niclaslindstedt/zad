//! End-to-end coverage for the global `--wait` flag and the
//! [`zad::ZadError::RateLimited`] human + JSON renderings.
//!
//! These tests exercise the *pre-call* gate by seeding a persisted
//! deadline directly on disk (no network involved). The Discord
//! subcommand is convenient because it forwards through the same
//! dispatch path every API-bearing subcommand uses; nothing in these
//! tests actually reaches Discord.

use std::path::Path;
use std::time::Duration;

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

/// Write a synthetic rate-limit deadline file so the pre-call gate
/// trips before any network code runs. `secs_in_future` controls how
/// far out the deadline lives; pass a small value for `--wait` tests
/// so the test isn't slow.
fn seed_deadline(home: &Path, service: &str, secs_in_future: i64) {
    let path = home
        .join(".zad")
        .join("state")
        .join(service)
        .join("rate_limit.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let deadline = jiff::Timestamp::now()
        .checked_add(jiff::Span::new().seconds(secs_in_future))
        .unwrap();
    let body = format!(r#"{{"retry_after_utc":"{deadline}"}}"#);
    std::fs::write(&path, body).unwrap();
}

#[test]
#[serial]
fn without_wait_a_pending_deadline_fails_fast_with_helpful_message() {
    let home = tempfile::tempdir().unwrap();
    seed_deadline(home.path(), "discord", 600);

    bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .args(["discord", "channels"])
        .assert()
        .failure()
        .stderr(contains("rate-limited"))
        .stderr(contains("--wait"))
        .stderr(contains("HTTP 429"));
}

#[test]
#[serial]
fn with_wait_and_no_deadline_is_a_noop_and_does_not_block() {
    let home = tempfile::tempdir().unwrap();
    // No state file written. --wait must not sleep, must not error.
    // We confirm that by giving the command a tight timeout: the
    // command will still fail (no creds), but it must fail quickly,
    // and the failure must NOT be a rate-limit failure.
    let start = std::time::Instant::now();
    let out = bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .args(["--wait", "discord", "channels"])
        .timeout(Duration::from_secs(15))
        .assert()
        .failure();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "--wait with no state must not block; took {elapsed:?}"
    );
    out.stderr(contains("rate-limited").not());
}

#[test]
#[serial]
fn json_flag_renders_structured_rate_limit_payload_to_stdout() {
    let home = tempfile::tempdir().unwrap();
    seed_deadline(home.path(), "discord", 1234);

    // The Discord subcommand exposes a `--json` flag on its leaf
    // verbs. We pass it through so the dispatch reaches the pre-call
    // gate and the global error renderer with --json set in argv.
    let out = bin()
        .env("ZAD_HOME_OVERRIDE", home.path())
        .args(["discord", "channels", "--json"])
        .assert()
        .failure();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("\"error\""),
        "expected JSON payload on stdout; got: {stdout}"
    );
    assert!(stdout.contains("\"rate_limited\""), "got: {stdout}");
    assert!(stdout.contains("\"service\""), "got: {stdout}");
    assert!(stdout.contains("\"discord\""), "got: {stdout}");
    assert!(stdout.contains("\"retry_after_seconds\""), "got: {stdout}");
    assert!(stdout.contains("\"retry_after_utc\""), "got: {stdout}");
    // The human stderr line MUST NOT also appear when --json is set;
    // otherwise scripts piping stderr to /dev/null still see ANSI
    // colour codes. The structured payload owns stdout and stderr
    // stays quiet for this error variant.
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        !stderr.contains("rate-limited"),
        "human error should be suppressed under --json; got stderr: {stderr}"
    );
}
