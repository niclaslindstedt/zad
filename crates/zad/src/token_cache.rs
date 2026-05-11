//! Cross-process access-token cache and refresh lock.
//!
//! ## Why this exists
//!
//! OAuth providers that use PKCE rotation (notably Spotify) issue a
//! fresh `refresh_token` on every `/api/token` call and immediately
//! revoke the previous one after a short grace window. When a fan-out
//! spawns several `zad` processes simultaneously, each process
//! cold-starts with the same refresh token from the keychain and races
//! to POST `/api/token`. The first winner rotates the token; every
//! subsequent caller sends the now-revoked token and gets
//! `invalid_grant`.
//!
//! The OS keychain also prompts for each new process that accesses it.
//! With four parallel processes that is four simultaneous prompts for
//! a single logical operation.
//!
//! This module eliminates both problems:
//!
//! - **Token cache** (`<service_dir>/access_token.json`): the first
//!   process to acquire the lock does the refresh and writes the
//!   resulting access token here. Every other process checks the cache
//!   before touching the keychain or the token endpoint — if the token
//!   is still valid (≥ 30 s remaining), they return it directly with
//!   zero keychain or network activity.
//!
//! - **Refresh lock** (`<service_dir>/refresh.lock.d`): an advisory
//!   cross-process lock implemented via atomic [`std::fs::create_dir`].
//!   At most one process performs the refresh at a time. Waiters
//!   re-check the cache after the lock is released and usually return
//!   the cached token without issuing a second network call.
//!
//! All functions accept `Option<&Path>`. When the caller passes `None`
//! (e.g. the home directory is unavailable or a test has disabled
//! caching), every operation is a no-op and the caller falls back to
//! the previous per-process-only behaviour. The `service_dir` helper
//! resolves the canonical `~/.zad/state/<service>` path from
//! [`crate::config::path::zad_home`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::config::path::zad_home;
use crate::error::{Result, ZadError};

/// How many seconds before expiry to treat a cached token as stale.
const EXPIRY_BUFFER_SECS: u64 = 30;

/// Fallback lifetime when the provider omits `expires_in`.
pub const DEFAULT_EXPIRES_IN_SECS: u64 = 3_600;

/// Maximum time to wait for the refresh lock.
const LOCK_WAIT_MS: u64 = 10_000;

/// Poll interval while waiting for the refresh lock.
const LOCK_POLL_MS: u64 = 50;

/// Age at which an unremoved lock directory is considered abandoned.
const LOCK_STALE_SECS: u64 = 30;

#[derive(Debug, Serialize, Deserialize)]
struct CachedToken {
    access_token: String,
    /// Unix timestamp (seconds) at which the token expires.
    expires_at: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Canonical cache directory for `service`: `~/.zad/state/<service>`.
///
/// Returns an error only when the user's home directory cannot be
/// resolved. Callers that receive an error should treat caching as
/// unavailable and pass `None` to the other functions in this module.
pub fn service_dir(service: &str) -> Result<PathBuf> {
    Ok(zad_home()?.join("state").join(service))
}

fn cache_path(dir: &Path) -> PathBuf {
    dir.join("access_token.json")
}

fn lock_dir_path(dir: &Path) -> PathBuf {
    dir.join("refresh.lock.d")
}

/// Read a cached access token from `service_dir`. Returns `None` when
/// `service_dir` is `None`, when the cache is absent, corrupt, or
/// within [`EXPIRY_BUFFER_SECS`] of expiry (the file is
/// opportunistically removed in the last case).
pub fn read(service_dir: Option<&Path>) -> Option<String> {
    let dir = service_dir?;
    let path = cache_path(dir);
    let bytes = std::fs::read(&path).ok()?;
    let cached: CachedToken = serde_json::from_slice(&bytes).ok()?;
    if cached.expires_at <= now_secs() + EXPIRY_BUFFER_SECS {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(cached.access_token)
}

/// Atomically write a fresh access token to `service_dir`. Uses a
/// write-then-rename so a concurrent reader never sees a partial file.
/// Silently succeeds when `service_dir` is `None`.
pub fn write(service_dir: Option<&Path>, access_token: &str, expires_in_secs: u64) -> Result<()> {
    let Some(dir) = service_dir else {
        return Ok(());
    };
    std::fs::create_dir_all(dir).map_err(|e| ZadError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;
    let path = cache_path(dir);
    let cached = CachedToken {
        access_token: access_token.to_string(),
        expires_at: now_secs() + expires_in_secs,
    };
    let json = serde_json::to_vec(&cached).map_err(|e| ZadError::Io {
        path: path.clone(),
        source: std::io::Error::other(e),
    })?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &json).map_err(|e| ZadError::Io {
        path: tmp.clone(),
        source: e,
    })?;
    std::fs::rename(&tmp, &path).map_err(|e| ZadError::Io {
        path: path.clone(),
        source: e,
    })?;
    Ok(())
}

/// Remove the cached access token (e.g. after a 401 indicates the
/// token has been revoked). No-op when `service_dir` is `None`.
pub fn clear(service_dir: Option<&Path>) {
    let Some(dir) = service_dir else { return };
    let _ = std::fs::remove_file(cache_path(dir));
}

/// RAII guard that releases the cross-process refresh lock on drop.
pub struct RefreshLock {
    path: PathBuf,
}

impl Drop for RefreshLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

/// Acquire the cross-process refresh lock for `service_dir`.
///
/// Returns `Ok(None)` immediately when `service_dir` is `None` — the
/// caller proceeds without cross-process protection.
///
/// Uses atomic [`std::fs::create_dir`] as a POSIX advisory lock:
/// `create_dir` is atomic on local filesystems and fails with
/// `AlreadyExists` when another process holds the lock. A lock
/// directory older than [`LOCK_STALE_SECS`] is treated as abandoned
/// and forcibly removed so a crashed holder never blocks callers.
///
/// Returns an error only when the lock directory cannot be created
/// for a reason other than contention, or when [`LOCK_WAIT_MS`]
/// elapses without acquiring.
pub async fn acquire_lock(service_dir: Option<&Path>) -> Result<Option<RefreshLock>> {
    let Some(dir) = service_dir else {
        return Ok(None);
    };
    std::fs::create_dir_all(dir).map_err(|e| ZadError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;
    let lock_path = lock_dir_path(dir);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(LOCK_WAIT_MS);

    loop {
        match std::fs::create_dir(&lock_path) {
            Ok(()) => return Ok(Some(RefreshLock { path: lock_path })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Remove the lock directory if the holder crashed and
                // left it behind.
                if let Ok(meta) = std::fs::metadata(&lock_path) {
                    if let Ok(modified) = meta.modified() {
                        if modified.elapsed().unwrap_or_default()
                            > Duration::from_secs(LOCK_STALE_SECS)
                        {
                            let _ = std::fs::remove_dir(&lock_path);
                            continue;
                        }
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ZadError::Invalid(format!(
                        "timed out ({LOCK_WAIT_MS} ms) waiting for the token-refresh \
                         lock at {}; another process may be stuck",
                        lock_path.display()
                    )));
                }
                sleep(Duration::from_millis(LOCK_POLL_MS)).await;
            }
            Err(e) => {
                return Err(ZadError::Io {
                    path: lock_path,
                    source: e,
                });
            }
        }
    }
}
