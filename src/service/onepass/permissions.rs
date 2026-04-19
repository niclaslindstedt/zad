//! 1Password permissions policy.
//!
//! Read-side checks are **filters**: anything out of scope is presented
//! to the agent as if it doesn't exist. That means:
//!
//! - `filter_vaults` strips vaults that don't pass `[vaults]`.
//! - `filter_items` strips items whose vault, tags, title, or category
//!   don't pass `[items]` (or the corresponding top-level defaults).
//! - `filter_tags` keeps only tags that appear on at least one visible
//!   item.
//! - `check_get` on a hidden item returns a `NotFound`-shaped service
//!   error matching `op`'s own "no item found" output.
//! - `check_read` on a hidden vault/item/field returns the same shape.
//! - `check_inject_refs` gates every `op://…` reference in a template
//!   through `check_read`, failing the whole call if any is hidden.
//! - `filter_fields` strips denied fields from an `Item` the agent
//!   otherwise can see — field labels and types stay (so the agent
//!   knows the field exists), but `value` is dropped.
//!
//! Write-side (`check_create`) is **not** a filter — creation failures
//! raise `PermissionDenied` naming the rule. `[create]` is also
//! treated as deny-by-default: if no `[create].vaults.allow` entry
//! covers the requested vault, the call is rejected, even if the
//! vault is allowed for reads.
//!
//! Files:
//! - global: `~/.zad/services/1pass/permissions.toml`
//! - local:  `~/.zad/projects/<slug>/services/1pass/permissions.toml`
//!
//! Both files intersect strictly; local can only tighten global.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::{Result, ZadError};
use crate::permissions::{
    content::{ContentRules, ContentRulesRaw},
    pattern::{DenyReason, PatternList, PatternListRaw},
    service::HasSignature,
    signing::{self, Signature, SigningKey},
    time::{TimeWindow, TimeWindowRaw},
};
use crate::service::onepass::client::{Item, ItemField, ItemSummary, ParsedOpRef, Vault};

const SERVICE_NAME: &str = "1pass";

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnePassPermissionsRaw {
    // Top-level defaults applied on every read-side verb unless the
    // per-verb block overrides. The five target axes do NOT merge
    // across layers within one file — a per-verb list replaces the
    // top-level list for that axis. Global ∩ local still intersect at
    // the outer layer.
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub vaults: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub tags: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub items: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub categories: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub fields: PatternListRaw,

    #[serde(default, skip_serializing_if = "ContentRulesRaw_is_default")]
    pub content: ContentRulesRaw,
    #[serde(default, skip_serializing_if = "TimeWindowRaw_is_default")]
    pub time: TimeWindowRaw,

    #[serde(
        default,
        rename = "vaults_cmd",
        skip_serializing_if = "FunctionBlockRaw_is_default"
    )]
    pub vaults_verb: FunctionBlockRaw,
    #[serde(default, skip_serializing_if = "FunctionBlockRaw_is_default")]
    pub items_verb: FunctionBlockRaw,
    #[serde(default, skip_serializing_if = "FunctionBlockRaw_is_default")]
    pub tags_verb: FunctionBlockRaw,
    #[serde(default, skip_serializing_if = "FunctionBlockRaw_is_default")]
    pub get: FunctionBlockRaw,
    #[serde(default, skip_serializing_if = "FunctionBlockRaw_is_default")]
    pub read: FunctionBlockRaw,
    #[serde(default, skip_serializing_if = "FunctionBlockRaw_is_default")]
    pub inject: FunctionBlockRaw,

    /// The `[create]` block is special: missing OR empty `vaults.allow`
    /// denies every create call. Operators must explicitly enumerate
    /// the vault(s) the agent may write to.
    #[serde(default, skip_serializing_if = "CreateBlockRaw_is_default")]
    pub create: CreateBlockRaw,

    /// Ed25519 signature over the canonical serialization of every
    /// other field. See [`crate::permissions::signing`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl HasSignature for OnePassPermissionsRaw {
    fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }
    fn set_signature(&mut self, sig: Option<Signature>) {
        self.signature = sig;
    }
}

