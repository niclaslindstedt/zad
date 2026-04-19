//! File-attachment filters for outbound messages.
//!
//! Four surfaces, all optional:
//!
//! - `max_count` — hard cap on the number of files in a single send.
//! - `max_size_bytes` — per-file size cap.
//! - `extensions` — allow/deny list matched against the lowercased file
//!   extension (no leading dot). Reuses [`PatternList`] so globs
//!   (`png`, `*`, `re:^[a-z]+$`) all work.
//! - `deny_filenames` — deny-only pattern list matched against the
//!   basename so blanket "never upload `.env*`" rules are expressible.
//!
//! The compiled [`AttachmentRules`] is merged across global + local
//! layers in the same "intersect" fashion as the other primitives:
//! minima win for caps, deny lists union, allow lists union too (the
//! underlying `PatternList::evaluate` already returns `Ok` on any allow
//! match, so a union is still strictly tighter than either half alone).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::permissions::pattern::{DenyReason, PatternList, PatternListRaw};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentRulesRaw {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub extensions: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub deny_filenames: PatternListRaw,
}

#[allow(non_snake_case)]
fn PatternListRaw_is_default(v: &PatternListRaw) -> bool {
    v.allow.is_empty() && v.deny.is_empty()
}

#[derive(Debug, Clone, Default)]
pub struct AttachmentRules {
    pub max_count: Option<usize>,
    pub max_size_bytes: Option<u64>,
    pub extensions: PatternList,
    pub deny_filenames: PatternList,
}

/// Metadata for a single attachment. Built by the CLI layer (which has
/// filesystem access) and handed to [`AttachmentRules::evaluate`]; kept
/// independent of any specific service so both Discord and Telegram
/// share the same check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentInfo {
    pub path: PathBuf,
    pub basename: String,
    /// Lowercased extension without the leading dot. Empty string when
    /// the file has no extension.
    pub extension: String,
    pub bytes: u64,
}

impl AttachmentInfo {
    /// Build an [`AttachmentInfo`] from a path by calling
    /// `std::fs::metadata`. Callers (only the CLI layer) turn the
    /// `io::Error` into a `ZadError::Invalid` so the user gets a
    /// sentence like `file "./foo.txt" not readable: No such file …`.
    pub fn probe(path: &Path) -> Result<Self, std::io::Error> {
        let md = std::fs::metadata(path)?;
        let basename = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let extension = path
            .extension()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        Ok(Self {
            path: path.to_path_buf(),
            basename,
            extension,
            bytes: md.len(),
        })
    }
}

impl AttachmentRules {
    pub fn compile(raw: &AttachmentRulesRaw) -> Result<Self, String> {
        Ok(Self {
            max_count: raw.max_count,
            max_size_bytes: raw.max_size_bytes,
            extensions: PatternList::compile(&raw.extensions)
                .map_err(|e| format!("invalid attachments.extensions: {e}"))?,
            deny_filenames: PatternList::compile(&raw.deny_filenames)
                .map_err(|e| format!("invalid attachments.deny_filenames: {e}"))?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.max_count.is_none()
            && self.max_size_bytes.is_none()
            && self.extensions.is_empty()
            && self.deny_filenames.is_empty()
    }

    /// Check a set of attachments. Returns on the first violation with
    /// an operator-facing reason. Empty input is always admitted —
    /// attachments-that-aren't-there can't violate anything.
    pub fn evaluate(&self, files: &[AttachmentInfo]) -> Result<(), AttachmentDenyReason> {
        if let Some(max) = self.max_count
            && files.len() > max
        {
            return Err(AttachmentDenyReason::TooMany {
                n: files.len(),
                max,
            });
        }
        for f in files {
            if let Some(max) = self.max_size_bytes
                && f.bytes > max
            {
                return Err(AttachmentDenyReason::TooLarge {
                    name: f.basename.clone(),
                    bytes: f.bytes,
                    max,
                });
            }
            if !self.deny_filenames.is_empty()
                && let Err(DenyReason::DenyMatched { pattern }) =
                    self.deny_filenames.evaluate([f.basename.as_str()])
            {
                return Err(AttachmentDenyReason::FilenameDenied {
                    name: f.basename.clone(),
                    pattern,
                });
            }
            if !self.extensions.is_empty() {
                match self.extensions.evaluate([f.extension.as_str()]) {
                    Ok(()) => {}
                    Err(DenyReason::DenyMatched { pattern }) => {
                        return Err(AttachmentDenyReason::ExtensionDenied {
                            name: f.basename.clone(),
                            ext: f.extension.clone(),
                            pattern,
                        });
                    }
                    Err(DenyReason::AllowUnmatched) => {
                        return Err(AttachmentDenyReason::ExtensionNotAllowed {
                            name: f.basename.clone(),
                            ext: f.extension.clone(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Combine two policies into a stricter one. Caps take the `min` of
    /// whatever is set; pattern lists union their `allow` and `deny`
    /// halves — both halves of any allow list still need to be satisfied
    /// at evaluation time, which is a strictly tighter constraint than
    /// either alone.
    pub fn merge(mut self, other: AttachmentRules) -> Self {
        self.max_count = merge_min(self.max_count, other.max_count);
        self.max_size_bytes = merge_min(self.max_size_bytes, other.max_size_bytes);
        self.extensions.allow.extend(other.extensions.allow);
        self.extensions.deny.extend(other.extensions.deny);
        self.deny_filenames.allow.extend(other.deny_filenames.allow);
        self.deny_filenames.deny.extend(other.deny_filenames.deny);
        self
    }
}

fn merge_min<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentDenyReason {
    TooMany {
        n: usize,
        max: usize,
    },
    TooLarge {
        name: String,
        bytes: u64,
        max: u64,
    },
    ExtensionDenied {
        name: String,
        ext: String,
        pattern: String,
    },
    ExtensionNotAllowed {
        name: String,
        ext: String,
    },
    FilenameDenied {
        name: String,
        pattern: String,
    },
}

impl AttachmentDenyReason {
    pub fn as_sentence(&self) -> String {
        match self {
            AttachmentDenyReason::TooMany { n, max } => {
                format!("{n} attachments exceeds permissions cap of {max}")
            }
            AttachmentDenyReason::TooLarge { name, bytes, max } => {
                format!("attachment `{name}` is {bytes} bytes; permissions cap is {max}")
            }
            AttachmentDenyReason::ExtensionDenied { name, ext, pattern } => {
                if ext.is_empty() {
                    format!("attachment `{name}` (no extension) matched deny pattern `{pattern}`")
                } else {
                    format!(
                        "attachment `{name}` extension `{ext}` matched deny pattern `{pattern}`"
                    )
                }
            }
            AttachmentDenyReason::ExtensionNotAllowed { name, ext } => {
                if ext.is_empty() {
                    format!("attachment `{name}` has no extension; not in allow list")
                } else {
                    format!("attachment `{name}` extension `{ext}` is not in allow list")
                }
            }
            AttachmentDenyReason::FilenameDenied { name, pattern } => {
                format!("attachment filename `{name}` matched deny pattern `{pattern}`")
            }
        }
    }
}
