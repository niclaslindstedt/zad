//! `zad 1pass <verb>` — runtime surface for the 1Password service.
//!
//! Every read-side verb runs the permission layer **before** calling
//! `op`, and the filter helpers strip out-of-scope targets so they
//! look as if they don't exist. `get`/`read` on a hidden target
//! return the same "no item found" shape the real `op` returns for a
//! missing item.
//!
//! `create` is the only write verb; it always surfaces
//! `PermissionDenied` (not `NotFound`) so the agent learns why its
//! write was refused.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::lifecycle::leak;
use crate::config::{self, OnePassServiceCfg};
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};
use crate::service::onepass::client::{
    CreateItemRequest, Item, ItemSummary, ListItemsFilter, OnePassClient, ParsedOpRef, Vault,
    parse_op_ref, scan_op_refs,
};
use crate::service::onepass::permissions::{self as perms, EffectivePermissions, OnePassFunction};

// ---------------------------------------------------------------------------
// top-level args
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct OnePassArgs {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List the vaults this account can see (filtered by policy).
    Vaults(VaultsArgs),
    /// List items (filtered by vault, tags, category, and policy).
    Items(ItemsArgs),
    /// List distinct tags across visible items.
    Tags(TagsArgs),
    /// Fetch metadata for one item. Fields are filtered by policy —
    /// labels/types stay visible, `value` on denied fields is dropped.
    Get(GetArgs),
    /// Resolve a single `op://vault/item/field` reference.
    Read(ReadArgs),
    /// Substitute every `op://…` reference in a template. Each ref
    /// is gated through the same policy as `read` before `op inject`
    /// runs, so the whole call aborts if any ref is hidden.
    Inject(InjectArgs),
    /// Create a new item. Gated by the deny-by-default `[create]`
    /// block; the agent must be explicitly allowed in a vault.
    Create(CreateItemArgs),
    /// Confirm the stored credentials work.
    Whoami(WhoamiArgs),
    /// Inspect or scaffold the permissions policy.
    Permissions(PermissionsArgs),
}

