//! YouTube Music's plug-in to the generic service lifecycle.
//!
//! YouTube Music has no dedicated public API; the runtime client
//! talks to the YouTube Data API v3. Authentication is identical to
//! Google Calendar — OAuth 2.0 "Desktop app": `client_id` +
//! `client_secret` + a long-lived `refresh_token`. All three live in
//! the OS keychain. The generic plumbing (flag parsing, path
//! resolution, JSON envelopes, human banners, keychain I/O
//! sequencing) lives in `src/cli/lifecycle.rs` and is shared with
//! every other service.
//!
//! See `docs/services.md#adding-a-new-service` for the full recipe.

use std::time::Duration;

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Confirm, Input, Password, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef, resolve_scopes,
};
use crate::config::{ProjectConfig, YmusicServiceCfg};
use crate::error::{Result, ZadError};
use crate::oauth::{LoopbackConfig, RedirectScheme, run_loopback_flow};
use crate::secrets::{self, Scope};
use crate::service::ymusic::{AUTH_URL, TOKEN_URL, YmusicHttp, youtube_scopes_for};

const DEFAULT_SCOPES: &[&str] = &[
    "search",
    "playlists.read",
    "playlists.write",
    "library.read",
];
const ALL_SCOPES: &[&str] = &[
    "search",
    "playlists.read",
    "playlists.write",
    "library.read",
    "library.write",
];

/// URL the user should open to create a Google Cloud OAuth client.
const GCP_CREDENTIALS_URL: &str = "https://console.cloud.google.com/apis/credentials";

/// Loopback callback deadline. Matches the default on
/// [`LoopbackConfig`] but spelled out here so the create flow can
/// print it to the user up front.
const LOOPBACK_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// credential shape
// ---------------------------------------------------------------------------

/// YouTube Music's credential shape — OAuth 2.0 "Desktop app":
/// `client_id` + `client_secret` + a long-lived `refresh_token`. All
/// three are persisted in the OS keychain; the access token is
/// re-minted at each CLI invocation.
pub struct YmusicSecrets {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

// ---------------------------------------------------------------------------
// `zad service create ymusic` args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub scopes: ScopesArg,

    /// OAuth 2.0 client ID from Google Cloud Console (Desktop app
    /// type). Not a secret, but zad still stores it in the keychain
    /// for co-location with the other OAuth fields.
    #[arg(long)]
    pub client_id: Option<String>,

    /// Read `--client-id` from this environment variable instead.
    #[arg(long, conflicts_with = "client_id")]
    pub client_id_env: Option<String>,

    /// OAuth 2.0 client secret issued alongside `--client-id`. Google
    /// calls this a "secret" even for Desktop-app clients; we treat
    /// it as one.
    #[arg(long, conflicts_with = "client_secret_env")]
    pub client_secret: Option<String>,

    /// Read `--client-secret` from this environment variable instead.
    #[arg(long, conflicts_with = "client_secret")]
    pub client_secret_env: Option<String>,

    /// Pre-minted OAuth refresh token. When provided, zad skips the
    /// browser loopback and stores the token verbatim. Useful for CI
    /// and for operators who already minted one via Google's OAuth
    /// Playground.
    #[arg(long, conflicts_with = "refresh_token_env")]
    pub refresh_token: Option<String>,

    /// Read `--refresh-token` from this environment variable instead.
    #[arg(long, conflicts_with = "refresh_token")]
    pub refresh_token_env: Option<String>,

    /// Optional default playlist for verbs that omit `--playlist`.
    /// Accepts a YouTube playlist ID (`PL…`) or a directory alias.
    #[arg(long)]
    pub default_playlist: Option<String>,

    /// The authenticated user's YouTube channel ID (`UC…`). Normally
    /// captured from `channels?mine=true` during `validate` — pass
    /// this only to pre-seed the value (non-interactive / testing).
    #[arg(long)]
    pub self_channel: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// the trait impl — the entire ymusic-specific lifecycle surface
// ---------------------------------------------------------------------------

pub struct YmusicLifecycle;

#[async_trait]
impl LifecycleService for YmusicLifecycle {
    const NAME: &'static str = "ymusic";
    const DISPLAY: &'static str = "YouTube Music";
    type Cfg = YmusicServiceCfg;
    type Secrets = YmusicSecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_ymusic();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_ymusic();
    }

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(YmusicServiceCfg, YmusicSecrets)> {
        let open_browser = !args.base.no_browser;

        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(),
            DEFAULT_SCOPES,
            ALL_SCOPES,
            non_interactive,
        )?;

