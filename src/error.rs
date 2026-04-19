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

    /// Generic service error surface. Every service reports provider-
    /// side problems through this variant (`name` is the service,
    /// `message` is the provider's text). Adding a new service never
    /// needs a new `ZadError` variant — keep per-service variants only
    /// for *structured* failures whose callers need to match on them
    /// (e.g. `DiscordChannelNotFound`).
    #[error("{name} API error: {message}")]
    Service { name: &'static str, message: String },

    #[error(
        "{service}: scope `{scope}` is not enabled for this project\n  config: {config_path}\n  tip: add `{scope}` to `scopes` in that file (or re-run `zad service create {service} --force`)"
    )]
    ScopeDenied {
        service: &'static str,
        scope: &'static str,
        config_path: PathBuf,
    },

    #[error(
        "discord requires the `{intent}` privileged intent — enable it in the Developer Portal (https://discord.com/developers/applications) and restart the bot"
    )]
    DiscordPrivilegedIntent { intent: &'static str },

    #[error("discord channel {id} not found or the bot cannot see it")]
    DiscordChannelNotFound { id: u64 },

    #[error(
        "permission denied for `{function}`: {reason}\n  config: {config_path}\n  tip: edit that file (or delete it) to adjust the rule"
    )]
    PermissionDenied {
        function: &'static str,
        reason: String,
        config_path: PathBuf,
    },

    #[error(
        "permission denied for `load`: signature missing\n  config: {path}\n  tip: run `zad <service> permissions sign` to sign the file, or re-init it"
    )]
    SignatureMissing { path: PathBuf },

    #[error(
        "permission denied for `load`: signature invalid ({reason})\n  config: {path}\n  tip: the file was modified after signing; re-sign it with `zad <service> permissions sign` or revert the edit"
    )]
    SignatureInvalid { path: PathBuf, reason: String },

    #[error(
        "permission denied for `load`: signing key mismatch (file signed with {found_fingerprint}, local keychain holds {expected_fingerprint})\n  config: {path}\n  tip: either re-sign the file with the local key or replace the keychain entry with the authoring key"
    )]
    SignatureKeyMismatch {
        path: PathBuf,
        expected_fingerprint: String,
        found_fingerprint: String,
    },

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("operation not supported by this service: {0}")]
    Unsupported(&'static str),

    #[error("interactive prompt error: {0}")]
    Prompt(#[from] dialoguer::Error),
}

impl From<serenity::Error> for ZadError {
    fn from(e: serenity::Error) -> Self {
        ZadError::Service {
            name: "discord",
            message: e.to_string(),
        }
    }
}
