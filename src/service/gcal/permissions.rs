//! Google Calendar permissions policy.
//!
//! A file at either of
//!
//! - `~/.zad/services/gcal/permissions.toml` (global)
//! - `~/.zad/projects/<slug>/services/gcal/permissions.toml` (local)
//!
//! narrows what a declared scope is actually allowed to do. Both files
//! are optional; when both exist, a call must pass **both** — local
//! can only add restrictions, never loosen the global baseline.
//!
//! The calendar surface has two target axes (`calendars` and
//! `attendees`) plus per-verb **numeric caps** (`max_future_days`,
//! `min_notice_minutes`, `max_attendees`) and a per-verb
//! `send_updates_allowed` pattern list that gates the `sendUpdates`
//! flag ("none" | "external" | "all"). Caps intersect across
//! global/local via `min()`; boolean caps intersect via `&&`
//! ("strictest wins").
//!
//! ## Verbs
//!
//! `list_calendars`, `get_calendar`, `list_events`, `get_event`,
//! `create_event`, `update_event`, `delete_event`, `invite`,
//! `remind`. `invite` and `remind` gate the `--add-attendee` and
//! `--add-reminder-minutes` flags on `events update` / `events create`
//! respectively — they're separate policy blocks precisely because an
//! operator might want to allow updates-in-general but not attendee
//! changes.
//!
//! ## Hard-coded safety caps (not configurable)
//!
//! Independent of the permissions file:
//!
//! - reminder minutes ≤ 40320 (4 weeks) — Google's own cap is 40320
//!   anyway; we fail early with a zad-shaped error so the operator
//!   sees `PermissionDenied` rather than a late Calendar API rejection.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::{Result, ZadError};
use crate::permissions::{
    content::{ContentRules, ContentRulesRaw},
    pattern::{PatternList, PatternListRaw},
    service::HasSignature,
    signing::{self, Signature, SigningKey},
    time::{TimeWindow, TimeWindowRaw},
};

/// Absolute hard cap on any single reminder's "minutes before start".
/// Google's own cap is 40320 (four weeks). We fail early so the error
/// points at the permissions file.
pub const HARD_REMINDER_MINUTES_CAP: u32 = 40320;

