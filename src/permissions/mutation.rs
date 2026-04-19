//! The generic mutation vocabulary that every service's
//! `apply_mutation` dispatches over.
//!
//! The goal: have one typed surface for "queue a policy change" that
//! every service understands, so the shared CLI runner can parse clap
//! args once (e.g. `--function send --target channel --deny admin-*`)
//! and hand a typed `Mutation` to whichever service is active. Each
//! service's `PermissionsService::apply_mutation` implementation then
//! dispatches the `Mutation` onto the right field of its `*Raw`
//! struct.
//!
//! Not every service understands every mutation. A service should
//! return `ZadError::Invalid` with a message pointing at the field
//! that doesn't exist in its schema ("1pass has no `guild` target";
//! "telegram has no `attendee` target") — callers surface that as a
//! clear CLI error.

use serde::{Deserialize, Serialize};

/// Which list to edit within a pattern block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListKind {
    Allow,
    Deny,
}

impl ListKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ListKind::Allow => "allow",
            ListKind::Deny => "deny",
        }
    }
}

/// A single typed mutation. Callers build one of these from CLI args
/// or from any other source and pass it to
/// `PermissionsService::apply_mutation`. `function = None` targets
/// the top-level defaults; `function = Some("send")` narrows to one
/// verb's block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutation {
    /// Append a pattern to an allow/deny list. No-op if already present.
    AddPattern {
        function: Option<String>,
        target: String,
        list: ListKind,
        value: String,
    },
    /// Remove a pattern from an allow/deny list. No-op if not present.
    RemovePattern {
        function: Option<String>,
        target: String,
        list: ListKind,
        value: String,
    },

    /// Append a case-insensitive deny-word to the content rules.
    AddDenyWord {
        function: Option<String>,
        word: String,
    },
    /// Remove a deny-word.
    RemoveDenyWord {
        function: Option<String>,
        word: String,
    },

    /// Append a deny regex to the content rules.
    AddDenyRegex {
        function: Option<String>,
        pattern: String,
    },
    /// Remove a deny regex.
    RemoveDenyRegex {
        function: Option<String>,
        pattern: String,
    },

    /// Set (or clear with `None`) the `max_length` codepoint cap.
    SetMaxLength {
        function: Option<String>,
        value: Option<u32>,
    },

    /// Replace the time-window days list. Empty means "every day".
    SetTimeDays {
        function: Option<String>,
        days: Vec<String>,
    },
    /// Replace the time-window ranges. Empty means "any time of day".
    SetTimeWindows {
        function: Option<String>,
        windows: Vec<String>,
    },
}

