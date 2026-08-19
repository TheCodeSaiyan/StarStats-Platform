//! Nightly reconcile for the `stat_event_counts` rollup.
//!
//! The rollup is maintained incrementally at ingest and rebuilt per-handle by
//! retention. This background job is defense-in-depth: once a day it recomputes
//! the whole rollup from `events` (the source of truth), corrects any drift,
//! removes orphan rows, and emits a drift metric so a silently-stuck rollup is
//! visible instead of quietly serving wrong numbers (the "green while doing
//! nothing" failure mode). It never touches `events` or any read path — worst
//! case it re-derives the rollup from truth, so it is self-correcting.

use sqlx::PgPool;
use std::time::Duration;

/// Cadence between successful reconciles.
pub const RECONCILE_INTERVAL_OK: Duration = Duration::from_secs(24 * 3600);
/// Backoff after a failed reconcile.
pub const RECONCILE_INTERVAL_FAIL: Duration = Duration::from_secs(3600);

/// Recompute `stat_event_counts` from `events` in one transaction: upsert every
/// `(handle, event_type)` count (updating only rows that actually differ, so the
/// drift count is meaningful), then delete rollup rows with no backing events.
/// The drift found is surfaced via metrics + a warn log, not the return value.
pub async fn run_reconcile(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let corrected: i64 = sqlx::query_scalar(
        r#"
        WITH truth AS (
            SELECT claimed_handle, event_type, COUNT(*) AS c,
                   MIN(event_timestamp) AS mn, MAX(event_timestamp) AS mx
            FROM events
            GROUP BY claimed_handle, event_type
        ), upsert AS (
            INSERT INTO stat_event_counts
                (claimed_handle, event_type, event_count, first_seen_at, last_seen_at)
            SELECT claimed_handle, event_type, c, mn, mx FROM truth
            ON CONFLICT (claimed_handle, event_type) DO UPDATE SET
                event_count   = EXCLUDED.event_count,
                first_seen_at = EXCLUDED.first_seen_at,
                last_seen_at  = EXCLUDED.last_seen_at,
                updated_at    = now()
            WHERE stat_event_counts.event_count IS DISTINCT FROM EXCLUDED.event_count
            RETURNING 1
        )
        SELECT COUNT(*)::BIGINT FROM upsert
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;

    let orphans_removed = sqlx::query(
        r#"
        DELETE FROM stat_event_counts s
        WHERE NOT EXISTS (
            SELECT 1 FROM events e
            WHERE e.claimed_handle = s.claimed_handle
              AND e.event_type = s.event_type
        )
        "#,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    tx.commit().await?;

    let corrected = corrected.max(0) as u64;
    let drift = corrected + orphans_removed;
    metrics::counter!("starstats_stat_rollup_reconcile_drift_total").increment(drift);
    metrics::gauge!("starstats_stat_rollup_reconcile_last_drift").set(drift as f64);
    if drift > 0 {
        tracing::warn!(
            corrected,
            orphans_removed,
            "stat_event_counts reconcile corrected drift"
        );
    }
    Ok(())
}

/// Spawn the daily reconcile loop (24h on success, 1h backoff on failure).
pub fn spawn_reconcile_loop(pool: PgPool) -> tokio::task::JoinHandle<()> {
    metrics::describe_counter!(
        "starstats_stat_rollup_reconcile_drift_total",
        "Cumulative rollup rows corrected/removed by the stat_event_counts reconcile"
    );
    tokio::spawn(async move {
        loop {
            let next = match run_reconcile(&pool).await {
                Ok(_) => RECONCILE_INTERVAL_OK,
                Err(e) => {
                    tracing::error!(error = %e, "stat rollup reconcile failed; backing off");
                    RECONCILE_INTERVAL_FAIL
                }
            };
            tokio::time::sleep(next).await;
        }
    })
}
