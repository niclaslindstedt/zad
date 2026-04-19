//! Ed25519 signatures over permission files.
//!
//! Signatures make permission files **tamper-evident**: every write
//! embeds a signature over the canonical TOML of the policy, and every
//! read verifies it before compiling the rules. An agent or stray
//! process that modifies a file on disk without re-signing will be
//! caught on the next load — and producing a valid signature requires
//! access to the signing key in the OS keychain.
//!
//! ## Canonicalization
//!
//! Signing operates on the `toml::to_string_pretty` serialization of
//! the raw struct with its `signature` field cleared — **not** the raw
//! bytes on disk. This insulates us from whitespace-reflow by editors
//! while still rejecting any semantic change to the policy.
//!
//! ## Trust model
//!
//! The signing public key is embedded in each file. When the local
//! keychain also holds a signing key, its public key is
//! **authoritative**: a file whose embedded pubkey disagrees with the
//! keychain fails closed. Without a keychain entry (agent-only / fresh
//! checkout) the embedded pubkey is trusted for verification — safe,
//! because without the private key nobody can forge a new signature.
//!
//! ## Crypto choice
//!
//! Ed25519 via `ed25519-dalek` v2: small keys, pure Rust,
//! deterministic signatures, widely audited.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signer as _, Verifier as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, ZadError};
use crate::secrets;

use super::service::HasSignature;

/// Algorithm identifier embedded in every `Signature`. Reserved name
/// so older zad versions produce readable errors when a newer scheme
/// lands.
pub const ALGORITHM: &str = "ed25519";

/// Keychain account name for the signing keypair. Versioned so a future
/// rotation command can migrate users off `"signing:v1"` without
/// orphaning stored keys.
pub const SIGNING_ACCOUNT: &str = "signing:v1";

/// Top-level `[signature]` block embedded in every permissions file.
///
/// All fields are `String` — `toml` serializes strings reliably across
/// crate versions and the format is human-readable when the user opens
/// the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature {
    /// Always `"ed25519"` today.
    pub algorithm: String,
    /// Base64-encoded 32-byte Ed25519 public key.
    pub public_key: String,
    /// RFC 3339 timestamp (UTC) recorded at signing time. Advisory
    /// only — signature validity is independent of freshness.
    pub signed_at: String,
    /// Base64-encoded 64-byte Ed25519 signature over the canonical
    /// serialization of the enclosing raw struct with `signature`
    /// cleared.
    pub value: String,
}

/// Signing keypair wrapper. Wraps `ed25519_dalek::SigningKey` with the
/// base64 encode/decode helpers the keychain layer needs.
#[derive(Clone)]
pub struct SigningKey {
    inner: ed25519_dalek::SigningKey,
}

impl SigningKey {
    /// Generate a fresh keypair using OS randomness.
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        let inner = ed25519_dalek::SigningKey::generate(&mut rng);
        SigningKey { inner }
    }

    /// Base64-encoded public key suitable for the `signature.public_key`
    /// field or the `~/.zad/signing/public_key.toml` cache.
    pub fn public_key_b64(&self) -> String {
        B64.encode(self.inner.verifying_key().to_bytes())
    }

    /// Short fingerprint for user-facing output (first 8 hex chars of
    /// SHA-256(public_key_bytes)). Not a security primitive — just a
    /// readable handle.
    pub fn fingerprint(&self) -> String {
        fingerprint_of_pubkey_bytes(&self.inner.verifying_key().to_bytes())
    }

    /// Base64 encoding of the 32-byte secret scalar. Used only to
    /// shuttle the key through the string-only keychain API; never
    /// displayed to the user.
    pub fn to_keychain_encoded(&self) -> String {
        B64.encode(self.inner.to_bytes())
    }

    /// Inverse of [`to_keychain_encoded`].
    pub fn from_keychain_encoded(encoded: &str) -> Result<Self> {
        let bytes = B64.decode(encoded).map_err(|e| {
            ZadError::Invalid(format!("keychain signing key is not valid base64: {e}"))
        })?;
        let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            ZadError::Invalid(format!(
                "keychain signing key is {} bytes, expected 32",
                bytes.len()
            ))
        })?;
        Ok(SigningKey {
            inner: ed25519_dalek::SigningKey::from_bytes(&arr),
        })
    }

    fn sign_bytes(&self, payload: &[u8]) -> ed25519_dalek::Signature {
        self.inner.sign(payload)
    }
}

fn fingerprint_of_pubkey_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    hex
}

/// Short fingerprint for an arbitrary base64-encoded public key. Used
/// in `SignatureKeyMismatch` error messages.
pub fn fingerprint_of_pubkey_b64(pubkey_b64: &str) -> String {
    match B64.decode(pubkey_b64) {
        Ok(bytes) => fingerprint_of_pubkey_bytes(&bytes),
        Err(_) => "<invalid>".to_string(),
    }
}

// ---------------------------------------------------------------------------
// keychain I/O
// ---------------------------------------------------------------------------

/// Load the signing key from the OS keychain, generating a fresh one
/// if none exists yet. The first call on a given machine is the TOFU
/// moment — the key's fingerprint is what subsequent signed files
/// will pin to.
pub fn load_or_create_from_keychain() -> Result<SigningKey> {
    if let Some(key) = load_from_keychain()? {
        return Ok(key);
    }
    let fresh = SigningKey::generate();
    secrets::store(SIGNING_ACCOUNT, &fresh.to_keychain_encoded())?;
    Ok(fresh)
}

