//! Profile-view counters with traffic-source breakdown.
//!
//! Piece 2 of the public-profile UX work. Every successful read of
//! `/v1/public/u/{handle}/profile` bumps a per-day, per-source counter
//! so the owner can see how their share is performing. The recorder
//! runs in a `tokio::spawn` off the public-profile path so a slow
//! counter UPSERT never holds up the response.
//!
//! Why a dedicated store (matching `share_metadata` / `share_reports`):
//!   * Postgres impl is the production path; the Memory impl lives in
//!     `mod test_support` and feeds the per-piece unit tests.
//!   * The `read_stats` aggregator is the same regardless of backend —
//!     callers reason about `ProfileViewStats` shape, not SQL.
//!
//! Vocabulary:
//!   * `ProfileViewSource` is closed at the application layer; stored
//!     as TEXT in the DB so adding a variant (e.g. `Embed`) doesn't
//!     need a migration. `parse()` / `as_str()` round-trip via the
//!     wire snake_case names. Anything we don't recognise on read maps
//!     to `Other`.
//!   * `read_stats(handle, days)` clamps `days` to `[1, 90]` at the
//!     handler layer so the SQL never has to defend a giant window.
//!     The store itself trusts the caller to pass a sane bound.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::BTreeMap;
use utoipa::ToSchema;

/// Closed traffic-source vocabulary. Stored as TEXT at the DB layer.
/// `Other` is the open variant for anything we couldn't classify (e.g.
/// a future referer source we haven't taught the recorder about) —
/// keeping a catch-all means a slipped classification still lands in
/// the counter rather than being silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileViewSource {
    Direct,
    Discover,
    Shared,
    Other,
}

impl ProfileViewSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Discover => "discover",
            Self::Shared => "shared",
            Self::Other => "other",
        }
    }

    /// Liberal parse — anything we don't recognise maps to `Other` so
    /// a stored value the application binary no longer knows about
    /// still surfaces under a catch-all bucket on read.
    pub fn parse(s: &str) -> Self {
        match s {
            "direct" => Self::Direct,
            "discover" => Self::Discover,
            "shared" => Self::Shared,
            _ => Self::Other,
        }
    }
}

/// One day's worth of views, broken down by source plus a daily total.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProfileViewDay {
    /// Day in `YYYY-MM-DD` UTC. ISO 8601 calendar date so callers can
    /// sort lexicographically without parsing.
    pub day: String,
    /// Per-source counts for the day. Sparse — sources with zero hits
    /// on a given day are absent.
    pub by_source: BTreeMap<String, u32>,
    /// Sum of `by_source` values for convenience.
    pub total: u32,
}

/// Aggregated totals across the window the caller requested.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProfileViewTotals {
    /// Lifetime views across all sources. Computed from the full table,
    /// not the windowed `days` slice, because owners care about the
    /// all-time view number on the card.
    pub all_time: u32,
    pub last_7d: u32,
    pub last_30d: u32,
    /// Per-source totals for the last 30 days. Same `Other`-bucketing
    /// applies as on the raw rows.
    pub by_source_30d: BTreeMap<String, u32>,
}

/// Response shape. `days` is newest-first; `totals` is pre-computed
/// so the card doesn't have to re-sum on the client.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProfileViewStats {
    pub days: Vec<ProfileViewDay>,
    pub totals: ProfileViewTotals,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileViewStatsError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, ProfileViewStatsError>;

#[async_trait]
pub trait ProfileViewStatsStore: Send + Sync + 'static {
    /// Bump the per-day, per-source counter by 1. Idempotent in the
    /// sense that re-running it bumps again — there is no de-dupe
    /// guard; that's a per-IP rate-limit problem upstream, not the
    /// counter's.
    async fn record_view(&self, profile_handle: &str, source: ProfileViewSource) -> Result<()>;

    /// Read the per-day breakdown plus the aggregate totals. `days` is
    /// the size of the per-day window; totals are computed across the
    /// whole table for `all_time` and over fixed 7-day / 30-day
    /// windows regardless of `days`. The caller (handler) is expected
    /// to have already clamped `days` to a sane range.
    async fn read_stats(&self, profile_handle: &str, days: u16) -> Result<ProfileViewStats>;
}

pub struct PostgresProfileViewStatsStore {
    pool: PgPool,
}

