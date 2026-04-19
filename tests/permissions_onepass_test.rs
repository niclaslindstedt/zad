//! Unit tests for `src/service/onepass/permissions.rs`.
//!
//! Covers the filter-vs-deny semantics that distinguish 1pass from
//! discord/telegram: read-side returns hidden targets as filtered-out
//! (or `NotFound`-shaped errors), write-side (`check_create`) always
//! uses `PermissionDenied`.

use std::path::PathBuf;

use zad::error::ZadError;
use zad::permissions::SigningKey;
use zad::permissions::{content::ContentRulesRaw, pattern::PatternListRaw};
use zad::service::onepass::client::{Item, ItemField, ItemSummary, ParsedOpRef, Vault, VaultRef};
use zad::service::onepass::permissions::{
    CreateBlockRaw, EffectivePermissions, FunctionBlockRaw, OnePassPermissions,
    OnePassPermissionsRaw,
};

fn test_key() -> SigningKey {
    zad::secrets::use_memory_backend();
    SigningKey::generate()
}

fn toml_to_effective(global: Option<&str>, local: Option<&str>) -> EffectivePermissions {
    let global = global.map(|b| {
        let raw: OnePassPermissionsRaw = toml::from_str(b).expect("parse global");
        compile_via_save(raw, "global")
    });
    let local = local.map(|b| {
        let raw: OnePassPermissionsRaw = toml::from_str(b).expect("parse local");
        compile_via_save(raw, "local")
    });
    EffectivePermissions { global, local }
}

/// Round-trip through save_file → load_file so the `source` path is
/// populated, because error-site reporting relies on it. We use a
/// tempdir per call so tests stay hermetic.
fn compile_via_save(raw: OnePassPermissionsRaw, label: &str) -> OnePassPermissions {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{label}-permissions.toml"));
    let key = test_key();
    zad::service::onepass::permissions::save_file(&path, &raw, &key).unwrap();
    let loaded = zad::service::onepass::permissions::load_file(&path).unwrap();
    std::mem::forget(dir); // keep the file alive for the duration of the test
    loaded.unwrap_or_else(|| panic!("file should have compiled"))
}

fn vault(name: &str) -> Vault {
    Vault {
        id: format!("{name}-id"),
        name: name.to_string(),
        content_version: None,
    }
}

fn summary(title: &str, vault: &str, category: &str, tags: &[&str]) -> ItemSummary {
    ItemSummary {
        id: format!("{title}-id"),
        title: title.to_string(),
        category: category.to_string(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        vault: VaultRef {
            id: format!("{vault}-id"),
            name: vault.to_string(),
        },
        updated_at: None,
        created_at: None,
    }
}

fn item(title: &str, vault: &str, category: &str, tags: &[&str], fields: &[&str]) -> Item {
    Item {
        id: format!("{title}-id"),
        title: title.to_string(),
        category: category.to_string(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        vault: VaultRef {
            id: format!("{vault}-id"),
            name: vault.to_string(),
        },
        fields: fields
            .iter()
            .map(|f| ItemField {
                id: (*f).to_string(),
                label: (*f).to_string(),
                field_type: "STRING".into(),
                purpose: None,
                value: Some("x".into()),
                section: None,
            })
            .collect(),
        sections: vec![],
        updated_at: None,
        created_at: None,
    }
}

// ---------------------------------------------------------------------------
// filter_vaults
// ---------------------------------------------------------------------------

#[test]
fn filter_vaults_empty_policy_returns_all() {
    let perms = EffectivePermissions::default();
    let v = vec![vault("Personal"), vault("AgentWork")];
    let out = perms.filter_vaults(v.clone());
    assert_eq!(out.len(), 2);
}

#[test]
fn filter_vaults_deny_strips_specific_name() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
deny = ["Personal"]
"#,
        ),
        None,
    );
    let v = vec![vault("Personal"), vault("AgentWork")];
    let out = perms.filter_vaults(v);
    let names: Vec<&str> = out.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["AgentWork"]);
}

#[test]
fn filter_vaults_allow_list_keeps_only_matches() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
allow = ["AgentWork", "Shared-*"]
"#,
        ),
        None,
    );
    let v = vec![
        vault("Personal"),
        vault("AgentWork"),
        vault("Shared-ops"),
        vault("Prod"),
    ];
    let out = perms.filter_vaults(v);
    let names: Vec<&str> = out.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["AgentWork", "Shared-ops"]);
}

#[test]
fn filter_vaults_global_intersects_local() {
    // Global: deny Personal. Local: deny Prod. Result: neither.
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
deny = ["Personal"]
"#,
        ),
        Some(
            r#"
[vaults]
deny = ["Prod"]
"#,
        ),
    );
    let v = vec![vault("Personal"), vault("AgentWork"), vault("Prod")];
    let out = perms.filter_vaults(v);
    let names: Vec<&str> = out.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["AgentWork"]);
}

// ---------------------------------------------------------------------------
// filter_items
// ---------------------------------------------------------------------------