impl Mutation {
    /// Short human-readable label used in diff/status output and error
    /// messages. Matches the shape operators would type.
    pub fn summary(&self) -> String {
        fn f(function: &Option<String>) -> String {
            match function {
                Some(name) => format!("[{name}]"),
                None => "[defaults]".to_string(),
            }
        }
        match self {
            Mutation::AddPattern {
                function,
                target,
                list,
                value,
            } => format!("{} {target}.{} += {value:?}", f(function), list.as_str()),
            Mutation::RemovePattern {
                function,
                target,
                list,
                value,
            } => format!("{} {target}.{} -= {value:?}", f(function), list.as_str()),
            Mutation::AddDenyWord { function, word } => {
                format!("{} content.deny_words += {word:?}", f(function))
            }
            Mutation::RemoveDenyWord { function, word } => {
                format!("{} content.deny_words -= {word:?}", f(function))
            }
            Mutation::AddDenyRegex { function, pattern } => {
                format!("{} content.deny_patterns += {pattern:?}", f(function))
            }
            Mutation::RemoveDenyRegex { function, pattern } => {
                format!("{} content.deny_patterns -= {pattern:?}", f(function))
            }
            Mutation::SetMaxLength { function, value } => {
                format!("{} content.max_length = {value:?}", f(function))
            }
            Mutation::SetTimeDays { function, days } => {
                format!("{} time.days = {days:?}", f(function))
            }
            Mutation::SetTimeWindows { function, windows } => {
                format!("{} time.windows = {windows:?}", f(function))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// helpers services reuse in apply_mutation
// ---------------------------------------------------------------------------

use crate::error::{Result, ZadError};

use super::content::ContentRulesRaw;
use super::pattern::PatternListRaw;
use super::time::{TimeWindowRaw, Weekday};

/// Append `value` to the right list on `plist` (idempotent).
pub fn apply_pattern_list(plist: &mut PatternListRaw, kind: ListKind, value: &str, add: bool) {
    let target = match kind {
        ListKind::Allow => &mut plist.allow,
        ListKind::Deny => &mut plist.deny,
    };
    if add {
        if !target.iter().any(|x| x == value) {
            target.push(value.to_string());
        }
    } else {
        target.retain(|x| x != value);
    }
}

/// Apply a `content.*` mutation to a [`ContentRulesRaw`] block. Returns
/// `Ok(true)` if `mutation` was handled, `Ok(false)` if it targets a
/// different axis (caller keeps trying), or `Err` if the requested
/// axis exists but the input is malformed.
pub fn apply_content(rules: &mut ContentRulesRaw, mutation: &Mutation) -> Result<bool> {
    match mutation {
        Mutation::AddDenyWord { word, .. } => {
            if !rules.deny_words.iter().any(|w| w == word) {
                rules.deny_words.push(word.clone());
            }
            Ok(true)
        }
        Mutation::RemoveDenyWord { word, .. } => {
            rules.deny_words.retain(|w| w != word);
            Ok(true)
        }
        Mutation::AddDenyRegex { pattern, .. } => {
            if !rules.deny_patterns.iter().any(|p| p == pattern) {
                rules.deny_patterns.push(pattern.clone());
            }
            Ok(true)
        }
        Mutation::RemoveDenyRegex { pattern, .. } => {
            rules.deny_patterns.retain(|p| p != pattern);
            Ok(true)
        }
        Mutation::SetMaxLength { value, .. } => {
            rules.max_length = value.map(|v| v as usize);
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Apply a `time.*` mutation to a [`TimeWindowRaw`] block. Validates
/// day names and `HH:MM-HH:MM` window strings at call time.
pub fn apply_time(window: &mut TimeWindowRaw, mutation: &Mutation) -> Result<bool> {
    match mutation {
        Mutation::SetTimeDays { days, .. } => {
            let mut parsed: Vec<Weekday> = Vec::with_capacity(days.len());
            for d in days {
                let w = match d.to_ascii_lowercase().as_str() {
                    "mon" => Weekday::Mon,
                    "tue" => Weekday::Tue,
                    "wed" => Weekday::Wed,
                    "thu" => Weekday::Thu,
                    "fri" => Weekday::Fri,
                    "sat" => Weekday::Sat,
                    "sun" => Weekday::Sun,
                    other => {
                        return Err(ZadError::Invalid(format!(
                            "invalid weekday `{other}`; expected mon|tue|wed|thu|fri|sat|sun"
                        )));
                    }
                };
                parsed.push(w);
            }
            window.days = parsed;
            Ok(true)
        }
        Mutation::SetTimeWindows { windows, .. } => {
            for w in windows {
                if w.split('-').count() != 2 {
                    return Err(ZadError::Invalid(format!(
                        "invalid time window `{w}`; expected `HH:MM-HH:MM`"
                    )));
                }
            }
            window.windows = windows.clone();
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Build a helpful error for unsupported mutations. Services call this
/// when none of their fields match.
pub fn unsupported(service: &str, mutation: &Mutation) -> ZadError {
    ZadError::Invalid(format!(
        "{service} permissions: mutation {} is not supported by this service's schema",
        mutation.summary()
    ))
}
