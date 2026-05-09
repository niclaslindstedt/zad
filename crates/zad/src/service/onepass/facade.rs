//! Typed library facade for 1Password (`op` shell-out).
//!
//! 1Password's auth shape is a service-account token + a sign-in
//! address (`my.1password.com` / similar). The `op` binary itself is
//! invoked under the hood; the `ZAD_OP_BINARY` env var swaps it for a
//! test shim and is the only env var the OnePass facade itself
//! consumes (the shell-out is unavoidable; the `op` binary is the
//! library). All other constructors are env-free.

use std::path::{Path, PathBuf};

use crate::config::{self, OnePassServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::onepass::client::{
    Item, ItemSummary, ListItemsFilter, OnePassClient, Vault, WhoAmI,
};
use crate::service::onepass::permissions::{self as perms, EffectivePermissions, OnePassFunction};

/// Typed library entry point for 1Password.
pub struct OnePass {
    client: OnePassClient,
    permissions: Option<EffectivePermissions>,
}

impl OnePass {
    pub fn from_default_config() -> Result<Self> {
        let (cfg, scope) = effective_config()?;
        let token = load_token(&scope)?;
        let client = OnePassClient::new(token, cfg.account);
        let permissions = perms::load_effective().ok();
        Ok(Self {
            client,
            permissions,
        })
    }

    /// Explicit service-account token + sign-in address. Reads no env
    /// vars (besides `ZAD_OP_BINARY` for the test shim, which is part
    /// of the underlying client's contract).
    pub fn with_token(token: impl Into<String>, account: impl Into<String>) -> Self {
        let client = OnePassClient::new(token.into(), account.into());
        Self {
            client,
            permissions: None,
        }
    }

    /// Fully explicit, env-free constructor (modulo `ZAD_OP_BINARY`
    /// for the test shim). Recommended for library code.
    pub fn with_paths(
        token: impl Into<String>,
        account: impl Into<String>,
        global_permissions: Option<&Path>,
        local_permissions: Option<&Path>,
    ) -> Result<Self> {
        let client = OnePassClient::new(token.into(), account.into());
        let permissions = perms::load_from(global_permissions, local_permissions)?;
        let permissions = if permissions.any() {
            Some(permissions)
        } else {
            None
        };
        Ok(Self {
            client,
            permissions,
        })
    }

    pub fn with_permissions(mut self, permissions: EffectivePermissions) -> Self {
        self.permissions = Some(permissions);
        self
    }

    pub async fn whoami(&self) -> Result<WhoAmI> {
        self.client.whoami().await
    }

    pub async fn vaults(&self) -> Result<Vec<Vault>> {
        if let Some(p) = &self.permissions {
            p.check_time(OnePassFunction::Vaults)?;
        }
        self.client.list_vaults().await
    }

    pub async fn items(&self, req: ListItemsRequest) -> Result<Vec<ItemSummary>> {
        if let Some(p) = &self.permissions {
            p.check_time(OnePassFunction::Items)?;
        }
        self.client.list_items(&req.filter).await
    }

    pub async fn get(&self, req: GetItemRequest) -> Result<Item> {
        if let Some(p) = &self.permissions {
            p.check_time(OnePassFunction::Get)?;
        }
        let item = self.client.get_item(&req.id, req.vault.as_deref()).await?;
        if let Some(p) = &self.permissions {
            p.check_get(&req.id, &item)?;
        }
        Ok(item)
    }

    pub async fn read(&self, req: ReadRequest) -> Result<String> {
        if let Some(p) = &self.permissions {
            p.check_time(OnePassFunction::Read)?;
        }
        self.client.read(&req.secret_ref).await
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ListItemsRequest {
    pub filter: ListItemsFilter,
}

impl ListItemsRequest {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_filter(filter: ListItemsFilter) -> Self {
        Self { filter }
    }
}

#[derive(Debug, Clone)]
pub struct GetItemRequest {
    pub id: String,
    pub vault: Option<String>,
}

impl GetItemRequest {
    pub fn new(id: impl Into<String>, vault: Option<String>) -> Result<Self> {
        let id = id.into();
        if id.is_empty() {
            return Err(ZadError::Invalid("item id must not be empty".into()));
        }
        Ok(Self { id, vault })
    }
}

#[derive(Debug, Clone)]
pub struct ReadRequest {
    pub secret_ref: String,
}

impl ReadRequest {
    /// `secret_ref` must be an `op://<vault>/<item>/<field>` reference.
    pub fn new(secret_ref: impl Into<String>) -> Result<Self> {
        let secret_ref = secret_ref.into();
        if !secret_ref.starts_with("op://") {
            return Err(ZadError::Invalid(format!(
                "secret reference must start with `op://`; got `{secret_ref}`"
            )));
        }
        Ok(Self { secret_ref })
    }
}

// ---------------------------------------------------------------------------
// Config / token plumbing
// ---------------------------------------------------------------------------

enum EffectiveScope {
    Global,
    Local(String),
}

fn effective_config() -> Result<(OnePassServiceCfg, EffectiveScope)> {
    let project_path = config::path::project_config_path()?;
    let project_cfg = config::load_from(&project_path)?;
    if !project_cfg.has_service("1pass") {
        return Err(ZadError::Invalid(format!(
            "1pass is not enabled for this project ({}). \
             Run `zad service enable 1pass` first.",
            project_path.display()
        )));
    }
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "1pass")?;
    if let Some(cfg) = config::load_flat::<OnePassServiceCfg>(&local_path)? {
        return Ok((cfg, EffectiveScope::Local(slug)));
    }
    let global_path = config::path::global_service_config_path("1pass")?;
    if let Some(cfg) = config::load_flat::<OnePassServiceCfg>(&global_path)? {
        return Ok((cfg, EffectiveScope::Global));
    }
    Err(ZadError::Invalid(format!(
        "no 1Password credentials found for this project.\n\
         looked in:\n  {}\n  {}",
        local_path.display(),
        global_path.display()
    )))
}

#[allow(dead_code)]
fn config_path_for(scope: &EffectiveScope) -> Result<PathBuf> {
    match scope {
        EffectiveScope::Local(slug) => config::path::project_service_config_path_for(slug, "1pass"),
        EffectiveScope::Global => config::path::global_service_config_path("1pass"),
    }
}

fn load_token(scope: &EffectiveScope) -> Result<String> {
    let account = match scope {
        EffectiveScope::Global => secrets::account("1pass", "service-account", Scope::Global),
        EffectiveScope::Local(slug) => {
            secrets::account("1pass", "service-account", Scope::Project(slug))
        }
    };
    secrets::load(&account)?.ok_or_else(|| {
        ZadError::Invalid(format!(
            "service-account token missing from keychain (account `{account}`). \
             Re-run `zad service create 1pass` to reinstall it."
        ))
    })
}