// ---------------------------------------------------------------------------
// on-disk schema (raw)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcalPermissionsRaw {
    #[serde(default)]
    pub content: ContentRulesRaw,
    #[serde(default)]
    pub time: TimeWindowRaw,

    #[serde(default)]
    pub list_calendars: FunctionBlockRaw,
    #[serde(default)]
    pub get_calendar: FunctionBlockRaw,
    #[serde(default)]
    pub list_events: FunctionBlockRaw,
    #[serde(default)]
    pub get_event: FunctionBlockRaw,
    #[serde(default)]
    pub create_event: FunctionBlockRaw,
    #[serde(default)]
    pub update_event: FunctionBlockRaw,
    #[serde(default)]
    pub delete_event: FunctionBlockRaw,
    #[serde(default)]
    pub invite: FunctionBlockRaw,
    #[serde(default)]
    pub remind: FunctionBlockRaw,

    /// Ed25519 signature over the canonical serialization of every
    /// other field. See [`crate::permissions::signing`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl HasSignature for GcalPermissionsRaw {
    fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }
    fn set_signature(&mut self, sig: Option<Signature>) {
        self.signature = sig;
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionBlockRaw {
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub calendars: PatternListRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub attendees: PatternListRaw,
    #[serde(default, skip_serializing_if = "ContentRulesRaw_is_default")]
    pub content: ContentRulesRaw,
    #[serde(default, skip_serializing_if = "TimeWindowRaw_is_default")]
    pub time: TimeWindowRaw,
    #[serde(default, skip_serializing_if = "PatternListRaw_is_default")]
    pub send_updates_allowed: PatternListRaw,

    // Numeric caps — intersected across global/local via min().
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_future_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_notice_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<u32>,
    /// When `true`, any write (`create_event`/`update_event`/
    /// `delete_event`) against a calendar whose `accessRole` in
    /// `calendarList` is NOT `"owner"` is denied. Guards against the
    /// "agent creates event on shared calendar and spams the whole
    /// team" class of incident.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_shared_calendars: Option<bool>,
}

#[allow(non_snake_case)]
fn PatternListRaw_is_default(v: &PatternListRaw) -> bool {
    v.allow.is_empty() && v.deny.is_empty()
}
#[allow(non_snake_case)]
fn ContentRulesRaw_is_default(v: &ContentRulesRaw) -> bool {
    v.deny_words.is_empty() && v.deny_patterns.is_empty() && v.max_length.is_none()
}
#[allow(non_snake_case)]
fn TimeWindowRaw_is_default(v: &TimeWindowRaw) -> bool {
    v.days.is_empty() && v.windows.is_empty()
}

// ---------------------------------------------------------------------------
// compiled form
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FunctionBlock {
    pub calendars: PatternList,
    pub attendees: PatternList,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub send_updates_allowed: PatternList,
    pub max_future_days: Option<u32>,
    pub min_notice_minutes: Option<u32>,
    pub max_attendees: Option<u32>,
    pub block_shared_calendars: Option<bool>,
}

impl FunctionBlock {
    fn compile(raw: &FunctionBlockRaw) -> Result<Self> {
        Ok(FunctionBlock {
            calendars: PatternList::compile(&raw.calendars).map_err(ZadError::Invalid)?,
            attendees: PatternList::compile(&raw.attendees).map_err(ZadError::Invalid)?,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            send_updates_allowed: PatternList::compile(&raw.send_updates_allowed)
                .map_err(ZadError::Invalid)?,
            max_future_days: raw.max_future_days,
            min_notice_minutes: raw.min_notice_minutes,
            max_attendees: raw.max_attendees,
            block_shared_calendars: raw.block_shared_calendars,
        })
    }
}

/// One file's worth of rules, compiled.
#[derive(Debug, Clone, Default)]
pub struct GcalPermissions {
    pub source: PathBuf,
    pub content: ContentRules,
    pub time: TimeWindow,
    pub list_calendars: FunctionBlock,
    pub get_calendar: FunctionBlock,
    pub list_events: FunctionBlock,
    pub get_event: FunctionBlock,
    pub create_event: FunctionBlock,
    pub update_event: FunctionBlock,
    pub delete_event: FunctionBlock,
    pub invite: FunctionBlock,
    pub remind: FunctionBlock,
}

impl GcalPermissions {
    fn compile(raw: &GcalPermissionsRaw, source: PathBuf) -> Result<Self> {
        Ok(GcalPermissions {
            source,
            content: ContentRules::compile(&raw.content).map_err(ZadError::Invalid)?,
            time: TimeWindow::compile(&raw.time).map_err(ZadError::Invalid)?,
            list_calendars: FunctionBlock::compile(&raw.list_calendars)?,
            get_calendar: FunctionBlock::compile(&raw.get_calendar)?,
            list_events: FunctionBlock::compile(&raw.list_events)?,
            get_event: FunctionBlock::compile(&raw.get_event)?,
            create_event: FunctionBlock::compile(&raw.create_event)?,
            update_event: FunctionBlock::compile(&raw.update_event)?,
            delete_event: FunctionBlock::compile(&raw.delete_event)?,
            invite: FunctionBlock::compile(&raw.invite)?,
            remind: FunctionBlock::compile(&raw.remind)?,
        })
    }

    fn block(&self, f: GcalFunction) -> &FunctionBlock {
        match f {
            GcalFunction::ListCalendars => &self.list_calendars,
            GcalFunction::GetCalendar => &self.get_calendar,
            GcalFunction::ListEvents => &self.list_events,
            GcalFunction::GetEvent => &self.get_event,
            GcalFunction::CreateEvent => &self.create_event,
            GcalFunction::UpdateEvent => &self.update_event,
            GcalFunction::DeleteEvent => &self.delete_event,
            GcalFunction::Invite => &self.invite,
            GcalFunction::Remind => &self.remind,
        }
    }
}

/// One per runtime verb + the two subsidiary policy blocks
/// (`Invite`, `Remind`) that gate attendee / reminder changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcalFunction {
    ListCalendars,
    GetCalendar,
    ListEvents,
    GetEvent,
    CreateEvent,
    UpdateEvent,
    DeleteEvent,
    Invite,
    Remind,
}

