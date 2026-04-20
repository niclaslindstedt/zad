//! GitHub service.
//!
//! Thin wrapper over the `gh` CLI. Runtime verbs shell out to `gh` with
//! the authenticated Personal Access Token supplied via the `GH_TOKEN`
//! environment variable; scope and permission checks happen locally
//! before the subprocess is spawned so a denied call never reaches the
//! network.
//!
//! Submodules:
//!
//! - `client` — subprocess wrapper around `gh` with typed error
//!   translation.
//! - `transport` — trait over the runtime verbs + live/dry-run impls.
//! - `permissions` — per-service policy composed from the shared
//!   `permissions/` primitives.
//!
//! Lifecycle (`zad service {create,enable,disable,show,delete}
//! github`) is wired via `src/cli/service_github.rs`; runtime verbs
//! (`zad github <verb>`) live in `src/cli/github.rs` and call through
//! the [`GithubTransport`] trait so `--dry-run` can swap in a preview
//! impl without touching the keychain.

pub mod client;
pub mod permissions;
pub mod transport;

pub use client::GhCli;
pub use transport::{DryRunGithubTransport, GithubTransport};
