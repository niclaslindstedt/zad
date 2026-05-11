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

    /// HTTP 429 (or provider-equivalent) from a service API. Carries
    /// enough structure that `--json` output can surface the wait
    /// deadline as machine-readable fields, and the human message
    /// always points at the `--wait` flag so callers know how to
    /// recover by re-issuing the same command.
    ///
    /// `retry_after_seconds` is the integer seconds until
    /// `retry_after_utc` from the moment the error was constructed; it
    /// drifts forward as time passes but is stable for the duration of
    /// any single response render. `retry_after_utc` is the absolute
    /// RFC 3339 timestamp callers can compare against the wall clock.
    #[error(
        "{service} rate-limited this call (HTTP 429); wait {retry_after_seconds}s (until {retry_after_utc}). Re-run the same command with --wait to block until ready and retry automatically."
    )]
    RateLimited {
        service: &'static str,
        retry_after_seconds: u64,
        retry_after_utc: String,
    },

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
        "permission denied for `load`: not trusted\n  config: {path}\n  trust store: {trust_store_path}\n  tip: run `zad <service> permissions sign` to sign and trust this file"
    )]
    NotTrusted {
        path: PathBuf,
        trust_store_path: PathBuf,
    },

    #[error(
        "permission denied for `load`: signature invalid ({reason})\n  config: {path}\n  tip: the file was modified after signing; re-sign it with `zad <service> permissions sign` or revert the edit"
    )]
    SignatureInvalid { path: PathBuf, reason: String },

    #[error(
        "permission denied for `load`: signing key mismatch (trust entry signed with {found_fingerprint}, local keychain holds {expected_fingerprint})\n  config: {path}\n  tip: either re-sign the file with the local key (`zad <service> permissions sign`) or rotate the keychain entry to the authoring key"
    )]
    SignatureKeyMismatch {
        path: PathBuf,
        expected_fingerprint: String,
        found_fingerprint: String,
    },

    #[error(
        "trust store at {path} is tampered or unreadable: {reason}\n  tip: run `zad signing init --force` to rotate the keychain key and rebuild the trust store; you will need to re-sign every permissions file"
    )]
    TrustStoreTampered { path: PathBuf, reason: String },

    #[error("no signing key in OS keychain — {hint}")]
    SigningKeyMissing { hint: String },

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("operation not supported by this service: {0}")]
    Unsupported(&'static str),

    /// Wraps a CLI-layer interactive-prompt failure. The library never
    /// produces this variant; the `zad-cli` crate maps `dialoguer::Error`
    /// into it at the CLI boundary so the rest of the error surface stays
    /// uniform.
    #[error("interactive prompt error: {0}")]
    Prompt(String),
}

impl From<serenity::Error> for ZadError {
    fn from(e: serenity::Error) -> Self {
        ZadError::Service {
            name: "discord",
            message: e.to_string(),
        }
    }
}
