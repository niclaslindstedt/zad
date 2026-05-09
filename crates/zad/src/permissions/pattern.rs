//! Glob/regex/exact-match patterns used by allow/deny lists.
//!
//! The TOML surface is strings. We parse them lazily (`PatternList::new`)
//! into a compiled representation so each inbound call is a set of cheap
//! regex matches instead of re-parsing every time. Three forms are
//! supported:
//!
//! - `re:<regex>` — full Rust `regex` syntax, anchored by the caller if
//!   they want it anchored. Compile errors surface at load time with the
//!   offending pattern.
//! - glob — `*` and `?` wildcards (`general`, `bot-*`, `team/*`).
//!   Everything else is treated literally, so `#admin` matches the
//!   literal string `#admin`.
//! - numeric — bare decimal strings match the resolved snowflake ID
//!   exactly. An agent that pastes an ID into an allow list gets the
//!   intuitive behavior without having to know about the name directory.

use regex::Regex;
use serde::{Deserialize, Serialize};

/// A compiled matcher. Cheap to clone (regex uses `Arc` internally).
#[derive(Debug, Clone)]
pub enum Pattern {
    /// Exact string match (case-sensitive). Used for numeric IDs and for
    /// plain glob-free names.
    Exact(String),
    /// A regex compiled from either the explicit `re:` prefix form or
    /// from a glob with wildcards translated to regex.
    Regex { source: String, compiled: Regex },
}

impl Pattern {
    /// Parse a single raw pattern. Returns an error with the offending
    /// input embedded so the permissions-file author can fix it without
    /// having to re-read the docs.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if let Some(body) = raw.strip_prefix("re:") {
            let re = Regex::new(body).map_err(|e| format!("invalid regex `{raw}`: {e}"))?;
            return Ok(Pattern::Regex {
                source: raw.to_string(),
                compiled: re,
            });
        }
        if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
            return Ok(Pattern::Exact(raw.to_string()));
        }
        if raw.contains('*') || raw.contains('?') {
            let re = glob_to_regex(raw);
            let compiled = Regex::new(&re).map_err(|e| format!("invalid glob `{raw}`: {e}"))?;
            return Ok(Pattern::Regex {
                source: raw.to_string(),
                compiled,
            });
        }
        Ok(Pattern::Exact(raw.to_string()))
    }

    /// Test the pattern against a single candidate string. Callers that
    /// hold multiple aliases for a target (input name, resolved ID,
    /// reverse-lookup names) should call this once per alias and OR the
    /// results.
    pub fn matches(&self, candidate: &str) -> bool {
        match self {
            Pattern::Exact(s) => s == candidate,
            Pattern::Regex { compiled, .. } => compiled.is_match(candidate),
        }
    }

    /// The original string the operator wrote, for use in error messages.
    pub fn source(&self) -> &str {
        match self {
            Pattern::Exact(s) => s,
            Pattern::Regex { source, .. } => source,
        }
    }
}

fn glob_to_regex(glob: &str) -> String {
    let mut out = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            // Regex metacharacters that could otherwise blow up when the
            // operator wrote a literal `.` in a channel name like
            // `team.ops.alerts`.
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('$');
    out
}

/// Allow+deny pair that together decide whether a candidate is admitted.
///
/// Evaluation order:
///  1. If any deny pattern matches, the candidate is denied.
///  2. Otherwise, if the allow list is empty, the candidate is admitted
///     (no allow-constraint expressed).
///  3. Otherwise, the candidate must match at least one allow pattern.
///
/// Operators can express "deny everything" with `allow = []` and
/// `deny = ["*"]`, or "only these" with an explicit allow list.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatternListRaw {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PatternList {
    pub allow: Vec<Pattern>,
    pub deny: Vec<Pattern>,
}

impl PatternList {
    pub fn compile(raw: &PatternListRaw) -> Result<Self, String> {
        let allow = raw
            .allow
            .iter()
            .map(|s| Pattern::parse(s))
            .collect::<Result<Vec<_>, _>>()?;
        let deny = raw
            .deny
            .iter()
            .map(|s| Pattern::parse(s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { allow, deny })
    }

    /// `true` if no rules were declared. Useful for short-circuiting
    /// merge logic and for the `show` CLI output.
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty()
    }

    /// Evaluate the list against a set of candidate names. The call is
    /// permitted iff none of the aliases is denied *and* either the
    /// allow list is empty or at least one alias is allowed.
    ///
    /// Returns `Ok(())` when admitted, `Err(matched_rule)` when denied —
    /// the caller embeds the rule text into the surfaced error message
    /// so the operator can find and edit the offending line.
    pub fn evaluate<'a, I>(&self, candidates: I) -> Result<(), DenyReason>
    where
        I: IntoIterator<Item = &'a str> + Clone,
    {
        for p in &self.deny {
            if candidates.clone().into_iter().any(|c| p.matches(c)) {
                return Err(DenyReason::DenyMatched {
                    pattern: p.source().to_string(),
                });
            }
        }
        if self.allow.is_empty() {
            return Ok(());
        }
        for p in &self.allow {
            if candidates.clone().into_iter().any(|c| p.matches(c)) {
                return Ok(());
            }
        }
        Err(DenyReason::AllowUnmatched)
    }
}

/// Reason a pattern list rejected a candidate. The caller turns this
/// into a `ZadError::PermissionDenied` with the specific function and
/// config path embedded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    DenyMatched { pattern: String },
    AllowUnmatched,
}

impl DenyReason {
    pub fn as_sentence(&self, target_label: &str) -> String {
        match self {
            DenyReason::DenyMatched { pattern } => {
                format!("{target_label} matched deny pattern `{pattern}`")
            }
            DenyReason::AllowUnmatched => {
                format!("{target_label} did not match any allow pattern")
            }
        }
    }
}
