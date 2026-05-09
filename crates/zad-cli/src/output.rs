//! Central semantic output module (OSS_SPEC.md §19.4).
//!
//! Non-contract CLI output (progress, warnings, headers, status lines)
//! should go through this module rather than raw `println!`/`eprintln!`
//! so every message is:
//!
//! 1. Echoed to stderr for humans, leaving stdout reserved for
//!    machine-readable output (`--json`, `--help-agent`, etc.) —
//!    tools piping stdout to `jq` never see incidental status text.
//! 2. Mirrored to the always-on tracing file log so we have a trail of
//!    what the CLI told the user.
//! 3. Coloured consistently on a TTY (via a tiny ANSI helper) and plain
//!    when stderr is redirected.
//!
//! Callers that emit machine-readable output (e.g. JSON replies to
//! `--json` flags) or dedicated discovery surfaces
//! (`--help-agent`, `--debug-agent`, `zad commands`, `zad docs`,
//! `zad man`) MUST continue to use raw `println!` so the contract holds
//! byte-for-byte.

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

/// Renders `msg` as a neutral status line — the default for user-facing
/// progress ("Loaded 4 channels").
pub fn status(msg: &str) {
    emit(Tone::Status, msg);
    tracing::info!("{msg}");
}

/// Renders `msg` as an informational note (same weight as `status` but
/// typed so future theming can distinguish them).
pub fn info(msg: &str) {
    emit(Tone::Info, msg);
    tracing::info!("{msg}");
}

/// Renders `msg` as a warning that doesn't abort the command.
pub fn warn(msg: &str) {
    emit(Tone::Warn, msg);
    tracing::warn!("{msg}");
}

/// Renders `msg` as a prominent header/section separator. Unlike the
/// others, headers include a blank line before the banner.
pub fn header(msg: &str) {
    eprintln!();
    emit(Tone::Header, msg);
    tracing::info!("{msg}");
}

/// Renders `msg` as an error. Does not exit — the caller propagates a
/// `Result` upward.
pub fn error(msg: &str) {
    emit(Tone::Error, msg);
    tracing::error!("{msg}");
}

#[derive(Clone, Copy)]
enum Tone {
    Status,
    Info,
    Warn,
    Header,
    Error,
}

fn emit(tone: Tone, msg: &str) {
    let ansi = ansi_enabled();
    let mut stderr = std::io::stderr().lock();
    match tone {
        Tone::Status => {
            if ansi {
                let _ = writeln!(stderr, "\x1b[2m{msg}\x1b[0m");
            } else {
                let _ = writeln!(stderr, "{msg}");
            }
        }
        Tone::Info => {
            let _ = writeln!(stderr, "{msg}");
        }
        Tone::Warn => {
            if ansi {
                let _ = writeln!(stderr, "\x1b[33mwarning:\x1b[0m {msg}");
            } else {
                let _ = writeln!(stderr, "warning: {msg}");
            }
        }
        Tone::Header => {
            if ansi {
                let _ = writeln!(stderr, "\x1b[1m{msg}\x1b[0m");
            } else {
                let _ = writeln!(stderr, "{msg}");
            }
        }
        Tone::Error => {
            if ansi {
                let _ = writeln!(stderr, "\x1b[31merror:\x1b[0m {msg}");
            } else {
                let _ = writeln!(stderr, "error: {msg}");
            }
        }
    }
}

fn ansi_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        std::io::stderr().is_terminal()
    })
}
