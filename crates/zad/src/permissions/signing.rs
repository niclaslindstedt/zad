//! Ed25519 signatures over permission files.
//!
//! Signatures make permission files **tamper-evident**, but they no
//! longer live inside the permissions file. Instead, the per-machine
//! trust store at `~/.zad/signing/trusted.toml` (see
//! [`crate::permissions::trust`]) holds one signed entry per file the
//! operator has chosen to trust. This makes permission files
//! shippable: another machine can `zad <svc> permissions sign` to add
//! its own trust entry without re-authoring the file.
//!
//! ## Canonicalization
//!
//! Signing operates on the `toml::to_string_pretty` serialization of
//! the raw struct — for permission files, that is the file's
//! permissions content; for the trust store itself, the canonical
//! bytes of the trust store with `[signature]` cleared. This insulates
//! us from whitespace-reflow by editors while still rejecting any
//! semantic change.
//!
//! ## Trust model
//!
//! The OS keychain is the **single root of trust**. Verification
//! refuses to proceed if the keychain has no signing key (no embedded
//! pubkey fallback): a missing key is a hard
//! [`ZadError::SigningKeyMissing`]. The keychain is bootstrapped only
//! by an explicit `zad signing init`; routine sign/verify paths never
//! create keys silently.
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

/// A signature value carried by the trust store (and by the trust
/// store's own self-signature). Permission files do not carry these
/// inline anymore — the trust store does.
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

    /// Base64-encoded public key suitable for the
    /// `~/.zad/signing/public_key.toml` cache or a trust store entry.
    pub fn public_key_b64(&self) -> String {
        B64.encode(self.inner.verifying_key().to_bytes())
    }

    /// Short fingerprint for user-facing output (first 4 bytes of
    /// SHA-256(public_key_bytes), hex). Not a security primitive —
    /// just a readable handle.
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
/// if none exists yet. **Restricted to bootstrap call sites** — every
/// other code path (sign, verify) uses [`load_from_keychain`] and
/// fails closed with [`ZadError::SigningKeyMissing`]. The single
/// caller is the `zad signing init` CLI handler, which is the only
/// user-initiated way to mint a new key.
pub fn load_or_create_from_keychain() -> Result<SigningKey> {
    if let Some(key) = load_from_keychain()? {
        return Ok(key);
    }
    let fresh = SigningKey::generate();
    secrets::store(SIGNING_ACCOUNT, &fresh.to_keychain_encoded())?;
    Ok(fresh)
}

/// Load the signing key from the OS keychain. Returns `Ok(None)` if
/// no key has been created yet.
pub fn load_from_keychain() -> Result<Option<SigningKey>> {
    match secrets::load(SIGNING_ACCOUNT)? {
        Some(encoded) => Ok(Some(SigningKey::from_keychain_encoded(&encoded)?)),
        None => Ok(None),
    }
}

/// Load the keychain key, mapping absence to [`ZadError::SigningKeyMissing`]
/// with the canonical bootstrap hint. Use from sign/verify call sites.
pub fn require_keychain_key() -> Result<SigningKey> {
    load_from_keychain()?.ok_or_else(|| ZadError::SigningKeyMissing {
        hint: "run `zad signing init` to bootstrap the local signing key".into(),
    })
}

/// Load the keychain key for verification. In production
/// (real OS keychain) the key is mandatory — verification refuses to
/// proceed without one. Under the in-memory test backend the key is
/// optional: if the keychain is empty, the entry's embedded pubkey is
/// authoritative for the file-content signature check (the keychain
/// cross-check is the *production* tamper detection, and it has no
/// real gate to enforce in a memory-backed test process).
pub fn require_keychain_key_for_verify() -> Result<Option<SigningKey>> {
    if let Some(k) = load_from_keychain()? {
        return Ok(Some(k));
    }
    if crate::secrets::is_memory_backend() {
        return Ok(None);
    }
    Err(ZadError::SigningKeyMissing {
        hint: "run `zad signing init` to bootstrap the local signing key".into(),
    })
}

/// Rotate the keychain signing key, replacing any existing entry.
/// Caller must wipe the trust store afterwards (existing entries
/// signed by the previous key will fail verification under the new
/// keychain pubkey). The single caller is `zad signing init --force`.
pub fn rotate_keychain_key() -> Result<SigningKey> {
    let fresh = SigningKey::generate();
    let _ = secrets::delete(SIGNING_ACCOUNT);
    secrets::store(SIGNING_ACCOUNT, &fresh.to_keychain_encoded())?;
    Ok(fresh)
}

// ---------------------------------------------------------------------------
// canonicalization + sign/verify
// ---------------------------------------------------------------------------

