//! Staged-commit workflow for permission files.
//!
//! An agent (or a human) proposes edits by running a mutator
//! subcommand. Each mutation writes to a `<path>.pending` file next
//! to the live policy — **unsigned**, so no keychain prompt happens.
//! The human reviews `diff`, then runs `commit` to sign and atomically
//! replace the live file with the pending contents. Discard throws
//! the pending file away.
//!
//! The machinery is generic over [`PermissionsService`]: adding a new
//! service doesn't require any changes here. Each service contributes
//! only its `*Raw` schema and its `apply_mutation` dispatcher.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ZadError};

use super::mutation::Mutation;
use super::service::{HasSignature, PermissionsService};
use super::signing::{self, SigningKey};

/// Two-file snapshot of a live/pending pair at a given scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingStatus {
    pub live_exists: bool,
    pub pending_exists: bool,
}

/// Path of the pending file that sits next to the live file.
///
/// Example: `permissions.toml` → `permissions.toml.pending`.
pub fn pending_path_for(live: &Path) -> PathBuf {
    let mut s = live.as_os_str().to_os_string();
    s.push(".pending");
    PathBuf::from(s)
}

/// Inspect the live+pending pair without touching either.
pub fn status(live: &Path) -> StagingStatus {
    let pending = pending_path_for(live);
    StagingStatus {
        live_exists: live.exists(),
        pending_exists: pending.exists(),
    }
}

/// Read the pending body (if any) and compute a unified diff against
/// the live body. Returns `Ok(None)` if there is no pending file.
pub fn diff(live: &Path) -> Result<Option<String>> {
    let pending = pending_path_for(live);
    if !pending.exists() {
        return Ok(None);
    }
    let live_body = if live.exists() {
        std::fs::read_to_string(live).map_err(|e| ZadError::Io {
            path: live.to_path_buf(),
            source: e,
        })?
    } else {
        String::new()
    };
    let pending_body = std::fs::read_to_string(&pending).map_err(|e| ZadError::Io {
        path: pending.clone(),
        source: e,
    })?;
    Ok(Some(unified_diff(&live_body, &pending_body)))
}

/// Delete the pending file, if present. Returns whether it existed.
pub fn discard(live: &Path) -> Result<bool> {
    let pending = pending_path_for(live);
    if !pending.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&pending).map_err(|e| ZadError::Io {
        path: pending,
        source: e,
    })?;
    Ok(true)
}

/// Apply a typed mutation to the pending policy, creating a pending
/// file from the live contents if one didn't already exist. The
/// result is always an **unsigned** pending file — signing happens
/// only at `commit` time.
pub fn mutate_pending<S>(live: &Path, mutation: &Mutation) -> Result<()>
where
    S: PermissionsService,
{
    let pending = pending_path_for(live);
    let mut raw: S::Raw = if pending.exists() {
        read_raw::<S>(&pending)?
    } else if live.exists() {
        read_raw::<S>(live)?
    } else {
        // No policy yet at this scope — start from the starter template.
        S::starter_template()
    };
    raw.set_signature(None);
    S::apply_mutation(&mut raw, mutation)?;
    write_unsigned::<S>(&pending, &raw)
}

/// Sign the pending policy with `key` and atomically replace the live
/// file. Removes the pending file on success.
pub fn commit<S>(live: &Path, key: &SigningKey) -> Result<()>
where
    S: PermissionsService,
{
    let pending = pending_path_for(live);
    if !pending.exists() {
        return Err(ZadError::Invalid(format!(
            "no pending changes at {}",
            pending.display()
        )));
    }
    let raw = read_raw::<S>(&pending)?;
    write_signed_atomic::<S>(live, &raw, key)?;
    std::fs::remove_file(&pending).map_err(|e| ZadError::Io {
        path: pending,
        source: e,
    })?;
    Ok(())
}

/// Re-sign the live file in place. Intended for the `sign` escape
/// hatch after a hand edit (the file parses but the signature is
/// stale).
pub fn sign_in_place<S>(live: &Path, key: &SigningKey) -> Result<()>
where
    S: PermissionsService,
{
    if !live.exists() {
        return Err(ZadError::Invalid(format!(
            "no permissions file at {}",
            live.display()
        )));
    }
    let raw = read_raw::<S>(live)?;
    write_signed_atomic::<S>(live, &raw, key)
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

fn read_raw<S: PermissionsService>(path: &Path) -> Result<S::Raw> {
    let body = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    toml::from_str(&body).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })
}

fn write_unsigned<S: PermissionsService>(path: &Path, raw: &S::Raw) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let body = serialize_canonical(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

fn write_signed_atomic<S: PermissionsService>(
    live: &Path,
    raw: &S::Raw,
    key: &SigningKey,
) -> Result<()> {
    if let Some(parent) = live.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let sig = signing::sign_raw(&to_write, key)?;
    to_write.set_signature(Some(sig));
    let body = serialize_canonical(&to_write)?;

    // Atomic replace via a same-directory tempfile + persist. Using
    // tempfile::persist handles the Windows MoveFileEx semantics
    // correctly (std::fs::rename refuses to overwrite on Windows).
    let parent = live.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| ZadError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    std::io::Write::write_all(tmp.as_file_mut(), body.as_bytes()).map_err(|e| ZadError::Io {
        path: tmp.path().to_path_buf(),
        source: e,
    })?;
    tmp.persist(live).map_err(|e| ZadError::Io {
        path: live.to_path_buf(),
        source: e.error,
    })?;
    Ok(())
}

fn serialize_canonical<T: Serialize>(raw: &T) -> Result<String> {
    Ok(toml::to_string_pretty(raw)?)
}

// ---------------------------------------------------------------------------
// unified diff (tiny, stdlib-only — we don't need to pull a diff crate
// for a CLI preview)
// ---------------------------------------------------------------------------

fn unified_diff(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut out = String::new();
    out.push_str("--- live\n");
    out.push_str("+++ pending\n");

    // Naive prefix/suffix match + emit the middle as a single hunk.
    // This produces a readable diff for small edits without pulling in
    // an LCS crate. Permission files are small (~100 lines); this is
    // fine.
    let prefix = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix_limit = old_lines.len().min(new_lines.len()).saturating_sub(prefix);
    let suffix = old_lines
        .iter()
        .rev()
        .zip(new_lines.iter().rev())
        .take(suffix_limit)
        .take_while(|(a, b)| a == b)
        .count();

    let old_start = prefix;
    let old_end = old_lines.len().saturating_sub(suffix);
    let new_start = prefix;
    let new_end = new_lines.len().saturating_sub(suffix);
    let ctx_before = prefix.saturating_sub(3);
    let ctx_after = (old_lines.len() - suffix)
        .min(old_lines.len())
        .min(old_end + 3);

    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        ctx_before + 1,
        (ctx_after - ctx_before).max(1),
        new_start.saturating_sub(3) + 1,
        new_end + 3 - new_start.saturating_sub(3),
    ));

    for line in &old_lines[ctx_before..old_start] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    for line in &old_lines[old_start..old_end] {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &new_lines[new_start..new_end] {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    for line in old_lines.get(old_end..ctx_after).unwrap_or(&[]) {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    out
}
