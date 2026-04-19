//! OAuth 2.0 loopback flow with PKCE, used by `zad service create
//! gcal`.
//!
//! Intentionally kept **generic** over the endpoints and scopes so
//! when Reddit / Spotify / the next OAuth service lands, this helper
//! can move to `src/service/oauth/` without a rewrite. Google-specific
//! constants (`AUTH_URL`, `TOKEN_URL`, scope strings) live in
//! `src/service/gcal/mod.rs` and are threaded in via [`LoopbackConfig`].
//!
//! ## Security properties
//!
//! - **PKCE S256** — the verifier is a 64-byte random string; the
//!   challenge is its URL-safe base64-encoded SHA-256. Google's
//!   Identity Platform now effectively requires PKCE even for
//!   "Desktop app" OAuth clients.
//! - **State parameter** — a 32-byte random token is encoded into the
//!   auth URL and verified on the callback. Prevents a co-resident
//!   process from racing the browser to the loopback port with a
//!   forged `code`.
//! - **`127.0.0.1` listener** (not `localhost`) — matches Google's
//!   current redirect-URI guidance and sidesteps browsers that refuse
//!   `http://localhost` redirects.
//! - **Prefetch-tolerant listener** — accepts only the first request
//!   whose path carries `?code=` or `?error=`; probes (favicon, DNS
//!   prefetch, HEAD) get a 404 and the listener keeps waiting.
//! - **Deadline** — overall 120 s budget; a hung browser can't block
//!   the CLI forever.
//!
//! ## Failure modes the call-site should surface clearly
//!
//! - `redirect_uri_mismatch` on token exchange → the operator picked
//!   the wrong Google Cloud OAuth client type. Only "Desktop app"
//!   clients accept any `http://127.0.0.1:<port>` redirect without
//!   pre-registration.
//! - `invalid_grant` on token exchange → clock skew or a code reused
//!   across runs. Tell the operator to check their machine time.
//! - Port-bind failure → another process is holding port 0 (extremely
//!   rare); surface the error as a plain I/O error.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, ZadError};

/// Endpoints + client identity the loopback flow needs.
#[derive(Debug, Clone)]
pub struct LoopbackConfig {
    /// OAuth 2.0 authorization endpoint, e.g.
    /// `https://accounts.google.com/o/oauth2/v2/auth`.
    pub auth_url: String,
    /// OAuth 2.0 token endpoint, e.g. `https://oauth2.googleapis.com/token`.
    pub token_url: String,
    /// Pre-registered OAuth client ID (non-secret).
    pub client_id: String,
    /// OAuth client secret. Google's "Desktop app" client type still
    /// issues one even though it's not strictly secret in the spec
    /// sense — we forward it verbatim to the token endpoint.
    pub client_secret: String,
    /// Provider scopes (space-joined in the auth URL).
    pub scopes: Vec<String>,
    /// How long to wait for the browser callback before giving up.
    pub timeout: Duration,
}

impl Default for LoopbackConfig {
    fn default() -> Self {
        Self {
            auth_url: String::new(),
            token_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scopes: Vec::new(),
            timeout: Duration::from_secs(120),
        }
    }
}

/// What the caller gets back on a successful exchange.
#[derive(Debug, Clone)]
pub struct TokenSet {
    pub access_token: String,
    /// Google only issues refresh tokens when `access_type=offline`
    /// **and** the user has consented — otherwise the server omits the
    /// field entirely. We surface `Option` so the caller can produce
    /// a useful error pointing at `prompt=consent`.
    pub refresh_token: Option<String>,
    /// Seconds until `access_token` expires. Informational only; the
    /// CLI refetches on every invocation instead of persisting this.
    pub expires_in: Option<u64>,
    /// OpenID token when the `openid` scope was requested. Contains a
    /// JWT we don't parse locally — the separate userinfo call is the
    /// source of truth for the authenticated email.
    pub id_token: Option<String>,
}