#[derive(Debug, Args)]
pub struct VaultsArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ItemsArgs {
    /// Filter to one vault by name or UUID. Default: every visible vault.
    #[arg(long)]
    pub vault: Option<String>,
    /// Filter to items carrying any of these tags (repeatable).
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Filter to items in any of these categories (repeatable).
    #[arg(long = "category")]
    pub categories: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TagsArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct GetArgs {
    /// Item title or UUID.
    pub item: String,
    #[arg(long)]
    pub vault: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReadArgs {
    /// A single `op://vault/item/field` reference.
    pub reference: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct InjectArgs {
    /// Path to the template file. Use `-` for stdin.
    #[arg(long = "in", default_value = "-")]
    pub input: String,
    /// Optional output path. Defaults to stdout.
    #[arg(long = "out")]
    pub output: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CreateItemArgs {
    #[arg(long)]
    pub title: String,
    #[arg(long)]
    pub vault: String,
    #[arg(long, default_value = "Login")]
    pub category: String,
    /// Repeatable. When `[create].tags.allow` is non-empty, at least
    /// one of these must match.
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Raw `op item create` field assignments
    /// (`username=bot`, `password=…`, `section.key[password]=…`).
    /// Repeatable.
    #[arg(long = "field")]
    pub fields: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WhoamiArgs {
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// permissions subgroup
// ---------------------------------------------------------------------------

#[derive(Debug, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum PermissionsAction {
    /// Print the effective policy (both file paths + bodies).
    Show(PermissionsShowArgs),
    /// Print the two candidate file paths, one per line.
    Path(PermissionsPathArgs),
    /// Write a starter policy to the selected scope.
    Init(PermissionsInitArgs),
    /// Dry-run a permissions check without hitting the network.
    Check(PermissionsCheckArgs),
    /// Staged-commit workflow: queue mutations in a `.pending` file and
    /// only sign on `commit`. See `cli::permissions`.
    #[command(flatten)]
    Staging(crate::cli::permissions::StagingAction),
}

#[derive(Debug, Args)]
pub struct PermissionsShowArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsPathArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsInitArgs {
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PermissionsCheckArgs {
    /// One of: `vaults`, `items`, `tags`, `get`, `read`, `inject`, `create`.
    #[arg(long)]
    pub function: String,
    #[arg(long)]
    pub vault: Option<String>,
    #[arg(long)]
    pub item: Option<String>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long)]
    pub category: Option<String>,
    #[arg(long)]
    pub field: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub reference: Option<String>,
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// dispatch
// ---------------------------------------------------------------------------

pub async fn run(args: OnePassArgs) -> Result<()> {
    match args.action {
        Action::Vaults(a) => run_vaults(a).await,
        Action::Items(a) => run_items(a).await,
        Action::Tags(a) => run_tags(a).await,
        Action::Get(a) => run_get(a).await,
        Action::Read(a) => run_read(a).await,
        Action::Inject(a) => run_inject(a).await,
        Action::Create(a) => run_create(a).await,
        Action::Whoami(a) => run_whoami(a).await,
        Action::Permissions(a) => run_permissions(a),
    }
}

// ---------------------------------------------------------------------------
// verbs
// ---------------------------------------------------------------------------

async fn run_vaults(args: VaultsArgs) -> Result<()> {
    let client = client_for(&["read"])?;
    let permissions = perms::load_effective()?;
    permissions.check_time(OnePassFunction::Vaults)?;

    let vaults = client.list_vaults().await?;
    let visible = permissions.filter_vaults(vaults);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&visible).unwrap());
        return Ok(());
    }
    if visible.is_empty() {
        println!("(no vaults visible to this account)");
        return Ok(());
    }
    for v in &visible {
        println!("{}\t{}", v.id, v.name);
    }
    Ok(())
}

async fn run_items(args: ItemsArgs) -> Result<()> {
    let client = client_for(&["read"])?;
    let permissions = perms::load_effective()?;
    permissions.check_time(OnePassFunction::Items)?;

    let filter = ListItemsFilter {
        vault: args.vault.clone(),
        tags: args.tags.clone(),
        categories: args.categories.clone(),
    };
    let items = client.list_items(&filter).await?;
    let visible = permissions.filter_items(items);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&visible).unwrap());
        return Ok(());
    }
    if visible.is_empty() {
        println!("(no items match)");
        return Ok(());
    }
    for it in &visible {
        let tags = if it.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", it.tags.join(","))
        };
        println!(
            "{}\t{}\t{}\t{}{tags}",
            it.id, it.vault.name, it.category, it.title
        );
    }
    Ok(())
}

async fn run_tags(args: TagsArgs) -> Result<()> {
    let client = client_for(&["read"])?;
    let permissions = perms::load_effective()?;
    permissions.check_time(OnePassFunction::Tags)?;

    let items = client.list_items(&ListItemsFilter::default()).await?;
    let visible_items = permissions.filter_items(items);
    let tags = permissions.filter_tags(&visible_items);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&tags).unwrap());
        return Ok(());
    }
    if tags.is_empty() {
        println!("(no tags visible)");
        return Ok(());
    }
    for t in &tags {
        println!("{t}");
    }
    Ok(())
}

async fn run_get(args: GetArgs) -> Result<()> {
    let client = client_for(&["read"])?;
    let permissions = perms::load_effective()?;
    permissions.check_time(OnePassFunction::Get)?;

    let vault = args.vault.clone().or_else(|| {
        effective_config()
            .ok()
            .and_then(|(c, _, _, _)| c.default_vault)
    });
    let item = client.get_item(&args.item, vault.as_deref()).await?;
    permissions.check_get(&args.item, &item)?;
    let filtered = permissions.filter_fields(item);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
        return Ok(());
    }
    println!("id       : {}", filtered.id);
    println!("title    : {}", filtered.title);
    println!("category : {}", filtered.category);
    println!("vault    : {} ({})", filtered.vault.name, filtered.vault.id);
    if !filtered.tags.is_empty() {
        println!("tags     : {}", filtered.tags.join(", "));
    }
    if !filtered.fields.is_empty() {
        println!("fields   :");
        for f in &filtered.fields {
            let value_marker = if f.value.is_some() {
                ""
            } else {
                " (value hidden)"
            };
            println!(
                "  - {label} [{ftype}]{marker}",
                label = if f.label.is_empty() {
                    f.id.as_str()
                } else {
                    f.label.as_str()
                },
                ftype = f.field_type,
                marker = value_marker
            );
        }
    }
    Ok(())
}