impl GcalFunction {
    pub fn name(self) -> &'static str {
        match self {
            GcalFunction::ListCalendars => "list_calendars",
            GcalFunction::GetCalendar => "get_calendar",
            GcalFunction::ListEvents => "list_events",
            GcalFunction::GetEvent => "get_event",
            GcalFunction::CreateEvent => "create_event",
            GcalFunction::UpdateEvent => "update_event",
            GcalFunction::DeleteEvent => "delete_event",
            GcalFunction::Invite => "invite",
            GcalFunction::Remind => "remind",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "list_calendars" => GcalFunction::ListCalendars,
            "get_calendar" => GcalFunction::GetCalendar,
            "list_events" => GcalFunction::ListEvents,
            "get_event" => GcalFunction::GetEvent,
            "create_event" => GcalFunction::CreateEvent,
            "update_event" => GcalFunction::UpdateEvent,
            "delete_event" => GcalFunction::DeleteEvent,
            "invite" => GcalFunction::Invite,
            "remind" => GcalFunction::Remind,
            other => {
                return Err(ZadError::Invalid(format!(
                    "unknown gcal function `{other}`; expected one of list_calendars, \
                     get_calendar, list_events, get_event, create_event, update_event, \
                     delete_event, invite, remind"
                )));
            }
        })
    }
}

// ---------------------------------------------------------------------------
// effective (global ∩ local)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EffectivePermissions {
    pub global: Option<GcalPermissions>,
    pub local: Option<GcalPermissions>,
}

impl EffectivePermissions {
    pub fn any(&self) -> bool {
        self.global.is_some() || self.local.is_some()
    }

    pub fn sources(&self) -> Vec<&Path> {
        let mut out: Vec<&Path> = vec![];
        if let Some(g) = &self.global {
            out.push(&g.source);
        }
        if let Some(l) = &self.local {
            out.push(&l.source);
        }
        out
    }

    fn layers(&self) -> impl Iterator<Item = &GcalPermissions> {
        self.global.iter().chain(self.local.iter())
    }

