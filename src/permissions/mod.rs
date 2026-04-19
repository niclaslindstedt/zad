//! Shared primitives for service permission policies.
//!
//! Every service in zad layers a per-service *permissions* policy on top
//! of its coarse *scopes*: scopes answer "is this family of operations
//! enabled at all?", permissions answer "is *this specific call* (to this
//! target, at this time, with this content) allowed?". Files live next to
//! the service's credentials:
//!
//! - global: `~/.zad/services/<service>/permissions.toml`
//! - local:  `~/.zad/projects/<slug>/services/<service>/permissions.toml`
//!
//! **Both files narrow independently** — a call must be allowed by every
//! file that exists. Absent files are treated as "no restrictions at this
//! scope". That way an operator can ship a strict global baseline and a
//! project can add *further* restrictions without ever loosening it.
//!
//! This module owns the reusable bits (pattern matching, content
//! filtering, time windows). Each service's schema (e.g.
//! `service::discord::permissions`) composes them into a per-function
//! policy.

pub mod attachments;
pub mod content;
pub mod pattern;
pub mod time;

pub use attachments::{AttachmentInfo, AttachmentRules};
pub use content::ContentRules;
pub use pattern::{Pattern, PatternList};
pub use time::{TimeWindow, Weekday};
