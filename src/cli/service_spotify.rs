//! Spotify's plug-in to the generic service lifecycle.
//!
//! Everything Spotify-specific lives here: the OAuth 2.0 PKCE
//! credential shape (a public client — no `client_secret`), the flags
//! that let the operator paste in a pre-minted refresh token (or run
//! the interactive loopback flow), and the `GET /me` call that
//! validates a credential set. The generic plumbing (flag parsing,
//! path resolution, JSON envelopes, human banners, keychain I/O
//! sequencing) lives in `src/cli/lifecycle.rs` and is shared with
//! every other service.
//!
//! See `docs/services.md#adding-a-new-service` for the full recipe.

use std::time::Duration;

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Confirm, Input, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef, resolve_scopes,
};
use crate::config::{ProjectConfig, SpotifyServiceCfg};
use crate::error::{Result, ZadError};
use crate::oauth::{LoopbackConfig, run_loopback_flow};
use crate::secrets::{self, Scope};
use crate::service::spotify::{AUTH_URL, SpotifyHttp, TOKEN_URL, spotify_scopes_for};

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

/// URL the operator should visit to create a Spotify Developer app.
const SPOTIFY_DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

/// Loopback callback deadline. Matches the default on
/// [`LoopbackConfig`] but spelled out here so the create flow can
/// print it to the user up front.
const LOOPBACK_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// credential shape
// ---------------------------------------------------------------------------

/// Spotify's credential shape — OAuth 2.0 "Authorization Code with
/// PKCE" public client: just `client_id` + a long-lived `refresh_token`.
/// No client secret is issued or accepted by Spotify for PKCE clients.
/// Both pieces are persisted in the OS keychain; the access token is
/// re-minted at each CLI invocation.
pub struct SpotifySecrets {
    pub client_id: String,
    pub refresh_token: String,
}

// ---------------------------------------------------------------------------
// `zad service create spotify` args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)]
    pub base: CreateArgsBase,
    #[command(flatten)]
    pub scopes: ScopesArg,

    /// OAuth 2.0 client ID from the Spotify Developer Dashboard. Not
    /// strictly secret, but zad still stores it in the keychain for
    /// co-location with the refresh token.
    #[arg(long)]
    pub client_id: Option<String>,

    /// Read `--client-id` from this environment variable instead.
    #[arg(long, conflicts_with = "client_id")]
    pub client_id_env: Option<String>,

    /// Pre-minted OAuth refresh token. When provided, zad skips the
    /// browser loopback and stores the token verbatim. Useful for CI
    /// and for operators who already minted one out-of-band.
    #[arg(long, conflicts_with = "refresh_token_env")]
    pub refresh_token: Option<String>,

    /// Read `--refresh-token` from this environment variable instead.
    #[arg(long, conflicts_with = "refresh_token")]
    pub refresh_token_env: Option<String>,

    /// Optional default playlist for verbs that omit `--playlist`.
    /// Accepts a Spotify playlist ID, a `spotify:playlist:<id>` URI,
    /// or a directory alias.
    #[arg(long)]
    pub default_playlist: Option<String>,

    /// The authenticated user's Spotify user ID. Normally captured
    /// from `GET /me` during `validate` — pass this only to pre-seed
    /// the value (non-interactive / testing).
    #[arg(long)]
    pub self_user: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// the trait impl — the entire spotify-specific lifecycle surface
// ---------------------------------------------------------------------------

pub struct SpotifyLifecycle;

