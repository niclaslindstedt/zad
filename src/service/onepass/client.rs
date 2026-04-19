//! Thin wrapper over the 1Password `op` CLI.
//!
//! Design notes:
//!
//! - Auth is **always** a Service Account token passed via
//!   `OP_SERVICE_ACCOUNT_TOKEN` in the child process's env. The parent
//!   process never exports it; we inject on every spawn. `OP_ACCOUNT`
//!   is set to the sign-in address so `op` doesn't try to walk
//!   multiple accounts.
//! - Every call asks for `--format=json`. `op` returns stable JSON for
//!   list/get verbs; we parse into the small structs below. Fields we
//!   don't use are dropped by `#[serde(default)]` / missing-field
//!   tolerance so forward-compat changes don't break parsing.
//! - We never log argv for `create_login` (field values could contain
//!   secrets). Other commands log normally.
//! - Errors are mapped to `ZadError::Service { name: "1pass", … }` so
//!   they flow through the same surface as every other service.
//! - The `op`'s own "item not found" is distinguished from a generic
//!   failure by substring-matching stderr — this lets the CLI layer
//!   turn it into our own `NotFound`-style error without parsing the
//!   whole stderr.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::{Result, ZadError};

const SERVICE_NAME: &str = "1pass";
const OP_BINARY: &str = "op";

/// Live 1Password client. Cloneable (two small strings).
#[derive(Debug, Clone)]
pub struct OnePassClient {
    token: String,
    account: String,
    /// Resolved binary name (`"op"` unless overridden via the env var
    /// below). Tests use `ZAD_OP_BINARY` to swap in a shim.
    binary: OsString,
}

impl OnePassClient {
    /// Build a client with the given service-account token + sign-in
    /// address (e.g. `my.1password.com`).
    pub fn new(token: String, account: String) -> Self {
        let binary =
            std::env::var_os("ZAD_OP_BINARY").unwrap_or_else(|| OsStr::new(OP_BINARY).to_owned());
        Self {
            token,
            account,
            binary,
        }
    }

    /// Run `op <args>` and return `stdout` as `String`. The token and
    /// sign-in address are passed in via env. Stderr is captured and
    /// folded into the error on non-zero exit.
    async fn run(&self, args: &[&str]) -> Result<String> {
        self.run_with_stdin(args, None).await
    }

