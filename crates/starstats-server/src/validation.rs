//! Shared validation + rendering helpers used by the read-side query
//! handlers (`query.rs`) and the sharing read endpoints
//! (`sharing_routes.rs`).
//!
//! Both modules apply the same `days` window to `/timeline` requests
//! and the same `[a-z0-9_]{1,64}` rule to `event_type` filters; keeping
//! the constants + validators in one place avoids drift between the
//! "owner" and "public/friend" code paths.

use chrono::{Duration, NaiveDate, Utc};

/// Hard cap for the trailing window on `/timeline` endpoints.
/// Bumped to 366 in v0.0.2-beta so the new `YearHeatmap` card can
/// consume `?days=365` without tripping the validator.
pub const TIMELINE_DAYS_MAX: u32 = 366;
/// Default window when the client omits `?days=`.
pub const TIMELINE_DAYS_DEFAULT: u32 = 30;
/// Max byte length of an `event_type` filter (matches the column).
pub const EVENT_TYPE_MAX_LEN: usize = 64;

/// `true` when the handle matches the RSI handle format used throughout
/// the sharing subsystem: ASCII alphanumeric + `_` + `-`, 1–64 chars.
///
/// Applied before any SpiceDB or Postgres call that takes a handle so
/// path/body arguments cannot smuggle whitespace or wildcards into
/// authz checks. Length cap matches the `claimed_handle` column
/// (varchar 64).
pub fn validate_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 64
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `true` when the input matches the `[a-z0-9_]{1,64}` pattern. Used
/// to gate the `event_type` query parameter before it touches SQL.
pub fn is_valid_event_type(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= EVENT_TYPE_MAX_LEN
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Resolve the `days` query parameter to a clamped, validated window.
/// Returns `Err(())` if the caller asked for `0` or anything above the
/// hard cap; the handler turns that into a 400 `invalid_days`.
pub fn resolve_timeline_days(days: Option<u32>) -> Result<u32, ()> {
    let d = days.unwrap_or(TIMELINE_DAYS_DEFAULT);
    if d == 0 || d > TIMELINE_DAYS_MAX {
        Err(())
    } else {
        Ok(d)
    }
}

/// Zero-pad the observed counts into exactly `days` `(YYYY-MM-DD, count)`
/// buckets ending today (UTC). The front-end renders a continuous bar
/// series, so missing days must materialise as zeroes.
pub fn build_timeline_buckets(rows: Vec<(NaiveDate, i64)>, days: u32) -> Vec<(String, u64)> {
    let observed: std::collections::HashMap<NaiveDate, u64> = rows
        .into_iter()
        .map(|(d, c)| (d, c.max(0) as u64))
        .collect();

    let today = Utc::now().date_naive();
    let start = today - Duration::days(days as i64 - 1);
    let mut buckets = Vec::with_capacity(days as usize);
    let mut cursor = start;
    while cursor <= today {
        buckets.push((
            cursor.format("%Y-%m-%d").to_string(),
            observed.get(&cursor).copied().unwrap_or(0),
        ));
        cursor += Duration::days(1);
    }
    buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_handle_accepts_alphanumeric_dash_underscore() {
        assert!(validate_handle("alice"));
        assert!(validate_handle("Bob123"));
        assert!(validate_handle("star-citizen_fan"));
        assert!(validate_handle("a"));
        assert!(validate_handle(&"x".repeat(64)));
    }

    #[test]
    fn validate_handle_rejects_invalid() {
        assert!(!validate_handle(""));
        assert!(!validate_handle("in valid")); // space
        assert!(!validate_handle("wild*card"));
        assert!(!validate_handle(&"x".repeat(65))); // too long
    }

    #[test]
    fn event_type_accepts_lowercase_digits_underscore() {
        assert!(is_valid_event_type("join_pu"));
        assert!(is_valid_event_type("a"));
        assert!(is_valid_event_type("event_42"));
    }

    #[test]
    fn event_type_rejects_uppercase_dash_empty_too_long() {
        assert!(!is_valid_event_type(""));
        assert!(!is_valid_event_type("Join_PU"));
        assert!(!is_valid_event_type("event-42"));
        let too_long = "a".repeat(EVENT_TYPE_MAX_LEN + 1);
        assert!(!is_valid_event_type(&too_long));
    }

    #[test]
    fn timeline_days_default_when_none() {
        assert_eq!(resolve_timeline_days(None), Ok(TIMELINE_DAYS_DEFAULT));
    }

    #[test]
    fn timeline_days_rejects_zero_and_overflow() {
        assert!(resolve_timeline_days(Some(0)).is_err());
        assert!(resolve_timeline_days(Some(TIMELINE_DAYS_MAX + 1)).is_err());
        assert_eq!(
            resolve_timeline_days(Some(TIMELINE_DAYS_MAX)),
            Ok(TIMELINE_DAYS_MAX)
        );
    }

    #[test]
    fn timeline_buckets_zero_pads() {
        let today = Utc::now().date_naive();
        let buckets = build_timeline_buckets(vec![(today, 5)], 3);
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets.last().unwrap().1, 5);
        // Earlier days zero-padded.
        assert_eq!(buckets[0].1, 0);
    }
}