        let client_id = resolve_client_id(
            args.client_id.as_deref(),
            args.client_id_env.as_deref(),
            open_browser,
            non_interactive,
        )?;

        let client_secret = resolve_client_secret(
            args.client_secret.as_deref(),
            args.client_secret_env.as_deref(),
            non_interactive,
        )?;

        let refresh_token = if let Some(v) = args.refresh_token.clone() {
            v
        } else if let Some(env) = args.refresh_token_env.as_deref() {
            std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()))?
        } else {
            resolve_refresh_via_loopback(
                &client_id,
                &client_secret,
                &scopes,
                open_browser,
                non_interactive,
            )
            .await?
        };

        Ok((
            YmusicServiceCfg {
                scopes,
                default_playlist: args.default_playlist.clone(),
                self_channel_id: args.self_channel.clone(),
            },
            YmusicSecrets {
                client_id,
                client_secret,
                refresh_token,
            },
        ))
    }

    async fn validate(_cfg: &YmusicServiceCfg, creds: &YmusicSecrets) -> Result<String> {
        let http = YmusicHttp::unscoped(
            creds.client_id.clone(),
            creds.client_secret.clone(),
            creds.refresh_token.clone(),
        );
        let info = http.userinfo().await?;
        let email = info.email.unwrap_or_else(|| "<unknown>".into());
        // Light sanity probe — confirms the token can read YouTube,
        // not just userinfo. A successful response also surfaces the
        // channel title, which is more useful than the raw email.
        match http.my_channel().await {
            Ok(c) => {
                let title = c
                    .snippet
                    .as_ref()
                    .and_then(|s| s.title.as_deref())
                    .unwrap_or(email.as_str());
                Ok(format!("{title} ({email})"))
            }
            // Account exists but has no YouTube channel yet — surface
            // the email and let the operator decide whether to create
            // a channel before running runtime verbs.
            Err(_) => Ok(format!("{email} (no YouTube channel)")),
        }
    }

    fn store_secrets(creds: &YmusicSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let client_id_acct = secrets::account(Self::NAME, "client-id", scope.clone());
        let client_secret_acct = secrets::account(Self::NAME, "client-secret", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        secrets::store(&client_id_acct, &creds.client_id)?;
        secrets::store(&client_secret_acct, &creds.client_secret)?;
        secrets::store(&refresh_acct, &creds.refresh_token)?;
        Ok(vec![
            SecretRef {
                label: "client id",
                account: client_id_acct,
                present: true,
            },
            SecretRef {
                label: "client secret",
                account: client_secret_acct,
                present: true,
            },
            SecretRef {
                label: "refresh token",
                account: refresh_acct,
                present: true,
            },
        ])
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let client_id_acct = secrets::account(Self::NAME, "client-id", scope.clone());
        let client_secret_acct = secrets::account(Self::NAME, "client-secret", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        secrets::delete(&client_id_acct)?;
        secrets::delete(&client_secret_acct)?;
        secrets::delete(&refresh_acct)?;
        Ok(vec![
            SecretRef {
                label: "client id",
                account: client_id_acct,
                present: false,
            },
            SecretRef {
                label: "client secret",
                account: client_secret_acct,
                present: false,
            },
            SecretRef {
                label: "refresh token",
                account: refresh_acct,
                present: false,
            },
        ])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let client_id_acct = secrets::account(Self::NAME, "client-id", scope.clone());
        let client_secret_acct = secrets::account(Self::NAME, "client-secret", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let client_id_present = secrets::load(&client_id_acct)?.is_some();
        let client_secret_present = secrets::load(&client_secret_acct)?.is_some();
        let refresh_present = secrets::load(&refresh_acct)?.is_some();
        Ok(vec![
            SecretRef {
                label: "client id",
                account: client_id_acct,
                present: client_id_present,
            },
            SecretRef {
                label: "client secret",
                account: client_secret_acct,
                present: client_secret_present,
            },
            SecretRef {
                label: "refresh token",
                account: refresh_acct,
                present: refresh_present,
            },
        ])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<YmusicSecrets>> {
        let client_id_acct = secrets::account(Self::NAME, "client-id", scope.clone());
        let client_secret_acct = secrets::account(Self::NAME, "client-secret", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let (Some(id), Some(secret), Some(refresh)) = (
            secrets::load(&client_id_acct)?,
            secrets::load(&client_secret_acct)?,
            secrets::load(&refresh_acct)?,
        ) else {
            return Ok(None);
        };
        Ok(Some(YmusicSecrets {
            client_id: id,
            client_secret: secret,
            refresh_token: refresh,
        }))
    }

    fn cfg_human(cfg: &YmusicServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(p) = &cfg.default_playlist {
            out.push(("playlist", p.clone()));
        }
        if let Some(c) = &cfg.self_channel_id {
            out.push(("channel", c.clone()));
        }
        out
    }

    fn cfg_json(cfg: &YmusicServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "default_playlist": cfg.default_playlist,
            "self_channel_id": cfg.self_channel_id,
        })
    }

    fn scopes_of(cfg: &YmusicServiceCfg) -> &[String] {
        &cfg.scopes
    }

    fn post_create_hint(_cfg: &YmusicServiceCfg) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// prompt helpers
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn resolve_client_id(
    flag: Option<&str>,
    env_flag: Option<&str>,
    open_browser: bool,
    non_interactive: bool,
) -> Result<String> {
    if let Some(env) = env_flag {
        return std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()));
    }
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired("--client-id or --client-id-env"));
    }

    println!();
    println!("YouTube Music uses Google OAuth 2.0. You need a Google Cloud OAuth client:");
    println!("  1. Open the Google Cloud Console credentials page:");
    println!("       {GCP_CREDENTIALS_URL}");
    println!("  2. Create an OAuth client of type \"Desktop app\".");
    println!("  3. Enable the \"YouTube Data API v3\" under APIs & Services → Library.");
    println!("  4. Copy the Client ID and Client Secret back here.");
    println!("Note: YouTube Music does not have its own API; the Data API covers");
    println!("      playlists, library (rated videos), and search the same way.");
    if open_browser {
        let _ = open::that(GCP_CREDENTIALS_URL);
    }

    let v: String = Input::with_theme(&theme())
        .with_prompt("Google OAuth Client ID")
        .interact_text()?;
    Ok(v.trim().to_string())
}