async fn run_read(args: ReadArgs) -> Result<()> {
    let client = client_for(&["read"])?;
    let permissions = perms::load_effective()?;
    permissions.check_time(OnePassFunction::Read)?;

    let parsed = parse_op_ref(&args.reference)?;
    // Resolve the item so we can gate vault/item/category/tags + field
    // through the full policy surface.
    let item = client
        .get_item(&parsed.item, Some(parsed.vault.as_str()))
        .await?;
    permissions.check_read(&args.reference, &item, &parsed.field)?;

    let value = client.read(&args.reference).await?;

    if args.json {
        let out = serde_json::json!({
            "command": "1pass.read",
            "reference": args.reference,
            "value": value,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    // Write to stdout without extra framing so `$(zad 1pass read …)`
    // works as expected.
    println!("{value}");
    Ok(())
}

async fn run_inject(args: InjectArgs) -> Result<()> {
    let client = client_for(&["read"])?;
    let permissions = perms::load_effective()?;
    permissions.check_time(OnePassFunction::Inject)?;

    let template = read_input(&args.input)?;
    // Pre-scan every `op://…` reference so a single out-of-scope ref
    // aborts the call before touching the network.
    let refs = scan_op_refs(&template);
    for r in &refs {
        permissions.check_inject_ref(r)?;
    }

    let rendered = client.inject(&template).await?;
    permissions.check_inject_body(&rendered)?;

    match args.output.as_deref() {
        Some(p) => {
            std::fs::write(p, &rendered).map_err(|e| ZadError::Io {
                path: PathBuf::from(p),
                source: e,
            })?;
        }
        None => {
            if args.json {
                let out = serde_json::json!({
                    "command": "1pass.inject",
                    "rendered": rendered,
                    "refs": refs.iter().map(|r| &r.source).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return Ok(());
            }
            print!("{rendered}");
        }
    }
    Ok(())
}

async fn run_create(args: CreateItemArgs) -> Result<()> {
    let client = client_for(&["write"])?;
    let permissions = perms::load_effective()?;
    permissions.check_create(&args.vault, &args.category, &args.title, &args.tags)?;

    let req = CreateItemRequest {
        title: args.title.clone(),
        vault: args.vault.clone(),
        category: args.category.clone(),
        tags: args.tags.iter().cloned().collect::<BTreeSet<_>>(),
        fields: args.fields.clone(),
    };
    let created = client.create_item(&req).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&created).unwrap());
        return Ok(());
    }
    println!(
        "created: {} ({}) in {}",
        created.title, created.id, created.vault.name
    );
    Ok(())
}

async fn run_whoami(args: WhoamiArgs) -> Result<()> {
    // whoami doesn't need a scope — it's a diagnostic.
    let client = client_for(&[])?;
    let me = client.whoami().await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&me).unwrap());
        return Ok(());
    }
    if !me.url.is_empty() {
        println!("url          : {}", me.url);
    }
    if !me.service_account_type.is_empty() {
        println!("account_type : {}", me.service_account_type);
    }
    if !me.user_uuid.is_empty() {
        println!("user_uuid    : {}", me.user_uuid);
    }
    if !me.account_uuid.is_empty() {
        println!("account_uuid : {}", me.account_uuid);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// permissions subcommands
// ---------------------------------------------------------------------------

fn run_permissions(args: PermissionsArgs) -> Result<()> {
    match args.action {
        None => run_permissions_show(PermissionsShowArgs { json: args.json }),
        Some(PermissionsAction::Show(a)) => run_permissions_show(a),
        Some(PermissionsAction::Path(a)) => run_permissions_path(a),
        Some(PermissionsAction::Init(a)) => run_permissions_init(a),
        Some(PermissionsAction::Check(a)) => run_permissions_check(a),
        Some(PermissionsAction::Staging(a)) => {
            crate::cli::permissions::run::<perms::PermissionsService>(a)
        }
    }
}

#[derive(Debug, Serialize)]
struct PermissionsScopeOut {
    path: String,
    present: bool,
}

#[derive(Debug, Serialize)]
struct PermissionsShowOut {
    command: &'static str,
    global: PermissionsScopeOut,
    local: PermissionsScopeOut,
}

fn run_permissions_show(args: PermissionsShowArgs) -> Result<()> {
    let global_path = perms::global_path()?;
    let local_path = perms::local_path_current()?;
    let _ = perms::load_effective()?;

    if args.json {
        let out = PermissionsShowOut {
            command: "1pass.permissions.show",
            global: PermissionsScopeOut {
                path: global_path.display().to_string(),
                present: global_path.exists(),
            },
            local: PermissionsScopeOut {
                path: local_path.display().to_string(),
                present: local_path.exists(),
            },
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("1Password permissions");
    print_scope_block("global", &global_path);
    print_scope_block("local", &local_path);
    Ok(())
}

fn print_scope_block(label: &str, path: &Path) {
    println!();
    println!("  [{label}] {}", path.display());
    if !path.exists() {
        println!("    status : not present (no restrictions from this scope)");
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(body) => {
            for line in body.lines() {
                println!("    {line}");
            }
        }
        Err(e) => println!("    status : read error — {e}"),
    }
}

#[derive(Debug, Serialize)]
struct PermissionsPathOut {
    command: &'static str,
    global: String,
    local: String,
}

fn run_permissions_path(args: PermissionsPathArgs) -> Result<()> {
    let global_path = perms::global_path()?;
    let local_path = perms::local_path_current()?;
    if args.json {
        let out = PermissionsPathOut {
            command: "1pass.permissions.path",
            global: global_path.display().to_string(),
            local: local_path.display().to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!("{}", global_path.display());
    println!("{}", local_path.display());
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsInitOut {
    command: &'static str,
    scope: &'static str,
    path: String,
    written: bool,
}

fn run_permissions_init(args: PermissionsInitArgs) -> Result<()> {
    let (path, scope_label): (PathBuf, &'static str) = if args.local {
        (perms::local_path_current()?, "local")
    } else {
        (perms::global_path()?, "global")
    };
    if path.exists() && !args.force {
        return Err(ZadError::Invalid(format!(
            "{} already exists — pass --force to overwrite",
            path.display()
        )));
    }
    let key = crate::permissions::signing::load_or_create_from_keychain()?;
    crate::permissions::signing::write_public_key_cache(&key)?;
    perms::save_file(&path, &perms::starter_template(), &key)?;
    if args.json {
        let out = PermissionsInitOut {
            command: "1pass.permissions.init",
            scope: scope_label,
            path: path.display().to_string(),
            written: true,
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }
    println!(
        "Wrote 1pass permissions starter policy to {} ({scope_label}).",
        path.display()
    );
    println!("Signed with key {}.", key.fingerprint());
    println!("Edit to narrow further; re-run `zad 1pass permissions show` to inspect.");
    Ok(())
}

#[derive(Debug, Serialize)]
struct PermissionsCheckOut {
    command: &'static str,
    function: String,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_path: Option<String>,
}

fn run_permissions_check(args: PermissionsCheckArgs) -> Result<()> {
    let permissions = perms::load_effective()?;
    let outcome = check_hypothetical(&permissions, &args);
    emit_check_result(&args, outcome)
}

fn emit_check_result(args: &PermissionsCheckArgs, outcome: Result<()>) -> Result<()> {
    match outcome {
        Ok(()) => {
            if args.json {
                let out = PermissionsCheckOut {
                    command: "1pass.permissions.check",
                    function: args.function.clone(),
                    allowed: true,
                    reason: None,
                    config_path: None,
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                println!("allowed");
            }
            Ok(())
        }
        Err(ZadError::PermissionDenied {
            function,
            reason,
            config_path,
        }) => {
            if args.json {
                let out = PermissionsCheckOut {
                    command: "1pass.permissions.check",
                    function: function.to_string(),
                    allowed: false,
                    reason: Some(reason.clone()),
                    config_path: Some(config_path.display().to_string()),
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                println!("denied: {reason}");
                println!("  edit: {}", config_path.display());
            }
            std::process::exit(1);
        }
        Err(ZadError::Service {
            name: "1pass",
            message,
        }) => {
            // NotFound-shaped result from a hidden-target read check.
            // Report as "denied (hidden)" so the operator running
            // `permissions check` can distinguish this from an allowed
            // outcome.
            if args.json {
                let out = PermissionsCheckOut {
                    command: "1pass.permissions.check",
                    function: args.function.clone(),
                    allowed: false,
                    reason: Some(format!("hidden: {message}")),
                    config_path: None,
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                println!("denied (hidden): {message}");
            }
            std::process::exit(1);
        }
        Err(other) => Err(other),
    }
}

fn check_hypothetical(
    permissions: &EffectivePermissions,
    args: &PermissionsCheckArgs,
) -> Result<()> {
    // `create` is a separate path — doesn't go through OnePassFunction.
    if args.function == "create" {
        let vault = args
            .vault
            .as_deref()
            .ok_or_else(|| ZadError::Invalid("--vault is required for create".into()))?;
        let category = args.category.as_deref().unwrap_or("Login");
        let title = args.title.as_deref().unwrap_or("");
        return permissions.check_create(vault, category, title, &args.tags);
    }

    let func = OnePassFunction::parse(&args.function)?;
    permissions.check_time(func)?;

    // Synthesize a minimal item from the provided axis flags so the
    // shared `item_admitted` path can gate everything we know about.
    if args.item.is_some()
        || args.vault.is_some()
        || !args.tags.is_empty()
        || args.category.is_some()
    {
        let item = synthetic_item(args);
        match func {
            OnePassFunction::Get | OnePassFunction::Read => {
                permissions.check_get(
                    args.item
                        .as_deref()
                        .or(args.reference.as_deref())
                        .unwrap_or(""),
                    &item,
                )?;
            }
            _ => {
                // list-style verbs use filter_items; reuse that via a
                // single-item slice.
                let vec = vec![summarize(&item)];
                if permissions.filter_items(vec).is_empty() {
                    return Err(ZadError::Service {
                        name: "1pass",
                        message: "item is hidden at this scope".into(),
                    });
                }
            }
        }
    }

    if func == OnePassFunction::Read {
        if let Some(r) = args.reference.as_deref() {
            let parsed = parse_op_ref(r)?;
            let item = synthetic_item_for_ref(&parsed, args);
            permissions.check_read(r, &item, &parsed.field)?;
        }
    }
    if func == OnePassFunction::Inject {
        if let Some(r) = args.reference.as_deref() {
            let parsed = parse_op_ref(r)?;
            permissions.check_inject_ref(&parsed)?;
        }
    }
    Ok(())
}

fn synthetic_item(args: &PermissionsCheckArgs) -> Item {
    Item {
        id: args.item.clone().unwrap_or_default(),
        title: args.item.clone().unwrap_or_default(),
        category: args.category.clone().unwrap_or_default(),
        tags: args.tags.clone(),
        vault: crate::service::onepass::client::VaultRef {
            id: args.vault.clone().unwrap_or_default(),
            name: args.vault.clone().unwrap_or_default(),
        },
        fields: args
            .field
            .as_ref()
            .map(|f| {
                vec![crate::service::onepass::client::ItemField {
                    id: f.clone(),
                    label: f.clone(),
                    field_type: String::new(),
                    purpose: None,
                    value: None,
                    section: None,
                }]
            })
            .unwrap_or_default(),
        sections: vec![],
        updated_at: None,
        created_at: None,
    }
}

fn synthetic_item_for_ref(r: &ParsedOpRef, args: &PermissionsCheckArgs) -> Item {
    Item {
        id: r.item.clone(),
        title: r.item.clone(),
        category: args.category.clone().unwrap_or_default(),
        tags: args.tags.clone(),
        vault: crate::service::onepass::client::VaultRef {
            id: r.vault.clone(),
            name: r.vault.clone(),
        },
        fields: vec![crate::service::onepass::client::ItemField {
            id: r.field.clone(),
            label: r.field.clone(),
            field_type: String::new(),
            purpose: None,
            value: None,
            section: None,
        }],
        sections: vec![],
        updated_at: None,
        created_at: None,
    }
}

fn summarize(item: &Item) -> ItemSummary {
    ItemSummary {
        id: item.id.clone(),
        title: item.title.clone(),
        category: item.category.clone(),
        tags: item.tags.clone(),
        vault: item.vault.clone(),
        updated_at: None,
        created_at: None,
    }
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Load the effective `OnePassServiceCfg` plus the scope label and the
/// keychain scope to read the token from. Mirrors `gcal::effective_config`.
pub(crate) fn effective_config()
-> Result<(OnePassServiceCfg, &'static str, Scope<'static>, PathBuf)> {
    let slug = config::path::project_slug()?;
    let local_path = config::path::project_service_config_path_for(&slug, "1pass")?;
    let global_path = config::path::global_service_config_path("1pass")?;

    let project_cfg = config::load()?;
    if !project_cfg.has_service("1pass") {
        return Err(ZadError::Invalid(format!(
            "1pass is not enabled for this project ({}). Run `zad service enable 1pass` first.",
            config::path::project_config_path()?.display()
        )));
    }

    if let Some(cfg) = config::load_flat::<OnePassServiceCfg>(&local_path)? {
        let slug_leaked = leak(slug);
        return Ok((cfg, "local", Scope::Project(slug_leaked), local_path));
    }
    if let Some(cfg) = config::load_flat::<OnePassServiceCfg>(&global_path)? {
        return Ok((cfg, "global", Scope::Global, global_path));
    }
    Err(ZadError::Invalid(format!(
        "no 1pass credentials found.\n  looked in:\n    {}\n    {}\n  Run `zad service create 1pass`.",
        local_path.display(),
        global_path.display()
    )))
}

/// Scope gate: each runtime verb names the zad-level scopes it needs;
/// if any are missing from the effective config we raise `ScopeDenied`
/// pointing at the config file.
fn client_for(required_scopes: &[&'static str]) -> Result<OnePassClient> {
    let (cfg, _label, scope, path) = effective_config()?;
    for s in required_scopes {
        if !cfg.scopes.iter().any(|x| x == s) {
            return Err(ZadError::ScopeDenied {
                service: "1pass",
                scope: s,
                config_path: path.clone(),
            });
        }
    }
    let token = secrets::load(&secrets::account("1pass", "service-account", scope))?.ok_or(
        ZadError::Service {
            name: "1pass",
            message:
                "service-account token missing from keychain; re-run `zad service create 1pass`"
                    .into(),
        },
    )?;
    Ok(OnePassClient::new(token, cfg.account))
}

/// Resolve an `--in` argument to a template body. `-` reads stdin.
fn read_input(input: &str) -> Result<String> {
    if input == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| ZadError::Io {
                path: PathBuf::from("<stdin>"),
                source: e,
            })?;
        return Ok(buf);
    }
    std::fs::read_to_string(input).map_err(|e| ZadError::Io {
        path: PathBuf::from(input),
        source: e,
    })
}

// Silence unused-import warnings that only fire with narrow feature
// subsets; the direct usages keep these types in the symbol table.
#[allow(dead_code)]
fn _type_anchors() -> (Vault,) {
    (Vault {
        id: String::new(),
        name: String::new(),
        content_version: None,
    },)
}
