//! Implementation of the `zad man [command]` subcommand mandated by
//! `OSS_SPEC.md` §12.3.
//!
//! Manpages under `man/*.md` are embedded via `include_str!`. The
//! command-to-file mapping is enumerated explicitly so the parity test
//! in `tests/manpage_parity_test.rs` can cross-check it against the
//! clap tree.

use std::fmt::Write as _;

use clap::Args;

use crate::error::{Result, ZadError};

#[derive(Debug, Args)]
pub struct ManArgs {
    /// Command name (e.g. `discord`, `service`). When omitted, lists the
    /// available manpages. `main` is the top-level overview.
    pub command: Option<String>,
}

pub const PAGES: &[(&str, &str)] = &[
    ("main", include_str!("../../man/main.md")),
    ("commands", include_str!("../../man/commands.md")),
    ("discord", include_str!("../../man/discord.md")),
    ("docs", include_str!("../../man/docs.md")),
    ("man", include_str!("../../man/man.md")),
    ("service", include_str!("../../man/service.md")),
    ("telegram", include_str!("../../man/telegram.md")),
];

pub fn run(args: ManArgs) -> Result<()> {
    match args.command {
        None => {
            let mut out = String::new();
            out.push_str("Available manpages (run `zad man <command>` to read):\n");
            for (name, _) in PAGES {
                let _ = writeln!(out, "  {name}");
            }
            print!("{out}");
            Ok(())
        }
        Some(command) => match PAGES.iter().find(|(n, _)| *n == command) {
            Some((_, body)) => {
                print!("{body}");
                Ok(())
            }
            None => Err(ZadError::Invalid(format!(
                "no manpage for: `{command}`. Run `zad man` to list available manpages."
            ))),
        },
    }
}
