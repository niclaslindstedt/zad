//! Unit tests for the GitHub permissions schema.
//!
//! These exercise the parser, compiler, and effective-layer logic
//! directly (no subprocess), covering the two target axes (repo, org),
//! content rules on bodies, and the deny-by-default write-verb policy
//! in the starter template.

use zad::permissions::content::ContentRulesRaw;
use zad::permissions::pattern::PatternListRaw;
use zad::permissions::signing::SigningKey;
use zad::service::github::permissions::{
    self as perms, EffectivePermissions, FunctionBlockRaw, GithubFunction, GithubPermissions,
    GithubPermissionsRaw,
};

fn compile(raw: GithubPermissionsRaw) -> GithubPermissions {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("permissions.toml");
    let key = SigningKey::generate();
    perms::save_file(&path, &raw, &key).unwrap();
    perms::load_file(&path).unwrap().unwrap()
}

fn effective(raw: GithubPermissionsRaw) -> EffectivePermissions {
    EffectivePermissions {
        global: Some(compile(raw)),
        local: None,
    }
}

// ---------------------------------------------------------------------------
// repo allow/deny
// ---------------------------------------------------------------------------

#[test]
fn repo_pattern_allow_admits_globbed_owner() {
    let raw = GithubPermissionsRaw {
        issue_list: FunctionBlockRaw {
            repos: PatternListRaw {
                allow: vec!["myorg/*".into()],
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..GithubPermissionsRaw::default()
    };
    let eff = effective(raw);
    assert!(
        eff.check_repo(GithubFunction::IssueList, "myorg/api")
            .is_ok()
    );
    assert!(
        eff.check_repo(GithubFunction::IssueList, "other/api")
            .is_err()
    );
}

#[test]
fn repo_deny_always_wins() {
    let raw = GithubPermissionsRaw {
        pr_comment: FunctionBlockRaw {
            repos: PatternListRaw {
                allow: vec!["*".into()],
                deny: vec!["*/secret-*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        ..GithubPermissionsRaw::default()
    };
    let eff = effective(raw);
    assert!(
        eff.check_repo(GithubFunction::PrComment, "myorg/api")
            .is_ok()
    );
    assert!(
        eff.check_repo(GithubFunction::PrComment, "myorg/secret-vault")
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// org allow/deny
// ---------------------------------------------------------------------------

#[test]
fn org_allow_list_narrows_code_search() {
    let raw = GithubPermissionsRaw {
        code_search: FunctionBlockRaw {
            orgs: PatternListRaw {
                allow: vec!["myorg".into()],
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..GithubPermissionsRaw::default()
    };
    let eff = effective(raw);
    assert!(eff.check_org(GithubFunction::CodeSearch, "myorg").is_ok());
    assert!(
        eff.check_org(GithubFunction::CodeSearch, "someone-else")
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// content rules
// ---------------------------------------------------------------------------

#[test]
fn content_deny_word_catches_body_leak() {
    let raw = GithubPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into()],
            deny_patterns: vec![],
            max_length: None,
        },
        issue_comment: FunctionBlockRaw {
            repos: PatternListRaw {
                allow: vec!["*".into()],
                deny: vec![],
            },
            ..FunctionBlockRaw::default()
        },
        ..GithubPermissionsRaw::default()
    };
    let eff = effective(raw);
    assert!(
        eff.check_body(GithubFunction::IssueComment, "all good")
            .is_ok()
    );
    assert!(
        eff.check_body(GithubFunction::IssueComment, "the password is hunter2")
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// starter template shape
// ---------------------------------------------------------------------------

#[test]
fn starter_template_denies_every_write_verb() {
    let raw = perms::starter_template();
    let eff = effective(raw);
    for f in [
        GithubFunction::IssueCreate,
        GithubFunction::IssueComment,
        GithubFunction::IssueClose,
        GithubFunction::PrCreate,
        GithubFunction::PrComment,
        GithubFunction::PrReview,
        GithubFunction::PrMerge,
    ] {
        assert!(
            eff.check_repo(f, "anyone/anything").is_err(),
            "{} should be denied in starter template",
            f.name()
        );
    }
}

#[test]
fn starter_template_allows_read_verbs() {
    let raw = perms::starter_template();
    let eff = effective(raw);
    for f in [
        GithubFunction::IssueList,
        GithubFunction::IssueView,
        GithubFunction::PrList,
        GithubFunction::PrView,
        GithubFunction::PrDiff,
        GithubFunction::PrChecks,
        GithubFunction::RepoView,
        GithubFunction::FileView,
        GithubFunction::RunList,
        GithubFunction::RunView,
    ] {
        assert!(
            eff.check_repo(f, "anyone/anything").is_ok(),
            "{} should be allowed in starter template",
            f.name()
        );
    }
    // code_search uses orgs, not repos
    assert!(eff.check_org(GithubFunction::CodeSearch, "anyone").is_ok());
}

// ---------------------------------------------------------------------------
// function name roundtrip
// ---------------------------------------------------------------------------

#[test]
fn every_declared_function_roundtrips_through_name() {
    for name in perms::ALL_FUNCTIONS {
        let f = GithubFunction::from_name(name).unwrap_or_else(|| panic!("unknown: {name}"));
        assert_eq!(f.name(), *name);
    }
}
