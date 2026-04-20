//! Runtime surface of the GitHub service.
//!
//! `GithubTransport` is a trait over the shapes `src/cli/github.rs`
//! calls. The live impl ([`GhCli`]) shells out to `gh`; the dry-run
//! impl ([`DryRunGithubTransport`]) records a [`DryRunOp`] to a shared
//! sink and returns a stub success value without invoking the
//! subprocess.
//!
//! Keeping both behind the same trait means the CLI layer holds a
//! `Box<dyn GithubTransport>` and stays oblivious to `--dry-run`: the
//! preview never opens the keychain, so the flag works even before a
//! PAT has been stored.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::error::Result;
use crate::service::github::client::GhCli;
use crate::service::{DryRunOp, DryRunSink};

/// Verb surface exposed to the CLI layer. Read verbs return the raw
/// JSON stdout from `gh`; write verbs also return stdout (typically a
/// URL or a one-line confirmation) so callers can forward it unchanged.
#[async_trait]
pub trait GithubTransport: Send + Sync {
    async fn run(&self, verb: &'static str, args: &[&str]) -> Result<String>;

    /// Write-verb entry point. Same shape as `run` but also records a
    /// human summary + structured payload for dry-run transports. The
    /// live impl ignores `summary`/`details` and just runs `gh`.
    async fn run_mutating(
        &self,
        verb: &'static str,
        args: &[&str],
        summary: String,
        details: serde_json::Value,
    ) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Live transport — shells out to gh
// ---------------------------------------------------------------------------

#[async_trait]
impl GithubTransport for GhCli {
    async fn run(&self, _verb: &'static str, args: &[&str]) -> Result<String> {
        GhCli::run(self, args).await
    }

    async fn run_mutating(
        &self,
        _verb: &'static str,
        args: &[&str],
        _summary: String,
        _details: serde_json::Value,
    ) -> Result<String> {
        GhCli::run(self, args).await
    }
}

// ---------------------------------------------------------------------------
// Dry-run transport — records intent, never spawns gh
// ---------------------------------------------------------------------------

pub struct DryRunGithubTransport {
    sink: Arc<dyn DryRunSink>,
}

impl DryRunGithubTransport {
    pub fn new(sink: Arc<dyn DryRunSink>) -> Self {
        Self { sink }
    }

    fn record(&self, verb: &'static str, summary: String, details: serde_json::Value) {
        self.sink.record(DryRunOp {
            service: "github",
            verb,
            summary,
            details,
        });
    }
}

#[async_trait]
impl GithubTransport for DryRunGithubTransport {
    /// Read verbs under dry-run return an empty JSON array. Dry-run is
    /// intentionally decoupled from credentials (no token loaded, no
    /// `gh` invoked) so read verbs have no live data to return.
    /// Emitting `[]` lets piped consumers (`| jq`) keep working.
    async fn run(&self, verb: &'static str, args: &[&str]) -> Result<String> {
        self.record(
            verb,
            format!("would run `gh {}` (dry-run; no network)", args.join(" ")),
            json!({
                "command": format!("github.{verb}"),
                "gh_args": args,
            }),
        );
        Ok("[]\n".to_string())
    }

    async fn run_mutating(
        &self,
        verb: &'static str,
        args: &[&str],
        summary: String,
        details: serde_json::Value,
    ) -> Result<String> {
        let _ = args;
        self.record(verb, summary, details);
        Ok(String::new())
    }
}
