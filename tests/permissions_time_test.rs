//! Time-window tests. `SystemTime` makes "now" awkward to test, so we
//! drive the lower-level `evaluate_at` entry point with explicit
//! `UNIX_EPOCH + secs` values instead.

use std::time::{Duration, UNIX_EPOCH};

use zad::permissions::time::{TimeDenyReason, TimeWindow, TimeWindowRaw, Weekday};

fn at_utc(y_mod: u16, min_of_day: u16, day_offset: u64) -> std::time::SystemTime {
    // Epoch Thu 1970-01-01 UTC. Offsetting by whole days keeps us
    // deterministic without pulling in a calendar crate.
    let _ = y_mod;
    UNIX_EPOCH + Duration::from_secs(day_offset * 86_400 + min_of_day as u64 * 60)
}

fn compile(raw: TimeWindowRaw) -> TimeWindow {
    TimeWindow::compile(&raw).unwrap()
}

#[test]
fn empty_policy_admits_every_instant() {
    let w = compile(TimeWindowRaw::default());
    assert!(w.is_empty());
    // Any arbitrary timestamp.
    assert!(w.evaluate_at(at_utc(0, 12 * 60, 10)).is_ok());
}

#[test]
fn day_list_blocks_outside_weekdays() {
    // Jan 1 1970 is Thursday. Restrict to Mon-Wed: Thursday must fail.
    let w = compile(TimeWindowRaw {
        days: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed],
        windows: vec![],
    });
    let err = w.evaluate_at(at_utc(0, 12 * 60, 0)).unwrap_err();
    assert!(matches!(
        err,
        TimeDenyReason::DayBlocked {
            today: Weekday::Thu
        }
    ));
    // Five days later (Tuesday) must admit.
    assert!(w.evaluate_at(at_utc(0, 12 * 60, 5)).is_ok());
}

#[test]
fn window_parse_rejects_garbage() {
    let err = TimeWindow::compile(&TimeWindowRaw {
        days: vec![],
        windows: vec!["09:00-25:00".into()],
    })
    .unwrap_err();
    assert!(err.contains("out-of-range"), "err: {err}");
}

#[test]
fn plain_window_admits_inside_excludes_outside() {
    let w = compile(TimeWindowRaw {
        days: vec![],
        windows: vec!["09:00-18:00".into()],
    });
    assert!(w.evaluate_at(at_utc(0, 9 * 60, 0)).is_ok(), "09:00 is in");
    assert!(
        w.evaluate_at(at_utc(0, 17 * 60 + 59, 0)).is_ok(),
        "17:59 is in"
    );
    assert!(
        w.evaluate_at(at_utc(0, 18 * 60, 0)).is_err(),
        "18:00 is the exclusive end"
    );
    assert!(w.evaluate_at(at_utc(0, 8 * 60 + 59, 0)).is_err());
}

#[test]
fn window_across_midnight_admits_both_sides() {
    let w = compile(TimeWindowRaw {
        days: vec![],
        windows: vec!["22:00-02:00".into()],
    });
    assert!(w.evaluate_at(at_utc(0, 23 * 60, 0)).is_ok());
    assert!(w.evaluate_at(at_utc(0, 60, 0)).is_ok()); // 00:01
    assert!(w.evaluate_at(at_utc(0, 12 * 60, 0)).is_err());
}

#[test]
fn merge_intersects_days_and_windows() {
    let a = compile(TimeWindowRaw {
        days: vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ],
        windows: vec!["09:00-18:00".into()],
    });
    let b = compile(TimeWindowRaw {
        days: vec![Weekday::Wed, Weekday::Thu],
        windows: vec!["10:00-17:00".into()],
    });
    let merged = a.merge(b);
    // Day 0 = Thu → intersection admits.
    assert!(merged.evaluate_at(at_utc(0, 11 * 60, 0)).is_ok());
    // 09:30 is inside a but outside b → merged must reject.
    assert!(merged.evaluate_at(at_utc(0, 9 * 60 + 30, 0)).is_err());
    // Day +6 = Wed (epoch+6 days = Wed) → admitted.
    assert!(merged.evaluate_at(at_utc(0, 12 * 60, 6)).is_ok());
    // Day +1 = Fri not in b → rejected.
    assert!(merged.evaluate_at(at_utc(0, 12 * 60, 1)).is_err());
}
