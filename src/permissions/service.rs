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

use super::mutation::Mutation;
use super::signing::Signature;

/// Companion trait used by raw structs that embed their own
/// `[signature]` block. Today this is only the trust store
/// ([`crate::permissions::trust::TrustStoreRaw`]) — service permission
/// files no longer carry signatures inline.
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

    /// Raw TOML schema. The shared signer signs over the canonical
    /// serialization of this struct and stores the signature in the
    /// per-machine trust store.
    type Raw: Serialize + DeserializeOwned + Default + Clone + PartialEq + std::fmt::Debug;

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

    /// Apply a typed [`Mutation`] to `raw`. Each service matches on the
    /// mutation variant and dispatches to the appropriate field of its
    /// `*Raw` struct. Unsupported mutations return
    /// [`ZadError::Invalid`] naming the service and the rejected
    /// mutation so the operator knows which axis is missing.
    fn apply_mutation(raw: &mut Self::Raw, mutation: &Mutation) -> Result<()>;
}

/// Global path helper generic over `S`.
pub fn global_path<S: PermissionsService>() -> Result<PathBuf> {
    Ok(config::path::global_service_dir(S::NAME)?.join("permissions.toml"))
}

/// Local path helper generic over `S`, resolved against the current
/// project slug.
///
/// Honors `ZAD_PERMISSIONS_PATH` / `ZAD_PERMISSIONS_ROOT` so the
/// operator can pin a "local" permissions file outside the cwd-derived
/// project tree.
pub fn local_path_current<S: PermissionsService>() -> Result<PathBuf> {
    if let Some(p) = config::path::permissions_local_override(S::NAME)? {
        return Ok(p);
    }
    let slug = config::path::project_slug()?;
    local_path_for::<S>(&slug)
}

/// Local path helper generic over `S`, resolved against an explicit
/// slug (used by tests that set up a throwaway project).
pub fn local_path_for<S: PermissionsService>(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, S::NAME)?.join("permissions.toml"))
}
