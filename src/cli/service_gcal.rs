//! Google Calendar's plug-in to the generic service lifecycle.
//!
//! Everything Google-specific lives here: the OAuth 2.0 three-field
//! credential shape, the flags that let the user paste in a
//! pre-minted refresh token (or run the interactive loopback flow),
//! and the token → `userinfo` call that validates a credential set.
//! The generic plumbing (flag parsing, path resolution, JSON
//! envelopes, human banners, keychain I/O sequencing) lives in
//! `src/cli/lifecycle.rs` and is shared with every other service.
//!
//! See `docs/services.md#adding-a-new-service` for the full recipe.

use std::time::Duration;

use async_trait::async_trait;
use clap::Args;
use dialoguer::{Confirm, Input, Password, theme::ColorfulTheme};

use crate::cli::lifecycle::{
    CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg, SecretRef, resolve_scopes,
};
use crate::config::{GcalServiceCfg, ProjectConfig};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::gcal::oauth::{LoopbackConfig, run_loopback_flow};
use crate::service::gcal::{AUTH_URL, GcalHttp, TOKEN_URL};

const DEFAULT_SCOPES: &[&str] = &["calendars.read", "events.read", "events.write"];
const ALL_SCOPES: &[&str] = &[
    "calendars.read",
    "events.read",
    "events.write",
    "events.invite",
    "events.remind",
];

/// URL the user should open to create a Google Cloud OAuth client.
/// Printed during interactive create so the operator doesn't have to
/// google around for it.
const GCP_CREDENTIALS_URL: &str = "https://console.cloud.google.com/apis/credentials";

/// Loopback callback deadline. Matches the default on
/// [`LoopbackConfig`] but spelled out here so the create flow can
/// print it to the user up front.
const LOOPBACK_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// credential shape
// ---------------------------------------------------------------------------

/// Google Calendar's credential shape — OAuth 2.0 "Desktop app":
/// `client_id` + `client_secret` + a long-lived `refresh_token`. All
/// three are persisted in the OS keychain; the access token is
/// re-minted at each CLI invocation.
pub struct GcalSecrets {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

// ---------------------------------------------------------------------------
// `zad service create gcal` args
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

    /// Optional default calendar ID (`primary`, an email, or an
    /// alias). Runtime verbs that omit `--calendar` will use this.
    #[arg(long)]
    pub default_calendar: Option<String>,

    /// The authenticated user's primary email. Normally captured from
    /// Google's userinfo endpoint during `validate` — pass this only
    /// to pre-seed the value (non-interactive / testing).
    #[arg(long)]
    pub self_email: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase {
        &self.base
    }
}

// ---------------------------------------------------------------------------
// the trait impl — the entire gcal-specific lifecycle surface
// ---------------------------------------------------------------------------

pub struct GcalLifecycle;

