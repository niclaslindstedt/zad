use std::path::PathBuf;

pub type Result<T, E = ZadError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum ZadError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse TOML at {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("could not resolve user home directory")]
    NoHomeDir,

    #[error("current working directory is not valid UTF-8: {0:?}")]
    NonUtf8Cwd(PathBuf),

    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("service '{name}' is already configured; pass --force to overwrite")]
    ServiceAlreadyConfigured { name: String },

    #[error("missing required value for '{0}' (running with --non-interactive)")]
    MissingRequired(&'static str),

    #[error("environment variable '{0}' is not set")]
    MissingEnv(String),

    #[error("discord API error: {0}")]
    Discord(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("operation not supported by this service: {0}")]
    Unsupported(&'static str),

    #[error("interactive prompt error: {0}")]
    Prompt(#[from] dialoguer::Error),
}

impl From<serenity::Error> for ZadError {
    fn from(e: serenity::Error) -> Self {
        ZadError::Discord(e.to_string())
    }
}