/// The full interactive flow: spin up a loopback listener, build the
/// auth URL, return its text so the caller can print/open it, then
/// wait for the callback, verify state, exchange code for tokens.
///
/// `open_browser` is honoured here — we call [`open::that`] when
/// `true` and the caller can still print the URL regardless.
pub async fn run_loopback_flow(cfg: &LoopbackConfig, open_browser: bool) -> Result<TokenSet> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| {
        ZadError::Invalid(format!(
            "could not bind 127.0.0.1 loopback listener for OAuth: {e}"
        ))
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| ZadError::Invalid(format!("loopback listener has no local address: {e}")))?
        .port();

    let pkce = Pkce::new();
    let state = random_url_safe(32);
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let auth_url = build_auth_url(cfg, &redirect_uri, &pkce.challenge, &state);

    println!();
    println!("Opening your browser to authorize zad with Google Calendar:");
    println!("  {auth_url}");
    println!();
    println!("Waiting up to {}s for the redirect…", cfg.timeout.as_secs());
    if open_browser {
        let _ = open::that(&auth_url);
    }

    let callback = accept_callback(&listener, cfg.timeout)?;
    if callback.state != state {
        return Err(ZadError::Invalid(
            "OAuth callback returned a state value that doesn't match the one we sent; \
             refusing to continue (possible CSRF attempt)"
                .into(),
        ));
    }
    let code = callback.code.ok_or_else(|| match callback.error {
        Some(e) => ZadError::Service {
            name: "gcal",
            message: format!("OAuth authorization failed: {e}"),
        },
        None => ZadError::Invalid(
            "OAuth callback carried neither `code` nor `error` — cannot continue".into(),
        ),
    })?;

    exchange_code(cfg, &redirect_uri, &code, &pkce.verifier).await
}

/// Exchange a refresh token for a fresh access token. Called on every
/// CLI run — we never persist access tokens.
pub async fn refresh_access_token(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenSet> {
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    post_token_endpoint(token_url, &params).await
}

/// Build the OAuth 2.0 authorization URL. Exposed as `pub(crate)` so
/// the OAuth URL test can reconstruct and verify it without running
/// the full flow.
pub(crate) fn build_auth_url(
    cfg: &LoopbackConfig,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&cfg.auth_url);
    let mut first = true;
    let mut push_param = |key: &str, value: &str, first: &mut bool| {
        out.push(if *first { '?' } else { '&' });
        *first = false;
        out.push_str(key);
        out.push('=');
        out.push_str(&urlencode(value));
    };
    push_param("client_id", &cfg.client_id, &mut first);
    push_param("redirect_uri", redirect_uri, &mut first);
    push_param("response_type", "code", &mut first);
    push_param("scope", &cfg.scopes.join(" "), &mut first);
    push_param("access_type", "offline", &mut first);
    // `prompt=consent` forces Google to re-issue a refresh token even
    // on a second authorization — without it the second run silently
    // succeeds with only an access token.
    push_param("prompt", "consent", &mut first);
    push_param("state", state, &mut first);
    push_param("code_challenge", code_challenge, &mut first);
    push_param("code_challenge_method", "S256", &mut first);
    // Unused but harmless: `include_granted_scopes=true` lets Google
    // carry over previously granted scopes, so a narrower second
    // request doesn't drop capabilities already consented to.
    push_param("include_granted_scopes", "true", &mut first);
    out
}

/// PKCE verifier + challenge pair. The verifier is kept around until
/// token exchange; the challenge is what we put in the auth URL.
#[derive(Debug, Clone)]
pub(crate) struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub(crate) fn new() -> Self {
        // 64 bytes → ~86 chars after URL-safe base64; Google accepts
        // 43..=128 chars.
        let verifier = random_url_safe(64);
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        Self {
            verifier,
            challenge,
        }
    }
}

/// Cryptographically-random URL-safe string of approximately `bytes`
/// bytes of entropy.
pub(crate) fn random_url_safe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Minimal percent-encoding — the RFC3986 unreserved set plus a few
/// practical safe chars. Pulled in here to avoid a whole-crate
/// dependency on `percent-encoding` / `url` for one tiny helper.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
struct Callback {
    code: Option<String>,
    state: String,
    error: Option<String>,
}

/// Accept connections until we see the real OAuth redirect or hit the
/// deadline. Non-callback requests (favicon, HEAD probes, DNS
/// prefetch) get a 404 and the loop continues.
fn accept_callback(listener: &TcpListener, timeout: Duration) -> Result<Callback> {
    listener
        .set_nonblocking(true)
        .map_err(|e| ZadError::Invalid(format!("could not configure loopback listener: {e}")))?;

    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(ZadError::Invalid(format!(
                "timed out after {}s waiting for the OAuth callback",
                timeout.as_secs()
            )));
        }
        match listener.accept() {
            Ok((stream, addr)) => {
                if let Some(cb) = handle_one(stream, addr)? {
                    return Ok(cb);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(ZadError::Invalid(format!(
                    "loopback listener accept failed: {e}"
                )));
            }
        }
    }
}

