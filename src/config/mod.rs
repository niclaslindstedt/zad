pub mod path;
pub mod schema;

use std::path::Path;

pub use schema::{DiscordProjectRef, DiscordServiceCfg, ProjectConfig, ServiceRef};

use crate::error::{Result, ZadError};

pub fn load_from(path: &Path) -> Result<ProjectConfig> {
    if !path.exists() {
        return Ok(ProjectConfig::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    toml::from_str(&raw).map_err(|e| ZadError::TomlParse {
        path: path.to_owned(),
        source: e,
    })
}

pub fn save_to(path: &Path, cfg: &ProjectConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_owned(),
            source: e,
        })?;
    }
    let body = toml::to_string_pretty(cfg)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_owned(),
        source: e,
    })
}

pub fn load() -> Result<ProjectConfig> {
    load_from(&path::project_config_path()?)
}

pub fn save(cfg: &ProjectConfig) -> Result<()> {
    save_to(&path::project_config_path()?, cfg)
}

/// Load a TOML-serializable value from `path`, returning `None` when the
/// file does not exist. Used for the flat global service configs.
pub fn load_flat<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    let v: T = toml::from_str(&raw).map_err(|e| ZadError::TomlParse {
        path: path.to_owned(),
        source: e,
    })?;
    Ok(Some(v))
}

pub fn save_flat<T: serde::Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_owned(),
            source: e,
        })?;
    }
    let body = toml::to_string_pretty(value)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_owned(),
        source: e,
    })
}