#[test]
fn filter_items_strips_items_in_denied_vault() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
deny = ["Personal"]
"#,
        ),
        None,
    );
    let items = vec![
        summary("GitHub", "Personal", "Login", &[]),
        summary("DBPass", "AgentWork", "Login", &[]),
    ];
    let out = perms.filter_items(items);
    let titles: Vec<&str> = out.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["DBPass"]);
}

#[test]
fn filter_items_strips_by_tag_deny() {
    let perms = toml_to_effective(
        Some(
            r#"
[tags]
deny = ["admin"]
"#,
        ),
        None,
    );
    let items = vec![
        summary("A", "V", "Login", &["team"]),
        summary("B", "V", "Login", &["admin", "team"]),
        summary("C", "V", "Login", &[]),
    ];
    let out = perms.filter_items(items);
    let titles: Vec<&str> = out.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["A", "C"]);
}

#[test]
fn filter_items_strips_by_category_allow_list() {
    let perms = toml_to_effective(
        Some(
            r#"
[categories]
allow = ["Login", "API Credential"]
"#,
        ),
        None,
    );
    let items = vec![
        summary("A", "V", "Login", &[]),
        summary("B", "V", "Secure Note", &[]),
        summary("C", "V", "API Credential", &[]),
    ];
    let out = perms.filter_items(items);
    let titles: Vec<&str> = out.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["A", "C"]);
}

// ---------------------------------------------------------------------------
// filter_tags
// ---------------------------------------------------------------------------

#[test]
fn filter_tags_derives_from_visible_items_and_honors_deny() {
    let perms = toml_to_effective(
        Some(
            r#"
[tags]
deny = ["secret"]
"#,
        ),
        None,
    );
    let visible = vec![
        summary("A", "V", "Login", &["team", "prod"]),
        summary("B", "V", "Login", &["secret"]), // already filtered in practice
    ];
    // In the real flow, filter_items would have stripped B; this
    // test pretends the caller is working from the raw list.
    let tags = perms.filter_tags(&visible);
    assert!(tags.iter().any(|t| t == "team"));
    assert!(tags.iter().any(|t| t == "prod"));
    assert!(!tags.iter().any(|t| t == "secret"));
}

// ---------------------------------------------------------------------------
// check_get — returns NotFound-shaped Service error, not PermissionDenied
// ---------------------------------------------------------------------------

#[test]
fn check_get_hidden_item_returns_service_not_permission_denied() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
deny = ["Personal"]
"#,
        ),
        None,
    );
    let it = item("Foo", "Personal", "Login", &[], &["password"]);
    let err = perms.check_get("Foo", &it).unwrap_err();
    match err {
        ZadError::Service { name, message } => {
            assert_eq!(name, "1pass");
            assert!(
                message.contains("isn't an item") || message.contains("not found"),
                "got: {message}"
            );
        }
        other => panic!("expected Service, got: {other:?}"),
    }
}

#[test]
fn check_get_allowed_item_passes() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
allow = ["AgentWork"]
"#,
        ),
        None,
    );
    let it = item("Foo", "AgentWork", "Login", &[], &["password"]);
    perms.check_get("Foo", &it).unwrap();
}

// ---------------------------------------------------------------------------
// filter_fields
// ---------------------------------------------------------------------------

#[test]
fn filter_fields_drops_denied_fields_but_keeps_metadata() {
    let perms = toml_to_effective(
        Some(
            r#"
[fields]
deny = ["notesPlain", "recovery_code"]
"#,
        ),
        None,
    );
    let it = item(
        "Foo",
        "V",
        "Login",
        &[],
        &["username", "password", "notesPlain", "recovery_code"],
    );
    let out = perms.filter_fields(it);
    let labels: Vec<&str> = out.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, vec!["username", "password"]);
}

#[test]
fn filter_fields_empty_policy_keeps_everything() {
    let perms = EffectivePermissions::default();
    let it = item("Foo", "V", "Login", &[], &["username", "password"]);
    let out = perms.filter_fields(it);
    assert_eq!(out.fields.len(), 2);
}

// ---------------------------------------------------------------------------
// check_read
// ---------------------------------------------------------------------------

#[test]
fn check_read_denies_hidden_field_as_notfound() {
    let perms = toml_to_effective(
        Some(
            r#"
[fields]
deny = ["notesPlain"]
"#,
        ),
        None,
    );
    let it = item("Foo", "V", "Login", &[], &["username", "notesPlain"]);
    let err = perms
        .check_read("op://V/Foo/notesPlain", &it, "notesPlain")
        .unwrap_err();
    match err {
        ZadError::Service { name, .. } => assert_eq!(name, "1pass"),
        other => panic!("expected Service, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// check_inject_ref
// ---------------------------------------------------------------------------

#[test]
fn check_inject_ref_denies_vault_outside_allowlist() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
allow = ["AgentWork"]
"#,
        ),
        None,
    );
    let parsed = ParsedOpRef {
        source: "op://Personal/foo/password".into(),
        vault: "Personal".into(),
        item: "foo".into(),
        field: "password".into(),
        section: None,
    };
    let err = perms.check_inject_ref(&parsed).unwrap_err();
    match err {
        ZadError::Service { name, .. } => assert_eq!(name, "1pass"),
        other => panic!("expected Service, got: {other:?}"),
    }
}

