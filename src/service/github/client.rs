//! Subprocess wrapper around the `gh` CLI.
//!
//! Every runtime verb resolves to one or more `gh` invocations. This
//! module owns the mechanics: building argv vectors, setting the env
//! (token, no-pager, no-color), spawning, capturing stdout/stderr, and
//! translating failures into `ZadError`.
//!
//! Authentication is a single Personal Access Token passed via
//! `GH_TOKEN`. `gh` also accepts `GITHUB_TOKEN`; we use `GH_TOKEN`
//! because it overrides any user-level `gh auth login` state, which is
//! the behaviour we want when the operator has deliberately set up a
//! zad-managed credential.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::Command;

use crate::error::{Result, ZadError};

/// Name of the executable we shell out to. A constant so tests that
/// want to shim `gh` can (in principle) redirect via `PATH`, and so
/// "not installed" errors can name it consistently.
pub const GH_BIN: &str = "gh";

/// Live client. Carries the token, the declared scope set (for local
/// enforcement), and the config path that produced them (for error
/// messages that name the file to edit).
#[derive(Debug, Clone)]
pub struct GhCli {
    token: String,
    scopes: BTreeSet<String>,
    config_path: PathBuf,
}

impl GhCli {
    pub fn new(token: &str, scopes: BTreeSet<String>, config_path: PathBuf) -> Self {
        Self {
            token: token.to_string(),
            scopes,
            config_path,
        }
    }

    /// Constructor for callers that only need to run *unscoped* probes
    /// (e.g. `gh api user` at credential-validation time). Skips the
    /// scope set entirely.
    pub fn unscoped(token: &str) -> Self {
        Self {
            token: token.to_string(),
            scopes: BTreeSet::new(),
            config_path: PathBuf::new(),
        }
    }

    /// Enforce the scope the caller claims before any subprocess spawn.
    /// Mirrors `DiscordHttp`'s per-call scope gate — keeps the fail
    /// path path-identical to other services'.
    pub fn require_scope(&self, scope: &'static str) -> Result<()> {
        if self.scopes.contains(scope) {
            return Ok(());
        }
        Err(ZadError::ScopeDenied {
            service: "github",
            scope,
            config_path: self.config_path.clone(),
        })
    }

    /// Run `gh` with the given args and return captured stdout as a
    /// `String` on success. On non-zero exit, wraps stderr into
    /// `ZadError::Service { name: "github", ... }`. If `gh` isn't on
    /// `PATH`, returns a clear `ZadError::Invalid` naming the binary
    /// and pointing at the install instructions.
    pub async fn run(&self, args: &[&str]) -> Result<String> {
        run_gh(&self.token, args).await
    }

    /// Unscoped run — used by the create-time validator which has a
    /// token but no stored config yet.
    pub async fn run_unscoped(token: &str, args: &[&str]) -> Result<String> {
        run_gh(token, args).await
    }

    /// Ping `gh api user` with the stored token and return the login
    /// string on success. Called by the lifecycle validator and by
    /// `zad service status github`.
    pub async fn validate_token(token: &str) -> Result<String> {
        let out = run_gh(token, &["api", "user", "--jq", ".login"]).await?;
        let login = out.trim().to_string();
        if login.is_empty() {
            return Err(ZadError::Service {
                name: "github",
                message: "gh api user returned an empty login".into(),
            });
        }
        Ok(login)
    }
}

async fn run_gh(token: &str, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new(GH_BIN);
    cmd.args(args)
        .env("GH_TOKEN", token)
        // gh prints an ANSI colour reset even under `--json`; disable
        // so piped consumers see clean output.
        .env("NO_COLOR", "1")
        // A TTY-attached `gh` pages long output; a subprocess doesn't
        // get a TTY, but setting this explicitly documents the intent
        // and matches how CI users already invoke gh.
        .env("GH_PAGER", "cat")
        .env("CLICOLOR", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ZadError::Invalid(format!(
                "`{GH_BIN}` is not installed or not on PATH. Install it from \
                 https://cli.github.com/ (brew install gh / apt install gh / \
                 scoop install gh), then retry."
            ))
        } else {
            ZadError::Service {
                name: "github",
                message: format!("failed to spawn `{GH_BIN}`: {e}"),
            }
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if stderr.is_empty() && stdout.is_empty() {
            format!("`gh {}` exited with {}", args.join(" "), output.status)
        } else if stderr.is_empty() {
            stdout
        } else {
            stderr
        };
        return Err(ZadError::Service {
            name: "github",
            message,
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