/// Load the signing key from the OS keychain. Returns `Ok(None)` if
/// no key has been created yet — callers that only verify (and never
/// sign) can continue without prompting the keychain further.
pub fn load_from_keychain() -> Result<Option<SigningKey>> {
    match secrets::load(SIGNING_ACCOUNT)? {
        Some(encoded) => Ok(Some(SigningKey::from_keychain_encoded(&encoded)?)),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// canonicalization + sign/verify
// ---------------------------------------------------------------------------

/// Serialize `raw` to canonical TOML with its signature stripped. The
/// returned bytes are what gets signed.
fn canonical_bytes<T>(raw: &T) -> Result<Vec<u8>>
where
    T: Serialize + HasSignature + Clone,
{
    let mut stripped = raw.clone();
    stripped.set_signature(None);
    let s = toml::to_string_pretty(&stripped)?;
    Ok(s.into_bytes())
}

/// Sign a raw struct. The returned `Signature` is what callers embed
/// via `set_signature(Some(sig))` before writing the file to disk.
pub fn sign_raw<T>(raw: &T, key: &SigningKey) -> Result<Signature>
where
    T: Serialize + HasSignature + Clone,
{
    let bytes = canonical_bytes(raw)?;
    let sig = key.sign_bytes(&bytes);
    let now = jiff::Timestamp::now();
    Ok(Signature {
        algorithm: ALGORITHM.to_string(),
        public_key: key.public_key_b64(),
        signed_at: now.to_string(),
        value: B64.encode(sig.to_bytes()),
    })
}

/// Verify a raw struct carries a valid signature, enforcing the
/// keychain-authoritative trust policy. Returns `Err` with one of
/// [`ZadError::SignatureMissing`], [`ZadError::SignatureInvalid`], or
/// [`ZadError::SignatureKeyMismatch`] if verification fails.
pub fn verify_raw<T>(raw: &T, path: &Path) -> Result<()>
where
    T: Serialize + HasSignature + Clone,
{
    let sig = raw.signature().ok_or_else(|| ZadError::SignatureMissing {
        path: path.to_path_buf(),
    })?;

    if sig.algorithm != ALGORITHM {
        return Err(ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!(
                "unsupported algorithm `{}` (expected `{ALGORITHM}`)",
                sig.algorithm
            ),
        });
    }

    let pubkey_bytes = B64
        .decode(&sig.public_key)
        .map_err(|e| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("public_key is not valid base64: {e}"),
        })?;
    let pubkey_arr: [u8; 32] =
        pubkey_bytes
            .as_slice()
            .try_into()
            .map_err(|_| ZadError::SignatureInvalid {
                path: path.to_path_buf(),
                reason: format!("public_key is {} bytes, expected 32", pubkey_bytes.len()),
            })?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pubkey_arr).map_err(|e| {
        ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("public_key is not a valid Ed25519 point: {e}"),
        }
    })?;

    let sig_bytes = B64
        .decode(&sig.value)
        .map_err(|e| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("value is not valid base64: {e}"),
        })?;
    let sig_arr: [u8; 64] =
        sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| ZadError::SignatureInvalid {
                path: path.to_path_buf(),
                reason: format!("signature value is {} bytes, expected 64", sig_bytes.len()),
            })?;
    let dalek_sig = ed25519_dalek::Signature::from_bytes(&sig_arr);

    let payload = canonical_bytes(raw)?;
    verifying_key
        .verify(&payload, &dalek_sig)
        .map_err(|_| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: "payload does not match signature (file was modified after signing)".into(),
        })?;

    // Keychain-authoritative cross-check: if the local keychain has a
    // signing key, its public key must match the one embedded in the
    // file. This is what prevents an attacker from swapping *both* the
    // body and the embedded pubkey.
    if let Some(local) = load_from_keychain()? {
        let local_pub = local.public_key_b64();
        if local_pub != sig.public_key {
            return Err(ZadError::SignatureKeyMismatch {
                path: path.to_path_buf(),
                expected_fingerprint: fingerprint_of_pubkey_b64(&local_pub),
                found_fingerprint: fingerprint_of_pubkey_b64(&sig.public_key),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// public-key cache
// ---------------------------------------------------------------------------

/// Path of the public-key cache that lets agents verify permission
/// files without touching the keychain. Mirrors the pubkey stored
/// alongside the private key.
pub fn public_key_cache_path() -> Result<PathBuf> {
    Ok(crate::config::path::zad_home()?
        .join("signing")
        .join("public_key.toml"))
}

/// Write the public-key cache next to the signing key so later reads
/// on the same machine can cross-check without prompting the keychain.
pub fn write_public_key_cache(key: &SigningKey) -> Result<()> {
    let path = public_key_cache_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let body = format!(
        "# Auto-generated by zad. Do not edit by hand.\n\
         algorithm = \"{ALGORITHM}\"\n\
         public_key = \"{}\"\n\
         fingerprint = \"{}\"\n",
        key.public_key_b64(),
        key.fingerprint(),
    );
    std::fs::write(&path, body).map_err(|e| ZadError::Io { path, source: e })
}