#[async_trait]
impl LifecycleService for SpotifyLifecycle {
    const NAME: &'static str = "spotify";
    const DISPLAY: &'static str = "Spotify";
    type Cfg = SpotifyServiceCfg;
    type Secrets = SpotifySecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_spotify();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_spotify();
    }

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(SpotifyServiceCfg, SpotifySecrets)> {
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

        let refresh_token = if let Some(v) = args.refresh_token.clone() {
            v
        } else if let Some(env) = args.refresh_token_env.as_deref() {
            std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()))?
        } else {
            resolve_refresh_via_loopback(&client_id, &scopes, open_browser, non_interactive).await?
        };

        Ok((
            SpotifyServiceCfg {
                scopes,
                default_playlist: args.default_playlist.clone(),
                self_user_id: args.self_user.clone(),
            },
            SpotifySecrets {
                client_id,
                refresh_token,
            },
        ))
    }

    async fn validate(_cfg: &SpotifyServiceCfg, creds: &SpotifySecrets) -> Result<String> {
        let http = SpotifyHttp::unscoped(creds.client_id.clone(), creds.refresh_token.clone());
        let me = http.me().await?;
        Ok(me.display_name.unwrap_or(me.id))
    }

    fn store_secrets(creds: &SpotifySecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let client_id_acct = secrets::account(Self::NAME, "client-id", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        secrets::store(&client_id_acct, &creds.client_id)?;
        secrets::store(&refresh_acct, &creds.refresh_token)?;
        Ok(vec![
            SecretRef {
                label: "client id",
                account: client_id_acct,
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
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        secrets::delete(&client_id_acct)?;
        secrets::delete(&refresh_acct)?;
        Ok(vec![
            SecretRef {
                label: "client id",
                account: client_id_acct,
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
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let client_id_present = secrets::load(&client_id_acct)?.is_some();
        let refresh_present = secrets::load(&refresh_acct)?.is_some();
        Ok(vec![
            SecretRef {
                label: "client id",
                account: client_id_acct,
                present: client_id_present,
            },
            SecretRef {
                label: "refresh token",
                account: refresh_acct,
                present: refresh_present,
            },
        ])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<SpotifySecrets>> {
        let client_id_acct = secrets::account(Self::NAME, "client-id", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let (Some(id), Some(refresh)) = (
            secrets::load(&client_id_acct)?,
            secrets::load(&refresh_acct)?,
        ) else {
            return Ok(None);
        };
        Ok(Some(SpotifySecrets {
            client_id: id,
            refresh_token: refresh,
        }))
    }

    fn cfg_human(cfg: &SpotifyServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(p) = &cfg.default_playlist {
            out.push(("playlist", p.clone()));
        }
        if let Some(u) = &cfg.self_user_id {
            out.push(("self", u.clone()));
        }
        out
    }

    fn cfg_json(cfg: &SpotifyServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "default_playlist": cfg.default_playlist,
            "self_user_id": cfg.self_user_id,
        })
    }

    fn scopes_of(cfg: &SpotifyServiceCfg) -> &[String] {
        &cfg.scopes
    }

    fn post_create_hint(_cfg: &SpotifyServiceCfg) -> Option<String> {
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
    println!("Spotify uses OAuth 2.0 (PKCE public client). You need a Spotify app:");
    println!("  1. Open the Spotify Developer Dashboard:");
    println!("       {SPOTIFY_DASHBOARD_URL}");
    println!("  2. Click \"Create app\". Name and description are arbitrary.");
    println!("  3. Under \"Redirect URIs\", add `http://127.0.0.1` and save.");
    println!("  4. Copy the Client ID from the app's Settings page back here.");
    println!("     (Spotify also shows a Client Secret — you do NOT need it for PKCE.)");
    if open_browser {
        let _ = open::that(SPOTIFY_DASHBOARD_URL);
    }

    let v: String = Input::with_theme(&theme())
        .with_prompt("Spotify Client ID")
        .interact_text()?;
    Ok(v.trim().to_string())
}

/// Interactive browser-based loopback flow for the refresh token.
/// Called only when the user didn't pass `--refresh-token` /
/// `--refresh-token-env`. Bails in non-interactive mode.
async fn resolve_refresh_via_loopback(
    client_id: &str,
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
        "Make sure your Spotify app's \"Redirect URIs\" list includes `http://127.0.0.1` \
         (the loopback listener picks a random port; Spotify accepts any port on 127.0.0.1 \
         once the host is registered)."
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

    let provider_scopes = spotify_scopes_for(zad_scopes);
    let cfg = LoopbackConfig {
        service_name: "spotify",
        display_name: "Spotify",
        auth_url: AUTH_URL.to_string(),
        token_url: TOKEN_URL.to_string(),
        client_id: client_id.to_string(),
        client_secret: None,
        scopes: provider_scopes,
        // `show_dialog=true` forces Spotify to re-prompt for consent
        // even if the user previously authorized this app — without
        // it, a second `create` run silently re-uses the existing
        // grant and we never see a refresh token.
        extra_auth_params: vec![("show_dialog".into(), "true".into())],
        timeout: LOOPBACK_TIMEOUT,
    };
    let tokens = run_loopback_flow(&cfg, open_browser).await?;
    tokens.refresh_token.ok_or_else(|| ZadError::Service {
        name: "spotify",
        message: "Spotify did not return a refresh token. Re-run \
                  `zad service create spotify` to retry the consent flow."
            .into(),
    })
}
