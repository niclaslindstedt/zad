//! Secret storage backed by the OS keychain.
//!
//! Tests can switch to an in-process store by setting
//! `ZAD_SECRETS_MEMORY=1` or calling [`use_memory_backend`] before
//! exercising the API. The memory backend keeps secrets in a process-
//! local `Mutex<HashMap>` and never touches the OS keychain.
//!
//! When `ZAD_HOME_OVERRIDE` is also set (the integration-test pattern),
//! the in-memory store is mirrored to `<home>/.test-secrets.json` on
//! every write and reloaded on every read. This makes the signing
//! key persist across `bin()` invocations within a single test, which
//! the trust-store flow depends on (the keychain is the single root
//! of trust, so every binary process must see the same key).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::error::{Result, ZadError};

const SERVICE: &str = "zad";

fn memory_store() -> &'static Mutex<HashMap<String, String>> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn memory_override() -> &'static OnceLock<bool> {
    static FLAG: OnceLock<bool> = OnceLock::new();
    &FLAG
}

/// Force the process to use the in-memory backend for the rest of its
/// lifetime. Intended for integration tests.
pub fn use_memory_backend() {
    let _ = memory_override().set(true);
}

fn is_memory() -> bool {
    memory_override().get().copied().unwrap_or(false)
        || std::env::var("ZAD_SECRETS_MEMORY")
            .map(|v| v == "1")
            .unwrap_or(false)
}

/// Public predicate: are we running with the in-memory secrets
/// backend? Used by the signing layer to relax the strict
/// "keychain key required" rule under tests — the OS keychain is
/// the security gate, and an in-process-only memory backend has no
/// gate to enforce.
pub fn is_memory_backend() -> bool {
    is_memory()
}

fn mem_key(account: &str) -> String {
    format!("{SERVICE}/{account}")
}

/// File mirror path for the memory backend. Returns `None` unless
/// `ZAD_HOME_OVERRIDE` is set (production never goes through this
/// path; the OS keychain is authoritative there).
fn memory_mirror_path() -> Option<PathBuf> {
    std::env::var("ZAD_HOME_OVERRIDE")
        .ok()
        .map(|h| PathBuf::from(h).join(".test-secrets.json"))
}

fn load_mirror_into(map: &mut HashMap<String, String>) {
    let Some(path) = memory_mirror_path() else {
        return;
    };
    if !path.exists() {
        return;
    }
    let Ok(body) = std::fs::read_to_string(&path) else {
        return;
    };
    if let Ok(parsed) = serde_json::from_str::<HashMap<String, String>>(&body) {
        for (k, v) in parsed {
            map.insert(k, v);
        }
    }
}

fn flush_mirror(map: &HashMap<String, String>) {
    let Some(path) = memory_mirror_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string(map) {
        let _ = std::fs::write(&path, body);
    }
}

pub fn store(account: &str, secret: &str) -> Result<()> {
    if is_memory() {
        let mut g = memory_store().lock().unwrap();
        load_mirror_into(&mut g);
        g.insert(mem_key(account), secret.to_string());
        flush_mirror(&g);
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, account)?;
    entry.set_password(secret)?;
    Ok(())
}

pub fn load(account: &str) -> Result<Option<String>> {
    if is_memory() {
        let mut g = memory_store().lock().unwrap();
        load_mirror_into(&mut g);
        return Ok(g.get(&mem_key(account)).cloned());
    }
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(ZadError::Keyring(e)),
    }
}

pub fn delete(account: &str) -> Result<()> {
    if is_memory() {
        let mut g = memory_store().lock().unwrap();
        load_mirror_into(&mut g);
        g.remove(&mem_key(account));
        flush_mirror(&g);
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(ZadError::Keyring(e)),
    }
}

/// Scope of a service credential: either shared across every project
/// (`Global`) or scoped to a single project slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope<'a> {
    Global,
    Project(&'a str),
}

impl<'a> Scope<'a> {
    pub fn suffix(&self) -> &'a str {
        match self {
            Scope::Global => "global",
            Scope::Project(slug) => slug,
        }
    }
}

/// Build the OS-keychain account name for a secret belonging to
/// `service`. `kind` names the specific piece of secret material —
/// `"bot"` for a single bot token (Discord, Telegram, Slack bots),
/// `"client-secret"` / `"refresh"` for OAuth services (Reddit,
/// Google), `"pem"` for keypair services (GitHub Apps), etc. The
/// resulting account string is user-visible (shown in `zad service
/// show` and in any future keychain UI), so treat it as a stable
/// identifier — renaming it would orphan every existing stored token.
pub fn account(service: &str, kind: &str, scope: Scope<'_>) -> String {
    format!("{service}-{kind}:{}", scope.suffix())
}
