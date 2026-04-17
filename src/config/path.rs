use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::error::{Result, ZadError};

/// Root of zad's per-project config tree. Defaults to `~/.zad/` but can be
/// overridden (primarily for tests) via [`set_home_override`].
static HOME_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

pub fn set_home_override(path: PathBuf) {
    let _ = HOME_OVERRIDE.set(path);
}

fn home_dir() -> Result<PathBuf> {
    if let Some(p) = HOME_OVERRIDE.get() {
        return Ok(p.clone());
    }
    if let Ok(env_home) = std::env::var("ZAD_HOME_OVERRIDE") {
        return Ok(PathBuf::from(env_home));
    }
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_owned())
        .ok_or(ZadError::NoHomeDir)
}

/// Slugify an absolute path using the Claude Code convention: replace every
/// `/` with `-`, preserving the leading separator as a leading `-`. On
/// Windows, back-slashes and drive colons are also collapsed.
pub fn project_slug_for(path: &Path) -> Result<String> {
    let s = path
        .to_str()
        .ok_or_else(|| ZadError::NonUtf8Cwd(path.to_owned()))?;
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '/' | '\\' | ':' => out.push('-'),
            _ => out.push(ch),
        }
    }
    Ok(out)
}

pub fn project_slug() -> Result<String> {
    let cwd = std::env::current_dir().map_err(|e| ZadError::Io {
        path: PathBuf::from("."),
        source: e,
    })?;
    project_slug_for(&cwd)
}

pub fn zad_home() -> Result<PathBuf> {
    Ok(home_dir()?.join(".zad"))
}

pub fn project_dir_for(slug: &str) -> Result<PathBuf> {
    Ok(zad_home()?.join("projects").join(slug))
}

pub fn project_dir() -> Result<PathBuf> {
    project_dir_for(&project_slug()?)
}

pub fn project_config_path() -> Result<PathBuf> {
    Ok(project_dir()?.join("config.toml"))
}

/// `~/.zad/adapters/<adapter>/` — home for shared adapter credentials
/// reused by every project that opts in via `zad adapter <name> add`.
pub fn global_adapter_dir(adapter: &str) -> Result<PathBuf> {
    Ok(zad_home()?.join("adapters").join(adapter))
}

pub fn global_adapter_config_path(adapter: &str) -> Result<PathBuf> {
    Ok(global_adapter_dir(adapter)?.join("config.toml"))
}

/// `~/.zad/projects/<slug>/adapters/<adapter>/` — home for credentials
/// that only apply to one project. When present, these take precedence
/// over the global adapter config.
pub fn project_adapter_dir_for(slug: &str, adapter: &str) -> Result<PathBuf> {
    Ok(project_dir_for(slug)?.join("adapters").join(adapter))
}

pub fn project_adapter_config_path_for(slug: &str, adapter: &str) -> Result<PathBuf> {
    Ok(project_adapter_dir_for(slug, adapter)?.join("config.toml"))
}

pub fn project_adapter_config_path(adapter: &str) -> Result<PathBuf> {
    project_adapter_config_path_for(&project_slug()?, adapter)
}
