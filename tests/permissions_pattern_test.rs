//! Unit-style tests for the shared pattern primitive. These never
//! touch the filesystem and never hit Discord — they pin the grammar
//! (exact / glob / regex / numeric) so that a future refactor can't
//! silently change what `*admin*` or `re:^mod-` matches.

use zad::permissions::pattern::{DenyReason, Pattern, PatternList, PatternListRaw};

#[test]
fn bare_name_is_parsed_as_exact_match() {
    let p = Pattern::parse("general").unwrap();
    assert!(p.matches("general"));
    assert!(!p.matches("general-2"));
    assert!(!p.matches("Generals"));
}

#[test]
fn numeric_input_becomes_an_exact_id_match() {
    // Numeric inputs must be exact: `111` should not match `1111`.
    let p = Pattern::parse("1234567890").unwrap();
    assert!(p.matches("1234567890"));
    assert!(!p.matches("123456789"));
    assert!(!p.matches("12345678901"));
}

#[test]
fn glob_star_expands_to_regex() {
    let p = Pattern::parse("bot-*").unwrap();
    assert!(p.matches("bot-"));
    assert!(p.matches("bot-foo"));
    assert!(p.matches("bot-admin-ops"));
    assert!(!p.matches("sbot-foo"));
}

#[test]
fn glob_star_in_the_middle_matches_substring() {
    let p = Pattern::parse("*admin*").unwrap();
    assert!(p.matches("admin"));
    assert!(p.matches("server-admin"));
    assert!(p.matches("admin-chatter"));
    assert!(!p.matches("moderator"));
}

#[test]
fn glob_question_mark_matches_exactly_one_char() {
    let p = Pattern::parse("mod-?").unwrap();
    assert!(p.matches("mod-a"));
    assert!(!p.matches("mod-"));
    assert!(!p.matches("mod-ab"));
}

#[test]
fn glob_does_not_let_regex_metachars_leak() {
    // A literal `.` in a channel name must not behave as regex `.`.
    let p = Pattern::parse("team.ops.*").unwrap();
    assert!(p.matches("team.ops.alerts"));
    // Literal period required before `ops`.
    assert!(!p.matches("team_ops_alerts"));
}

#[test]
fn explicit_regex_prefix_honors_full_regex_syntax() {
    let p = Pattern::parse(r"re:^mod-[0-9]+$").unwrap();
    assert!(p.matches("mod-42"));
    assert!(!p.matches("mod-abc"));
    assert!(!p.matches("prefix-mod-42"));
}

#[test]
fn invalid_regex_surfaces_at_parse_time() {
    let err = Pattern::parse("re:(").unwrap_err();
    assert!(err.contains("invalid regex"), "err was: {err}");
}

#[test]
fn deny_takes_precedence_over_allow() {
    // A target that matches *both* allow and deny is still denied.
    let list = PatternList::compile(&PatternListRaw {
        allow: vec!["*admin*".into()],
        deny: vec!["*admin*".into()],
    })
    .unwrap();
    let err = list.evaluate(["server-admin"].iter().copied()).unwrap_err();
    assert!(
        matches!(err, DenyReason::DenyMatched { .. }),
        "expected DenyMatched, got {err:?}"
    );
}

#[test]
fn empty_allow_list_admits_unless_denied() {
    let list = PatternList::compile(&PatternListRaw {
        allow: vec![],
        deny: vec!["*admin*".into()],
    })
    .unwrap();
    assert!(list.evaluate(["general"].iter().copied()).is_ok());
    assert!(
        list.evaluate(["admin-chatter"].iter().copied()).is_err(),
        "admin-chatter should be denied by *admin*"
    );
}

#[test]
fn non_empty_allow_list_requires_an_allow_hit() {
    let list = PatternList::compile(&PatternListRaw {
        allow: vec!["bot-*".into(), "team/*".into()],
        deny: vec![],
    })
    .unwrap();
    assert!(list.evaluate(["bot-foo"].iter().copied()).is_ok());
    assert!(list.evaluate(["team/alerts"].iter().copied()).is_ok());
    let err = list.evaluate(["random"].iter().copied()).unwrap_err();
    assert_eq!(err, DenyReason::AllowUnmatched);
}

#[test]
fn any_candidate_alias_satisfies_the_allow_list() {
    // The caller passes the input, the resolved snowflake, *and* any
    // reverse-lookup names. Matching any one of them must be enough.
    let list = PatternList::compile(&PatternListRaw {
        allow: vec!["general".into()],
        deny: vec![],
    })
    .unwrap();
    assert!(
        list.evaluate(["999000000000", "general"].iter().copied())
            .is_ok(),
        "the ID alone doesn't match but `general` does — list should admit"
    );
}

#[test]
fn any_alias_can_trip_a_deny() {
    // Conversely, if the directory knows the ID also lives under the
    // name `admin-chatter`, a deny on `*admin*` must still fire when
    // the agent typed only the snowflake.
    let list = PatternList::compile(&PatternListRaw {
        allow: vec![],
        deny: vec!["*admin*".into()],
    })
    .unwrap();
    let err = list
        .evaluate(["111222333", "server-admin"].iter().copied())
        .unwrap_err();
    assert!(matches!(err, DenyReason::DenyMatched { .. }));
}
