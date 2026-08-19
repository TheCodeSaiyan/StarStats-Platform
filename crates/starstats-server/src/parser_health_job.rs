//! Parser-health background pass: window query, orchestration, scheduling.
//!
//! Once a day this recomputes the recent-vs-baseline window from `events`,
//! runs the pure detector in [`crate::parser_health`], and persists findings
//! plus a heartbeat via [`crate::parser_health_store`].
//!
//! Deliberately NOT incremental. An earlier design kept a per-day rollup, but
//! events arrive late — the tray's rotated-log backfill inserts rows dated
//! weeks earlier — so "day D is final" is never true and the trailing window
//! would need recomputing anyway. Recomputing the whole 35-day window each
//! pass is simpler and cannot drift. Result cardinality is `types × handles`,
//! not events, so it stays small however much history accumulates.

use crate::parser_health::{detect, DetectorConfig, Finding, HandleTypeCount};
use crate::parser_health_store::ParserHealthStore;
use chrono::{DateTime, Duration, Utc};
use metrics::{counter, gauge};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration as StdDuration;

/// Cadence between successful passes.
pub const PASS_INTERVAL_OK: StdDuration = StdDuration::from_secs(24 * 3600);
/// Backoff after a failed pass.
pub const PASS_INTERVAL_FAIL: StdDuration = StdDuration::from_secs(3600);
/// Delay before the first pass so startup isn't competing with migrations
/// and cache priming for the pool.
pub const PASS_INITIAL_DELAY: StdDuration = StdDuration::from_secs(120);

/// Distinct from `retention::SWEEP_ADVISORY_LOCK_KEY` — 'parsHLTH'.
const HEALTH_ADVISORY_LOCK_KEY: i64 = 0x7061_7273_484c_5448;

#[derive(Debug, thiserror::Error)]
pub enum HealthPassError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("store error: {0}")]
    Store(#[from] crate::repo::RepoError),
}

/// The window boundaries a pass measured, so the heartbeat can record them.
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub baseline_start: DateTime<Utc>,
    pub recent_start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Window {
    pub fn ending_at(now: DateTime<Utc>, cfg: &DetectorConfig) -> Self {
        let recent_start = now - Duration::days(cfg.recent_days);
        Self {
            baseline_start: recent_start - Duration::days(cfg.baseline_days),
            recent_start,
            end: now,
        }
    }
}

/// One aggregate over `events` covering baseline + recent.
///
/// Uses `event_timestamp` (when the thing happened in game) rather than
/// `received_at` (when we got it). A user whose tray was offline and uploads
/// a week of play in one batch would otherwise register as a single-day
/// spike. Rows with NULL `event_timestamp` carry no position on the axis
/// being measured and are excluded by the range predicate.
pub async fn fetch_window(
    pool: &PgPool,
    window: Window,
) -> Result<Vec<HandleTypeCount>, sqlx::Error> {
    let rows: Vec<(String, String, i64, i64, Option<DateTime<Utc>>)> = sqlx::query_as(
        r#"
        SELECT event_type,
               claimed_handle,
               COUNT(*) FILTER (WHERE event_timestamp >= $2)::BIGINT AS recent_n,
               COUNT(*) FILTER (WHERE event_timestamp <  $2)::BIGINT AS baseline_n,
               MAX(event_timestamp)                                  AS last_event_at
        FROM events
        WHERE event_timestamp >= $1
          AND event_timestamp <  $3
        GROUP BY event_type, claimed_handle
        "#,
    )
    .bind(window.baseline_start)
    .bind(window.recent_start)
    .bind(window.end)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(event_type, claimed_handle, recent_n, baseline_n, last_event_at)| HandleTypeCount {
                event_type,
                claimed_handle,
                recent_n,
                baseline_n,
                last_event_at,
            },
        )
        .collect())
}

