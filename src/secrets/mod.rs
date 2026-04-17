//! Secret storage backed by the OS keychain.
//!
//! Tests can switch to an in-process store by setting
//! `ZAD_SECRETS_MEMORY=1` or calling [`use_memory_backend`] before
//! exercising the API. The memory backend keeps secrets in a process-
//! local `Mutex<HashMap>` and never touches the OS keychain.

use std::collections::HashMap;
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

fn mem_key(account: &str) -> String {
    format!("{SERVICE}/{account}")
}

pub fn store(account: &str, secret: &str) -> Result<()> {
    if is_memory() {
        memory_store()
            .lock()
            .unwrap()
            .insert(mem_key(account), secret.to_string());
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, account)?;
    entry.set_password(secret)?;
    Ok(())
}

pub fn load(account: &str) -> Result<Option<String>> {
    if is_memory() {
        return Ok(memory_store()
            .lock()
            .unwrap()
            .get(&mem_key(account))
            .cloned());
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
        memory_store().lock().unwrap().remove(&mem_key(account));
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(ZadError::Keyring(e)),
    }
}

/// Scope of an adapter credential: either shared across every project
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

/// Account key for a Discord bot token at the given scope.
pub fn discord_bot_account(scope: Scope<'_>) -> String {
    format!("discord-bot:{}", scope.suffix())
}
