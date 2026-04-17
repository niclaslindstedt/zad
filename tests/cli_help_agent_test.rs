//! Tests for the top-level `zad --help-agent` flag mandated by
//! `OSS_SPEC.md` §12.1: compact, prompt-injectable plain text that lists
//! the CLI's commands, its most important flags and env vars, and points
//! an agent at the discovery surfaces (`commands`, `man`, `docs`).

use assert_cmd::Command;
use predicates::str::contains;

fn bin() -> Command {
    let mut c = Command::cargo_bin("zad").expect("zad binary built");
    c.env("ZAD_SECRETS_MEMORY", "1");
    c
}

#[test]
fn help_agent_succeeds_without_a_subcommand() {
    bin().arg("--help-agent").assert().success();
}

#[test]
fn help_agent_emits_plain_ascii_no_escape_sequences() {
    let out = bin()
        .arg("--help-agent")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("utf8 stdout");

    // Spliceable into a prompt: no ANSI escape byte.
    assert!(
        !body.contains('\u{1b}'),
        "help-agent output must contain no ANSI escapes"
    );
    // Short enough to not dominate a prompt — spec says 50-200 lines
    // typical. Give a generous ceiling so tiny copy tweaks don't fail
    // the test, but catch runaway output.
    let lines = body.lines().count();
    assert!(
        (5..=250).contains(&lines),
        "help-agent output was {lines} lines; expected 5..=250"
    );
}

#[test]
fn help_agent_names_the_binary_and_version() {
    let out = bin()
        .arg("--help-agent")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("utf8 stdout");

    assert!(body.contains("zad"), "binary name must appear");
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        body.contains(version),
        "version `{version}` must appear in help-agent output"
    );
}

#[test]
fn help_agent_lists_top_level_commands() {
    bin()
        .arg("--help-agent")
        .assert()
        .success()
        .stdout(contains("service"))
        .stdout(contains("discord"));
}

#[test]
fn help_agent_points_at_discovery_surfaces() {
    // Per §12.1 requirements 4 & 5: must point at `commands`,
    // `commands <name>`, `docs`, `man`.
    bin()
        .arg("--help-agent")
        .assert()
        .success()
        .stdout(contains("zad commands"))
        .stdout(contains("zad man"))
        .stdout(contains("zad docs"));
}

#[test]
fn help_agent_documents_important_flags_and_env_vars() {
    bin()
        .arg("--help-agent")
        .assert()
        .success()
        .stdout(contains("--debug"))
        .stdout(contains("--help-agent"))
        .stdout(contains("ZAD_HOME_OVERRIDE"))
        .stdout(contains("ZAD_SECRETS_MEMORY"));
}
