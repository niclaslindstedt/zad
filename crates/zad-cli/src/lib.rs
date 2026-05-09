//! zad-cli — command-line interface for the [`zad`] library.
//!
//! The binary entry point ([`main`](../zad/index.html)) parses CLI
//! arguments and dispatches into [`cli::run`], which then drives the
//! library to do the actual work. Everything here is CLI-bound:
//! clap argument structs, interactive prompts via `dialoguer`,
//! stderr/stdout formatting, and the dry-run echo machinery.
//!
//! Rust projects that want to embed zad's functionality should depend
//! on the `zad` library directly, not on this crate.

#![allow(clippy::result_large_err)]

pub mod cli;
pub mod output;