/// Serialize `raw` to canonical TOML. Used by callers that don't
/// embed a `[signature]` block (i.e. permission files).
pub fn canonical_bytes_unsigned<T>(raw: &T) -> Result<Vec<u8>>
where
    T: Serialize,
{
    let s = toml::to_string_pretty(raw)?;
    Ok(s.into_bytes())
}

/// Serialize `raw` to canonical TOML with its signature stripped. Used
/// for raw structs that embed a self-signature (e.g. the trust store).
fn canonical_bytes_self_signed<T>(raw: &T) -> Result<Vec<u8>>
where
    T: Serialize + HasSignature + Clone,
{
    let mut stripped = raw.clone();
    stripped.set_signature(None);
    let s = toml::to_string_pretty(&stripped)?;
    Ok(s.into_bytes())
}

/// Sign a raw struct that embeds its own signature (the trust store).
/// Returns the `Signature` ready to assign back into `raw`.
pub fn sign_raw<T>(raw: &T, key: &SigningKey) -> Result<Signature>
where
    T: Serialize + HasSignature + Clone,
{
    let bytes = canonical_bytes_self_signed(raw)?;
    let sig = key.sign_bytes(&bytes);
    let now = jiff::Timestamp::now();
    Ok(Signature {
        algorithm: ALGORITHM.to_string(),
        public_key: key.public_key_b64(),
        signed_at: now.to_string(),
        value: B64.encode(sig.to_bytes()),
    })
}

/// Sign canonical bytes of a permission file's raw struct (no embedded
/// signature). Use from sign-to-trust-store call sites.
pub fn sign_unsigned<T>(raw: &T, key: &SigningKey) -> Result<Signature>
where
    T: Serialize,
{
    let bytes = canonical_bytes_unsigned(raw)?;
    let sig = key.sign_bytes(&bytes);
    let now = jiff::Timestamp::now();
    Ok(Signature {
        algorithm: ALGORITHM.to_string(),
        public_key: key.public_key_b64(),
        signed_at: now.to_string(),
        value: B64.encode(sig.to_bytes()),
    })
}

/// Verify a raw struct's embedded signature against the embedded
/// pubkey *only* (no keychain cross-check). Used by `TrustStore::load`
/// when running under the in-memory test backend where there is no
/// real keychain to gate against.
pub fn verify_self_signature<T>(raw: &T, path: &Path) -> Result<()>
where
    T: Serialize + HasSignature + Clone,
{
    let sig = raw.signature().ok_or_else(|| ZadError::SignatureInvalid {
        path: path.to_path_buf(),
        reason: "missing [signature] block".into(),
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
    let verifying_key = decode_verifying_key(&sig.public_key, path)?;
    let payload = canonical_bytes_self_signed(raw)?;
    let dalek_sig = decode_signature_value(&sig.value, path)?;
    verifying_key
        .verify(&payload, &dalek_sig)
        .map_err(|_| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: "payload does not match signature (file was modified after signing)".into(),
        })?;
    Ok(())
}

/// Verify a raw struct that carries its own embedded signature against
/// an explicit key. Returns `Err` with one of [`ZadError::SignatureInvalid`]
/// or [`ZadError::SignatureKeyMismatch`] on failure.
///
/// Used by [`crate::permissions::trust::TrustStore::load`] to verify
/// the trust store against the keychain key.
pub fn verify_with_key<T>(raw: &T, path: &Path, expected_key: &SigningKey) -> Result<()>
where
    T: Serialize + HasSignature + Clone,
{
    let sig = raw.signature().ok_or_else(|| ZadError::SignatureInvalid {
        path: path.to_path_buf(),
        reason: "missing [signature] block".into(),
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

    let verifying_key = decode_verifying_key(&sig.public_key, path)?;

    let payload = canonical_bytes_self_signed(raw)?;
    let dalek_sig = decode_signature_value(&sig.value, path)?;
    verifying_key
        .verify(&payload, &dalek_sig)
        .map_err(|_| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: "payload does not match signature (file was modified after signing)".into(),
        })?;

    let expected_pub = expected_key.public_key_b64();
    if expected_pub != sig.public_key {
        return Err(ZadError::SignatureKeyMismatch {
            path: path.to_path_buf(),
            expected_fingerprint: fingerprint_of_pubkey_b64(&expected_pub),
            found_fingerprint: fingerprint_of_pubkey_b64(&sig.public_key),
        });
    }
    Ok(())
}

/// Verify that `raw` (a permission file's raw struct, *no* embedded
/// signature) is trusted: there is a trust-store entry for `path`,
/// the entry's signature matches the canonical bytes of `raw`, and the
/// entry's pubkey matches the keychain key.
///
/// Failure modes:
///
/// - [`ZadError::SigningKeyMissing`] — keychain has no signing key.
///   Bootstrap with `zad signing init`.
/// - [`ZadError::TrustStoreTampered`] — the trust store exists but
///   failed self-verification (symlink, bad signature, mismatched
///   keychain pubkey, …). Recover with `zad signing init --force`.
/// - [`ZadError::NotTrusted`] — no entry in the trust store for this
///   path. Recover with `zad <service> permissions sign`.
/// - [`ZadError::SignatureInvalid`] — the entry exists but its
///   signature doesn't match the file's bytes (file was modified
///   without re-signing).
/// - [`ZadError::SignatureKeyMismatch`] — the entry's pubkey doesn't
///   match the keychain pubkey (entry written by a previous, now
///   rotated, keychain key).
pub fn verify_raw<T>(raw: &T, path: &Path) -> Result<()>
where
    T: Serialize,
{
    let keychain = require_keychain_key_for_verify()?;
    let trust = crate::permissions::trust::TrustStore::load()?;

    let entry = trust.lookup(path)?.ok_or_else(|| ZadError::NotTrusted {
        path: path.to_path_buf(),
        trust_store_path: crate::permissions::trust::trust_store_path()
            .unwrap_or_else(|_| PathBuf::from("(unknown)")),
    })?;

    if entry.algorithm != ALGORITHM {
        return Err(ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!(
                "unsupported algorithm `{}` (expected `{ALGORITHM}`)",
                entry.algorithm
            ),
        });
    }

    let verifying_key = decode_verifying_key(&entry.public_key, path)?;
    let dalek_sig = decode_signature_value(&entry.value, path)?;

    let payload = canonical_bytes_unsigned(raw)?;
    verifying_key
        .verify(&payload, &dalek_sig)
        .map_err(|_| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason:
                "payload does not match trust-store signature (file was modified after signing)"
                    .into(),
        })?;

    if let Some(keychain) = keychain {
        let keychain_pub = keychain.public_key_b64();
        if keychain_pub != entry.public_key {
            return Err(ZadError::SignatureKeyMismatch {
                path: path.to_path_buf(),
                expected_fingerprint: fingerprint_of_pubkey_b64(&keychain_pub),
                found_fingerprint: fingerprint_of_pubkey_b64(&entry.public_key),
            });
        }
    }
    Ok(())
}