/// Per-verb read-side block. Any field that's absent falls back to the
/// corresponding top-level default. Treat an empty per-verb list as
/// "inherit from top-level"; a non-empty list replaces the top-level
/// value for that axis on this verb.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionBlockRaw {
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub vaults: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub tags: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub items: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub categories: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub fields: PatternListRaw,
    #[serde(default, skip_serializing_if = "ContentRulesRaw_is_default")]
    pub content: ContentRulesRaw,
    #[serde(default, skip_serializing_if = "TimeWindowRaw_is_default")]
    pub time: TimeWindowRaw,
}

/// `[create]` block — deny-by-default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateBlockRaw {
    /// Vaults the agent may create new items in. Empty / missing →
    /// every `create` call is denied.
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub vaults: PatternListRaw,
    /// Categories the agent may create. Empty → any category is OK
    /// (subject to `op`'s own schema); non-empty → the requested
    /// category must match.
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub categories: PatternListRaw,
    /// If non-empty, every created item must carry at least one
    /// matching tag from the allow list (denies still fire normally).
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub tags: PatternListRaw,
    /// If non-empty, every created item's title must match.
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub titles: PatternListRaw,
    #[serde(default, skip_serializing_if = "TimeWindowRaw_is_default")]
    pub time: TimeWindowRaw,
}

#[allow(non_snake_case)]
fn PatternListRaw_is_default(v: &PatternListRaw) -> bool {
    v.allow.is_empty() && v.deny.is_empty()
}
#[allow(non_snake_case)]
fn ContentRulesRaw_is_default(v: &ContentRulesRaw) -> bool {
    v.deny_words.is_empty() && v.deny_patterns.is_empty() && v.max_length.is_none()
}
#[allow(non_snake_case)]
fn TimeWindowRaw_is_default(v: &TimeWindowRaw) -> bool {
    v.days.is_empty() && v.windows.is_empty()
}
#[allow(non_snake_case)]
fn FunctionBlockRaw_is_default(v: &FunctionBlockRaw) -> bool {
    PatternListRaw_is_default(&v.vaults)
        && PatternListRaw_is_default(&v.tags)
        && PatternListRaw_is_default(&v.items)
        && PatternListRaw_is_default(&v.categories)
        && PatternListRaw_is_default(&v.fields)
        && ContentRulesRaw_is_default(&v.content)
        && TimeWindowRaw_is_default(&v.time)
}
#[allow(non_snake_case)]
fn CreateBlockRaw_is_default(v: &CreateBlockRaw) -> bool {
    PatternListRaw_is_default(&v.vaults)
        && PatternListRaw_is_default(&v.categories)
        && PatternListRaw_is_default(&v.tags)
        && PatternListRaw_is_default(&v.titles)
        && TimeWindowRaw_is_default(&v.time)
}

// ---------------------------------------------------------------------------
// compiled form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FunctionBlock {
    pub vaults: PatternList,
    pub tags: PatternList,
    pub items: PatternList,
    pub categories: PatternList,
    pub fields: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            vaults: PatternList::compile(&raw.vaults).map_err(ZadError::Invalid)?,
            tags: PatternList::compile(&raw.tags).map_err(ZadError::Invalid)?,
            items: PatternList::compile(&raw.items).map_err(ZadError::Invalid)?,
            categories: PatternList::compile(&raw.categories).map_err(ZadError::Invalid)?,
            fields: PatternList::compile(&raw.fields).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct CreateBlock {
    pub vaults: PatternList,
    pub categories: PatternList,
    pub tags: PatternList,
    pub titles: PatternList,
    pub time: TimeWindow,
    /// `true` iff the raw `[create]` section was present with an
    /// explicit vaults allow list. Used to surface a clearer error
    /// when an operator forgot to configure `[create]` entirely.
    pub vaults_configured: bool,
}

