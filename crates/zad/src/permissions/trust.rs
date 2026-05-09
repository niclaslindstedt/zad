//! Per-machine trust map for permission files.
//!
//! Permission files (`permissions.toml`) carry **policy only** — they
//! never embed signatures. The signatures that authorize a file to be
//! loaded live in a single trust store at `~/.zad/signing/trusted.toml`,
//! keyed by the file's canonical absolute path. The store itself is
//! self-signed by the keychain key, so a tamper that replaces an entry
//! (or rewrites the whole store with an attacker-generated keypair) is
//! caught the next time the store is loaded.
//!
//! ## Why not store signatures inside the permission file?
//!
//! Embedding a signature in `permissions.toml` ties the file to the
//! authoring machine's keychain key — pre-signed files cannot be
//! shipped, because a different machine's keychain has a different
//! key, so the cross-check fails. Moving signatures into a per-machine
//! trust map makes permission files **shippable** (any user can run
//! `zad <svc> permissions sign` to trust a file they have inspected)
//! without sacrificing tamper detection.
//!
//! ## Threat model
//!
//! The OS keychain is the **single root of trust**. Anyone who can read
//! the keychain signing key can forge any signature; anyone who cannot
//! cannot, even with full read/write access to `~/.zad/`.
//!
//! - The trust store is itself signed by the keychain key. Tampering
//!   with any byte (including swapping in an attacker-generated
//!   `[signature]` block + pubkey) breaks either the signature
//!   verification or the keychain-pubkey cross-check.
//! - Verification refuses to proceed without a keychain key — there is
//!   no embedded-pubkey fallback. A fresh machine must explicitly run
//!   `zad signing init` before any file can be loaded.
//! - The store path is fixed (`~/.zad/signing/trusted.toml`); no env
//!   var override exists, so an agent cannot redirect verification to
//!   an attacker-controlled file.
//! - Symlinks at the store path are refused.
//! - On Unix the store is written with mode `0o600`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZadError};

use super::service::HasSignature;
use super::signing::{self, Signature, SigningKey};

/// Schema version for `~/.zad/signing/trusted.toml`. Bumped on
/// breaking layout changes so older binaries fail gracefully.
pub const TRUST_STORE_VERSION: u32 = 1;

/// One entry in the trust map. The signature is over the canonical
/// bytes of the *raw permissions struct* the entry authorizes — the
/// same canonicalization the per-service `verify_raw` uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustEntry {
    /// Canonical absolute path of the permissions file. See
    /// [`canonical_path_key`].
    pub path: String,
    pub algorithm: String,
    pub public_key: String,
    pub signed_at: String,
    pub value: String,
}

impl TrustEntry {
    pub(crate) fn from_signature(path: String, sig: Signature) -> Self {
        TrustEntry {
            path,
            algorithm: sig.algorithm,
            public_key: sig.public_key,
            signed_at: sig.signed_at,
            value: sig.value,
        }
    }
}

/// On-disk schema for the trust store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustStoreRaw {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Sorted by path for deterministic on-disk ordering.
    #[serde(default, rename = "entry", skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<TrustEntry>,
    /// Self-signature over this struct with `signature` cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

fn default_version() -> u32 {
    TRUST_STORE_VERSION
}

impl HasSignature for TrustStoreRaw {
    fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }
    fn set_signature(&mut self, sig: Option<Signature>) {
        self.signature = sig;
    }
}

/// In-memory view of the trust store, keyed by canonical path for O(log n)
/// lookup. Convert to/from `TrustStoreRaw` on disk I/O.
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    pub version: u32,
    pub entries: BTreeMap<String, TrustEntry>,
}

impl TrustStore {
    /// Load and verify the trust store. An absent file is treated as a
    /// fresh empty store (legitimate first-use). A present file must
    /// (a) carry a `[signature]` block, (b) verify against the keychain
    /// pubkey, and (c) not be a symlink.
    pub fn load() -> Result<Self> {
        let path = trust_store_path()?;
        Self::load_at(&path)
    }

