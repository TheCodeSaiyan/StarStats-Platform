//! Per-user operational insight for the admin console: device sync
//! state, entry counts, and retention context.
//!
//! Everything here is BATCHED by design. The users list renders 50 rows
//! at a time and already paid one staff-role query per row; adding
//! three more per-user lookups would have made a page ~200 queries.
//! `sync_states` and `activity` take the whole page's key set and
//! return a map.
//!
//! Two joins in here are sharp:
//!
//!   * `stat_event_counts` is keyed by `claimed_handle` and carries
//!     `CHECK (claimed_handle = lower(claimed_handle))`, while `users`
//!     stores the handle in its display casing. Passing the users-table
//!     casing matches NOTHING and returns zero counts — with a 200 and
//!     no error. Always lowercase before querying.
//!
//!   * Retention tier is NOT the supporter pill. A `lapsed` supporter
//!     keeps the pill but reverts to free-tier retention (migration
//!     0017), so only `state = 'active'` maps to the `supporter` tier.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

/// How long after a device's last check-in we stop calling it live.
///
/// Seven days, not 24 hours: the tray only reports while the game is
/// running, so a shorter window would mark ordinary weekday players
/// stale and make the column meaningless.
pub const SYNC_STALE_AFTER_DAYS: i64 = 7;

/// Whether a user's tray is actually feeding us data.
///
/// Four states rather than a boolean because they need different
/// operator responses: `Off` is a user choice, `Stale` is usually a
/// broken install, and `Never` means they never finished pairing. A
/// boolean would collapse all three into "not syncing".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Never,
    Off,
    Stale,
    Live,
}