impl CreateBlock {
    fn compile(raw: &CreateBlockRaw) -> Result<Self> {
        Ok(CreateBlock {
            vaults: PatternList::compile(&raw.vaults).map_err(ZadError::Invalid)?,
            categories: PatternList::compile(&raw.categories).map_err(ZadError::Invalid)?,
            tags: PatternList::compile(&raw.tags).map_err(ZadError::Invalid)?,
            titles: PatternList::compile(&raw.titles).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            vaults_configured: !raw.vaults.allow.is_empty(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct OnePassPermissions {
    pub source: PathBuf,
    pub vaults: PatternList,
    pub tags: PatternList,
    pub items: PatternList,
    pub categories: PatternList,
    pub fields: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub vaults_verb: FunctionBlock,
    pub items_verb: FunctionBlock,
    pub tags_verb: FunctionBlock,
    pub get: FunctionBlock,
    pub read: FunctionBlock,
    pub inject: FunctionBlock,
    pub create: CreateBlock,
}

impl OnePassPermissions {
    fn compile(raw: &OnePassPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(OnePassPermissions {
            source,
            vaults: PatternList::compile(&raw.vaults).map_err(ZadError::Invalid)?,
            tags: PatternList::compile(&raw.tags).map_err(ZadError::Invalid)?,
            items: PatternList::compile(&raw.items).map_err(ZadError::Invalid)?,
            categories: PatternList::compile(&raw.categories).map_err(ZadError::Invalid)?,
            fields: PatternList::compile(&raw.fields).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            vaults_verb: FunctionBlock::compile(&raw.vaults_verb)?,
            items_verb: FunctionBlock::compile(&raw.items_verb)?,
            tags_verb: FunctionBlock::compile(&raw.tags_verb)?,
            get: FunctionBlock::compile(&raw.get)?,
            read: FunctionBlock::compile(&raw.read)?,
            inject: FunctionBlock::compile(&raw.inject)?,
            create: CreateBlock::compile(&raw.create)?,
        })
    }

    fn block(&self, f: OnePassFunction) -> &FunctionBlock {
        match f {
            OnePassFunction::Vaults => &self.vaults_verb,
            OnePassFunction::Items => &self.items_verb,
            OnePassFunction::Tags => &self.tags_verb,
            OnePassFunction::Get => &self.get,
            OnePassFunction::Read => &self.read,
            OnePassFunction::Inject => &self.inject,
        }
    }

    /// Effective vault patterns for a verb. An empty per-verb list
    /// falls back to the top-level default.
    fn axis_vaults(&self, f: OnePassFunction) -> &PatternList {
        let b = self.block(f);
        if b.vaults.is_empty() {
            &self.vaults
        } else {
            &b.vaults
        }
    }
    fn axis_tags(&self, f: OnePassFunction) -> &PatternList {
        let b = self.block(f);
        if b.tags.is_empty() {
            &self.tags
        } else {
            &b.tags
        }
    }
    fn axis_items(&self, f: OnePassFunction) -> &PatternList {
        let b = self.block(f);
        if b.items.is_empty() {
            &self.items
        } else {
            &b.items
        }
    }
    fn axis_categories(&self, f: OnePassFunction) -> &PatternList {
        let b = self.block(f);
        if b.categories.is_empty() {
            &self.categories
        } else {
            &b.categories
        }
    }
    fn axis_fields(&self, f: OnePassFunction) -> &PatternList {
        let b = self.block(f);
        if b.fields.is_empty() {
            &self.fields
        } else {
            &b.fields
        }
    }
}

/// One per runtime verb. `Vaults`, `Items`, `Tags` are **read-side
/// list** verbs; `Get` is a single-item metadata fetch; `Read` is a
/// single-field value fetch; `Inject` is bulk substitution. `Create`
/// is the only write verb and lives on its own axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnePassFunction {
    Vaults,
    Items,
    Tags,
    Get,
    Read,
    Inject,
}

impl OnePassFunction {
    pub fn name(self) -> &'static str {
        match self {
            OnePassFunction::Vaults => "vaults",
            OnePassFunction::Items => "items",
            OnePassFunction::Tags => "tags",
            OnePassFunction::Get => "get",
            OnePassFunction::Read => "read",
            OnePassFunction::Inject => "inject",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "vaults" => OnePassFunction::Vaults,
            "items" => OnePassFunction::Items,
            "tags" => OnePassFunction::Tags,
            "get" => OnePassFunction::Get,
            "read" => OnePassFunction::Read,
            "inject" => OnePassFunction::Inject,
            "create" => {
                return Err(ZadError::Invalid(
                    "use `--function create` via the explicit create-check flow; \
                     read-side checks use vaults/items/tags/get/read/inject"
                        .into(),
                ));
            }
            other => {
                return Err(ZadError::Invalid(format!(
                    "unknown 1pass function `{other}`; expected one of \
                     vaults, items, tags, get, read, inject"
                )));
            }
        })
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<OnePassPermissions>,
    pub local: Option<OnePassPermissions>,
}

impl EffectivePermissions {
    pub fn any(&self) -> bool {
        self.global.is_some() || self.local.is_some()
    }

    pub fn sources(&self) -> Vec<&Path> {
        let mut out: Vec<&Path> = vec![];
        if let Some(g) = &self.global {
            out.push(&g.source);
        }
        if let Some(l) = &self.local {
            out.push(&l.source);
        }
        out
    }

    fn layers(&self) -> impl Iterator<Item = &OnePassPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    /// Time-window gate for a read-side verb.
    pub fn check_time(&self, f: OnePassFunction) -> Result<()> {
        for p in self.layers() {
            let merged = p.time.clone().merge(p.block(f).time.clone());
            if let Err(e) = merged.evaluate_now() {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// `true` when every layer admits `vault` for `f`. Used by the
    /// filter helpers that must stay silent on denial.
    fn vault_admitted(&self, f: OnePassFunction, vault_name: &str, vault_id: &str) -> bool {
        let aliases = aliases_for(vault_name, vault_id);
        let refs = as_refs(&aliases);
        self.layers()
            .all(|p| p.axis_vaults(f).evaluate(refs.iter().copied()).is_ok())
    }

    fn item_admitted(&self, f: OnePassFunction, item: &ItemSummaryLike) -> bool {
        if !self.vault_admitted(f, &item.vault_name, &item.vault_id) {
            return false;
        }
        let item_aliases = aliases_for(&item.title, &item.id);
        let item_refs = as_refs(&item_aliases);
        for p in self.layers() {
            if p.axis_items(f).evaluate(item_refs.iter().copied()).is_err() {
                return false;
            }
            if !p.axis_categories(f).is_empty()
                && p.axis_categories(f)
                    .evaluate([item.category.as_str()])
                    .is_err()
            {
                return false;
            }
            // Tag axis: an item is admitted when no tag is denied AND
            // either the axis has no allow list or at least one tag
            // matches.
            let tag_axis = p.axis_tags(f);
            if !tag_axis.is_empty() {
                let ok = evaluate_tags(tag_axis, &item.tags);
                if !ok {
                    return false;
                }
            }
        }
        true
    }

    /// Keep only the vaults visible under `[vaults]` at every layer.
    pub fn filter_vaults(&self, vaults: Vec<Vault>) -> Vec<Vault> {
        if !self.any() {
            return vaults;
        }
        vaults
            .into_iter()
            .filter(|v| self.vault_admitted(OnePassFunction::Vaults, &v.name, &v.id))
            .collect()
    }

    /// Keep only items visible under `[items]` at every layer.
    pub fn filter_items(&self, items: Vec<ItemSummary>) -> Vec<ItemSummary> {
        if !self.any() {
            return items;
        }
        items
            .into_iter()
            .filter(|it| {
                self.item_admitted(OnePassFunction::Items, &ItemSummaryLike::from_summary(it))
            })
            .collect()
    }

    /// Return the distinct tags that appear on at least one visible
    /// item. `items` is expected to be the already-visible set (caller
    /// ran `filter_items` first); we additionally gate each tag
    /// through the `[tags]` axis so an allow-list on tags is honored.
    pub fn filter_tags(&self, visible_items: &[ItemSummary]) -> Vec<String> {
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for it in visible_items {
            for t in &it.tags {
                seen.insert(t.clone());
            }
        }
        if !self.any() {
            return seen.into_iter().collect();
        }
        seen.into_iter()
            .filter(|t| {
                self.layers().all(|p| {
                    p.axis_tags(OnePassFunction::Tags)
                        .evaluate([t.as_str()])
                        .is_ok()
                })
            })
            .collect()
    }

    /// Gate a resolved `Item` for the `get` verb. Returns `NotFound`
    /// (shaped as a `ZadError::Service`) when hidden — indistinguishable
    /// from the item genuinely not existing.
    pub fn check_get(&self, input: &str, item: &Item) -> Result<()> {
        if !self.item_admitted(OnePassFunction::Get, &ItemSummaryLike::from_item(item)) {
            return Err(not_found(input));
        }
        Ok(())
    }

    /// Return a clone of `item` with every denied field dropped. Empty
    /// result is legal — the agent still sees the item metadata.
    pub fn filter_fields(&self, item: Item) -> Item {
        if !self.any() {
            return item;
        }
        let mut out = item;
        out.fields
            .retain(|f| self.field_admitted(OnePassFunction::Get, f));
        out
    }

    fn field_admitted(&self, f: OnePassFunction, field: &ItemField) -> bool {
        let aliases = aliases_for(&field.label, &field.id);
        let refs = as_refs(&aliases);
        self.layers()
            .all(|p| p.axis_fields(f).evaluate(refs.iter().copied()).is_ok())
    }

    /// Gate a single `op://…` reference. Returns `NotFound` on any
    /// hidden axis (vault / item / field). The caller supplies the
    /// resolved `Item` (which we need for tag + category gating).
    pub fn check_read(&self, reference: &str, resolved: &Item, field: &str) -> Result<()> {
        if !self.item_admitted(OnePassFunction::Read, &ItemSummaryLike::from_item(resolved)) {
            return Err(not_found(reference));
        }
        let synthetic = ItemField {
            id: field.to_string(),
            label: field.to_string(),
            field_type: String::new(),
            purpose: None,
            value: None,
            section: None,
        };
        if !self.field_admitted(OnePassFunction::Read, &synthetic) {
            return Err(not_found(reference));
        }
        Ok(())
    }

    /// Gate a parsed reference by vault+item name only (no resolved
    /// `Item` available yet — used by `inject` pre-scan). Only checks
    /// the vault axis + the item *name* axis + the field axis. The
    /// full (tag/category) gate runs at `op` invocation time via the
    /// client, but reference-level pre-scan is enough to stop an
    /// agent from learning about items that exist in vaults they
    /// don't have read access to.
    pub fn check_inject_ref(&self, r: &ParsedOpRef) -> Result<()> {
        let vault_aliases = aliases_for(&r.vault, &r.vault);
        let vault_refs = as_refs(&vault_aliases);
        let item_aliases = aliases_for(&r.item, &r.item);
        let item_refs = as_refs(&item_aliases);
        let field_refs: [&str; 1] = [r.field.as_str()];
        for p in self.layers() {
            if p.axis_vaults(OnePassFunction::Inject)
                .evaluate(vault_refs.iter().copied())
                .is_err()
            {
                return Err(not_found(&r.source));
            }
            if p.axis_items(OnePassFunction::Inject)
                .evaluate(item_refs.iter().copied())
                .is_err()
            {
                return Err(not_found(&r.source));
            }
            if p.axis_fields(OnePassFunction::Inject)
                .evaluate(field_refs.iter().copied())
                .is_err()
            {
                return Err(not_found(&r.source));
            }
        }
        Ok(())
    }

    /// Gate an injected template's rendered body against `[content]` /
    /// `[inject].content` rules. Returns `PermissionDenied` because by
    /// the time we're running this the template was legal — the user
    /// knows what they sent.
    pub fn check_inject_body(&self, body: &str) -> Result<()> {
        for p in self.layers() {
            let merged = p.content.clone().merge(p.inject.content.clone());
            if let Err(e) = merged.evaluate(body) {
                return Err(ZadError::PermissionDenied {
                    function: OnePassFunction::Inject.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Gate a create request. `PermissionDenied` (not `NotFound`) —
    /// writes never pretend their target doesn't exist.
    pub fn check_create(
        &self,
        vault: &str,
        category: &str,
        title: &str,
        tags: &[String],
    ) -> Result<()> {
        if !self.any() {
            // No policy file at all — default deny on writes. The user
            // must opt in explicitly by writing a permissions.toml with
            // a `[create]` block.
            return Err(ZadError::PermissionDenied {
                function: "create",
                reason: "no permissions file configured — `create` is deny-by-default; \
                         run `zad 1pass permissions init` and edit the `[create]` section"
                    .into(),
                config_path: global_path().unwrap_or_default(),
            });
        }
        for p in self.layers() {
            let c = &p.create;
            // Time window intersects global-level `[time]` too.
            let merged_time = p.time.clone().merge(c.time.clone());
            if let Err(e) = merged_time.evaluate_now() {
                return Err(ZadError::PermissionDenied {
                    function: "create",
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
            // Deny-by-default: vaults.allow MUST be explicitly set.
            if !c.vaults_configured {
                return Err(ZadError::PermissionDenied {
                    function: "create",
                    reason: "`[create].vaults.allow` is empty — add the vault name(s) this \
                             agent may create items in"
                        .into(),
                    config_path: p.source.clone(),
                });
            }
            if let Err(e) = c.vaults.evaluate([vault]) {
                return Err(ZadError::PermissionDenied {
                    function: "create",
                    reason: deny_to_sentence(&e, &format!("vault `{vault}`")),
                    config_path: p.source.clone(),
                });
            }
            if !c.categories.is_empty()
                && let Err(e) = c.categories.evaluate([category])
            {
                return Err(ZadError::PermissionDenied {
                    function: "create",
                    reason: deny_to_sentence(&e, &format!("category `{category}`")),
                    config_path: p.source.clone(),
                });
            }
            if !c.titles.is_empty()
                && let Err(e) = c.titles.evaluate([title])
            {
                return Err(ZadError::PermissionDenied {
                    function: "create",
                    reason: deny_to_sentence(&e, &format!("title `{title}`")),
                    config_path: p.source.clone(),
                });
            }
            if !c.tags.is_empty() {
                if tags.is_empty() {
                    return Err(ZadError::PermissionDenied {
                        function: "create",
                        reason: "`[create].tags.allow` is non-empty but the request carries \
                                 no tags; add `--tag <name>`"
                            .into(),
                        config_path: p.source.clone(),
                    });
                }
                let matched = tags.iter().any(|t| c.tags.evaluate([t.as_str()]).is_ok());
                if !matched {
                    return Err(ZadError::PermissionDenied {
                        function: "create",
                        reason: format!(
                            "none of the requested tags {tags:?} match `[create].tags.allow`"
                        ),
                        config_path: p.source.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Build the alias set a pattern list is evaluated against. We include
/// both the human-facing label and the UUID form; callers that only
/// have one form pass it through both slots. Returned as a pair of
/// owned `String`s so the caller can borrow them as `&str` for
/// `PatternList::evaluate`.
fn aliases_for(name: &str, id: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if id != name {
        out.push(id.to_string());
    }
    out.sort();
    out.dedup();
    out
}

/// Borrow an owned-string alias vec as a `Vec<&str>` suitable for
/// `PatternList::evaluate`.
fn as_refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn evaluate_tags(list: &PatternList, tags: &[String]) -> bool {
    // Hand the whole tag set to the pattern list so deny-wins and
    // "at least one allow match" semantics match the rest of the
    // codebase. An item with zero tags fails a non-empty allow list.
    let iter = tags.iter().map(String::as_str);
    list.evaluate(iter).is_ok()
}

fn deny_to_sentence(reason: &DenyReason, target_label: &str) -> String {
    reason.as_sentence(target_label)
}

/// Shape `ZadError::Service` the same way `op` does when an item isn't
/// found: the caller can't tell whether the target genuinely doesn't
/// exist or is just hidden by policy.
fn not_found(input: &str) -> ZadError {
    ZadError::Service {
        name: SERVICE_NAME,
        message: format!("\"{input}\" isn't an item in any vault visible to this account"),
    }
}

/// Smaller shape used by `item_admitted` so both `ItemSummary` and
/// `Item` can feed through the same gate.
struct ItemSummaryLike {
    id: String,
    title: String,
    category: String,
    tags: Vec<String>,
    vault_id: String,
    vault_name: String,
}

impl ItemSummaryLike {
    fn from_summary(it: &ItemSummary) -> Self {
        Self {
            id: it.id.clone(),
            title: it.title.clone(),
            category: it.category.clone(),
            tags: it.tags.clone(),
            vault_id: it.vault.id.clone(),
            vault_name: it.vault.name.clone(),
        }
    }
    fn from_item(it: &Item) -> Self {
        Self {
            id: it.id.clone(),
            title: it.title.clone(),
            category: it.category.clone(),
            tags: it.tags.clone(),
            vault_id: it.vault.id.clone(),
            vault_name: it.vault.name.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// paths + load
// ---------------------------------------------------------------------------

pub fn global_path() -> Result<PathBuf> {
    Ok(config::path::global_service_dir(SERVICE_NAME)?.join("permissions.toml"))
}

pub fn local_path_for(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, SERVICE_NAME)?.join("permissions.toml"))
}

pub fn local_path_current() -> Result<PathBuf> {
    local_path_for(&config::path::project_slug()?)
}

pub fn load_file(path: &Path) -> Result<Option<OnePassPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: OnePassPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    signing::verify_raw(&raw, path)?;
    let compiled = OnePassPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

/// Read a file's raw policy (signature included) without compiling.
pub fn load_raw_file(path: &Path) -> Result<Option<OnePassPermissionsRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: OnePassPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(Some(raw))
}

fn wrap_compile_error(err: ZadError, path: &Path) -> ZadError {
    match err {
        ZadError::Invalid(msg) => ZadError::Invalid(format!(
            "invalid permissions file {}: {msg}",
            path.display()
        )),
        other => other,
    }
}

pub fn load_effective() -> Result<EffectivePermissions> {
    let slug = config::path::project_slug()?;
    load_effective_for(&slug)
}

pub fn load_effective_for(slug: &str) -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_for(slug)?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(path: &Path, raw: &OnePassPermissionsRaw, key: &SigningKey) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let sig = signing::sign_raw(&to_write, key)?;
    to_write.set_signature(Some(sig));
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Write `raw` without signing. Staging-only.
pub fn save_unsigned(path: &Path, raw: &OnePassPermissionsRaw) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Starter template. Read-side is wide-open; write-side (`[create]`)
/// is narrowly scoped to an `AgentWork` vault with mandatory tags —
/// operators can loosen in place.
pub fn starter_template() -> OnePassPermissionsRaw {
    OnePassPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into()],
            deny_patterns: vec![],
            max_length: Some(50_000),
        },
        fields: PatternListRaw {
            allow: vec![],
            deny: vec!["notesPlain".into(), "recovery_code".into()],
        },
        create: CreateBlockRaw {
            vaults: PatternListRaw {
                allow: vec!["AgentWork".into()],
                deny: vec![],
            },
            categories: PatternListRaw {
                allow: vec![
                    "Login".into(),
                    "API Credential".into(),
                    "Secure Note".into(),
                ],
                deny: vec![],
            },
            tags: PatternListRaw {
                allow: vec!["agent-managed".into()],
                deny: vec![],
            },
            ..CreateBlockRaw::default()
        },
        ..OnePassPermissionsRaw::default()
    }
}

// ---------------------------------------------------------------------------
// PermissionsService binding
// ---------------------------------------------------------------------------

/// Zero-sized type used to feed the shared permissions runner with
/// 1Password-specific bindings. See
/// [`crate::permissions::service::PermissionsService`].
pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = SERVICE_NAME;
    type Raw = OnePassPermissionsRaw;

    fn starter_template() -> Self::Raw {
        starter_template()
    }

    fn all_functions() -> &'static [&'static str] {
        &["vaults", "items", "tags", "get", "read", "inject", "create"]
    }

    fn target_kinds() -> &'static [&'static str] {
        &["vault", "item", "tag", "category", "field"]
    }
}