    async fn run_with_stdin(&self, args: &[&str], stdin_body: Option<&str>) -> Result<String> {
        tracing::debug!(target: "onepass", cmd = ?args, "op invocation");
        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .env("OP_SERVICE_ACCOUNT_TOKEN", &self.token)
            .env("OP_ACCOUNT", &self.account)
            // Keep plugin / biometric prompts from ever appearing in
            // an agent context.
            .env("OP_BIOMETRIC_UNLOCK_ENABLED", "false")
            .env("OP_FORMAT", "json")
            .stdin(if stdin_body.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| ZadError::Service {
            name: SERVICE_NAME,
            message: format!(
                "failed to spawn `{}`: {e} — is the 1Password CLI installed and on PATH?",
                self.binary.to_string_lossy()
            ),
        })?;

        if let Some(body) = stdin_body
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin
                .write_all(body.as_bytes())
                .await
                .map_err(|e| ZadError::Service {
                    name: SERVICE_NAME,
                    message: format!("failed to write stdin to `op`: {e}"),
                })?;
            // Closing stdin by dropping the handle — explicit for clarity.
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| ZadError::Service {
                name: SERVICE_NAME,
                message: format!("failed to wait on `op`: {e}"),
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            return Ok(stdout);
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let code = output.status.code().unwrap_or(-1);
        if is_not_found_stderr(&stderr) {
            return Err(ZadError::Service {
                name: SERVICE_NAME,
                message: format!("not found: {stderr}"),
            });
        }
        Err(ZadError::Service {
            name: SERVICE_NAME,
            message: format!("op exited with status {code}: {stderr}"),
        })
    }

    // ------------------------------------------------------------------
    // read verbs
    // ------------------------------------------------------------------

    pub async fn whoami(&self) -> Result<WhoAmI> {
        let body = self.run(&["whoami", "--format=json"]).await?;
        parse_json::<WhoAmI>(&body)
    }

    pub async fn list_vaults(&self) -> Result<Vec<Vault>> {
        let body = self.run(&["vault", "list", "--format=json"]).await?;
        parse_json::<Vec<Vault>>(&body)
    }

    pub async fn list_items(&self, filter: &ListItemsFilter) -> Result<Vec<ItemSummary>> {
        let mut argv: Vec<String> = vec!["item".into(), "list".into(), "--format=json".into()];
        if let Some(v) = filter.vault.as_deref() {
            argv.push("--vault".into());
            argv.push(v.into());
        }
        if !filter.tags.is_empty() {
            argv.push("--tags".into());
            argv.push(filter.tags.join(","));
        }
        if !filter.categories.is_empty() {
            argv.push("--categories".into());
            argv.push(filter.categories.join(","));
        }
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let body = self.run(&argv_ref).await?;
        parse_json::<Vec<ItemSummary>>(&body)
    }

    pub async fn get_item(&self, id: &str, vault: Option<&str>) -> Result<Item> {
        let mut argv: Vec<String> = vec![
            "item".into(),
            "get".into(),
            id.into(),
            "--format=json".into(),
        ];
        if let Some(v) = vault {
            argv.push("--vault".into());
            argv.push(v.into());
        }
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let body = self.run(&argv_ref).await?;
        parse_json::<Item>(&body)
    }

    /// Resolve a single `op://vault/item/field` reference. Returns the
    /// plaintext value on stdout; we trim the trailing newline that
    /// `op read` adds.
    pub async fn read(&self, secret_ref: &str) -> Result<String> {
        let body = self.run(&["read", secret_ref]).await?;
        Ok(body.trim_end_matches('\n').to_string())
    }

    /// Pipe a template through `op inject` on stdin and return the
    /// rendered output. The caller is expected to have already gated
    /// every `op://…` reference in the template through the
    /// permissions layer.
    pub async fn inject(&self, template: &str) -> Result<String> {
        self.run_with_stdin(&["inject"], Some(template)).await
    }

    // ------------------------------------------------------------------
    // write verb (scoped)
    // ------------------------------------------------------------------

    pub async fn create_item(&self, req: &CreateItemRequest) -> Result<ItemSummary> {
        let mut argv: Vec<String> = vec![
            "item".into(),
            "create".into(),
            "--format=json".into(),
            "--category".into(),
            req.category.clone(),
            "--title".into(),
            req.title.clone(),
            "--vault".into(),
            req.vault.clone(),
        ];
        if !req.tags.is_empty() {
            argv.push("--tags".into());
            argv.push(req.tags.iter().cloned().collect::<Vec<_>>().join(","));
        }
        // Each assignment of the form "label=value" creates a field in
        // the default section. `op` also supports `section.field[type]=value`
        // — we accept whatever the caller passes.
        for assignment in &req.fields {
            argv.push(assignment.clone());
        }
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();

        // Do NOT log argv for create — field values may contain secrets.
        tracing::debug!(target: "onepass", verb = "create_item", vault = %req.vault, category = %req.category, "op item create");
        let mut cmd = Command::new(&self.binary);
        cmd.args(&argv_ref)
            .env("OP_SERVICE_ACCOUNT_TOKEN", &self.token)
            .env("OP_ACCOUNT", &self.account)
            .env("OP_BIOMETRIC_UNLOCK_ENABLED", "false")
            .env("OP_FORMAT", "json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| ZadError::Service {
            name: SERVICE_NAME,
            message: format!("failed to run `op item create`: {e}"),
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ZadError::Service {
                name: SERVICE_NAME,
                message: format!(
                    "op item create failed (status {}): {stderr}",
                    output.status.code().unwrap_or(-1)
                ),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        parse_json::<ItemSummary>(&stdout)
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T> {
    serde_json::from_str::<T>(body).map_err(|e| ZadError::Service {
        name: SERVICE_NAME,
        message: format!("failed to parse op JSON: {e}"),
    })
}

/// Heuristic match for "op said the thing doesn't exist". `op`'s error
/// text varies between versions — match a small set of stable
/// fragments rather than parsing the whole output.
pub(crate) fn is_not_found_stderr(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("isn't an item")
        || s.contains("isn't a vault")
        || s.contains("doesn't exist")
        || s.contains("no item found")
        || s.contains("no vault found")
        || s.contains("no item matching")
        || s.contains("not found")
}

// ---------------------------------------------------------------------------
// request / response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ListItemsFilter {
    pub vault: Option<String>,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
}

/// `op item create` invocation request. `fields` is a list of raw
/// assignments passed to `op` verbatim (e.g. `"username=bot"`,
/// `"password=[generate]"`, `"section.token[password]=…"`).
#[derive(Debug, Clone)]
pub struct CreateItemRequest {
    pub title: String,
    pub vault: String,
    pub category: String,
    pub tags: BTreeSet<String>,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoAmI {
    #[serde(default, rename = "URL")]
    pub url: String,
    #[serde(default, rename = "ServiceAccountType")]
    pub service_account_type: String,
    #[serde(default, rename = "UserUUID")]
    pub user_uuid: String,
    #[serde(default, rename = "AccountUUID")]
    pub account_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vault {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub content_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub vault: VaultRef,
    #[serde(default, rename = "updated_at")]
    pub updated_at: Option<String>,
    #[serde(default, rename = "created_at")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultRef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub vault: VaultRef,
    #[serde(default)]
    pub fields: Vec<ItemField>,
    #[serde(default)]
    pub sections: Vec<ItemSection>,
    #[serde(default, rename = "updated_at")]
    pub updated_at: Option<String>,
    #[serde(default, rename = "created_at")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemField {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub section: Option<SectionRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionRef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemSection {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// Parsed `op://vault/item/field` reference. Returned by
/// [`parse_op_ref`] so the permissions layer can gate individual
/// fragments before the template ever reaches `op inject`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOpRef {
    pub source: String,
    pub vault: String,
    pub item: String,
    pub field: String,
    /// Empty when the reference has the standard 4 segments; for
    /// section-qualified refs (`op://v/i/s/f`) this carries the
    /// section name.
    pub section: Option<String>,
}

/// Parse a single `op://…` reference. Returns `Err` on obvious shape
/// violations — the error message is propagated back to the user so
/// they can fix their template.
pub fn parse_op_ref(raw: &str) -> Result<ParsedOpRef> {
    let stripped = raw
        .strip_prefix("op://")
        .ok_or_else(|| ZadError::Invalid(format!("`{raw}` is not an op:// reference")))?;
    let parts: Vec<&str> = stripped.split('/').collect();
    match parts.as_slice() {
        [vault, item, field] => Ok(ParsedOpRef {
            source: raw.to_string(),
            vault: (*vault).to_string(),
            item: (*item).to_string(),
            field: (*field).to_string(),
            section: None,
        }),
        [vault, item, section, field] => Ok(ParsedOpRef {
            source: raw.to_string(),
            vault: (*vault).to_string(),
            item: (*item).to_string(),
            field: (*field).to_string(),
            section: Some((*section).to_string()),
        }),
        _ => Err(ZadError::Invalid(format!(
            "`{raw}` is not a valid op:// reference \
             (expected op://<vault>/<item>/<field> or op://<vault>/<item>/<section>/<field>)"
        ))),
    }
}

/// Walk a template body and return every `op://…` reference it
/// contains. Greedy — refs can appear inside strings, after `=`, etc.
pub fn scan_op_refs(body: &str) -> Vec<ParsedOpRef> {
    // Cheap hand-rolled scanner: look for the literal "op://" prefix
    // and consume until the next whitespace, quote, or end-of-line.
    // That matches both `"op://vault/item/field"` (JSON) and bare
    // `op://…` (env-file) shapes.
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find("op://") {
        let (_, tail) = rest.split_at(pos);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`')
            .unwrap_or(tail.len());
        let raw = &tail[..end];
        if let Ok(parsed) = parse_op_ref(raw) {
            out.push(parsed);
        }
        rest = &tail[end..];
    }
    out
}
