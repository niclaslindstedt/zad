//! Implementation of the top-level `--help-agent` flag mandated by
//! `OSS_SPEC.md` §12.1.
//!
//! The output is a compact, plain-text, prompt-injectable description of
//! the CLI. Agents splice it into a larger prompt via command
//! substitution (`$(zad --help-agent)`) so they don't need to probe the
//! tool from scratch. To keep the text from going stale, command names
//! and descriptions are introspected from the clap tree — the single
//! source of truth for `--help` too — rather than hand-maintained here.

use std::fmt::Write as _;

use clap::CommandFactory;

use super::Cli;

pub fn render() -> String {
    let cmd = Cli::command();
    let binary = cmd.get_name().to_string();
    let version = crate::version();
    let about = cmd
        .get_about()
        .map(|s| s.to_string())
        .unwrap_or_else(|| String::from("A CLI."));

    let mut out = String::new();

    // (1) One-sentence description, (6) binary name + version.
    let _ = writeln!(out, "{binary} {version} — {about}");
    out.push('\n');

    // (2) Top-level commands, each with their leaf verbs for context.
    out.push_str("Commands:\n");
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let name = sub.get_name();
        let desc = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
        let _ = writeln!(out, "  {binary} {name:<10} {desc}");
        for inner in sub.get_subcommands() {
            if inner.is_hide_set() {
                continue;
            }
            let iname = inner.get_name();
            let idesc = inner.get_about().map(|s| s.to_string()).unwrap_or_default();
            let _ = writeln!(out, "    {binary} {name} {iname:<10} {idesc}");
        }
    }
    out.push('\n');

    // (3) Most important flags + env vars.
    out.push_str("Global flags:\n");
    out.push_str("  --debug          Emit debug-level logs to stderr.\n");
    out.push_str("  --help-agent     Print this block (compact, prompt-injectable).\n");
    out.push_str("  --help           Per-command usage with flag descriptions.\n");
    out.push_str("  --version        Print version and exit.\n");
    out.push('\n');
    out.push_str("Environment variables:\n");
    out.push_str("  ZAD_HOME_OVERRIDE   Override $HOME when resolving ~/.zad (tests only).\n");
    out.push_str("  ZAD_SECRETS_MEMORY  `1` = in-memory keychain backend (tests only).\n");
    out.push('\n');

    // (4) Pointer to `commands`, (5) pointer to `docs` / `man`.
    out.push_str("Discovery (see OSS_SPEC.md §12):\n");
    let _ = writeln!(
        out,
        "  {binary} commands              List every command, grep-friendly."
    );
    let _ = writeln!(
        out,
        "  {binary} commands <name>       Flags, types, exit codes for <name>."
    );
    let _ = writeln!(
        out,
        "  {binary} commands --examples   Realistic example invocations for every command."
    );
    let _ = writeln!(
        out,
        "  {binary} man [command]         Reference manpages (embedded at build time)."
    );
    let _ = writeln!(
        out,
        "  {binary} docs [topic]          Topic docs (embedded at build time)."
    );
    let _ = writeln!(
        out,
        "  {binary} --debug-agent         Troubleshooting context (log paths, env vars, config)."
    );
    out.push('\n');

    out.push_str(
        "Config lives at ~/.zad/; long-lived secrets (bot tokens, API keys) go to the OS\n\
         keychain via the `secrets` module and never appear in the TOML.\n",
    );

    out
}
