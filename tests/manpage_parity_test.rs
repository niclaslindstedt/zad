//! Enforces OSS_SPEC §12.3: every top-level `zad` subcommand has a
//! corresponding `man/<name>.md` page, and every `man/*.md` page is
//! either `main.md` (the overview) or corresponds to a real clap
//! subcommand. This keeps the manpage set and the parser in lockstep.

use clap::CommandFactory;
use zad::cli::{Cli, man};

#[test]
fn every_top_level_subcommand_has_a_manpage() {
    let cmd = Cli::command();
    let pages: Vec<&str> = man::PAGES.iter().map(|(n, _)| *n).collect();

    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let name = sub.get_name();
        assert!(
            pages.contains(&name),
            "subcommand `{name}` has no corresponding `man/{name}.md`; \
             add the page or remove the subcommand. Registered pages: {pages:?}"
        );
    }
}

#[test]
fn every_manpage_corresponds_to_a_subcommand_or_is_main() {
    let cmd = Cli::command();
    let subs: Vec<String> = cmd
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .collect();

    for (page, _) in man::PAGES {
        if *page == "main" {
            continue;
        }
        assert!(
            subs.iter().any(|s| s == page),
            "manpage `man/{page}.md` has no corresponding clap subcommand; \
             remove the page or wire up the subcommand. Subcommands: {subs:?}"
        );
    }
}

#[test]
fn main_manpage_is_present() {
    assert!(
        man::PAGES.iter().any(|(n, _)| *n == "main"),
        "`man/main.md` must be registered as the top-level overview (OSS_SPEC §12.3)"
    );
}
