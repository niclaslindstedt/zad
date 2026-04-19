//! The [`PermissionsService`] trait — the single per-service abstraction
//! that the shared CLI runner, signing, staging, and mutation engines are
//! generic over.
//!
//! Adding a new permissions-bearing service is a checklist:
//!
//! 1. Define a `*Raw` struct that (de)serializes the TOML policy and
//!    carries an `Option<Signature>` field.
//! 2. `impl HasSignature for MyRaw` (three lines).
//! 3. Declare a zero-sized type (e.g. `pub struct PermissionsService;`)
//!    and `impl PermissionsService for ...` over it.
//! 4. Delegate the service's CLI permissions subcommand to
//!    `cli::permissions::run::<MyService>(args)`.
//!
//! Everything else — `show`, `path`, `init`, `check`, and (in PR 2)
//! `commit` / `discard` / `diff` / `status` / `sign` / typed mutators —
//! lives in shared code.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config;
use crate::error::Result;

use super::signing::Signature;

/// Small companion trait so the generic signer can get/set the signature
/// without knowing the concrete raw type. Each service implements it in
/// three lines.
pub trait HasSignature {
    fn signature(&self) -> Option<&Signature>;
    fn set_signature(&mut self, sig: Option<Signature>);
}

/// The service-side bindings the shared permissions runner needs. Every
/// permissions-bearing service implements this trait exactly once, on a
/// zero-sized type that lives next to its `*Raw` struct.
pub trait PermissionsService: 'static {
    /// Stable service name used to compute paths under `~/.zad/` and
    /// to label error output. Must match the directory layout used by
    /// the rest of zad (Discord is `"discord"`, Telegram `"telegram"`,
    /// Google Calendar `"gcal"`, etc).
    const NAME: &'static str;

    /// Raw TOML schema. Carries an `Option<Signature>` so the generic
    /// signer can verify it on load and populate it on save.
    type Raw: Serialize
        + DeserializeOwned
        + HasSignature
        + Default
        + Clone
        + PartialEq
        + std::fmt::Debug;

    /// Starter policy emitted by `init` when no file exists at the
    /// chosen scope.
    fn starter_template() -> Self::Raw;

    /// Function names this service exposes (e.g. `&["send", "read",
    /// …]`). Used by the shared CLI to validate `--function`.
    fn all_functions() -> &'static [&'static str];

    /// Target kinds accepted by mutators (e.g. Discord: `&["channel",
    /// "user", "guild"]`, Telegram: `&["chat"]`, Gcal:
    /// `&["calendar", "event"]`). Used by the shared CLI to validate
    /// `--target`.
    fn target_kinds() -> &'static [&'static str];
}

/// Global path helper generic over `S`.
pub fn global_path<S: PermissionsService>() -> Result<PathBuf> {
    Ok(config::path::global_service_dir(S::NAME)?.join("permissions.toml"))
}

/// Local path helper generic over `S`, resolved against the current
/// project slug.
pub fn local_path_current<S: PermissionsService>() -> Result<PathBuf> {
    let slug = config::path::project_slug()?;
    local_path_for::<S>(&slug)
}

/// Local path helper generic over `S`, resolved against an explicit
/// slug (used by tests that set up a throwaway project).
pub fn local_path_for<S: PermissionsService>(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, S::NAME)?.join("permissions.toml"))
}
