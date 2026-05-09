//! Content-based filters for outbound message bodies.
//!
//! Two surfaces:
//!
//! - `deny_words` — literal, case-insensitive substring matches. Meant
//!   for blunt "never mention this token" rules: `api_key`, `password`,
//!   a leaked customer name, etc. Word-boundary-aware (we lowercase both
//!   sides and substring-match) so an agent can't slip by with casing.
//! - `deny_patterns` — full regex. Anchor and escape as needed. Compile
//!   errors surface at load time.
//!
//! Plus an optional `max_length` cap that narrows Discord's 2000-char
//! hard limit further for an agent that should only ever send short
//! status pings.

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentRulesRaw {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_words: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct ContentRules {
    pub deny_words: Vec<String>,
    pub deny_patterns: Vec<Regex>,
    pub max_length: Option<usize>,
}

impl ContentRules {
    pub fn compile(raw: &ContentRulesRaw) -> Result<Self, String> {
        let deny_patterns = raw
            .deny_patterns
            .iter()
            .map(|s| Regex::new(s).map_err(|e| format!("invalid content deny pattern `{s}`: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        // Normalize the word list once at load time so the per-call
        // check is a single `to_lowercase` + substring sweep.
        let deny_words = raw.deny_words.iter().map(|s| s.to_lowercase()).collect();
        Ok(Self {
            deny_words,
            deny_patterns,
            max_length: raw.max_length,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.deny_words.is_empty() && self.deny_patterns.is_empty() && self.max_length.is_none()
    }

    /// Check a message body. Returns `Err` with an operator-facing
    /// reason on the first violation. Length is checked against
    /// codepoints (not bytes) to line up with Discord's own accounting.
    pub fn evaluate(&self, body: &str) -> Result<(), ContentDenyReason> {
        if let Some(max) = self.max_length {
            let len = body.chars().count();
            if len > max {
                return Err(ContentDenyReason::TooLong { len, max });
            }
        }
        if !self.deny_words.is_empty() {
            let lower = body.to_lowercase();
            for w in &self.deny_words {
                if lower.contains(w) {
                    return Err(ContentDenyReason::WordMatched { word: w.clone() });
                }
            }
        }
        for pat in &self.deny_patterns {
            if pat.is_match(body) {
                return Err(ContentDenyReason::PatternMatched {
                    pattern: pat.as_str().to_string(),
                });
            }
        }
        Ok(())
    }

    /// Combine two content policies into a stricter one. Deny lists are
    /// unioned (more denies = stricter). `max_length` takes the minimum
    /// of whatever is set.
    pub fn merge(mut self, other: ContentRules) -> Self {
        self.deny_words.extend(other.deny_words);
        self.deny_patterns.extend(other.deny_patterns);
        self.max_length = match (self.max_length, other.max_length) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentDenyReason {
    WordMatched { word: String },
    PatternMatched { pattern: String },
    TooLong { len: usize, max: usize },
}

impl ContentDenyReason {
    pub fn as_sentence(&self) -> String {
        match self {
            ContentDenyReason::WordMatched { word } => {
                format!("body contains denied word `{word}`")
            }
            ContentDenyReason::PatternMatched { pattern } => {
                format!("body matched deny pattern `{pattern}`")
            }
            ContentDenyReason::TooLong { len, max } => {
                format!("body is {len} characters; permissions cap is {max}")
            }
        }
    }
}
