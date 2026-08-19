//! Database connection-pool observability.
//!
//! Samples the sqlx pool's connection counts into Prometheus gauges so
//! pool-exhaustion — the primary risk under the audit advisory-lock
//! contention path (see `docs/audit/postgres-performance-review-2026-07-22.md`
//! POOL-1) — is visible on `/metrics` instead of surfacing only as opaque
//! 5-second acquire timeouts. Complements `pg_stat_statements` (enabled in the
//! baseline migration) for the server-side query view.

use sqlx::PgPool;
use std::time::Duration;

/// Interval between pool-gauge samples.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);

/// Spawn a detached task that periodically publishes pool-connection gauges.
///
/// Emits:
/// - `starstats_db_pool_connections` — total connections currently owned by the pool.
/// - `starstats_db_pool_idle_connections` — idle (available) connections;
///   `connections - idle` approximates in-flight checkouts.
pub fn spawn(pool: PgPool) {
    metrics::describe_gauge!(
        "starstats_db_pool_connections",
        "Total Postgres connections owned by the sqlx pool"
    );
    metrics::describe_gauge!(
        "starstats_db_pool_idle_connections",
        "Idle (available) Postgres connections in the sqlx pool"
    );
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            tick.tick().await;
            metrics::gauge!("starstats_db_pool_connections").set(f64::from(pool.size()));
            metrics::gauge!("starstats_db_pool_idle_connections").set(pool.num_idle() as f64);
        }
    });
}
