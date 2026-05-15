//! OAuth 2.0 device flow for YouTube Music (RFC 8628).
//!
//! YouTube Music's real backend — the InnerTube endpoint family at
//! `music.youtube.com/youtubei/v1` — is not part of the YouTube Data
//! API v3 and is not metered by the Data API daily quota. To talk to
//! it, we authenticate with the TVHTML5 client ID Google issues to
//! TV / limited-input devices. That client supports the device
//! authorization grant (RFC 8628), which fits zad's "library, no
//! local web server" shape better than the loopback flow we use for
//! Spotify and gcal: the user opens a short URL on any browser, types
//! a 9-character code, and approves once.
//!
//! ## Why TVHTML5 and not a desktop client
//!
//! The TVHTML5 client is the only Google OAuth client allowed to mint
//! tokens with the `https://www.googleapis.com/auth/youtube` scope
//! *and* whose access tokens are accepted by the InnerTube backend.
//! Personal-issued "Desktop app" credentials are accepted by the
//! public Data API but rejected by InnerTube. The client_id /
//! client_secret pair below is the same one `ytmusicapi oauth` uses
//! and is widely documented; treating it as confidential is
//! impossible (every install ships it). We forward both fields to the
//! token endpoint verbatim — Google does not treat the TVHTML5 client
//! as confidential.
//!
//! ## Flow summary
//!
//! 1. `POST https://oauth2.googleapis.com/device/code` with
//!    `client_id` and the requested scope → response with
//!    `device_code`, `user_code`, `verification_url`, `expires_in`,
//!    `interval`.
//! 2. Surface the `verification_url` + `user_code` to the user and
//!    poll `POST https://oauth2.googleapis.com/token` every
//!    `interval` seconds with `grant_type=urn:ietf:params:oauth:
//!    grant-type:device_code`. On success the response carries
//!    `access_token` and `refresh_token`.
//! 3. Persist the refresh token via the shared
//!    [`crate::oauth::KeychainRefreshStore`] — same slot ymusic has
//!    always used, so `Ymusic::from_default_config` works unchanged.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{Result, ZadError};
use crate::oauth::TokenSet;

/// OAuth client_id for the TVHTML5 client used by `ytmusicapi`'s
/// device flow. Shared across every install — Google does not treat
/// the value as confidential.
pub const TVHTML5_CLIENT_ID: &str =
    "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com";

/// OAuth client_secret paired with [`TVHTML5_CLIENT_ID`]. Documented
/// in `ytmusicapi`'s source; required by Google's token endpoint
/// even though it ships in every install.
pub const TVHTML5_CLIENT_SECRET: &str = "SboVhoG9s0rNafixCSGGKXAT";

/// Google's RFC 8628 device-authorization endpoint.
pub const DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";

/// Scope requested for InnerTube access. `youtube` is the read+write
/// superset; the InnerTube backend ignores narrower scopes today.
pub const TVHTML5_SCOPE: &str = "https://www.googleapis.com/auth/youtube";

/// RFC 8628 grant type for exchanging a `device_code` for a token.
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// One step of the device-authorization handshake — the values the
/// caller needs to display to the user. Surfaced as a struct so the
/// CLI layer can decide whether to open a browser, copy the code to
/// the clipboard, or render a QR.
#[derive(Debug, Clone)]
pub struct DeviceCode {
    /// Opaque code zad replays to the token endpoint while polling.
    /// Never shown to the user.
    pub device_code: String,
    /// Short human-typeable code the user enters in the browser.
    pub user_code: String,
    /// URL the user visits to type [`Self::user_code`]. Google
    /// returns the unbranded `verification_url` field; if Google ever
    /// also emits `verification_url_complete` we ignore it (the
    /// "complete" variant pre-fills the code but is not consistently
    /// emitted across clients).
    pub verification_url: String,
    /// Seconds until [`Self::user_code`] expires server-side. The
    /// caller should bail out of the poll loop once this deadline is
    /// reached even if the server has not yet returned `expired_token`.
    pub expires_in: u64,
    /// Minimum seconds between consecutive poll requests, per RFC 8628.
    /// The server may upgrade this on the fly via `slow_down` errors;
    /// [`poll_for_token`] honours both.
    pub interval: u64,
}

/// Configurable knobs for the device flow. Defaults match Google's
/// TVHTML5 client; tests override `device_code_url` / `token_url` to
/// point at a wiremock server.
#[derive(Debug, Clone)]
pub struct DeviceFlowConfig {
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    pub device_code_url: String,
    pub token_url: String,
}

impl Default for DeviceFlowConfig {
    fn default() -> Self {
        Self {
            client_id: TVHTML5_CLIENT_ID.to_string(),
            client_secret: TVHTML5_CLIENT_SECRET.to_string(),
            scope: TVHTML5_SCOPE.to_string(),
            device_code_url: DEVICE_CODE_URL.to_string(),
            token_url: super::TOKEN_URL.to_string(),
        }
    }
}

