//! Canonical list of services built into this binary.
//!
//! Each entry here is a promise that:
//!
//! 1. There is a module under `src/service/<name>/` with the runtime
//!    implementation (may be a stub).
//! 2. There is a type implementing
//!    [`crate::cli::lifecycle::LifecycleService`] and wired into the
//!    dispatch match in `src/cli/service.rs` — that's what makes
//!    `zad service {create,enable,disable,show,delete} <name>` work.
//! 3. `ProjectConfig` (in `src/config/schema.rs`) has getters and
//!    enable/disable helpers for the service so local config can
//!    opt it in.
//!
//! `zad service list` walks this list to decide which rows to print.
//! Any tool that wants to iterate "every service this build knows
//! about" should read from here rather than hard-coding a list.
//!
//! Adding an entry is one line; see
//! `docs/services.md#adding-a-new-service` for the full checklist.

pub const SERVICES: &[&str] = &["1pass", "discord", "gcal", "github", "telegram"];
