//! Implementation of the `zad docs [topic]` subcommand mandated by
//! `OSS_SPEC.md` §12.3.
//!
//! Every topic doc under `docs/*.md` is embedded into the binary via
//! `include_str!` so the exact text a contributor ships is the exact
//! text an agent sees at runtime — no filesystem lookups, no version
//! skew between the installed binary and the source tree.

use std::fmt::Write as _;

use clap::Args;

use crate::error::{Result, ZadError};

#[derive(Debug, Args)]
pub struct DocsArgs {
    /// Topic name (without the `.md` extension). When omitted, lists the
    /// available topics.
    pub topic: Option<String>,
}

const TOPICS: &[(&str, &str)] = &[
    ("architecture", include_str!("../../docs/architecture.md")),
    ("configuration", include_str!("../../docs/configuration.md")),
    (
        "getting-started",
        include_str!("../../docs/getting-started.md"),
    ),
    (
        "troubleshooting",
        include_str!("../../docs/troubleshooting.md"),
    ),
];

pub fn run(args: DocsArgs) -> Result<()> {
    match args.topic {
        None => {
            let mut out = String::new();
            out.push_str("Available topics (run `zad docs <topic>` to read):\n");
            for (name, _) in TOPICS {
                let _ = writeln!(out, "  {name}");
            }
            print!("{out}");
            Ok(())
        }
        Some(topic) => match TOPICS.iter().find(|(n, _)| *n == topic) {
            Some((_, body)) => {
                print!("{body}");
                Ok(())
            }
            None => Err(ZadError::Invalid(format!(
                "no such docs topic: `{topic}`. Run `zad docs` to list available topics."
            ))),
        },
    }
}