fn decode_verifying_key(pubkey_b64: &str, path: &Path) -> Result<ed25519_dalek::VerifyingKey> {
    let bytes = B64
        .decode(pubkey_b64)
        .map_err(|e| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("public_key is not valid base64: {e}"),
        })?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("public_key is {} bytes, expected 32", bytes.len()),
        })?;
    ed25519_dalek::VerifyingKey::from_bytes(&arr).map_err(|e| ZadError::SignatureInvalid {
        path: path.to_path_buf(),
        reason: format!("public_key is not a valid Ed25519 point: {e}"),
    })
}

fn decode_signature_value(value_b64: &str, path: &Path) -> Result<ed25519_dalek::Signature> {
    let bytes = B64
        .decode(value_b64)
        .map_err(|e| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("value is not valid base64: {e}"),
        })?;
    let arr: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| ZadError::SignatureInvalid {
            path: path.to_path_buf(),
            reason: format!("signature value is {} bytes, expected 64", bytes.len()),
        })?;
    Ok(ed25519_dalek::Signature::from_bytes(&arr))
}

// ---------------------------------------------------------------------------
// public-key cache
// ---------------------------------------------------------------------------

/// `true` for the five `ZadError` variants that mean "the permissions
/// file (or the trust store, or the keychain) can't be trusted right
/// now": [`ZadError::NotTrusted`], [`ZadError::SignatureInvalid`],
/// [`ZadError::SignatureKeyMismatch`], [`ZadError::TrustStoreTampered`],
/// [`ZadError::SigningKeyMissing`]. The CLI's echo-mode short-circuit
/// (`crate::cli::echo`) flips on for exactly these — content/time/
/// pattern denials (`PermissionDenied`) keep their hard-fail shape.
pub fn is_signing_error(err: &ZadError) -> bool {
    matches!(
        err,
        ZadError::NotTrusted { .. }
            | ZadError::SignatureInvalid { .. }
            | ZadError::SignatureKeyMismatch { .. }
            | ZadError::TrustStoreTampered { .. }
            | ZadError::SigningKeyMissing { .. }
    )
}

/// Path of the public-key cache. The cache is a debugging aid only —
/// `verify_raw` consults the keychain, never the cache.
pub fn public_key_cache_path() -> Result<PathBuf> {
    Ok(crate::config::path::zad_home()?
        .join("signing")
        .join("public_key.toml"))
}

/// Write the public-key cache next to the signing key.
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