fn handle_one(mut stream: TcpStream, _addr: SocketAddr) -> Result<Option<Callback>> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| ZadError::Invalid(format!("loopback read-timeout set failed: {e}")))?;

    // Read just the request line — we don't need headers or body.
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return Ok(None);
    }
    // Drain headers so the socket shuts cleanly.
    let mut scratch = [0u8; 1024];
    let _ = stream.read(&mut scratch);

    // Request line looks like: `GET /?code=…&state=… HTTP/1.1\r\n`
    let mut parts = request_line.split_whitespace();
    let _method = parts.next();
    let path = parts.next().unwrap_or("");

    // Anything without a `code=` or `error=` query is a probe.
    if !path.contains("code=") && !path.contains("error=") {
        let _ = write_response(&mut stream, 404, "text/plain", b"not found");
        return Ok(None);
    }

    let cb = parse_callback_query(path);
    let body = if cb.code.is_some() {
        b"<!doctype html><html><body style='font-family:system-ui'>\
          <h2>zad: authorization complete</h2>\
          <p>You can close this tab and return to the terminal.</p>\
          </body></html>"
            .to_vec()
    } else {
        let err = cb.error.as_deref().unwrap_or("unknown");
        format!(
            "<!doctype html><html><body style='font-family:system-ui'>\
             <h2>zad: authorization failed</h2>\
             <p>Google reported: <code>{err}</code></p>\
             <p>Return to the terminal for details.</p></body></html>"
        )
        .into_bytes()
    };
    let _ = write_response(&mut stream, 200, "text/html; charset=utf-8", &body);
    Ok(Some(cb))
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let status_line = match status {
        200 => "200 OK",
        _ => "404 Not Found",
    };
    let headers = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn parse_callback_query(path: &str) -> Callback {
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut out = Callback::default();
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let decoded = urldecode(v);
        match k {
            "code" => out.code = Some(decoded),
            "state" => out.state = decoded,
            "error" => out.error = Some(decoded),
            _ => {}
        }
    }
    out
}

fn urldecode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                let val = u8::from_str_radix(hex, 16).unwrap_or(0);
                out.push(val as char);
                i += 3;
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

async fn exchange_code(
    cfg: &LoopbackConfig,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenSet> {
    let params = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];
    post_token_endpoint(&cfg.token_url, &params).await
}

#[derive(Debug, Deserialize)]
struct RawTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn post_token_endpoint(url: &str, form: &[(&str, &str)]) -> Result<TokenSet> {
    let resp = reqwest::Client::new()
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|e| ZadError::Service {
            name: "gcal",
            message: format!("network error talking to Google's OAuth endpoint: {e}"),
        })?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| ZadError::Service {
        name: "gcal",
        message: format!("failed to read Google's OAuth response body: {e}"),
    })?;

    let parsed: RawTokenResponse = serde_json::from_str(&body).map_err(|e| ZadError::Service {
        name: "gcal",
        message: format!(
            "failed to decode Google's OAuth response (HTTP {status}): {e}; body: {body}"
        ),
    })?;

    if let Some(err) = parsed.error {
        let desc = parsed
            .error_description
            .as_deref()
            .unwrap_or("no description provided");
        // `invalid_grant` specifically means the refresh token was
        // revoked at the provider — surface with a pointer to re-run
        // create so the operator isn't left guessing.
        if err == "invalid_grant" {
            return Err(ZadError::Service {
                name: "gcal",
                message: format!(
                    "refresh token is no longer valid ({desc}); re-run `zad service create gcal`"
                ),
            });
        }
        if err == "redirect_uri_mismatch" {
            return Err(ZadError::Service {
                name: "gcal",
                message: format!(
                    "Google rejected the loopback redirect URI ({desc}). The OAuth client in Google Cloud Console must be type 'Desktop app', not 'Web application' — see `zad man gcal`."
                ),
            });
        }
        return Err(ZadError::Service {
            name: "gcal",
            message: format!("OAuth error `{err}`: {desc}"),
        });
    }

    let access_token = parsed.access_token.ok_or(ZadError::Service {
        name: "gcal",
        message: "Google's OAuth response contained no access_token".into(),
    })?;

    Ok(TokenSet {
        access_token,
        refresh_token: parsed.refresh_token,
        expires_in: parsed.expires_in,
        id_token: parsed.id_token,
    })
}
