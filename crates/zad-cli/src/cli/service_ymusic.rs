//! YouTube Music's plug-in to the generic service lifecycle.
//!
//! YouTube Music's runtime client talks to the InnerTube backend
//! under `music.youtube.com/youtubei/v1`. Authentication is OAuth
//! 2.0 device flow (RFC 8628) against Google's TVHTML5 client —
//! there is no per-operator OAuth client to register. The lifecycle
//! collects a refresh token by walking the user through the device
//! flow once; subsequent CLI runs mint access tokens from the
//! refresh token without further interaction. The generic plumbing
//! (flag parsing, path resolution, JSON envelopes, human banners,
//! keychain I/O sequencing) lives in `src/cli/lifecycle.rs` and is
//! shared with every other service.
//!
//! See `docs/services.md#adding-a-new-service` for the full recipe.

use crate::cli::DialoguerExt;
use async_trait::async_trait;
use clap::Args;
use dialoguer::{Confirm, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    CliLifecycle, CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef,
    resolve_scopes,
};
use std::sync::{Arc, Mutex};

use zad::config::{ProjectConfig, YmusicServiceCfg};
use zad::error::{Result, ZadError};
use zad::oauth::RefreshTokenStore;
use zad::secrets::{self, Scope};
use zad::service::ymusic::YmusicHttp;
use zad::service::ymusic::oauth_device::{DeviceFlowConfig, run_device_flow};

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

// ---------------------------------------------------------------------------
// credential shape
// ---------------------------------------------------------------------------

/// YouTube Music's credential shape. The device-flow refresh token
/// is the only per-user secret; the OAuth client_id / client_secret
/// are TVHTML5 constants shared across every install (see
/// `zad::service::ymusic::oauth_device`) and therefore not stored.
pub struct YmusicSecrets {
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