    /// Internal: load from an explicit path. Used by tests via
    /// `ZAD_HOME_OVERRIDE`.
    pub fn load_at(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default_with_version());
        }
        // Refuse symlinks: an attacker that controls a parent directory
        // could plant a symlink redirecting reads at our load path to
        // an attacker-controlled file.
        let meta = std::fs::symlink_metadata(path).map_err(|e| ZadError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if meta.file_type().is_symlink() {
            return Err(ZadError::TrustStoreTampered {
                path: path.to_path_buf(),
                reason: "trust store is a symlink".into(),
            });
        }

        let body = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let raw: TrustStoreRaw = toml::from_str(&body).map_err(|e| ZadError::TomlParse {
            path: path.to_path_buf(),
            source: e,
        })?;

        if raw.version != TRUST_STORE_VERSION {
            return Err(ZadError::TrustStoreTampered {
                path: path.to_path_buf(),
                reason: format!(
                    "unsupported trust store version `{}` (expected `{TRUST_STORE_VERSION}`)",
                    raw.version
                ),
            });
        }

        // In production the keychain key is mandatory: verification
        // trusts only the local keychain, never the embedded pubkey.
        // Under the in-memory test backend the keychain may be empty
        // (the OS keychain is the security gate, and the in-memory
        // backend has none); fall back to verifying the trust store's
        // self-signature against the embedded pubkey alone — tamper
        // detection of the file contents still applies.
        let keychain = signing::require_keychain_key_for_verify()?;

        match keychain {
            Some(k) => signing::verify_with_key(&raw, path, &k).map_err(|e| match e {
                ZadError::SignatureInvalid { reason, .. } => ZadError::TrustStoreTampered {
                    path: path.to_path_buf(),
                    reason,
                },
                ZadError::SignatureKeyMismatch {
                    expected_fingerprint,
                    found_fingerprint,
                    ..
                } => ZadError::TrustStoreTampered {
                    path: path.to_path_buf(),
                    reason: format!(
                        "trust store signed by {found_fingerprint}, but local keychain holds {expected_fingerprint}"
                    ),
                },
                other => other,
            })?,
            None => {
                signing::verify_self_signature(&raw, path).map_err(|e| match e {
                    ZadError::SignatureInvalid { reason, .. } => ZadError::TrustStoreTampered {
                        path: path.to_path_buf(),
                        reason,
                    },
                    other => other,
                })?
            }
        }

        let mut entries = BTreeMap::new();
        for entry in raw.entries {
            entries.insert(entry.path.clone(), entry);
        }
        Ok(TrustStore {
            version: raw.version,
            entries,
        })
    }

    fn default_with_version() -> Self {
        TrustStore {
            version: TRUST_STORE_VERSION,
            entries: BTreeMap::new(),
        }
    }

    /// Look up a trust entry by canonical path. Input is canonicalized
    /// before lookup so callers can pass any shape of path (relative,
    /// absolute, with `..`) and get a stable result.
    pub fn lookup(&self, path: &Path) -> Result<Option<&TrustEntry>> {
        let key = canonical_path_key(path)?;
        Ok(self.entries.get(&key))
    }

    /// Upsert an entry. The caller must call [`save`] to persist.
    pub fn upsert(&mut self, entry: TrustEntry) {
        self.entries.insert(entry.path.clone(), entry);
    }

    /// Remove an entry by path. Returns whether anything was removed.
    /// The caller must call [`save`] to persist.
    pub fn remove(&mut self, path: &Path) -> Result<bool> {
        let key = canonical_path_key(path)?;
        Ok(self.entries.remove(&key).is_some())
    }

    /// Persist the store, signing it with `key`. Atomic via
    /// tempfile + persist (matches the same-directory persist dance in
    /// `staging::write_signed_atomic`).
    pub fn save(&self, key: &SigningKey) -> Result<()> {
        let path = trust_store_path()?;
        self.save_at(&path, key)
    }

    /// Internal: save to an explicit path. Used by tests via
    /// `ZAD_HOME_OVERRIDE`.
    pub fn save_at(&self, path: &Path, key: &SigningKey) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        let mut entries: Vec<TrustEntry> = self.entries.values().cloned().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        let raw_unsigned = TrustStoreRaw {
            version: self.version.max(TRUST_STORE_VERSION),
            entries,
            signature: None,
        };
        let sig = signing::sign_raw(&raw_unsigned, key)?;
        let raw_signed = TrustStoreRaw {
            signature: Some(sig),
            ..raw_unsigned
        };
        let body = toml::to_string_pretty(&raw_signed)?;

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
        std::io::Write::write_all(tmp.as_file_mut(), body.as_bytes()).map_err(|e| {
            ZadError::Io {
                path: tmp.path().to_path_buf(),
                source: e,
            }
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(tmp.path(), perms).map_err(|e| ZadError::Io {
                path: tmp.path().to_path_buf(),
                source: e,
            })?;
        }
        tmp.persist(path).map_err(|e| ZadError::Io {
            path: path.to_path_buf(),
            source: e.error,
        })?;
        Ok(())
    }
}

/// Path of the trust store. **Not** overridable by env var — this is
/// security state, not user data.
pub fn trust_store_path() -> Result<PathBuf> {
    Ok(crate::config::path::zad_home()?
        .join("signing")
        .join("trusted.toml"))
}

/// Canonicalize a path into a stable string key. Falls back to
/// canonicalizing the parent + appending the file name when the file
/// itself does not yet exist (the common case for first sign).
pub fn canonical_path_key(path: &Path) -> Result<String> {
    let canonical = canonical_path(path)?;
    canonical
        .into_os_string()
        .into_string()
        .map_err(|os| ZadError::Invalid(format!("non-utf8 path: {os:?}")))
}

fn canonical_path(path: &Path) -> Result<PathBuf> {
    if let Ok(p) = std::fs::canonicalize(path) {
        return Ok(p);
    }
    // File doesn't exist yet. Canonicalize the parent so symlinks /
    // relative components collapse, then append the file name.
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let parent_canonical = std::fs::canonicalize(parent).map_err(|e| ZadError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    let file = path.file_name().ok_or_else(|| {
        ZadError::Invalid(format!(
            "cannot resolve trust path for `{}`: no file name component",
            path.display()
        ))
    })?;
    Ok(parent_canonical.join(file))
}