/// Stage 1 — request a device code from Google.
pub async fn request_device_code(cfg: &DeviceFlowConfig) -> Result<DeviceCode> {
    let form = [
        ("client_id", cfg.client_id.as_str()),
        ("scope", cfg.scope.as_str()),
    ];
    let resp = reqwest::Client::new()
        .post(&cfg.device_code_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| ZadError::Service {
            name: "ymusic",
            message: format!("network error requesting device code: {e}"),
        })?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| ZadError::Service {
        name: "ymusic",
        message: format!("failed to read device-code response body: {e}"),
    })?;
    if !status.is_success() {
        return Err(ZadError::Service {
            name: "ymusic",
            message: format!("device-code request rejected (HTTP {status}): {body}"),
        });
    }
    let raw: DeviceCodeResponse = serde_json::from_str(&body).map_err(|e| ZadError::Service {
        name: "ymusic",
        message: format!("failed to decode device-code response: {e}; body: {body}"),
    })?;
    Ok(DeviceCode {
        device_code: raw.device_code,
        user_code: raw.user_code,
        verification_url: raw.verification_url,
        expires_in: raw.expires_in.unwrap_or(600),
        interval: raw.interval.unwrap_or(5),
    })
}

/// Stage 2 — poll the token endpoint until the user finishes the
/// approval flow in their browser. Returns the full token set on
/// success. Honours `slow_down` errors per RFC 8628 by widening the
/// poll interval; surfaces `access_denied` and `expired_token` as
/// terminal errors with operator-friendly messages.
pub async fn poll_for_token(cfg: &DeviceFlowConfig, code: &DeviceCode) -> Result<TokenSet> {
    let mut interval = Duration::from_secs(code.interval.max(1));
    let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in);
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(ZadError::Service {
                name: "ymusic",
                message: format!(
                    "device-flow user code `{}` expired before the user finished approval",
                    code.user_code
                ),
            });
        }
        tokio::time::sleep(interval).await;
        match poll_once(cfg, &code.device_code).await? {
            PollOutcome::Pending => {}
            PollOutcome::SlowDown => {
                // RFC 8628 says: bump the interval by at least 5 s.
                interval += Duration::from_secs(5);
            }
            PollOutcome::Granted(t) => return Ok(t),
            PollOutcome::Denied(msg) => {
                return Err(ZadError::Service {
                    name: "ymusic",
                    message: format!("device-flow approval denied: {msg}"),
                });
            }
            PollOutcome::Expired => {
                return Err(ZadError::Service {
                    name: "ymusic",
                    message: format!(
                        "device-flow user code `{}` expired before the user finished approval",
                        code.user_code
                    ),
                });
            }
        }
    }
}

/// Convenience wrapper around stages 1 and 2 with a caller-supplied
/// presenter for the verification URL + user code. The presenter runs
/// before the poll loop starts so the operator sees the URL before
/// any blocking. Returns the granted [`TokenSet`].
pub async fn run_device_flow<F>(cfg: &DeviceFlowConfig, present: F) -> Result<TokenSet>
where
    F: FnOnce(&DeviceCode),
{
    let code = request_device_code(cfg).await?;
    present(&code);
    poll_for_token(cfg, &code).await
}

enum PollOutcome {
    Pending,
    SlowDown,
    Granted(TokenSet),
    Denied(String),
    Expired,
}

async fn poll_once(cfg: &DeviceFlowConfig, device_code: &str) -> Result<PollOutcome> {
    let form = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("device_code", device_code),
        ("grant_type", DEVICE_GRANT_TYPE),
    ];
    let resp = reqwest::Client::new()
        .post(&cfg.token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| ZadError::Service {
            name: "ymusic",
            message: format!("network error polling device-flow token endpoint: {e}"),
        })?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| ZadError::Service {
        name: "ymusic",
        message: format!("failed to read device-flow poll response: {e}"),
    })?;
    let raw: RawPollResponse = serde_json::from_str(&body).map_err(|e| ZadError::Service {
        name: "ymusic",
        message: format!("failed to decode device-flow poll response (HTTP {status}): {e}; body: {body}"),
    })?;
    if let Some(err) = raw.error.as_deref() {
        return Ok(match err {
            "authorization_pending" => PollOutcome::Pending,
            "slow_down" => PollOutcome::SlowDown,
            "access_denied" => PollOutcome::Denied(
                raw.error_description
                    .unwrap_or_else(|| "user denied the approval request".into()),
            ),
            "expired_token" => PollOutcome::Expired,
            other => {
                return Err(ZadError::Service {
                    name: "ymusic",
                    message: format!(
                        "device-flow token endpoint returned error `{other}`: {}",
                        raw.error_description.as_deref().unwrap_or("(no description)")
                    ),
                });
            }
        });
    }
    let access_token = raw.access_token.ok_or(ZadError::Service {
        name: "ymusic",
        message: format!("device-flow success response missing access_token; body: {body}"),
    })?;
    Ok(PollOutcome::Granted(TokenSet {
        access_token,
        refresh_token: raw.refresh_token,
        expires_in: raw.expires_in,
        id_token: raw.id_token,
        scope: raw.scope,
    }))
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_url: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawPollResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}