impl SyncState {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncState::Never => "never",
            SyncState::Off => "off",
            SyncState::Stale => "stale",
            SyncState::Live => "live",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub label: String,
    pub sync_enabled: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Summary counts for the users LIST.
///
/// Deliberately no `oldest_entry_at`: that value belongs to
/// [`RetentionContext`], which is the only place it is rendered.
/// Carrying it here too would mean two sources for one number.
#[derive(Debug, Clone, Default)]
pub struct ActivitySummary {
    pub entry_count: i64,
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct EventTypeCount {
    pub event_type: String,
    pub event_count: i64,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct RetentionContext {
    /// `free` or `supporter` — the tier used for RETENTION, which is
    /// not necessarily the tier shown on the supporter pill.
    pub tier: String,
    /// `None` means unlimited retention. Never render this as `0`.
    pub retention_days: Option<i32>,
    pub oldest_entry_at: Option<DateTime<Utc>>,
    /// Timestamp before which this user's events are eligible for
    /// purging. `None` when retention is unlimited.
    pub cutoff: Option<DateTime<Utc>>,
}

/// Classify a user's sync state from their device fleet.
///
/// Pure, and takes `now` explicitly rather than reading the clock, so
/// the tests can pin boundaries. (Building test times by SUBTRACTING
/// from `Utc::now()` is also how you get a CI-only underflow panic on a
/// freshly booted runner; the tests add to a fixed base instead.)
///
/// Precedence: a live device beats a stale one beats a disabled one.
/// The newest check-in across the fleet wins — a user with one dead
/// laptop and one active desktop is syncing.
pub fn classify_sync(devices: &[DeviceRow], now: DateTime<Utc>) -> SyncState {
    let active: Vec<&DeviceRow> = devices.iter().filter(|d| d.revoked_at.is_none()).collect();
    if active.is_empty() {
        return SyncState::Never;
    }

    let syncing: Vec<&&DeviceRow> = active.iter().filter(|d| d.sync_enabled).collect();
    if syncing.is_empty() {
        return SyncState::Off;
    }

    let threshold = now - Duration::days(SYNC_STALE_AFTER_DAYS);
    // `>` not `>=`: a check-in exactly ON the threshold is already
    // stale, which is what the boundary test pins.
    let any_fresh = syncing
        .iter()
        .any(|d| d.last_seen_at.is_some_and(|seen| seen > threshold));

    if any_fresh {
        SyncState::Live
    } else {
        SyncState::Stale
    }
}

/// Batched per-user insight lookups.
#[async_trait::async_trait]
pub trait AdminUserInsightsStore: Send + Sync {
    /// One query for the whole page. Users with no device rows are
    /// absent from the map and must be treated as [`SyncState::Never`].
    async fn sync_states(&self, user_ids: &[Uuid])
        -> Result<HashMap<Uuid, SyncState>, sqlx::Error>;

    /// One query for the whole page. **Keys are LOWERCASED handles** —
    /// callers must lowercase before lookup as well as before querying.
    async fn activity(
        &self,
        lowercased_handles: &[String],
    ) -> Result<HashMap<String, ActivitySummary>, sqlx::Error>;

    async fn devices(&self, user_id: Uuid) -> Result<Vec<DeviceRow>, sqlx::Error>;

    async fn event_type_counts(&self, handle: &str) -> Result<Vec<EventTypeCount>, sqlx::Error>;

    async fn retention(&self, user_id: Uuid, handle: &str)
        -> Result<RetentionContext, sqlx::Error>;
}

// -- Postgres impl ---------------------------------------------------------

pub struct PostgresAdminUserInsights {
    pool: sqlx::PgPool,
}

impl PostgresAdminUserInsights {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl AdminUserInsightsStore for PostgresAdminUserInsights {
    /// ONE query for the whole page, grouped in Rust. Emphatically not
    /// one query per user — the list renders 50 rows.
    async fn sync_states(
        &self,
        user_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, SyncState>, sqlx::Error> {
        if user_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(
            Uuid,
            String,
            bool,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT user_id, label, sync_enabled, last_seen_at, revoked_at
             FROM devices
             WHERE user_id = ANY($1)",
        )
        .bind(user_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut by_user: HashMap<Uuid, Vec<DeviceRow>> = HashMap::new();
        for (user_id, label, sync_enabled, last_seen_at, revoked_at) in rows {
            by_user.entry(user_id).or_default().push(DeviceRow {
                label,
                sync_enabled,
                last_seen_at,
                revoked_at,
            });
        }

        let now = Utc::now();
        Ok(by_user
            .into_iter()
            .map(|(id, devices)| (id, classify_sync(&devices, now)))
            .collect())
    }

    /// ONE query for the whole page.
    ///
    /// `lowercased_handles` must already be lowercased: the rollup
    /// enforces `CHECK (claimed_handle = lower(claimed_handle))`, so
    /// display-cased handles match nothing and every user silently
    /// reads zero entries.
    async fn activity(
        &self,
        lowercased_handles: &[String],
    ) -> Result<HashMap<String, ActivitySummary>, sqlx::Error> {
        if lowercased_handles.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(String, i64, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT claimed_handle,
                    COALESCE(SUM(event_count), 0)::BIGINT AS entry_count,
                    MAX(last_seen_at) AS last_activity_at
             FROM stat_event_counts
             WHERE claimed_handle = ANY($1)
             GROUP BY claimed_handle",
        )
        .bind(lowercased_handles)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(handle, entry_count, last_activity_at)| {
                (
                    handle,
                    ActivitySummary {
                        entry_count,
                        last_activity_at,
                    },
                )
            })
            .collect())
    }

    async fn devices(&self, user_id: Uuid) -> Result<Vec<DeviceRow>, sqlx::Error> {
        let rows: Vec<(String, bool, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
            sqlx::query_as(
                "SELECT label, sync_enabled, last_seen_at, revoked_at
                 FROM devices
                 WHERE user_id = $1
                 ORDER BY created_at DESC",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(label, sync_enabled, last_seen_at, revoked_at)| DeviceRow {
                    label,
                    sync_enabled,
                    last_seen_at,
                    revoked_at,
                },
            )
            .collect())
    }

    async fn event_type_counts(&self, handle: &str) -> Result<Vec<EventTypeCount>, sqlx::Error> {
        let rows: Vec<(String, i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
            sqlx::query_as(
                "SELECT event_type, event_count, first_seen_at, last_seen_at
                 FROM stat_event_counts
                 WHERE claimed_handle = $1
                 ORDER BY event_count DESC",
            )
            .bind(handle.to_lowercase())
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(event_type, event_count, first_seen_at, last_seen_at)| EventTypeCount {
                    event_type,
                    event_count,
                    first_seen_at,
                    last_seen_at,
                },
            )
            .collect())
    }

    async fn retention(
        &self,
        user_id: Uuid,
        handle: &str,
    ) -> Result<RetentionContext, sqlx::Error> {
        // Only an ACTIVE supporter gets supporter retention. `lapsed`
        // keeps the pill but reverts to free-tier retention, so mapping
        // the pill state straight to a tier would over-report how long
        // a lapsed user's data survives.
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM supporter_status WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
        let tier = if state.as_deref() == Some("active") {
            "supporter"
        } else {
            "free"
        };

        let retention_days: Option<i32> =
            sqlx::query_scalar("SELECT retention_days FROM retention_policies WHERE tier = $1")
                .bind(tier)
                .fetch_optional(&self.pool)
                .await?
                .flatten();

        let oldest_entry_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT MIN(first_seen_at) FROM stat_event_counts WHERE claimed_handle = $1",
        )
        .bind(handle.to_lowercase())
        .fetch_optional(&self.pool)
        .await?
        .flatten();

        Ok(RetentionContext {
            tier: tier.to_string(),
            retention_days,
            oldest_entry_at,
            // Unlimited retention has no cutoff. Rendering one as an
            // epoch date would claim data is being purged when it isn't.
            cutoff: retention_days.map(|d| Utc::now() - Duration::days(d as i64)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// `seen_days_ago == None` means paired but never checked in.
    fn device(sync: bool, seen_days_ago: Option<i64>, revoked: bool) -> DeviceRow {
        DeviceRow {
            label: "DESKTOP".into(),
            sync_enabled: sync,
            last_seen_at: seen_days_ago.map(|d| base() - Duration::days(d)),
            revoked_at: if revoked { Some(base()) } else { None },
        }
    }

    #[test]
    fn no_devices_is_never() {
        assert_eq!(classify_sync(&[], base()), SyncState::Never);
    }

    #[test]
    fn only_revoked_devices_is_never() {
        let d = vec![device(true, Some(0), true)];
        assert_eq!(classify_sync(&d, base()), SyncState::Never);
    }

    #[test]
    fn active_device_without_sync_is_off() {
        let d = vec![device(false, Some(0), false)];
        assert_eq!(classify_sync(&d, base()), SyncState::Off);
    }

    #[test]
    fn sync_enabled_seen_recently_is_live() {
        let d = vec![device(true, Some(1), false)];
        assert_eq!(classify_sync(&d, base()), SyncState::Live);
    }

    #[test]
    fn sync_enabled_seen_long_ago_is_stale() {
        let d = vec![device(true, Some(8), false)];
        assert_eq!(classify_sync(&d, base()), SyncState::Stale);
    }

    #[test]
    fn sync_enabled_never_seen_is_stale_not_live() {
        // A paired device that has never reported is not "live" — that
        // would paint a green chip on an install that never worked.
        let d = vec![device(true, None, false)];
        assert_eq!(classify_sync(&d, base()), SyncState::Stale);
    }

    #[test]
    fn newest_device_wins_across_a_mixed_fleet() {
        let d = vec![
            device(true, Some(30), false),
            device(true, Some(1), false),
            device(false, Some(0), false),
        ];
        assert_eq!(classify_sync(&d, base()), SyncState::Live);
    }

    #[test]
    fn boundary_exactly_at_threshold_is_stale() {
        let d = vec![device(true, Some(SYNC_STALE_AFTER_DAYS), false)];
        assert_eq!(classify_sync(&d, base()), SyncState::Stale);
    }

    #[test]
    fn revoked_device_does_not_mask_an_active_one() {
        let d = vec![
            device(true, Some(0), true),   // revoked, would look live
            device(false, Some(0), false), // the only real device
        ];
        assert_eq!(classify_sync(&d, base()), SyncState::Off);
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// Empty insights double for route tests that need the extension
    /// present but do not exercise it.
    #[derive(Default)]
    pub struct MemoryAdminUserInsights;

    impl MemoryAdminUserInsights {
        pub fn new() -> Self {
            Self
        }
    }

    #[async_trait::async_trait]
    impl AdminUserInsightsStore for MemoryAdminUserInsights {
        async fn sync_states(
            &self,
            _user_ids: &[Uuid],
        ) -> Result<HashMap<Uuid, SyncState>, sqlx::Error> {
            Ok(HashMap::new())
        }

        async fn activity(
            &self,
            _lowercased_handles: &[String],
        ) -> Result<HashMap<String, ActivitySummary>, sqlx::Error> {
            Ok(HashMap::new())
        }

        async fn devices(&self, _user_id: Uuid) -> Result<Vec<DeviceRow>, sqlx::Error> {
            Ok(Vec::new())
        }

        async fn event_type_counts(
            &self,
            _handle: &str,
        ) -> Result<Vec<EventTypeCount>, sqlx::Error> {
            Ok(Vec::new())
        }

        async fn retention(
            &self,
            _user_id: Uuid,
            _handle: &str,
        ) -> Result<RetentionContext, sqlx::Error> {
            Ok(RetentionContext {
                tier: "free".into(),
                retention_days: Some(90),
                oldest_entry_at: None,
                cutoff: None,
            })
        }
    }
}
