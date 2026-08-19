//! Tier-based data retention purge.
//!
//! Per docs/ENGINEERING.md mission: server-side personal events are kept for a
//! tier-specific window, after which they are deleted. Tier is derived
//! at read-time from [`supporter_status`] (active -> supporter, else
//! free). The retention window per tier lives in the
//! `retention_policies` table (migration 0036) so it is tunable via a
//! SQL update without a code deploy.
//!
//! Only the `events` table is in scope. Hangar snapshots, share rows,
//! audit log, and devices are NOT purged here -- they have their own
//! lifecycles (see migration comments + admin sharing surface).
//!
//! Concurrency: `run_sweep` wraps the whole pass in a Postgres
//! advisory lock so multi-replica deployments don't double-delete.
//! A non-blocking `pg_try_advisory_lock` is used; if the lock is held
//! we log + skip the tick.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use metrics::{counter, gauge};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use crate::audit::{AuditEntry, AuditLog};

/// Advisory lock key used to fence concurrent sweeps across replicas.
/// Arbitrary 64-bit constant; only this module reads it.
const SWEEP_ADVISORY_LOCK_KEY: i64 = 0x7374_6172_5354_4154; // 'starSTAT' in ASCII

/// Maximum rows deleted per single DELETE statement. Bounds the
/// transaction size so a backlog purge can't blow up Postgres' write
/// log. Picked to match the public-profile-views purge batching.
const DELETE_BATCH_SIZE: i64 = 1_000;

/// Throttle between consecutive batches for the same user, so a
/// backlog purge doesn't monopolise the connection or the write log.
const BATCH_THROTTLE: StdDuration = StdDuration::from_millis(100);

/// Safety cap: never delete more than this many rows for a single
/// user in a single sweep, even if their backlog is huge. The next
/// sweep picks up where we left off. Prevents one outlier from
/// starving every other user's purge in the same tick.
const PER_USER_BATCH_CAP: usize = 100;

/// Closed-vocabulary tier enum stored implicitly via supporter_status.
/// Per docs/ENGINEERING.md convention: TEXT round-trip helpers, adding a variant
/// does not need a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    Free,
    Supporter,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Supporter => "supporter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "free" => Self::Free,
            "supporter" => Self::Supporter,
            _ => return None,
        })
    }
}

/// Derive a user's tier from a supporter_status `state` string. Anything
/// other than `active` (none, lapsed, unrecognised) maps to Free -- the
/// supporter_status migration comment makes this explicit: lapsed users
/// keep the pill but lose the retention extension.
pub fn tier_from_supporter_state(state: &str) -> Tier {
    if state == "active" {
        Tier::Supporter
    } else {
        Tier::Free
    }
}

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub tier: Tier,
    /// `None` means unlimited retention (no purge runs).
    pub retention_days: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait RetentionPolicyStore: Send + Sync + 'static {
    /// Returns every tier policy. The job loop reads this once per
    /// sweep, so callers don't need a `get_one` helper.
    async fn list_all(&self) -> Result<Vec<RetentionPolicy>, RetentionError>;
}

pub struct PostgresRetentionPolicyStore {
    pool: PgPool,
}

impl PostgresRetentionPolicyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RetentionPolicyStore for PostgresRetentionPolicyStore {
    async fn list_all(&self) -> Result<Vec<RetentionPolicy>, RetentionError> {
        let rows: Vec<(String, Option<i32>)> =
            sqlx::query_as("SELECT tier, retention_days FROM retention_policies ORDER BY tier")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(tier, days)| {
                Tier::parse(&tier).map(|t| RetentionPolicy {
                    tier: t,
                    retention_days: days,
                })
            })
            .collect())
    }
}

/// Summary of a single sweep. Surfaced by the admin trigger endpoint
/// + logged at INFO at the end of each scheduled sweep.
#[derive(Debug, Clone, Default)]
pub struct SweepSummary {
    /// Number of users inspected (had at least one events row newer
    /// than 0 -- actually we just count every user we considered).
    pub users_considered: u64,
    /// Number of users whose tier maps to NULL retention_days, so
    /// nothing was deleted for them.
    pub users_unlimited: u64,
    /// Number of users for whom DELETE ran one or more batches.
    pub users_purged: u64,
    /// Total event rows deleted across all users this sweep.
    pub events_deleted: u64,
    /// Number of users where we hit `PER_USER_BATCH_CAP` and the
    /// next sweep will continue from where we stopped.
    pub users_truncated: u64,
}