#[test]
fn check_inject_ref_passes_inside_allowlist() {
    let perms = toml_to_effective(
        Some(
            r#"
[vaults]
allow = ["AgentWork"]
"#,
        ),
        None,
    );
    let parsed = ParsedOpRef {
        source: "op://AgentWork/foo/password".into(),
        vault: "AgentWork".into(),
        item: "foo".into(),
        field: "password".into(),
        section: None,
    };
    perms.check_inject_ref(&parsed).unwrap();
}

// ---------------------------------------------------------------------------
// check_create — deny-by-default, intersecting, error shape
// ---------------------------------------------------------------------------

#[test]
fn check_create_no_policy_at_all_denies() {
    let perms = EffectivePermissions::default();
    let err = perms
        .check_create("AgentWork", "Login", "foo", &["agent-managed".into()])
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { function, .. } => assert_eq!(function, "create"),
        other => panic!("expected PermissionDenied, got: {other:?}"),
    }
}

#[test]
fn check_create_policy_without_vaults_allow_denies() {
    let perms = toml_to_effective(
        Some(
            r#"
[create]
# No vaults.allow configured — deny-by-default.
"#,
        ),
        None,
    );
    let err = perms
        .check_create("AgentWork", "Login", "foo", &[])
        .unwrap_err();
    match err {
        ZadError::PermissionDenied {
            function, reason, ..
        } => {
            assert_eq!(function, "create");
            assert!(reason.contains("vaults.allow"), "reason: {reason}");
        }
        other => panic!("expected PermissionDenied, got: {other:?}"),
    }
}

#[test]
fn check_create_passes_when_vault_and_tags_match() {
    let perms = toml_to_effective(
        Some(
            r#"
[create.vaults]
allow = ["AgentWork"]
[create.tags]
allow = ["agent-managed"]
[create.categories]
allow = ["Login", "API Credential"]
"#,
        ),
        None,
    );
    perms
        .check_create("AgentWork", "Login", "foo", &["agent-managed".into()])
        .unwrap();
}

#[test]
fn check_create_denies_vault_outside_allowlist() {
    let perms = toml_to_effective(
        Some(
            r#"
[create.vaults]
allow = ["AgentWork"]
"#,
        ),
        None,
    );
    let err = perms
        .check_create("Personal", "Login", "foo", &[])
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { function, .. } => assert_eq!(function, "create"),
        other => panic!("expected PermissionDenied, got: {other:?}"),
    }
}

#[test]
fn check_create_denies_disallowed_category() {
    let perms = toml_to_effective(
        Some(
            r#"
[create.vaults]
allow = ["AgentWork"]
[create.categories]
allow = ["Login"]
"#,
        ),
        None,
    );
    let err = perms
        .check_create("AgentWork", "Secure Note", "foo", &[])
        .unwrap_err();
    match err {
        ZadError::PermissionDenied {
            function, reason, ..
        } => {
            assert_eq!(function, "create");
            assert!(reason.contains("category"), "reason: {reason}");
        }
        other => panic!("expected PermissionDenied, got: {other:?}"),
    }
}

#[test]
fn check_create_denies_without_required_tag() {
    let perms = toml_to_effective(
        Some(
            r#"
[create.vaults]
allow = ["AgentWork"]
[create.tags]
allow = ["agent-managed"]
"#,
        ),
        None,
    );
    let err = perms
        .check_create("AgentWork", "Login", "foo", &[])
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { function, .. } => assert_eq!(function, "create"),
        other => panic!("expected PermissionDenied, got: {other:?}"),
    }
}

#[test]
fn check_create_local_tightens_global() {
    // Global allows both vaults; local narrows to Ops only.
    let perms = toml_to_effective(
        Some(
            r#"
[create.vaults]
allow = ["AgentWork", "Ops"]
"#,
        ),
        Some(
            r#"
[create.vaults]
allow = ["Ops"]
"#,
        ),
    );
    // AgentWork passes global but is denied by local → overall deny.
    let err = perms
        .check_create("AgentWork", "Login", "foo", &[])
        .unwrap_err();
    match err {
        ZadError::PermissionDenied { .. } => {}
        other => panic!("expected PermissionDenied, got: {other:?}"),
    }
    // Ops passes both → allowed.
    perms.check_create("Ops", "Login", "foo", &[]).unwrap();
}

// ---------------------------------------------------------------------------
// exhaustive type touchpoints — keep unused-import warnings off
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn _type_anchors() -> (
    PatternListRaw,
    ContentRulesRaw,
    CreateBlockRaw,
    FunctionBlockRaw,
    PathBuf,
) {
    (
        PatternListRaw::default(),
        ContentRulesRaw::default(),
        CreateBlockRaw::default(),
        FunctionBlockRaw::default(),
        PathBuf::new(),
    )
}