fn resolve_client_secret(
    flag: Option<&str>,
    env_flag: Option<&str>,
    non_interactive: bool,
) -> Result<String> {
    if let Some(env) = env_flag {
        return std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()));
    }
    if let Some(v) = flag {
        return Ok(v.to_string());
    }
    if non_interactive {
        return Err(ZadError::MissingRequired(
            "--client-secret or --client-secret-env",
        ));
    }

    let v = Password::with_theme(&theme())
        .with_prompt("Google OAuth Client Secret")
        .interact()?;
    Ok(v)
}

/// Interactive browser-based loopback flow for the refresh token.
async fn resolve_refresh_via_loopback(
    client_id: &str,
    client_secret: &str,
    zad_scopes: &[String],
    open_browser: bool,
    non_interactive: bool,
) -> Result<String> {
    if non_interactive {
        return Err(ZadError::MissingRequired(
            "--refresh-token or --refresh-token-env (non-interactive mode cannot open a browser)",
        ));
    }

    println!();
    println!("No refresh token provided — starting the browser OAuth flow.");
    println!(
        "Make sure the OAuth client you created in Google Cloud Console is of type \"Desktop app\"."
    );
    let want = Confirm::with_theme(&theme())
        .with_prompt("Continue with the browser flow?")
        .default(true)
        .interact()?;
    if !want {
        return Err(ZadError::Invalid(
            "browser OAuth flow declined by operator; pass --refresh-token to skip it".into(),
        ));
    }

    let google_scopes = youtube_scopes_for(zad_scopes);
    let cfg = LoopbackConfig {
        service_name: "ymusic",
        display_name: "YouTube Music",
        auth_url: AUTH_URL.to_string(),
        token_url: TOKEN_URL.to_string(),
        client_id: client_id.to_string(),
        client_secret: Some(client_secret.to_string()),
        scopes: google_scopes,
        extra_auth_params: vec![
            // Google needs `access_type=offline` to issue a refresh
            // token at all, and `prompt=consent` to re-issue one on
            // any subsequent authorization.
            ("access_type".into(), "offline".into()),
            ("prompt".into(), "consent".into()),
            ("include_granted_scopes".into(), "true".into()),
        ],
        timeout: LOOPBACK_TIMEOUT,
        redirect_scheme: RedirectScheme::Http,
    };
    let tokens = run_loopback_flow(&cfg, open_browser).await?;
    tokens.refresh_token.ok_or_else(|| ZadError::Service {
        name: "ymusic",
        message: "Google did not return a refresh token. Check that the consent screen \
                  granted access and that the OAuth client is type 'Desktop app'. \
                  Re-run `zad service create ymusic` to retry."
            .into(),
    })
}