/// Persist a pass's findings: upsert each, then auto-resolve anything
/// previously flagged that is no longer collapsing.
async fn persist(
    store: &dyn ParserHealthStore,
    findings: &[Finding],
) -> Result<i64, crate::repo::RepoError> {
    for f in findings {
        store.upsert_finding(f).await?;
    }
    let still: Vec<String> = findings.iter().map(|f| f.event_type.clone()).collect();
    let recovered = store.auto_resolve_absent(&still).await?;
    if recovered > 0 {
        tracing::info!(
            recovered,
            "parser-health findings auto-resolved as recovered"
        );
    }
    store.count_open().await
}

/// Run one full pass. Records a heartbeat row whether or not anything is
/// found, and whether or not the pass succeeds.
///
/// The heartbeat is load-bearing, not bookkeeping: without it "no findings"
/// and "the detector is dead" look identical from outside, which is exactly
/// the failure mode this feature exists to catch.
pub async fn run_pass(
    pool: &PgPool,
    store: &dyn ParserHealthStore,
    cfg: &DetectorConfig,
) -> Result<usize, HealthPassError> {
    let window = Window::ending_at(Utc::now(), cfg);
    let run_id = store.start_run().await?;

    let rows = match fetch_window(pool, window).await {
        Ok(rows) => rows,
        Err(e) => {
            // Record the failure ON the heartbeat before propagating, so a
            // persistently broken pass is visible in the admin surface
            // rather than only in logs.
            let _ = store
                .finish_run(
                    run_id,
                    window.baseline_start,
                    window.end,
                    0,
                    0,
                    Some(e.to_string()),
                )
                .await;
            return Err(e.into());
        }
    };

    let types_examined = {
        let mut types: Vec<&str> = rows.iter().map(|r| r.event_type.as_str()).collect();
        types.sort_unstable();
        types.dedup();
        types.len()
    };

    let findings = detect(&rows, cfg);
    let open = persist(store, &findings).await?;

    store
        .finish_run(
            run_id,
            window.baseline_start,
            window.end,
            types_examined as i64,
            open,
            None,
        )
        .await?;

    gauge!("starstats_parser_health_open_findings").set(open as f64);
    gauge!("starstats_parser_health_last_success_timestamp").set(Utc::now().timestamp() as f64);
    counter!("starstats_parser_health_passes_total", "outcome" => "completed").increment(1);

    for f in &findings {
        tracing::warn!(
            event_type = %f.event_type,
            severity = f.severity.as_str(),
            baseline_events = f.baseline_events,
            recent_events = f.recent_events,
            carried_handles = f.carried_handles,
            affected_handles = f.affected_handles,
            "parser-health: event type has collapsed"
        );
    }

    Ok(findings.len())
}