/// Per-user purge: keep deleting batches until either 0 rows come
/// back or we hit [`PER_USER_BATCH_CAP`]. Returns (rows_deleted,
/// hit_cap).
async fn purge_user_events(
    pool: &PgPool,
    claimed_handle: &str,
    cutoff: DateTime<Utc>,
) -> Result<(u64, bool), RetentionError> {
    let mut total: u64 = 0;
    let mut batches: usize = 0;
    let hit_cap = loop {
        let res = sqlx::query(
            "DELETE FROM events
             WHERE ctid IN (
                 SELECT ctid FROM events
                 WHERE claimed_handle = $1 AND received_at < $2
                 ORDER BY received_at ASC
                 LIMIT $3
             )",
        )
        .bind(claimed_handle)
        .bind(cutoff)
        .bind(DELETE_BATCH_SIZE)
        .execute(pool)
        .await?;
        let deleted = res.rows_affected();
        total += deleted;
        batches += 1;
        if deleted == 0 {
            break false;
        }
        if batches >= PER_USER_BATCH_CAP {
            break true;
        }
        tokio::time::sleep(BATCH_THROTTLE).await;
    };

    // Keep the stat_event_counts rollup consistent after a delete. The ingest
    // path only ever INCREMENTS the rollup, so a retention purge would leave it
    // overcounting (and the hard-cut summary read would report stale totals).
    // Recompute this handle's rollup from the surviving rows — bounded per-handle
    // work, atomic (DELETE + rebuild in one statement). No-op if nothing deleted.
    if total > 0 {
        // DELETE + rebuild must be two statements in one transaction: in a
        // single wCTE both would see the same pre-statement snapshot, so the
        // rebuild INSERT would collide with the not-yet-visible deleted rows.
        // Sequential-in-tx makes the DELETE visible to the INSERT and keeps the
        // recompute atomic. Rebuilds from surviving rows; event types purged to
        // zero simply produce no row.
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM stat_event_counts WHERE claimed_handle = $1")
            .bind(claimed_handle)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO stat_event_counts
                 (claimed_handle, event_type, event_count, first_seen_at, last_seen_at)
             SELECT claimed_handle, event_type, COUNT(*),
                    MIN(event_timestamp), MAX(event_timestamp)
             FROM events
             WHERE claimed_handle = $1
             GROUP BY claimed_handle, event_type",
        )
        .bind(claimed_handle)
        .execute(&mut *tx)
        .await?;
        // The session rollups (session_summary, character_records) and the
        // contract_runs rollup are also derived from events; a purge
        // invalidates them too. Flag the handle dirty so the next read
        // recomputes them from the surviving rows via
        // repo::PostgresStore::ensure_session_stats_fresh /
        // ensure_contract_runs_fresh. Lazy (not an eager rebuild here) keeps
        // the purge bounded; bare $1 matches the already-lowercased
        // claimed_handle used by the statements above.
        sqlx::query(
            "INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, contracts_dirty, updated_at)
             VALUES ($1, TRUE, TRUE, now())
             ON CONFLICT (claimed_handle) DO UPDATE
                 SET sessions_dirty = TRUE, contracts_dirty = TRUE, updated_at = now()",
        )
        .bind(claimed_handle)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    Ok((total, hit_cap))
}