    /// Pre-minted OAuth refresh token. When provided, zad skips the
    /// device-flow prompt and stores the token verbatim. Useful for
    /// CI and for operators who already minted one elsewhere.
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
    /// captured during `validate` — pass this only to pre-seed the
    /// value (non-interactive / testing).
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

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_ymusic();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_ymusic();
    }

    async fn validate(_cfg: &YmusicServiceCfg, creds: &mut YmusicSecrets) -> Result<String> {
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let store = Arc::new(CaptureRefreshToken(captured.clone()));
        let http = YmusicHttp::with_store(
            String::new(),
            String::new(),
            creds.refresh_token.clone(),
            std::collections::BTreeSet::new(),
            std::path::PathBuf::new(),
            Some(store),
        );
        let info = http.userinfo().await?;
        let email = info.email.unwrap_or_else(|| "<unknown>".into());
        let identity = match http.my_channel().await {
            Ok(c) => {
                let title = c
                    .snippet
                    .as_ref()
                    .and_then(|s| s.title.as_deref())
                    .unwrap_or(email.as_str())
                    .to_string();
                format!("{title} ({email})")
            }
            Err(_) => format!("{email} (no YouTube channel)"),
        };
        if let Some(rotated) = captured.lock().unwrap().take() {
            creds.refresh_token = rotated;
        }
        Ok(identity)
    }

    fn store_secrets(creds: &YmusicSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        // Delete the legacy `client-id` / `client-secret` slots so
        // operators upgrading from the Data API era don't carry
        // stale values around. The new TVHTML5 constants live in
        // `zad::service::ymusic::oauth_device` and ship in the
        // binary.
        let legacy_client_id = secrets::account(Self::NAME, "client-id", scope.clone());
        let legacy_client_secret = secrets::account(Self::NAME, "client-secret", scope.clone());
        let _ = secrets::delete(&legacy_client_id);
        let _ = secrets::delete(&legacy_client_secret);

        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        secrets::store(&refresh_acct, &creds.refresh_token)?;
        Ok(vec![SecretRef {
            label: "refresh token",
            account: refresh_acct,
            present: true,
        }])
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let legacy_client_id = secrets::account(Self::NAME, "client-id", scope.clone());
        let legacy_client_secret = secrets::account(Self::NAME, "client-secret", scope.clone());
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let _ = secrets::delete(&legacy_client_id);
        let _ = secrets::delete(&legacy_client_secret);
        secrets::delete(&refresh_acct)?;
        Ok(vec![SecretRef {
            label: "refresh token",
            account: refresh_acct,
            present: false,
        }])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let refresh_present = secrets::load(&refresh_acct)?.is_some();
        Ok(vec![SecretRef {
            label: "refresh token",
            account: refresh_acct,
            present: refresh_present,
        }])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<YmusicSecrets>> {
        let refresh_acct = secrets::account(Self::NAME, "refresh", scope);
        let Some(refresh) = secrets::load(&refresh_acct)? else {
            return Ok(None);
        };
        Ok(Some(YmusicSecrets {
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

#[async_trait]
impl CliLifecycle for YmusicLifecycle {
    type CreateArgs = CreateArgs;

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

        let refresh_token = if let Some(v) = args.refresh_token.clone() {
            v
        } else if let Some(env) = args.refresh_token_env.as_deref() {
            std::env::var(env).map_err(|_| ZadError::MissingEnv(env.to_string()))?
        } else {
            resolve_refresh_via_device_flow(open_browser, non_interactive).await?
        };

        Ok((
            YmusicServiceCfg {
                scopes,
                default_playlist: args.default_playlist.clone(),
                self_channel_id: args.self_channel.clone(),
            },
            YmusicSecrets { refresh_token },
        ))
    }
}

// ---------------------------------------------------------------------------
// device-flow helper
// ---------------------------------------------------------------------------

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

/// Interactive device-flow OAuth. Surfaces the verification URL +
/// user code to the operator and waits for the polling loop to
/// resolve.
async fn resolve_refresh_via_device_flow(
    open_browser: bool,
    non_interactive: bool,
) -> Result<String> {
    if non_interactive {
        return Err(ZadError::MissingRequired(
            "--refresh-token or --refresh-token-env (non-interactive mode cannot run the \
             device-flow prompt)",
        ));
    }

    println!();
    println!(
        "YouTube Music uses Google's OAuth 2.0 device flow (the same one TV apps use).\n\
         No client-id or client-secret to configure — zad ships the shared TVHTML5\n\
         credentials. You'll get a short URL and a 9-character code; visit the URL in\n\
         any browser (it does not have to be on this machine), enter the code, and\n\
         approve. This window will keep polling until you finish or the code expires."
    );

    let want = Confirm::with_theme(&theme())
        .with_prompt("Continue with the device-flow prompt?")
        .default(true)
        .interact()
        .into_zad()?;
    if !want {
        return Err(ZadError::Invalid(
            "device-flow declined by operator; pass --refresh-token to skip it".into(),
        ));
    }

    let cfg = DeviceFlowConfig::default();
    let tokens = run_device_flow(&cfg, |code| {
        println!();
        println!("  Visit:  {}", code.verification_url);
        println!("  Enter:  {}", code.user_code);
        println!();
        println!(
            "Waiting up to {}s for approval (polling every {}s)…",
            code.expires_in, code.interval
        );
        if open_browser {
            let _ = open::that(&code.verification_url);
        }
    })
    .await?;

    tokens.refresh_token.ok_or_else(|| ZadError::Service {
        name: "ymusic",
        message: "Google did not return a refresh token from the device flow. Re-run \
                  `zad service create ymusic` to retry."
            .into(),
    })
}

/// Captures a rotated refresh token into a shared cell. Used by
/// `validate` so the refresh-token-on-rotation safety net survives
/// the device-flow refactor.
struct CaptureRefreshToken(Arc<Mutex<Option<String>>>);

impl RefreshTokenStore for CaptureRefreshToken {
    fn store(&self, refresh_token: &str) -> Result<()> {
        *self.0.lock().unwrap() = Some(refresh_token.to_string());
        Ok(())
    }
}
