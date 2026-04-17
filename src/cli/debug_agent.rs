//! Implementation of the top-level `--debug-agent` flag mandated by
//! `OSS_SPEC.md` §12.2.
//!
//! The output is a compact, plain-text troubleshooting block that an
//! agent can splice into a prompt via command substitution when it hits
//! an unexpected failure. It answers: where do logs live? where do
//! configs live, and in what precedence order? which env vars matter?
//! how do I turn on verbose output? which commands help me investigate?
//! and which version am I talking to?

use std::fmt::Write as _;

pub fn render() -> String {
    let mut out = String::new();
    let version = crate::version();

    let _ = writeln!(out, "zad {version} — diagnostics for agents");
    out.push('\n');

    out.push_str("Log files:\n");
    match crate::logging::log_path() {
        Some(p) => {
            let _ = writeln!(out, "  {}", p.display());
            let _ = writeln!(
                out,
                "  (rolled daily; the file log is always on, regardless of --debug)"
            );
        }
        None => out.push_str("  <unable to resolve a log directory on this platform>\n"),
    }
    out.push('\n');

    out.push_str("Config precedence (first match wins):\n");
    out.push_str("  1. Project-local   ~/.zad/projects/<slug>/services/<svc>/\n");
    out.push_str("  2. Global service  ~/.zad/services/<svc>/\n");
    out.push_str("  3. Built-in defaults\n");
    out.push_str("Permissions intersect rather than replace: project-local can only tighten\n");
    out.push_str("the global file, never loosen it.\n");
    out.push('\n');

    out.push_str("Resolved paths for the current working directory:\n");
    match crate::config::path::zad_home() {
        Ok(p) => {
            let _ = writeln!(out, "  ZAD_HOME      {}", p.display());
        }
        Err(e) => {
            let _ = writeln!(out, "  ZAD_HOME      <unresolved: {e}>");
        }
    }
    match crate::config::path::project_slug() {
        Ok(s) => {
            let _ = writeln!(out, "  project slug  {s}");
        }
        Err(e) => {
            let _ = writeln!(out, "  project slug  <unresolved: {e}>");
        }
    }
    match crate::config::path::project_dir() {
        Ok(p) => {
            let _ = writeln!(out, "  project dir   {}", p.display());
        }
        Err(e) => {
            let _ = writeln!(out, "  project dir   <unresolved: {e}>");
        }
    }
    out.push('\n');

    out.push_str("Environment variables:\n");
    out.push_str("  ZAD_HOME_OVERRIDE   Override $HOME when resolving ~/.zad (tests only).\n");
    out.push_str("  ZAD_SECRETS_MEMORY  `1` = in-memory keychain backend (tests only).\n");
    out.push_str(
        "  RUST_LOG            Standard tracing filter, e.g. `zad=debug`. Overrides --debug.\n",
    );
    out.push('\n');

    out.push_str("Verbose output:\n");
    out.push_str(
        "  --debug             Emit debug-level logs to stderr in addition to the file log.\n",
    );
    out.push('\n');

    out.push_str("Diagnostic commands:\n");
    out.push_str("  zad --help-agent              Compact prompt-injectable CLI summary.\n");
    out.push_str("  zad commands                  Enumerate every command.\n");
    out.push_str("  zad commands <name>           Flags and exit codes for one command.\n");
    out.push_str("  zad man [command]             Reference manpages embedded at build time.\n");
    out.push_str("  zad docs [topic]              Topic docs embedded at build time.\n");
    out.push_str("  zad service list              Configured services for this project.\n");
    out.push_str(
        "  zad <svc> permissions show    Effective permissions for a service (global ∩ local).\n",
    );
    out.push_str(
        "  zad <svc> permissions path    Where the TOML files that drive permissions live.\n",
    );
    out.push('\n');

    out.push_str("Build metadata:\n");
    let _ = writeln!(out, "  version       {version}");
    let _ = writeln!(
        out,
        "  target        {}",
        option_env!("TARGET").unwrap_or(std::env::consts::ARCH)
    );
    let _ = writeln!(out, "  profile       {}", profile());
    out.push('\n');

    out
}

const fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}