/// One full sweep across every user. Reads policies, joins users to
/// supporter_status, derives tier per user, and purges where the
/// policy has a finite window.
///
/// Audit emission is best-effort per docs/ENGINEERING.md: an audit hiccup
/// never poisons the sweep.
pub async fn run_sweep(
    pool: &PgPool,
    policy_store: &dyn RetentionPolicyStore,
    audit: &dyn AuditLog,
) -> Result<SweepSummary, RetentionError> {
    // Acquire the cross-replica advisory lock. Non-blocking try: if
    // another replica is already sweeping, we skip this tick rather
    // than queue up behind it. Returns Ok(SweepSummary::default()) so
    // the caller can log "skipped".
    //
    // SESSION-level advisory locks are per-CONNECTION, so the lock AND the
    // unlock MUST run on the SAME connection. Running them on `pool`
    // (different pooled connections) leaks the lock: the unlock lands on a
    // connection that doesn't own it ("you don't own a lock of type
    // ExclusiveLock" NOTICE) and fails, so the lock persists on the original
    // connection and every later tick that lands elsewhere sees it as held
    // and silently skips — retention quietly stops running until a restart.
    // Pin one connection for the lock's whole lifetime; the sweep body still
    // uses the pool.
    let mut lock_conn = pool.acquire().await?;
    let acquired: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(SWEEP_ADVISORY_LOCK_KEY)
        .fetch_one(&mut *lock_conn)
        .await?;
    if !acquired.0 {
        // A skip is legitimate (another replica holds the lock) but is ALSO
        // the exact signature of the leaked-lock bug fixed above: every tick
        // after the first skipping forever. Count it so the two are
        // distinguishable from outside the process — a steady stream of
        // `outcome="skipped_lock"` with no `outcome="completed"` means the
        // lock is stuck, not that a peer is busy.
        counter!("starstats_retention_sweeps_total", "outcome" => "skipped_lock").increment(1);
        tracing::info!("retention sweep: advisory lock held by another replica; skipping tick");
        return Ok(SweepSummary::default());
    }

    // Wrap the body so we always release the lock, even on error.
    let result = run_sweep_locked(pool, policy_store, audit).await;

    // Release the lock on the SAME connection it was taken on. Failure here
    // is logged but does not mask the body's outcome. Dropping `lock_conn`
    // afterwards returns it to the pool with the lock already released.
    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(SWEEP_ADVISORY_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await
    {
        tracing::error!(error = %e, "retention sweep: pg_advisory_unlock failed");
    }

    // Outcome accounting. The sweep runs on a 24h cadence and only writes an
    // audit row when it actually deletes something (`deleted > 0`), so a
    // healthy no-op sweep and a sweep that never ran are indistinguishable
    // from the outside — which is precisely how the leaked-lock bug went
    // unnoticed. These three series close that gap:
    //
    //   starstats_retention_sweeps_total{outcome="completed"|"skipped_lock"|"failed"}
    //   starstats_retention_events_deleted_total
    //   starstats_retention_last_success_timestamp_seconds
    //
    // The gauge is the alertable one: `time() - <gauge> > 26h` fires whenever
    // the sweep stops completing for any reason (stuck lock, panic in the
    // loop, DB outage) without needing to know why. 26h gives the 24h cadence
    // headroom for a slow sweep or a restart.
    match &result {
        Ok(summary) => {
            counter!("starstats_retention_sweeps_total", "outcome" => "completed").increment(1);
            counter!("starstats_retention_events_deleted_total").increment(summary.events_deleted);
            gauge!("starstats_retention_last_success_timestamp_seconds")
                .set(Utc::now().timestamp() as f64);
        }
        Err(_) => {
            // The error itself is logged by the caller (spawn_sweep_loop),
            // which owns the backoff decision; here we only count it.
            counter!("starstats_retention_sweeps_total", "outcome" => "failed").increment(1);
        }
    }

    result
}

async fn run_sweep_locked(
    pool: &PgPool,
    policy_store: &dyn RetentionPolicyStore,
    audit: &dyn AuditLog,
) -> Result<SweepSummary, RetentionError> {
    let policies = policy_store.list_all().await?;
    let policy_for = |t: Tier| -> Option<i32> {
        policies
            .iter()
            .find(|p| p.tier == t)
            .and_then(|p| p.retention_days)
    };

    // Pull every user + their effective supporter state in one shot.
    // LEFT JOIN so users without a supporter_status row still appear
    // (treated as 'none' -> Free tier).
    let users: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT u.claimed_handle, ss.state
         FROM users u
         LEFT JOIN supporter_status ss ON ss.user_id = u.id",
    )
    .fetch_all(pool)
    .await?;

    let mut summary = SweepSummary::default();
    let now = Utc::now();

    for (handle, state) in users {
        summary.users_considered += 1;
        let tier = tier_from_supporter_state(state.as_deref().unwrap_or("none"));
        let Some(days) = policy_for(tier) else {
            summary.users_unlimited += 1;
            continue;
        };
        let cutoff = now - Duration::days(days as i64);
        let (deleted, hit_cap) = match purge_user_events(pool, &handle, cutoff).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    handle = %handle,
                    tier = tier.as_str(),
                    "retention sweep: per-user purge failed"
                );
                continue;
            }
        };
        if deleted > 0 {
            summary.users_purged += 1;
            summary.events_deleted += deleted;
            if hit_cap {
                summary.users_truncated += 1;
            }
            // Best-effort audit per docs/ENGINEERING.md: warn but continue.
            let entry = AuditEntry {
                actor_sub: None,
                actor_handle: Some("system".to_string()),
                action: "data.retention_purged".to_string(),
                payload: serde_json::json!({
                    "claimed_handle": handle,
                    "tier": tier.as_str(),
                    "retention_days": days,
                    "cutoff": cutoff.to_rfc3339(),
                    "events_deleted": deleted,
                    "hit_batch_cap": hit_cap,
                }),
            };
            if let Err(e) = audit.append(entry).await {
                tracing::warn!(
                    error = %e,
                    handle = %handle,
                    "retention sweep: audit append failed (continuing)"
                );
            }
        }
    }

    tracing::info!(
        users_considered = summary.users_considered,
        users_purged = summary.users_purged,
        events_deleted = summary.events_deleted,
        users_truncated = summary.users_truncated,
        "retention sweep complete"
    );
    Ok(summary)
}

