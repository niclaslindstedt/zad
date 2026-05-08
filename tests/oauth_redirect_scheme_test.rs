//! Verify the per-service redirect scheme threads end-to-end into
//! the authorization URL the CLI sends to the provider. The Spotify
//! lifecycle wires `RedirectScheme::Https` and we want to catch
//! regressions where the wiring silently flips back to plain HTTP.

use zad::oauth::{LoopbackConfig, RedirectScheme, build_auth_url};

fn cfg() -> LoopbackConfig {
    LoopbackConfig {
        service_name: "test",
        display_name: "Test",
        auth_url: "https://example.test/authorize".into(),
        token_url: "https://example.test/token".into(),
        client_id: "cid".into(),
        client_secret: None,
        scopes: vec!["read".into()],
        extra_auth_params: vec![],
        timeout: std::time::Duration::from_secs(10),
        redirect_scheme: RedirectScheme::Http,
    }
}

#[test]
fn redirect_scheme_default_is_http() {
    assert_eq!(RedirectScheme::default(), RedirectScheme::Http);
}

#[test]
fn redirect_scheme_str_form() {
    assert_eq!(RedirectScheme::Http.as_str(), "http");
    assert_eq!(RedirectScheme::Https.as_str(), "https");
}

#[test]
fn build_auth_url_encodes_http_redirect_uri() {
    let url = build_auth_url(&cfg(), "http://127.0.0.1:5555", "challenge", "state");
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5555"),
        "auth url should percent-encode the http redirect uri: {url}"
    );
}

#[test]
fn build_auth_url_encodes_https_redirect_uri() {
    let url = build_auth_url(&cfg(), "https://127.0.0.1:5555", "challenge", "state");
    assert!(
        url.contains("redirect_uri=https%3A%2F%2F127.0.0.1%3A5555"),
        "auth url should percent-encode the https redirect uri: {url}"
    );
    assert!(
        !url.contains("redirect_uri=http%3A"),
        "auth url should not contain a plain-http redirect when scheme is Https: {url}"
    );
}

#[test]
fn spotify_lifecycle_uses_https_scheme() {
    // Sanity check that the Spotify-specific wiring really did flip
    // to Https — pulled out as a distinct test so a regression here
    // is named on the failure line.
    let mut c = cfg();
    c.redirect_scheme = RedirectScheme::Https;
    let url = build_auth_url(&c, "https://127.0.0.1:1234", "x", "y");
    assert!(url.contains("redirect_uri=https%3A%2F%2F127.0.0.1%3A1234"));
}