/// Wrap [`run_pass`] in a cross-replica advisory lock.
///
/// Double-running would not corrupt anything — the upserts are idempotent —
/// but a concurrent pass could observe a half-written finding set and
/// auto-resolve a finding that is still live. A transiently hidden finding is
/// precisely what this feature must never produce.
///
/// SESSION-level advisory locks are per-CONNECTION, so lock and unlock MUST
/// run on the SAME connection; taking them from the pool separately leaks the
/// lock and every later tick skips forever. Pin one connection for the lock's
/// lifetime (same lesson as `retention::run_sweep`).
pub async fn run_pass_locked(
    pool: &PgPool,
    store: &dyn ParserHealthStore,
    cfg: &DetectorConfig,
) -> Result<usize, HealthPassError> {
    let mut lock_conn = pool.acquire().await?;
    let acquired: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(HEALTH_ADVISORY_LOCK_KEY)
        .fetch_one(&mut *lock_conn)
        .await?;
    if !acquired.0 {
        // Counted so a stuck lock (every tick skipping, none completing) is
        // distinguishable from a busy peer.
        counter!("starstats_parser_health_passes_total", "outcome" => "skipped_lock").increment(1);
        tracing::info!("parser-health pass: advisory lock held elsewhere; skipping tick");
        return Ok(0);
    }

    let result = run_pass(pool, store, cfg).await;

    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(HEALTH_ADVISORY_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await
    {
        tracing::error!(error = %e, "parser-health: pg_advisory_unlock failed");
    }

    result
}

/// Background loop. Never touches the ingest path; a failed pass logs, backs
/// off, and leaves no state to repair — the next pass recomputes the window
/// from scratch.
pub fn spawn_health_loop(
    pool: PgPool,
    store: Arc<dyn ParserHealthStore>,
) -> tokio::task::JoinHandle<()> {
    let cfg = DetectorConfig::from_env();
    tokio::spawn(async move {
        tokio::time::sleep(PASS_INITIAL_DELAY).await;
        loop {
            let next = match run_pass_locked(&pool, store.as_ref(), &cfg).await {
                Ok(n) => {
                    tracing::info!(findings = n, "parser-health pass complete");
                    PASS_INTERVAL_OK
                }
                Err(e) => {
                    counter!("starstats_parser_health_passes_total", "outcome" => "failed")
                        .increment(1);
                    tracing::error!(error = %e, "parser-health pass failed; retrying after backoff");
                    PASS_INTERVAL_FAIL
                }
            };
            tokio::time::sleep(next).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser_health::Severity;
    use crate::parser_health_store::test_support::MemoryParserHealthStore;
    use crate::parser_health_store::FindingStatus;

    fn cfg() -> DetectorConfig {
        DetectorConfig::default()
    }

    #[test]
    fn window_places_recent_immediately_after_baseline() {
        let now = DateTime::parse_from_rfc3339("2026-08-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let w = Window::ending_at(now, &cfg());

        assert_eq!(w.end, now);
        assert_eq!(w.recent_start, now - Duration::days(7));
        assert_eq!(w.baseline_start, now - Duration::days(35));
        // No gap and no overlap between the two windows.
        assert_eq!(w.recent_start - w.baseline_start, Duration::days(28));
    }

    #[tokio::test]
    async fn persist_upserts_findings_and_reports_open_count() {
        let store = MemoryParserHealthStore::new();
        let findings = vec![Finding {
            event_type: "vehicle_stowed".into(),
            severity: Severity::Dark,
            baseline_events: 1_900,
            recent_events: 0,
            share_baseline: 0.1,
            share_recent: 0.0,
            baseline_handles: 3,
            carried_handles: 3,
            affected_handles: 3,
            last_event_at: None,
        }];

        let open = persist(&store, &findings).await.unwrap();

        assert_eq!(open, 1);
        assert_eq!(store.list_findings().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn persist_auto_resolves_a_finding_that_stopped_collapsing() {
        let store = MemoryParserHealthStore::new();
        let findings = vec![Finding {
            event_type: "vehicle_stowed".into(),
            severity: Severity::Dark,
            baseline_events: 1_900,
            recent_events: 0,
            share_baseline: 0.1,
            share_recent: 0.0,
            baseline_handles: 3,
            carried_handles: 3,
            affected_handles: 3,
            last_event_at: None,
        }];
        persist(&store, &findings).await.unwrap();

        // Next pass finds nothing — the parser fix landed.
        let open = persist(&store, &[]).await.unwrap();

        assert_eq!(open, 0);
        let all = store.list_findings().await.unwrap();
        assert_eq!(all[0].status, FindingStatus::Resolved);
        assert_eq!(all[0].resolved_reason.as_deref(), Some("recovered"));
    }

    #[test]
    fn detector_config_reads_overrides_from_env() {
        // Scoped to a unique key so it cannot collide with a parallel test.
        std::env::set_var("STARSTATS_PARSER_HEALTH_MIN_BASELINE_EVENTS", "42");
        let c = DetectorConfig::from_env();
        std::env::remove_var("STARSTATS_PARSER_HEALTH_MIN_BASELINE_EVENTS");

        assert_eq!(c.min_baseline_events, 42);
        assert_eq!(c.recent_days, DetectorConfig::default().recent_days);
    }

    #[test]
    fn malformed_env_override_falls_back_to_default() {
        std::env::set_var("STARSTATS_PARSER_HEALTH_COLLAPSE_FRACTION", "not-a-number");
        let c = DetectorConfig::from_env();
        std::env::remove_var("STARSTATS_PARSER_HEALTH_COLLAPSE_FRACTION");

        assert_eq!(
            c.collapse_fraction,
            DetectorConfig::default().collapse_fraction
        );
    }
}
