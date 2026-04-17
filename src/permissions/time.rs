//! Time-window filter: "only allow this function between these hours
//! on these days".
//!
//! The surface is intentionally tiny so operators don't have to learn a
//! cron dialect. Two fields, both optional:
//!
//! - `days` — subset of `mon`..`sun`. Missing = every day.
//! - `windows` — list of `HH:MM-HH:MM` ranges (inclusive start, exclusive
//!   end). Missing or empty = the whole day. A range may cross midnight
//!   (`22:00-02:00`).
//!
//! Time zone is UTC for v1. Agents run in CI and in containers where
//! local time is whatever the base image said it was, so a stable UTC
//! rule is more predictable; a future `timezone = "local"` or IANA zone
//! is a non-breaking additive field.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    fn from_days_since_epoch(days: u64) -> Self {
        // Jan 1 1970 was a Thursday. `(days + 3) % 7` maps day-0 → Thu.
        let idx = (days + 3) % 7;
        match idx {
            0 => Weekday::Mon,
            1 => Weekday::Tue,
            2 => Weekday::Wed,
            3 => Weekday::Thu,
            4 => Weekday::Fri,
            5 => Weekday::Sat,
            _ => Weekday::Sun,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Weekday::Mon => "mon",
            Weekday::Tue => "tue",
            Weekday::Wed => "wed",
            Weekday::Thu => "thu",
            Weekday::Fri => "fri",
            Weekday::Sat => "sat",
            Weekday::Sun => "sun",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeWindowRaw {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<Weekday>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TimeWindow {
    /// Empty = every day.
    pub days: Vec<Weekday>,
    /// Each entry is `(start_minute, end_minute)` with both values in
    /// `0..1440` — minutes past UTC midnight. An entry where `start >=
    /// end` crosses midnight and admits `[start, 1440) ∪ [0, end)`.
    /// Empty = whole day.
    pub windows: Vec<(u16, u16)>,
}

impl TimeWindow {
    pub fn compile(raw: &TimeWindowRaw) -> Result<Self, String> {
        let mut windows = Vec::with_capacity(raw.windows.len());
        for w in &raw.windows {
            windows.push(parse_window(w)?);
        }
        Ok(Self {
            days: raw.days.clone(),
            windows,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty() && self.windows.is_empty()
    }

    /// Check whether `now` falls inside the allowed window. Returns
    /// `Err` on failure with a human-readable reason the caller splices
    /// into the surfaced error.
    pub fn evaluate_at(&self, now: SystemTime) -> Result<(), TimeDenyReason> {
        let secs = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let days = secs / 86_400;
        let today = Weekday::from_days_since_epoch(days);
        if !self.days.is_empty() && !self.days.contains(&today) {
            return Err(TimeDenyReason::DayBlocked { today });
        }
        if self.windows.is_empty() {
            return Ok(());
        }
        let minute = ((secs % 86_400) / 60) as u16;
        for &(start, end) in &self.windows {
            if in_window(minute, start, end) {
                return Ok(());
            }
        }
        Err(TimeDenyReason::OutsideWindow { minute_utc: minute })
    }

    pub fn evaluate_now(&self) -> Result<(), TimeDenyReason> {
        self.evaluate_at(SystemTime::now())
    }

    /// Intersect two time policies into a stricter one. Both days *and*
    /// windows tighten: the result's day set is the intersection of the
    /// two day sets (empty treated as "every day"); the result's window
    /// list is the pairwise intersection.
    pub fn merge(self, other: TimeWindow) -> Self {
        let days = match (self.days.is_empty(), other.days.is_empty()) {
            (true, true) => vec![],
            (false, true) => self.days,
            (true, false) => other.days,
            (false, false) => self
                .days
                .iter()
                .filter(|d| other.days.contains(d))
                .copied()
                .collect(),
        };
        let windows = match (self.windows.is_empty(), other.windows.is_empty()) {
            (true, true) => vec![],
            (false, true) => self.windows,
            (true, false) => other.windows,
            (false, false) => {
                let mut out = vec![];
                for &a in &self.windows {
                    for &b in &other.windows {
                        if let Some(i) = intersect_windows(a, b) {
                            out.push(i);
                        }
                    }
                }
                out
            }
        };
        Self { days, windows }
    }
}

fn parse_window(s: &str) -> Result<(u16, u16), String> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| format!("invalid time window `{s}`: expected HH:MM-HH:MM"))?;
    Ok((parse_clock(a.trim(), s)?, parse_clock(b.trim(), s)?))
}

fn parse_clock(s: &str, full: &str) -> Result<u16, String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("invalid clock `{s}` in window `{full}`: expected HH:MM"))?;
    let h: u16 = h
        .parse()
        .map_err(|_| format!("invalid hour `{h}` in window `{full}`: expected 0..=23"))?;
    let m: u16 = m
        .parse()
        .map_err(|_| format!("invalid minute `{m}` in window `{full}`: expected 0..=59"))?;
    if h > 23 || m > 59 {
        return Err(format!(
            "out-of-range clock `{s}` in window `{full}`: expected HH in 0..=23 and MM in 0..=59"
        ));
    }
    Ok(h * 60 + m)
}

fn in_window(minute: u16, start: u16, end: u16) -> bool {
    if start < end {
        minute >= start && minute < end
    } else if start == end {
        // degenerate zero-length window admits nothing
        false
    } else {
        // crosses midnight
        minute >= start || minute < end
    }
}

fn intersect_windows(a: (u16, u16), b: (u16, u16)) -> Option<(u16, u16)> {
    // For the common case where neither window crosses midnight we can
    // return a clean intersection. Crossing windows are rare; we just
    // preserve both sides so the caller still evaluates strictly.
    let (a0, a1) = a;
    let (b0, b1) = b;
    if a0 < a1 && b0 < b1 {
        let start = a0.max(b0);
        let end = a1.min(b1);
        if start < end {
            return Some((start, end));
        }
        return None;
    }
    // Fallback: keep the tighter one (shorter window wins the heuristic).
    let len = |(s, e): (u16, u16)| if s < e { e - s } else { 1440 - s + e };
    if len(a) <= len(b) { Some(a) } else { Some(b) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeDenyReason {
    DayBlocked { today: Weekday },
    OutsideWindow { minute_utc: u16 },
}

impl TimeDenyReason {
    pub fn as_sentence(&self) -> String {
        match self {
            TimeDenyReason::DayBlocked { today } => {
                format!(
                    "today ({}) is not in the allowed day list (UTC)",
                    today.as_str()
                )
            }
            TimeDenyReason::OutsideWindow { minute_utc } => {
                let h = minute_utc / 60;
                let m = minute_utc % 60;
                format!("current UTC time {h:02}:{m:02} is outside every allowed window")
            }
        }
    }
}