#[async_trait]
impl LifecycleService for GcalLifecycle {
    const NAME: &'static str = "gcal";
    const DISPLAY: &'static str = "Google Calendar";
    type Cfg = GcalServiceCfg;
    type Secrets = GcalSecrets;
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) {
        cfg.enable_gcal();
    }

    fn disable_in_project(cfg: &mut ProjectConfig) {
        cfg.disable_gcal();
    }

    async fn resolve(
        args: &CreateArgs,
        non_interactive: bool,
    ) -> Result<(GcalServiceCfg, GcalSecrets)> {
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
            GcalServiceCfg {
                scopes,
                default_calendar: args.default_calendar.clone(),
                self_email: args.self_email.clone(),
            },
            GcalSecrets {
                client_id,
                client_secret,
                refresh_token,
            },
        ))
    }

    async fn validate(_cfg: &GcalServiceCfg, creds: &GcalSecrets) -> Result<String> {
        let http = GcalHttp::unscoped(
            creds.client_id.clone(),
            creds.client_secret.clone(),
            creds.refresh_token.clone(),
        );
        let info = http.userinfo().await?;
        let email = info.email.unwrap_or_else(|| "<unknown>".into());
        // Light sanity probe — confirms the access token can actually
        // read the calendar API, not just userinfo.
        http.probe_calendar_list().await?;
        Ok(email)
    }

    fn store_secrets(creds: &GcalSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
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

    fn load_secrets(scope: Scope<'_>) -> Result<Option<GcalSecrets>> {
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
        Ok(Some(GcalSecrets {
            client_id: id,
            client_secret: secret,
            refresh_token: refresh,
        }))
    }

    fn cfg_human(cfg: &GcalServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![];
        if let Some(c) = &cfg.default_calendar {
            out.push(("calendar", c.clone()));
        }
        if let Some(e) = &cfg.self_email {
            out.push(("email", e.clone()));
        }
        out
    }

    fn cfg_json(cfg: &GcalServiceCfg) -> serde_json::Value {
        serde_json::json!({
            "default_calendar": cfg.default_calendar,
            "self_email": cfg.self_email,
        })
    }

    fn scopes_of(cfg: &GcalServiceCfg) -> &[String] {
        &cfg.scopes
    }

    fn post_create_hint(_cfg: &GcalServiceCfg) -> Option<String> {
        // `create` succeeded → tokens are stored. No follow-up URL
        // needed; the banner already tells the operator to run
        // `zad service enable gcal` next.
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
    println!("Google Calendar uses OAuth 2.0. You need a Google Cloud OAuth client:");
    println!("  1. Open the Google Cloud Console credentials page:");
    println!("       {GCP_CREDENTIALS_URL}");
    println!("  2. Create an OAuth client of type \"Desktop app\".");
    println!("  3. Enable the \"Google Calendar API\" under APIs & Services → Library.");
    println!("  4. Copy the Client ID and Client Secret back here.");
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
/// Called only when the user didn't pass `--refresh-token` /
/// `--refresh-token-env`. Bails in non-interactive mode.
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

    let google_scopes = google_scopes_for(zad_scopes);
    let cfg = LoopbackConfig {
        auth_url: AUTH_URL.to_string(),
        token_url: TOKEN_URL.to_string(),
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        scopes: google_scopes,
        timeout: LOOPBACK_TIMEOUT,
    };
    let tokens = run_loopback_flow(&cfg, open_browser).await?;
    tokens.refresh_token.ok_or_else(|| ZadError::Service {
        name: "gcal",
        message: "Google did not return a refresh token. Check that the consent screen \
                  granted access and that the OAuth client is type 'Desktop app'. \
                  Re-run `zad service create gcal` to retry."
            .into(),
    })
}

/// Compute the minimal set of Google OAuth scopes to request, given
/// the zad-level scopes the operator declared. We keep the consent
/// screen as narrow as possible.
///
/// Note that the OpenID Connect `openid email` scopes are always
/// requested so `userinfo` can populate `self_email` during validate.
pub fn google_scopes_for(zad_scopes: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec!["openid".into(), "email".into()];
    let has = |s: &str| zad_scopes.iter().any(|z| z == s);

    if has("events.write")
        || has("events.invite")
        || has("events.remind")
        || (!has("calendars.read") && !has("events.read"))
    {
        // Any write — or no explicit scope at all — gets the rw events
        // scope, which also admits reading events.
        out.push("https://www.googleapis.com/auth/calendar.events".into());
    } else if has("events.read") {
        out.push("https://www.googleapis.com/auth/calendar.events.readonly".into());
    }

    if has("calendars.read") && !has("events.write") {
        // calendarList endpoint needs the calendarlist scope; when
        // `events.write` is set we already have the broader
        // `calendar.events` scope, which covers listing the user's
        // calendar list too.
        out.push("https://www.googleapis.com/auth/calendar.calendarlist.readonly".into());
    }

    out.sort();
    out.dedup();
    out
}