impl PostgresProfileViewStatsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProfileViewStatsStore for PostgresProfileViewStatsStore {
    async fn record_view(&self, profile_handle: &str, source: ProfileViewSource) -> Result<()> {
        // CURRENT_DATE is server-clock UTC; the read side aggregates by
        // the same column so the date arithmetic stays self-consistent
        // even if the host TZ is non-UTC. ON CONFLICT bumps in place.
        sqlx::query(
            r#"
            INSERT INTO public_profile_view_counters
                (profile_handle, day, source, view_count)
            VALUES (lower($1), CURRENT_DATE, $2, 1)
            ON CONFLICT (profile_handle, day, source) DO UPDATE
                SET view_count = public_profile_view_counters.view_count + 1
            "#,
        )
        .bind(profile_handle)
        .bind(source.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn read_stats(&self, profile_handle: &str, days: u16) -> Result<ProfileViewStats> {
        // Lifetime + windowed rollups in a single query so the handler
        // doesn't issue 4 separate round-trips. We compute everything
        // server-side and then re-shape into the response DTOs.
        //
        // `days` is bounded by the caller to ≤ 90, so the LIMIT-style
        // filter on `day >= CURRENT_DATE - $2::INTEGER` is safe.
        let day_rows: Vec<(NaiveDate, String, i32)> = sqlx::query_as(
            r#"
            SELECT day, source, view_count
            FROM public_profile_view_counters
            WHERE profile_handle = lower($1)
              AND day >= CURRENT_DATE - ($2::INTEGER - 1)
            ORDER BY day DESC, source ASC
            "#,
        )
        .bind(profile_handle)
        .bind(days as i32)
        .fetch_all(&self.pool)
        .await?;

        let totals_rows: Vec<(String, i32, i32, i32)> = sqlx::query_as(
            r#"
            SELECT
                source,
                SUM(view_count)::INTEGER AS all_time,
                COALESCE(SUM(view_count) FILTER (
                    WHERE day >= CURRENT_DATE - 6
                ), 0)::INTEGER AS last_7d,
                COALESCE(SUM(view_count) FILTER (
                    WHERE day >= CURRENT_DATE - 29
                ), 0)::INTEGER AS last_30d
            FROM public_profile_view_counters
            WHERE profile_handle = lower($1)
            GROUP BY source
            "#,
        )
        .bind(profile_handle)
        .fetch_all(&self.pool)
        .await?;

        Ok(aggregate_stats(&day_rows, &totals_rows))
    }
}

/// Reshape the two SQL projections into the wire `ProfileViewStats`.
/// Pure function so the unit tests cover the aggregation logic without
/// a live database.
fn aggregate_stats(
    day_rows: &[(NaiveDate, String, i32)],
    totals_rows: &[(String, i32, i32, i32)],
) -> ProfileViewStats {
    // Fold day rows into a NaiveDate → BTreeMap<String, u32> map.
    let mut days_by_date: BTreeMap<NaiveDate, BTreeMap<String, u32>> = BTreeMap::new();
    for (date, source, count) in day_rows {
        let normalised = ProfileViewSource::parse(source).as_str().to_string();
        let entry = days_by_date.entry(*date).or_default();
        *entry.entry(normalised).or_insert(0) += (*count).max(0) as u32;
    }

    // Convert to newest-first vec with per-day totals.
    let mut days: Vec<ProfileViewDay> = days_by_date
        .into_iter()
        .map(|(date, by_source)| {
            let total = by_source.values().copied().sum();
            ProfileViewDay {
                day: date.format("%Y-%m-%d").to_string(),
                by_source,
                total,
            }
        })
        .collect();
    days.sort_by(|a, b| b.day.cmp(&a.day));

    let mut all_time: u32 = 0;
    let mut last_7d: u32 = 0;
    let mut last_30d: u32 = 0;
    let mut by_source_30d: BTreeMap<String, u32> = BTreeMap::new();
    for (source, at, sevend, thirtyd) in totals_rows {
        let normalised = ProfileViewSource::parse(source).as_str().to_string();
        all_time = all_time.saturating_add((*at).max(0) as u32);
        last_7d = last_7d.saturating_add((*sevend).max(0) as u32);
        last_30d = last_30d.saturating_add((*thirtyd).max(0) as u32);
        if *thirtyd > 0 {
            *by_source_30d.entry(normalised).or_insert(0) += (*thirtyd).max(0) as u32;
        }
    }

    ProfileViewStats {
        days,
        totals: ProfileViewTotals {
            all_time,
            last_7d,
            last_30d,
            by_source_30d,
        },
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use chrono::{Duration, Utc};
    use std::sync::Mutex;

    /// In-memory `ProfileViewStatsStore` for route-layer + per-piece
    /// tests. Identity is `(profile_handle.lower(), day, source.as_str())`
    /// to mirror the production UPSERT key.
    #[derive(Default)]
    pub struct MemoryProfileViewStatsStore {
        rows: Mutex<Vec<(String, NaiveDate, ProfileViewSource, u32)>>,
    }

    impl MemoryProfileViewStatsStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// Test-only helper to seed rows for arbitrary days. Production
        /// recording always stamps `today` — this lets the read-side
        /// tests build a multi-day fixture without sleeping.
        pub fn seed(&self, handle: &str, day: NaiveDate, source: ProfileViewSource, count: u32) {
            let normalised = handle.to_ascii_lowercase();
            let mut rows = self.rows.lock().unwrap();
            for row in rows.iter_mut() {
                if row.0 == normalised && row.1 == day && row.2 == source {
                    row.3 = row.3.saturating_add(count);
                    return;
                }
            }
            rows.push((normalised, day, source, count));
        }
    }

    #[async_trait]
    impl ProfileViewStatsStore for MemoryProfileViewStatsStore {
        async fn record_view(&self, profile_handle: &str, source: ProfileViewSource) -> Result<()> {
            // Use today (UTC) to match the Postgres `CURRENT_DATE`.
            let today = Utc::now().date_naive();
            self.seed(profile_handle, today, source, 1);
            Ok(())
        }

        async fn read_stats(&self, profile_handle: &str, days: u16) -> Result<ProfileViewStats> {
            let needle = profile_handle.to_ascii_lowercase();
            let today = Utc::now().date_naive();
            let window_start = today - Duration::days((days as i64 - 1).max(0));
            let cutoff_7d = today - Duration::days(6);
            let cutoff_30d = today - Duration::days(29);

            let rows = self.rows.lock().unwrap();
            // Per-day projection.
            let day_rows: Vec<(NaiveDate, String, i32)> = rows
                .iter()
                .filter(|(h, d, _, _)| h == &needle && *d >= window_start)
                .map(|(_, d, s, c)| (*d, s.as_str().to_string(), *c as i32))
                .collect();

            // Per-source rollups (all-time + windowed).
            let mut by_source: BTreeMap<String, (i32, i32, i32)> = BTreeMap::new();
            for (h, d, s, c) in rows.iter() {
                if h != &needle {
                    continue;
                }
                let entry = by_source.entry(s.as_str().to_string()).or_insert((0, 0, 0));
                entry.0 += *c as i32;
                if *d >= cutoff_7d {
                    entry.1 += *c as i32;
                }
                if *d >= cutoff_30d {
                    entry.2 += *c as i32;
                }
            }
            let totals_rows: Vec<(String, i32, i32, i32)> = by_source
                .into_iter()
                .map(|(s, (at, sevend, thirtyd))| (s, at, sevend, thirtyd))
                .collect();

            Ok(aggregate_stats(&day_rows, &totals_rows))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use test_support::MemoryProfileViewStatsStore;

    #[test]
    fn source_round_trips_through_str() {
        for s in [
            ProfileViewSource::Direct,
            ProfileViewSource::Discover,
            ProfileViewSource::Shared,
            ProfileViewSource::Other,
        ] {
            assert_eq!(ProfileViewSource::parse(s.as_str()), s);
        }
    }

    #[test]
    fn source_parse_unknown_falls_back_to_other() {
        // Forward-compat: a future binary that stored a variant we don't
        // know about must still surface under a bucket on read.
        assert_eq!(ProfileViewSource::parse("embed"), ProfileViewSource::Other);
        assert_eq!(ProfileViewSource::parse(""), ProfileViewSource::Other);
    }

    #[tokio::test]
    async fn memory_store_records_today_under_correct_source() {
        let store = MemoryProfileViewStatsStore::new();
        store
            .record_view("Alice", ProfileViewSource::Direct)
            .await
            .unwrap();
        store
            .record_view("Alice", ProfileViewSource::Direct)
            .await
            .unwrap();
        store
            .record_view("Alice", ProfileViewSource::Discover)
            .await
            .unwrap();
        let stats = store.read_stats("alice", 30).await.unwrap();
        assert_eq!(stats.totals.all_time, 3);
        assert_eq!(stats.totals.last_7d, 3);
        assert_eq!(stats.totals.last_30d, 3);
        assert_eq!(stats.totals.by_source_30d.get("direct"), Some(&2));
        assert_eq!(stats.totals.by_source_30d.get("discover"), Some(&1));
        assert_eq!(stats.days.len(), 1);
        assert_eq!(stats.days[0].total, 3);
        assert_eq!(stats.days[0].by_source.get("direct"), Some(&2));
    }

    #[tokio::test]
    async fn memory_store_handle_lookup_is_case_insensitive() {
        // The recorder normalises to lower-case to match the Postgres
        // `lower()` predicate; reading under the original-case handle
        // must still find the row.
        let store = MemoryProfileViewStatsStore::new();
        store
            .record_view("ALICE", ProfileViewSource::Direct)
            .await
            .unwrap();
        let stats = store.read_stats("alice", 30).await.unwrap();
        assert_eq!(stats.totals.all_time, 1);
        let stats2 = store.read_stats("ALICE", 30).await.unwrap();
        assert_eq!(stats2.totals.all_time, 1);
    }

    #[tokio::test]
    async fn memory_store_empty_for_unknown_handle() {
        let store = MemoryProfileViewStatsStore::new();
        store
            .record_view("alice", ProfileViewSource::Direct)
            .await
            .unwrap();
        let stats = store.read_stats("bob", 30).await.unwrap();
        assert_eq!(stats.totals.all_time, 0);
        assert!(stats.days.is_empty());
        assert!(stats.totals.by_source_30d.is_empty());
    }

    #[tokio::test]
    async fn memory_store_windows_filter_by_age() {
        let store = MemoryProfileViewStatsStore::new();
        let today = Utc::now().date_naive();
        store.seed("alice", today, ProfileViewSource::Direct, 1);
        store.seed(
            "alice",
            today - Duration::days(3),
            ProfileViewSource::Direct,
            2,
        );
        store.seed(
            "alice",
            today - Duration::days(10),
            ProfileViewSource::Shared,
            5,
        );
        store.seed(
            "alice",
            today - Duration::days(45),
            ProfileViewSource::Discover,
            7,
        );

        let stats = store.read_stats("alice", 30).await.unwrap();
        // last_7d = today + 3-day-old direct = 3
        assert_eq!(stats.totals.last_7d, 3);
        // last_30d = today + 3d + 10d = 8
        assert_eq!(stats.totals.last_30d, 8);
        // all_time includes the 45-day-old discover hit too.
        assert_eq!(stats.totals.all_time, 15);
        // by_source_30d only counts the within-30d slice.
        assert_eq!(stats.totals.by_source_30d.get("direct"), Some(&3));
        assert_eq!(stats.totals.by_source_30d.get("shared"), Some(&5));
        // The 45-day-old discover hit is older than the 30d window.
        assert!(!stats.totals.by_source_30d.contains_key("discover"));
        // Day vec respects the 30-day window argument — the 45-day-old
        // row is excluded.
        let day_strs: Vec<&str> = stats.days.iter().map(|d| d.day.as_str()).collect();
        let excluded = (today - Duration::days(45)).format("%Y-%m-%d").to_string();
        assert!(!day_strs.contains(&excluded.as_str()));
    }

    #[tokio::test]
    async fn memory_store_day_window_is_inclusive_and_descending() {
        let store = MemoryProfileViewStatsStore::new();
        let today = Utc::now().date_naive();
        store.seed("alice", today, ProfileViewSource::Direct, 1);
        store.seed(
            "alice",
            today - Duration::days(1),
            ProfileViewSource::Direct,
            1,
        );
        store.seed(
            "alice",
            today - Duration::days(2),
            ProfileViewSource::Direct,
            1,
        );
        // days = 2 covers today + yesterday, NOT 2 days ago.
        let stats = store.read_stats("alice", 2).await.unwrap();
        assert_eq!(stats.days.len(), 2);
        assert_eq!(stats.days[0].day, today.format("%Y-%m-%d").to_string());
        assert_eq!(
            stats.days[1].day,
            (today - Duration::days(1)).format("%Y-%m-%d").to_string()
        );
    }

    #[test]
    fn aggregate_stats_buckets_unknown_source_into_other() {
        // Future-compat: a row stored under a source string the current
        // binary doesn't know about (because a newer binary persisted
        // it) lands in `other`. Production never writes such a value
        // today, but a forward-compat read prevents a stalled deploy.
        let today = Utc::now().date_naive();
        let day_rows = vec![(today, "embed".to_string(), 4)];
        let totals_rows = vec![("embed".to_string(), 4, 4, 4)];
        let stats = aggregate_stats(&day_rows, &totals_rows);
        assert_eq!(stats.days.len(), 1);
        assert_eq!(stats.days[0].by_source.get("other"), Some(&4));
        assert_eq!(stats.totals.by_source_30d.get("other"), Some(&4));
    }

    #[tokio::test]
    async fn postgres_store_constructs_against_lazy_pool() {
        // Smoke test: verifying the Postgres impl's connect_lazy path
        // builds without panicking. Any actual query would fail without
        // a live DB; the trait surface itself must be reachable so the
        // route layer that holds an `Arc<dyn ProfileViewStatsStore>` can
        // be instantiated for the negative-path tests.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/starstats_test_unused")
            .expect("connect_lazy is infallible for a syntactically valid URL");
        let _store = PostgresProfileViewStatsStore::new(pool);
    }
}