    /// Time-window gate for a given function. Runs at the top of every
    /// verb before any network call so a "denied" doesn't leak the
    /// resolved target name.
    pub fn check_time(&self, f: GcalFunction) -> Result<()> {
        for p in self.layers() {
            let merged = p.time.clone().merge(p.block(f).time.clone());
            if let Err(e) = merged.evaluate_now() {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Match a calendar against the `calendars` allow/deny list of the
    /// given function block. The aliases evaluated are the raw input
    /// (sigils stripped) and the resolved calendar ID. Calendar IDs
    /// are email-shaped (`primary`, `user@gmail.com`,
    /// `xxx@group.calendar.google.com`), so allow-listing by email
    /// glob works naturally.
    pub fn check_calendar(&self, f: GcalFunction, input: &str, resolved_id: &str) -> Result<()> {
        let stripped = input.strip_prefix('@').unwrap_or(input);
        let mut names: Vec<String> = vec![stripped.to_string(), resolved_id.to_string()];
        names.sort();
        names.dedup();
        let aliases: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

        for p in self.layers() {
            let list = &p.block(f).calendars;
            if list.is_empty() {
                continue;
            }
            if let Err(e) = list.evaluate(aliases.iter().copied()) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(&format!("calendar `{input}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Match an attendee email against the `attendees` list of the
    /// given function block. `self_email` (when non-`None`) is also
    /// considered an alias so `@me` → the authenticated email resolves
    /// patterns like `attendees.allow = ["@me"]`.
    pub fn check_attendee(
        &self,
        f: GcalFunction,
        email_input: &str,
        self_email: Option<&str>,
    ) -> Result<()> {
        let trimmed = email_input.trim();
        let stripped = trimmed.strip_prefix('@').unwrap_or(trimmed);
        let mut names: Vec<String> = vec![trimmed.to_string(), stripped.to_string()];
        // `@me` — substitute the authenticated email so policies
        // written against real addresses still match.
        if trimmed.eq_ignore_ascii_case("@me")
            && let Some(e) = self_email
        {
            names.push(e.to_string());
        }
        names.sort();
        names.dedup();
        let aliases: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

        for p in self.layers() {
            let list = &p.block(f).attendees;
            if list.is_empty() {
                continue;
            }
            if let Err(e) = list.evaluate(aliases.iter().copied()) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(&format!("attendee `{email_input}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Run content checks on a free-text body (event summary or
    /// description). Top-level `[content]` defaults merge into the
    /// per-function block (strictest wins).
    pub fn check_body(&self, f: GcalFunction, body: &str) -> Result<()> {
        for p in self.layers() {
            let merged = p.content.clone().merge(p.block(f).content.clone());
            if let Err(e) = merged.evaluate(body) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Gate `--send-updates`. When the function block has an empty
    /// allow-list (the common case), any value is accepted; otherwise
    /// the literal string (`"none"` | `"external"` | `"all"`) must
    /// match the allow/deny list.
    pub fn check_send_updates(&self, f: GcalFunction, value: &str) -> Result<()> {
        for p in self.layers() {
            let list = &p.block(f).send_updates_allowed;
            if list.is_empty() {
                continue;
            }
            if let Err(e) = list.evaluate([value]) {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: e.as_sentence(&format!("send-updates value `{value}`")),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// Enforce the numeric caps on an event write. The caller has
    /// already computed:
    ///
    /// - `start_in_future_days` — number of whole days between now
    ///   and `start`; negative if `start` is in the past.
    /// - `start_in_future_minutes` — whole minutes to `start`.
    /// - `attendee_count` — total attendees after the write is
    ///   applied (so `update --add-attendee` should pass `len() + 1`).
    ///
    /// Caps missing at a given layer contribute no constraint.
    pub fn check_event_caps(
        &self,
        f: GcalFunction,
        start_in_future_days: Option<i64>,
        start_in_future_minutes: Option<i64>,
        attendee_count: Option<u32>,
    ) -> Result<()> {
        for p in self.layers() {
            let b = p.block(f);
            if let Some(cap) = b.max_future_days
                && let Some(actual) = start_in_future_days
                && actual > cap as i64
            {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: format!(
                        "event start is {actual} days in the future; permissions cap is {cap}"
                    ),
                    config_path: p.source.clone(),
                });
            }
            if let Some(cap) = b.min_notice_minutes
                && let Some(actual) = start_in_future_minutes
                && actual < cap as i64
            {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: format!(
                        "event starts in {actual} minutes; permissions require at least {cap} minutes of notice"
                    ),
                    config_path: p.source.clone(),
                });
            }
            if let Some(cap) = b.max_attendees
                && let Some(actual) = attendee_count
                && actual > cap
            {
                return Err(ZadError::PermissionDenied {
                    function: f.name(),
                    reason: format!(
                        "event would have {actual} attendees; permissions cap is {cap}"
                    ),
                    config_path: p.source.clone(),
                });
            }
        }
        Ok(())
    }

    /// `true` iff any layer has `block_shared_calendars = true` for
    /// the given verb.
    pub fn block_shared_calendars(&self, f: GcalFunction) -> Option<PathBuf> {
        for p in self.layers() {
            if p.block(f).block_shared_calendars == Some(true) {
                return Some(p.source.clone());
            }
        }
        None
    }

    /// Enforce the hard-coded reminder cap. Called for every entry of
    /// `--reminder-minutes`. Not configurable by the permissions file;
    /// returns `PermissionDenied` with a synthesised "policy: built-in"
    /// config-path pointing at whichever permissions file is closest
    /// to the caller (or a zero path when neither file exists).
    pub fn check_reminder_minutes(&self, f: GcalFunction, minutes: u32) -> Result<()> {
        if minutes > HARD_REMINDER_MINUTES_CAP {
            let path = self
                .sources()
                .first()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            return Err(ZadError::PermissionDenied {
                function: f.name(),
                reason: format!(
                    "reminder `{minutes}` minutes exceeds the built-in safety cap of {HARD_REMINDER_MINUTES_CAP} minutes (four weeks)"
                ),
                config_path: path,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// paths + load
// ---------------------------------------------------------------------------

pub fn global_path() -> Result<PathBuf> {
    Ok(config::path::global_service_dir("gcal")?.join("permissions.toml"))
}

pub fn local_path_for(slug: &str) -> Result<PathBuf> {
    Ok(config::path::project_service_dir_for(slug, "gcal")?.join("permissions.toml"))
}

pub fn local_path_current() -> Result<PathBuf> {
    local_path_for(&config::path::project_slug()?)
}

pub fn load_file(path: &Path) -> Result<Option<GcalPermissions>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: GcalPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    signing::verify_raw(&raw, path)?;
    let compiled = GcalPermissions::compile(&raw, path.to_path_buf())
        .map_err(|e| wrap_compile_error(e, path))?;
    Ok(Some(compiled))
}

/// Read a file's raw policy (signature included) without compiling.
pub fn load_raw_file(path: &Path) -> Result<Option<GcalPermissionsRaw>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw_str = std::fs::read_to_string(path).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let raw: GcalPermissionsRaw = toml::from_str(&raw_str).map_err(|e| ZadError::TomlParse {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(Some(raw))
}

fn wrap_compile_error(err: ZadError, path: &Path) -> ZadError {
    match err {
        ZadError::Invalid(msg) => ZadError::Invalid(format!(
            "invalid permissions file {}: {msg}",
            path.display()
        )),
        other => other,
    }
}

pub fn load_effective() -> Result<EffectivePermissions> {
    let slug = config::path::project_slug()?;
    load_effective_for(&slug)
}

pub fn load_effective_for(slug: &str) -> Result<EffectivePermissions> {
    let global = load_file(&global_path()?)?;
    let local = load_file(&local_path_for(slug)?)?;
    Ok(EffectivePermissions { global, local })
}

pub fn save_file(path: &Path, raw: &GcalPermissionsRaw, key: &SigningKey) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let sig = signing::sign_raw(&to_write, key)?;
    to_write.set_signature(Some(sig));
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Write `raw` without signing. Staging-only.
pub fn save_unsigned(path: &Path, raw: &GcalPermissionsRaw) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ZadError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut to_write = raw.clone();
    to_write.set_signature(None);
    let body = toml::to_string_pretty(&to_write)?;
    std::fs::write(path, body).map_err(|e| ZadError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Starter policy written by `zad gcal permissions init`. Biased
/// toward safe defaults: narrow content filter, default-deny on
/// `delete_event`, and `block_shared_calendars` set on every write
/// verb so an agent can't accidentally touch a team calendar.
pub fn starter_template() -> GcalPermissionsRaw {
    GcalPermissionsRaw {
        content: ContentRulesRaw {
            deny_words: vec!["password".into(), "api_key".into(), "secret".into()],
            deny_patterns: vec![],
            max_length: Some(5000),
        },
        time: TimeWindowRaw::default(),
        list_calendars: FunctionBlockRaw::default(),
        get_calendar: FunctionBlockRaw::default(),
        list_events: FunctionBlockRaw::default(),
        get_event: FunctionBlockRaw::default(),
        create_event: FunctionBlockRaw {
            calendars: PatternListRaw {
                allow: vec!["primary".into()],
                deny: vec![],
            },
            max_future_days: Some(365),
            min_notice_minutes: Some(15),
            max_attendees: Some(20),
            send_updates_allowed: PatternListRaw {
                allow: vec!["none".into(), "external".into()],
                deny: vec![],
            },
            block_shared_calendars: Some(true),
            ..FunctionBlockRaw::default()
        },
        update_event: FunctionBlockRaw {
            calendars: PatternListRaw {
                allow: vec!["primary".into()],
                deny: vec![],
            },
            block_shared_calendars: Some(true),
            ..FunctionBlockRaw::default()
        },
        delete_event: FunctionBlockRaw {
            calendars: PatternListRaw {
                allow: vec![],
                deny: vec!["*".into()],
            },
            ..FunctionBlockRaw::default()
        },
        invite: FunctionBlockRaw::default(),
        remind: FunctionBlockRaw::default(),
        signature: None,
    }
}

// ---------------------------------------------------------------------------
// PermissionsService binding
// ---------------------------------------------------------------------------

/// Zero-sized type used to feed the shared permissions runner with
/// Google Calendar-specific bindings. See
/// [`crate::permissions::service::PermissionsService`].
pub struct PermissionsService;

impl crate::permissions::service::PermissionsService for PermissionsService {
    const NAME: &'static str = "gcal";
    type Raw = GcalPermissionsRaw;

    fn starter_template() -> Self::Raw {
        starter_template()
    }

    fn all_functions() -> &'static [&'static str] {
        &[
            "list_calendars",
            "get_calendar",
            "list_events",
            "get_event",
            "create_event",
            "update_event",
            "delete_event",
            "invite",
            "remind",
        ]
    }

    fn target_kinds() -> &'static [&'static str] {
        &["calendar", "attendee"]
    }
}