/// Cadence for the scheduled loop. 24h matches the reference-data
/// refresh cadence (see main.rs). On failure we back off to 1h so a
/// transient DB hiccup doesn't block the next attempt for a full day.
pub const SWEEP_INTERVAL_OK: StdDuration = StdDuration::from_secs(24 * 3600);
pub const SWEEP_INTERVAL_FAIL: StdDuration = StdDuration::from_secs(3600);

/// Spawn the long-running retention loop. Returns the JoinHandle for
/// completeness; main.rs ignores it (the loop runs for the lifetime
/// of the process).
pub fn spawn_sweep_loop(
    pool: PgPool,
    policy_store: Arc<dyn RetentionPolicyStore>,
    audit: Arc<dyn AuditLog>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let next = match run_sweep(&pool, policy_store.as_ref(), audit.as_ref()).await {
                Ok(_) => SWEEP_INTERVAL_OK,
                Err(e) => {
                    tracing::error!(error = %e, "retention sweep failed; retrying after backoff");
                    SWEEP_INTERVAL_FAIL
                }
            };
            tokio::time::sleep(next).await;
        }
    })
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// In-memory policy store for route + unit tests. Backs `list_all`
    /// off a Vec the test seeds explicitly.
    #[derive(Default)]
    pub struct MemoryRetentionPolicyStore {
        policies: Mutex<Vec<RetentionPolicy>>,
    }

    impl MemoryRetentionPolicyStore {
        pub fn with_defaults() -> Self {
            let s = Self::default();
            s.seed(Tier::Free, Some(90));
            s.seed(Tier::Supporter, None);
            s
        }

        pub fn seed(&self, tier: Tier, retention_days: Option<i32>) {
            let mut v = self.policies.lock().expect("policy memstore poisoned");
            v.retain(|p| p.tier != tier);
            v.push(RetentionPolicy {
                tier,
                retention_days,
            });
        }
    }

    #[async_trait]
    impl RetentionPolicyStore for MemoryRetentionPolicyStore {
        async fn list_all(&self) -> Result<Vec<RetentionPolicy>, RetentionError> {
            Ok(self
                .policies
                .lock()
                .expect("policy memstore poisoned")
                .clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryRetentionPolicyStore;
    use super::*;

    #[test]
    fn tier_round_trips_through_string() {
        for t in [Tier::Free, Tier::Supporter] {
            assert_eq!(Tier::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn tier_parse_rejects_unknown_strings() {
        assert!(Tier::parse("legend").is_none());
        assert!(Tier::parse("").is_none());
    }

    #[test]
    fn tier_from_supporter_state_only_active_is_supporter() {
        assert_eq!(tier_from_supporter_state("active"), Tier::Supporter);
        // lapsed is explicitly Free per migration 0017 docstring.
        assert_eq!(tier_from_supporter_state("lapsed"), Tier::Free);
        assert_eq!(tier_from_supporter_state("none"), Tier::Free);
        // Unrecognised input falls back to Free (defensive default).
        assert_eq!(tier_from_supporter_state(""), Tier::Free);
        assert_eq!(tier_from_supporter_state("ACTIVE"), Tier::Free); // case-sensitive on purpose
    }

    #[tokio::test]
    async fn memory_store_lists_seeded_policies() {
        let store = MemoryRetentionPolicyStore::with_defaults();
        let policies = store.list_all().await.unwrap();
        let free = policies
            .iter()
            .find(|p| p.tier == Tier::Free)
            .expect("free seeded");
        let sup = policies
            .iter()
            .find(|p| p.tier == Tier::Supporter)
            .expect("supporter seeded");
        assert_eq!(free.retention_days, Some(90));
        assert_eq!(sup.retention_days, None);
    }

    #[tokio::test]
    async fn memory_store_seed_overrides_prior_value_for_same_tier() {
        let store = MemoryRetentionPolicyStore::with_defaults();
        store.seed(Tier::Free, Some(30));
        let policies = store.list_all().await.unwrap();
        let free = policies.iter().find(|p| p.tier == Tier::Free).unwrap();
        assert_eq!(free.retention_days, Some(30));
        // Supporter unchanged.
        let sup = policies.iter().find(|p| p.tier == Tier::Supporter).unwrap();
        assert_eq!(sup.retention_days, None);
    }

    #[tokio::test]
    async fn empty_memory_store_lists_nothing() {
        let store = MemoryRetentionPolicyStore::default();
        assert!(store.list_all().await.unwrap().is_empty());
    }

    #[test]
    fn sweep_summary_default_is_zeroed() {
        let s = SweepSummary::default();
        assert_eq!(s.users_considered, 0);
        assert_eq!(s.users_unlimited, 0);
        assert_eq!(s.users_purged, 0);
        assert_eq!(s.events_deleted, 0);
        assert_eq!(s.users_truncated, 0);
    }
}
