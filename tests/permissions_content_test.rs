//! Content-filter tests: denied words, denied regex patterns, and the
//! optional length cap. These pin the semantics that matter for
//! agents — case-insensitive word matching, codepoint-based length
//! accounting, and merge-produces-stricter behavior.

use zad::permissions::content::{ContentDenyReason, ContentRules, ContentRulesRaw};

fn compile(raw: ContentRulesRaw) -> ContentRules {
    ContentRules::compile(&raw).unwrap()
}

#[test]
fn denied_word_matches_regardless_of_case() {
    let rules = compile(ContentRulesRaw {
        deny_words: vec!["password".into()],
        deny_patterns: vec![],
        max_length: None,
    });
    let err = rules.evaluate("Here's my Password: hunter2").unwrap_err();
    assert!(matches!(err, ContentDenyReason::WordMatched { .. }));
}

#[test]
fn denied_word_can_match_in_the_middle_of_a_larger_word() {
    // This is intentional: `deny_words` is substring-based. If an
    // operator wants word-boundary semantics, they can use
    // `deny_patterns` with `\b...\b`.
    let rules = compile(ContentRulesRaw {
        deny_words: vec!["pass".into()],
        deny_patterns: vec![],
        max_length: None,
    });
    assert!(rules.evaluate("passing it over").is_err());
}

#[test]
fn deny_pattern_fires_on_regex_match() {
    let rules = compile(ContentRulesRaw {
        deny_words: vec![],
        deny_patterns: vec![r"(?i)bearer\s+[a-z0-9]+".into()],
        max_length: None,
    });
    let err = rules.evaluate("Authorization: Bearer abc123").unwrap_err();
    assert!(matches!(err, ContentDenyReason::PatternMatched { .. }));
}

#[test]
fn invalid_deny_pattern_surfaces_at_compile_time() {
    let err = ContentRules::compile(&ContentRulesRaw {
        deny_patterns: vec!["(".into()],
        ..Default::default()
    })
    .unwrap_err();
    assert!(err.contains("invalid content deny pattern"), "err: {err}");
}

#[test]
fn max_length_counts_codepoints_not_bytes() {
    // Each emoji is a single codepoint but 4 bytes in UTF-8.
    let rules = compile(ContentRulesRaw {
        deny_words: vec![],
        deny_patterns: vec![],
        max_length: Some(3),
    });
    assert!(rules.evaluate("a🙂c").is_ok(), "3 codepoints should fit");
    let err = rules.evaluate("a🙂cd").unwrap_err();
    assert!(
        matches!(err, ContentDenyReason::TooLong { len: 4, max: 3 }),
        "unexpected: {err:?}"
    );
}

#[test]
fn merge_unions_deny_lists_and_tightens_max_length() {
    let a = compile(ContentRulesRaw {
        deny_words: vec!["foo".into()],
        deny_patterns: vec![],
        max_length: Some(1000),
    });
    let b = compile(ContentRulesRaw {
        deny_words: vec!["bar".into()],
        deny_patterns: vec![],
        max_length: Some(500),
    });
    let merged = a.merge(b);
    assert_eq!(merged.max_length, Some(500));
    // The merged rule should reject both words.
    assert!(merged.evaluate("say foo").is_err());
    assert!(merged.evaluate("say bar").is_err());
    assert!(merged.evaluate("say anything else").is_ok());
}

#[test]
fn empty_rules_admit_everything() {
    let rules = compile(ContentRulesRaw::default());
    assert!(rules.is_empty());
    assert!(rules.evaluate("anything goes").is_ok());
}
