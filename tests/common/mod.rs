//! Shared utilities for zad's integration tests.
//!
//! Rust's integration-test harness compiles every `.rs` in `tests/` as
//! its own binary, so there is no automatic `pub` surface between test
//! files. Each test that needs these helpers declares `mod common;` at
//! the top — a `mod.rs` under `tests/common/` is the canonical way to
//! share code without creating another test binary.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use predicates::BoxPredicate;
use predicates::prelude::*;
use predicates::str::contains;

/// `predicates::str::contains` that tolerates OS-specific path
/// separators. Authors write forward-slash paths (`services/discord/
/// config.toml`); on Windows the rendered path uses backslashes
/// (`services\discord\config.toml`) and a plain `contains` assertion
/// silently fails. This predicate matches either form.
///
/// Use this for **every** assertion that substrings a filesystem path
/// out of stdout/stderr — otherwise the test passes on Unix CI and
/// fails on Windows CI after the branch is pushed.
pub fn contains_path(fragment: &str) -> BoxPredicate<str> {
    let unix = fragment.to_owned();
    let windows = unix.replace('/', "\\");
    BoxPredicate::new(contains(unix).or(contains(windows)))
}

/// Slugify a project path the same way the `zad` binary does, including
/// the macOS realpath workaround.
///
/// The binary derives its slug from `std::env::current_dir()`. On macOS
/// tempfile hands out paths under `/var/folders/...` (a symlink to
/// `/private/var/folders/...`), and `getcwd(3)` inside the spawned
/// child resolves the symlink — so a test that builds a path from
/// `tempfile::tempdir().path()` and expects to read back what the
/// binary wrote must canonicalize first or the slugs won't match.
///
/// Windows canonical paths carry a `\\?\` prefix that (a) the child's
/// `current_dir()` does *not* return and (b) slugifies to filenames
/// with `?` in them, which Windows rejects — so canonicalization is
/// skipped there.
pub fn project_slug(p: &Path) -> String {
    let effective = if cfg!(target_os = "macos") {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    } else {
        p.to_path_buf()
    };
    effective
        .to_str()
        .expect("tempdir path must be UTF-8")
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            _ => c,
        })
        .collect()
}

/// `<home>/.zad/projects/<slug>/` for the given tempdir project.
pub fn project_dir(home: &Path, project: &Path) -> PathBuf {
    home.join(".zad")
        .join("projects")
        .join(project_slug(project))
}

/// `<home>/.zad/projects/<slug>/services/<service>/` for the given
/// tempdir project and service name.
pub fn project_service_dir(home: &Path, project: &Path, service: &str) -> PathBuf {
    project_dir(home, project).join("services").join(service)
}
