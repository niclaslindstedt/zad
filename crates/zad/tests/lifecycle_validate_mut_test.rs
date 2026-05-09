//! Verifies the lifecycle driver's "validate may mutate secrets in
//! place; store_secrets sees the mutation" contract.
//!
//! This is the trait-level guarantee that PR #65's `SpotifyHttp`
//! rotation handling rides on top of: at create time, Spotify's
//! validate ping rotates the refresh token, the rotated value lands
//! in `creds.refresh_token` via a `RefreshTokenStore` callback, and
//! the lifecycle driver then writes that rotated value to the
//! keychain rather than the original.
//!
//! The test stands up a tiny `LifecycleService` impl that mutates
//! `secrets.token` inside `validate`, drives `lifecycle::create`,
//! and asserts the keychain (in-memory backend) ends up holding the
//! mutated token.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use zad::config::ProjectConfig;
use zad::error::Result;
use zad::secrets::{self, Scope};
use zad::service::lifecycle::{self, CreateOpts, LifecycleService, SecretRef};

#[derive(Clone, Serialize, Deserialize)]
struct DummyCfg {
    note: String,
}

struct DummySecrets {
    token: String,
}

struct RotatingMockLifecycle;

#[async_trait]
impl LifecycleService for RotatingMockLifecycle {
    const NAME: &'static str = "lifecycle-mock";
    const DISPLAY: &'static str = "LifecycleMock";
    type Cfg = DummyCfg;
    type Secrets = DummySecrets;

    fn enable_in_project(_cfg: &mut ProjectConfig) {}
    fn disable_in_project(_cfg: &mut ProjectConfig) {}

    async fn validate(_cfg: &Self::Cfg, secrets: &mut Self::Secrets) -> Result<String> {
        // The whole point of the &mut signature: simulate a credential
        // rotation that the surrounding driver must honour.
        secrets.token = "ROTATED".into();
        Ok("authenticated".into())
    }

    fn store_secrets(secrets: &Self::Secrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let acct = secrets::account(Self::NAME, "token", scope);
        secrets::store(&acct, &secrets.token)?;
        Ok(vec![SecretRef {
            label: "token",
            account: acct,
            present: true,
        }])
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let acct = secrets::account(Self::NAME, "token", scope);
        secrets::delete(&acct)?;
        Ok(vec![SecretRef {
            label: "token",
            account: acct,
            present: false,
        }])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let acct = secrets::account(Self::NAME, "token", scope);
        let present = secrets::load(&acct)?.is_some();
        Ok(vec![SecretRef {
            label: "token",
            account: acct,
            present,
        }])
    }

    fn load_secrets(scope: Scope<'_>) -> Result<Option<Self::Secrets>> {
        Ok(
            secrets::load(&secrets::account(Self::NAME, "token", scope))?
                .map(|t| DummySecrets { token: t }),
        )
    }

    fn cfg_human(_cfg: &Self::Cfg) -> Vec<(&'static str, String)> {
        Vec::new()
    }
    fn cfg_json(_cfg: &Self::Cfg) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn scopes_of(_cfg: &Self::Cfg) -> &[String] {
        &[]
    }
}

#[tokio::test]
async fn validate_mutation_propagates_to_store_secrets() {
    secrets::use_memory_backend();

    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("cfg.toml");
    let cfg = DummyCfg {
        note: "smoke".into(),
    };
    let mut creds = DummySecrets {
        token: "ORIGINAL".into(),
    };

    let opts = CreateOpts {
        scope_label: "global",
        scope: Scope::Global,
        config_path: cfg_path.clone(),
        force: false,
        validate: true,
    };

    let outcome = lifecycle::create::<RotatingMockLifecycle>(&cfg, &mut creds, opts)
        .await
        .expect("create succeeds");
    assert_eq!(outcome.authenticated_as.as_deref(), Some("authenticated"));
    assert_eq!(creds.token, "ROTATED");

    // The actual invariant under test: the keychain holds the rotated
    // value, not the pre-validate one. This is exactly the property
    // that the spotifai bug report exposed in the Spotify lifecycle
    // path.
    let stored = secrets::load(&secrets::account(
        RotatingMockLifecycle::NAME,
        "token",
        Scope::Global,
    ))
    .unwrap();
    assert_eq!(stored.as_deref(), Some("ROTATED"));
}

#[tokio::test]
async fn validate_skipped_when_disabled() {
    secrets::use_memory_backend();

    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("cfg-skip.toml");
    let cfg = DummyCfg {
        note: "skip".into(),
    };
    let mut creds = DummySecrets {
        token: "ORIGINAL".into(),
    };

    let opts = CreateOpts {
        scope_label: "global",
        scope: Scope::Project("skip-validate"),
        config_path: cfg_path,
        force: false,
        validate: false,
    };

    lifecycle::create::<RotatingMockLifecycle>(&cfg, &mut creds, opts)
        .await
        .expect("create succeeds");
    // Without validate, the original token is what reaches the
    // keychain — this is the "no rotation, no surprise" guarantee.
    // (Validate is what flips the token to ROTATED, so observing the
    // original is itself proof that validate was skipped.)
    assert_eq!(creds.token, "ORIGINAL");
    let stored = secrets::load(&secrets::account(
        RotatingMockLifecycle::NAME,
        "token",
        Scope::Project("skip-validate"),
    ))
    .unwrap();
    assert_eq!(stored.as_deref(), Some("ORIGINAL"));
}
