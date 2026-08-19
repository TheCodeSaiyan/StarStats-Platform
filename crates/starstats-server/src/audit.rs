//! Hash-chained audit log writer.
//!
//! Every API call that changes server state appends one row. The
//! database trigger (`audit_log_check_chain`) verifies the chain on
//! insert; the application computes the hash before sending the
//! INSERT, and uses a transaction with `SELECT ... FOR UPDATE` on the
//! tail row to serialise concurrent writers.
//!
//! Hash construction:
//!   prev_hash || canonical(payload) || seq.to_string()
//! Canonical JSON has a fixed key order so two equal logical payloads
//! always produce the same digest.
//!
//! ## MinIO mirror
//! After the Postgres INSERT commits, the same row is replicated to
//! the configured S3-compatible bucket via [`MinioMirror`]. Mirror
//! failures are logged at `warn` and do **not** roll back or retry —
//! Postgres remains the system of record (see `docs/AUDIT.md`
//! "Mirroring").

use crate::audit_mirror::{AuditEntryRow, MinioMirror};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

// -- Audit action constants -------------------------------------------------
//
// Closed-vocabulary strings stored in the `action` column. Route handlers
// reference these constants rather than inlining bare string literals so
// typos are caught at compile time and the full vocabulary is enumerable
// in one place.
//
// Existing actions (share surface):
//   share.created | share.revoked | share.viewed | share.reported |
//   share.report_resolved | share.visibility_changed
//
// Profile surface:
//   profile_layout.updated | share_scopes.updated
//
/// Wire string for the `profile_layout.updated` audit action.
/// Emitted by the PUT /v1/users/me/profile-layout handler when the
/// owner saves a new widget arrangement.
pub const ACTION_PROFILE_LAYOUT_UPDATED: &str = "profile_layout.updated";

/// Wire string for the `share_scopes.updated` audit action.
/// Emitted by the PUT /v1/users/me/share-scopes handler when the
/// owner changes their per-widget visibility toggles (Plan 3b Option A).
pub const ACTION_SHARE_SCOPES_UPDATED: &str = "share_scopes.updated";

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("payload serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub actor_sub: Option<String>,
    pub actor_handle: Option<String>,
    pub action: String,
    pub payload: Value,
}

/// Read-side record. Includes the DB-assigned `seq` + `occurred_at` so
/// the admin audit page can paginate and timeline-sort. Hash columns
/// (`prev_hash`/`row_hash`) are NOT surfaced — those are integrity
/// metadata, not user-visible.
#[derive(Debug, Clone)]
pub struct AuditEntryRecord {
    pub seq: i64,
    pub occurred_at: DateTime<Utc>,
    pub actor_sub: Option<String>,
    pub actor_handle: Option<String>,
    pub action: String,
    pub payload: Value,
}

/// Filters for `AuditQuery::list`. All fields are optional; an empty
/// filter returns the most recent rows up to `limit`. Pagination is
/// offset-based — cursor pagination deferred until volume warrants
/// the extra plumbing.
#[derive(Debug, Clone, Default)]
pub struct AuditFilters {
    /// Match against `actor_handle` (case-insensitive substring).
    /// Picked over `actor_sub` because admins reason about handles,
    /// not UUIDs.
    pub actor_handle: Option<String>,
    /// Match against `action` (exact; the field is a small enum-like
    /// dictionary on the write side).
    pub action: Option<String>,
    /// Inclusive lower bound on `occurred_at`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `occurred_at`.
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

#[async_trait]
pub trait AuditLog: Send + Sync + 'static {
    async fn append(&self, entry: AuditEntry) -> Result<(), AuditError>;
}

/// Aggregated `share.viewed` stats for one (owner, recipient) pair.
/// Surfaced by [`AuditQuery::share_views_for_owner`] so the outbound
/// shares list can annotate each pill with a "viewed N times · last Xh
/// ago" line without the caller having to know the audit-log schema.
#[derive(Debug, Clone)]
pub struct ShareViewStat {
    pub recipient_handle: String,
    pub view_count: i64,
    pub last_viewed_at: Option<DateTime<Utc>>,
}

/// Read-side trait — separate from [`AuditLog`] so the existing
/// `Arc<dyn AuditLog>` plumbing stays focused on writes. Admin
/// surfaces inject `Arc<dyn AuditQuery>` independently.
#[async_trait]
pub trait AuditQuery: Send + Sync + 'static {
    /// Return up to `filters.limit` rows matching the filters,
    /// ordered by `seq DESC` (most recent first), skipping
    /// `filters.offset` rows. The returned `Vec` length plus
    /// whatever the caller knows about `offset` is enough to drive
    /// "has more" — explicit count queries are deferred.
    async fn list(&self, filters: AuditFilters) -> Result<Vec<AuditEntryRecord>, AuditError>;

    /// Aggregate `share.viewed` rows for one owner, grouping by
    /// `payload->>'recipient_handle'`. Returns one stat row per
    /// recipient that has ever viewed. Dedicated method (instead of
    /// extending `AuditFilters` with a JSONB predicate) because the
    /// only consumer today is the `/v1/me/shares` enrichment path —
    /// keeping the SQL local lets it run a single `GROUP BY` instead
    /// of fetch-all + bin-in-memory.
    async fn share_views_for_owner(
        &self,
        owner_handle: &str,
    ) -> Result<Vec<ShareViewStat>, AuditError>;
}

/// Bounded capacity of the audit-writer channel. Sized so a burst of
/// state-changing requests queues without blocking the request path; on
/// overflow `append` applies back-pressure (an awaited `send`) rather than
/// dropping an audit record or growing memory unboundedly.
const AUDIT_WRITER_CHANNEL_CAPACITY: usize = 2048;

/// Shutdown handle for the background audit writer.
///
/// Held by `main` so the queue can be flushed before the process exits.
/// Dropping it is deliberately INERT — the writer keeps running — so any
/// call site that ignores the handle still gets a working audit log.
pub struct AuditWriterHandle {
    shutdown: Arc<Notify>,
    task: JoinHandle<()>,
}

impl AuditWriterHandle {
    /// Stop accepting new entries, flush everything already queued, then
    /// wait for the writer to exit — bounded by `timeout` so a wedged DB
    /// can't hold the process past the container's SIGKILL grace period.
    ///
    /// Call this AFTER the HTTP server has stopped serving: once no handler
    /// can enqueue, the queue has a stable end and is actually drainable.
    /// Without it, an in-memory backlog dies with the process even though
    /// `append` already returned `Ok` to its caller.
    pub async fn shutdown_and_drain(self, timeout: Duration) {
        self.shutdown.notify_one();
        match tokio::time::timeout(timeout, self.task).await {
            Ok(Ok(())) => tracing::info!("audit writer drained cleanly"),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "audit writer task failed while draining")
            }
            Err(_) => tracing::warn!(
                timeout_secs = timeout.as_secs(),
                "audit writer did not finish draining before the timeout; \
                 queued audit entries were lost"
            ),
        }
    }
}

pub struct PostgresAuditLog {
    /// Retained for the read side ([`AuditQuery`]) — the write side goes
    /// through the background writer, which owns its own pool + mirror.
    pool: PgPool,
    /// Hand-off to the single background writer task. `append` enqueues
    /// `(entry, occurred_at)` and returns immediately (fire-and-forget).
    /// `occurred_at` is captured at ENQUEUE time so the persisted timestamp
    /// reflects when the action happened, not when the writer drained it.
    sender: mpsc::Sender<(AuditEntry, DateTime<Utc>)>,
}

impl PostgresAuditLog {
    /// Construct the audit log and spawn its single background writer.
    ///
    /// The writer drains the channel and performs each chained INSERT
    /// serially, keeping the `pg_advisory_xact_lock` (load-bearing for
    /// cross-replica chain integrity) OFF the request latency path. Pass
    /// `mirror = Some(..)` to also best-effort-mirror each row to S3.
    ///
    /// Must be called within a Tokio runtime (it `tokio::spawn`s the
    /// writer). Every current call site (main startup + tests) has one.
    ///
    /// Returns the log plus its [`AuditWriterHandle`] — pass the handle to
    /// `shutdown_and_drain` at exit so a queued backlog isn't lost.
    pub fn new(pool: PgPool, mirror: Option<Arc<MinioMirror>>) -> (Self, AuditWriterHandle) {
        // A stuck or lossy writer is otherwise invisible: `append` returns
        // Ok the moment the entry is queued, so absence of audit rows is
        // NOT evidence of absence of audited actions.
        metrics::describe_gauge!(
            "starstats_audit_writer_queue_depth",
            "Audit entries queued for the background writer"
        );
        metrics::describe_counter!(
            "starstats_audit_writer_appends_total",
            "Audit entries successfully persisted by the background writer"
        );
        metrics::describe_counter!(
            "starstats_audit_writer_failures_total",
            "Audit entries the background writer failed to persist"
        );
        metrics::describe_counter!(
            "starstats_audit_writer_dropped_total",
            "Audit entries dropped because the writer channel was closed"
        );
        metrics::describe_counter!(
            "starstats_audit_writer_backpressure_total",
            "Appends that had to wait on a full audit-writer queue"
        );

        let (sender, receiver) = mpsc::channel(AUDIT_WRITER_CHANNEL_CAPACITY);
        let shutdown = Arc::new(Notify::new());
        // The writer owns the pool clone + the mirror; the struct keeps
        // only `pool` (for the read side) and the channel sender.
        let task = tokio::spawn(Self::writer_loop(
            pool.clone(),
            mirror,
            receiver,
            shutdown.clone(),
        ));
        (Self { pool, sender }, AuditWriterHandle { shutdown, task })
    }

    /// The single background writer: drains `receiver` and writes each
    /// entry via [`Self::write_one`], logging (never propagating) errors.
    ///
    /// Exits when all senders drop, or when signalled by
    /// [`AuditWriterHandle::shutdown_and_drain`] — which closes the
    /// channel (refusing new entries) and flushes the remaining backlog
    /// before returning, so shutdown doesn't discard queued rows.
    async fn writer_loop(
        pool: PgPool,
        mirror: Option<Arc<MinioMirror>>,
        mut receiver: mpsc::Receiver<(AuditEntry, DateTime<Utc>)>,
        shutdown: Arc<Notify>,
    ) {
        loop {
            tokio::select! {
                // `Receiver::recv` is cancel-safe, so losing this branch to
                // the shutdown branch cannot drop an entry.
                queued = receiver.recv() => match queued {
                    Some((entry, occurred_at)) => {
                        Self::write_metered(&pool, &mirror, entry, occurred_at).await;
                        metrics::gauge!("starstats_audit_writer_queue_depth")
                            .set(receiver.len() as f64);
                    }
                    // Every sender dropped: nothing more can arrive.
                    None => break,
                },
                _ = shutdown.notified() => {
                    // Refuse new entries, then flush what is already buffered:
                    // `recv` yields None once a CLOSED channel runs empty.
                    receiver.close();
                    while let Some((entry, occurred_at)) = receiver.recv().await {
                        Self::write_metered(&pool, &mirror, entry, occurred_at).await;
                    }
                    metrics::gauge!("starstats_audit_writer_queue_depth").set(0.0);
                    break;
                }
            }
        }
    }

    /// [`Self::write_one`] plus the success/failure counters. Errors are
    /// logged and counted, never propagated — there is no caller left to
    /// return them to once the entry is off the request path.
    async fn write_metered(
        pool: &PgPool,
        mirror: &Option<Arc<MinioMirror>>,
        entry: AuditEntry,
        occurred_at: DateTime<Utc>,
    ) {
        match Self::write_one(pool, mirror, entry, occurred_at).await {
            Ok(()) => metrics::counter!("starstats_audit_writer_appends_total").increment(1),
            Err(e) => {
                metrics::counter!("starstats_audit_writer_failures_total").increment(1);
                tracing::warn!(error = %e, "audit_log append failed in background writer");
            }
        }
    }

    /// Perform one chained append: advisory-lock the global chain, read
    /// the tail, compute the hash, INSERT, commit, then best-effort mirror.
    /// This is the former inline `append` body, now run by the single
    /// writer so the advisory lock never sits on a request's latency path.
    /// The lock is still required: `seq` is a Rust-computed `tail + 1` and
    /// the chain is global, so multiple replicas' writers must serialize
    /// across processes (see [`AUDIT_LOG_ADVISORY_LOCK`]).
    async fn write_one(
        pool: &PgPool,
        mirror: &Option<Arc<MinioMirror>>,
        entry: AuditEntry,
        occurred_at: DateTime<Utc>,
    ) -> Result<(), AuditError> {
        let canonical = canonicalize(&entry.payload)?;
        let mut tx = pool.begin().await?;

        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(AUDIT_LOG_ADVISORY_LOCK)
            .execute(&mut *tx)
            .await?;

        let row: Option<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT seq, row_hash FROM audit_log
             ORDER BY seq DESC LIMIT 1
             FOR UPDATE",
        )
        .fetch_optional(&mut *tx)
        .await?;

        let (next_seq, prev_hash) = match row {
            Some((seq, hash)) => (seq + 1, hash),
            None => (1i64, vec![0u8; 32]),
        };

        let mut hasher = Sha256::new();
        hasher.update(&prev_hash);
        hasher.update(&canonical);
        hasher.update(next_seq.to_string().as_bytes());
        let row_hash: [u8; 32] = hasher.finalize().into();

        // `occurred_at` is the caller-side enqueue time (event time), passed
        // in so a queue backlog can't skew the persisted timestamp; the mirror
        // row is partitioned by the same value the DB row records.
        sqlx::query(
            "INSERT INTO audit_log
                (occurred_at, actor_sub, actor_handle, action, payload, prev_hash, row_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(occurred_at)
        .bind(&entry.actor_sub)
        .bind(&entry.actor_handle)
        .bind(&entry.action)
        .bind(&entry.payload)
        .bind(&prev_hash)
        .bind(row_hash.as_slice())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        // Best-effort mirror write (AFTER commit — Postgres is authoritative).
        if let Some(mirror) = mirror {
            let row = AuditEntryRow {
                seq: next_seq,
                occurred_at,
                actor_sub: entry.actor_sub.clone(),
                actor_handle: entry.actor_handle.clone(),
                action: entry.action.clone(),
                payload: entry.payload.clone(),
                prev_hash_hex: hex::encode(&prev_hash),
                row_hash_hex: hex::encode(row_hash),
            };
            if let Err(e) = mirror.append(&row).await {
                tracing::warn!(
                    error = %e,
                    seq = next_seq,
                    action = %entry.action,
                    "MinIO audit mirror write failed; Postgres row is authoritative"
                );
            }
        }

        Ok(())
    }
}

/// Postgres advisory-lock key for serializing audit_log appends.
///
/// The transaction-scoped `pg_advisory_xact_lock` releases automatically
/// on COMMIT or ROLLBACK, so we don't need to manage release manually.
/// Key chosen as a memorable hex constant ("Alog" || 0x00000001) — any
/// stable i64 works as long as no other code in this DB uses the same.
///
/// Why this is needed even though the SELECT below uses FOR UPDATE:
/// Postgres `FOR UPDATE` row-locks the rows the SELECT returned, but
/// does NOT block concurrent INSERTs from creating NEW rows at higher
/// seq. Two concurrent transactions can both lock the same tail row
/// (seq=N), compute prev_hash = row_hash(N), and then both INSERT
/// (seq=N+1, prev_hash=row_hash(N)). Whichever commits second has
/// prev_hash pointing at seq=N's row_hash, but the now-prior row is
/// the OTHER transaction's seq=N+1 — chain breaks with
/// "prev_hash does not match prior row_hash". The advisory lock
/// serializes the whole append, eliminating the race.
///
/// Observed in production 2026-05-24 02:53:18 from concurrent ingest
/// batches (three 200-event batches landing in a 200ms window from
/// the same user — the chain is global, not per-user).
const AUDIT_LOG_ADVISORY_LOCK: i64 = 0x416C_6F67_0000_0001u64 as i64;

#[async_trait]
impl AuditLog for PostgresAuditLog {
    /// Hand the entry to the single background writer and return, so the
    /// advisory-locked chained INSERT never sits on the request latency
    /// path. The fast path (`try_send`) is the common case; if the bounded
    /// queue is momentarily full we fall back to an awaited `send`
    /// (back-pressure) rather than DROP an audit record — losing audit rows
    /// is worse than briefly slowing a request under overload. A closed
    /// channel only happens at shutdown (writer stopped); the entry is then
    /// unavoidably dropped and logged. Every caller already
    /// logs-and-continues and none read the result (project invariant).
    async fn append(&self, entry: AuditEntry) -> Result<(), AuditError> {
        // Stamp the event time NOW (enqueue), so a queue backlog can't skew
        // the persisted occurred_at away from when the action happened.
        let queued = (entry, Utc::now());
        match self.sender.try_send(queued) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(queued)) => {
                metrics::counter!("starstats_audit_writer_backpressure_total").increment(1);
                tracing::warn!("audit writer queue full; applying back-pressure to the caller");
                if self.sender.send(queued).await.is_err() {
                    metrics::counter!("starstats_audit_writer_dropped_total").increment(1);
                    tracing::warn!("audit writer channel closed; audit entry dropped");
                }
            }
            Err(mpsc::error::TrySendError::Closed((entry, _))) => {
                metrics::counter!("starstats_audit_writer_dropped_total").increment(1);
                tracing::warn!(
                    action = %entry.action,
                    "audit writer channel closed; audit entry dropped"
                );
            }
        }
        metrics::gauge!("starstats_audit_writer_queue_depth")
            .set((self.sender.max_capacity() - self.sender.capacity()) as f64);
        Ok(())
    }
}

#[async_trait]
impl AuditQuery for PostgresAuditLog {
    async fn list(&self, filters: AuditFilters) -> Result<Vec<AuditEntryRecord>, AuditError> {
        // The handler clamps these into a safe range before calling;
        // defence-in-depth here keeps a misuse from issuing an
        // unbounded scan.
        let limit = filters.limit.clamp(1, 500);
        let offset = filters.offset.max(0);

        // Filters are composed conditionally so a wide-open query
        // doesn't pay for noop predicates. `actor_handle` uses ILIKE
        // for substring search — handles are ASCII so the lower(...)
        // ICU concern doesn't apply, but ILIKE makes the intent
        // obvious to the next reader.
        let mut sql = String::from(
            "SELECT seq, occurred_at, actor_sub, actor_handle, action, payload
             FROM audit_log
             WHERE 1=1",
        );
        if filters.actor_handle.is_some() {
            sql.push_str(" AND actor_handle ILIKE $1");
        }
        if filters.action.is_some() {
            sql.push_str(if filters.actor_handle.is_some() {
                " AND action = $2"
            } else {
                " AND action = $1"
            });
        }
        // since/until use bind indices that depend on whether the
        // earlier filters are present, so build the placeholders
        // dynamically.
        let mut next_idx =
            1 + filters.actor_handle.is_some() as usize + filters.action.is_some() as usize;
        if filters.since.is_some() {
            sql.push_str(&format!(" AND occurred_at >= ${next_idx}"));
            next_idx += 1;
        }
        if filters.until.is_some() {
            sql.push_str(&format!(" AND occurred_at <= ${next_idx}"));
            next_idx += 1;
        }
        sql.push_str(&format!(
            " ORDER BY seq DESC LIMIT ${} OFFSET ${}",
            next_idx,
            next_idx + 1,
        ));

        let mut q = sqlx::query_as::<
            _,
            (
                i64,
                DateTime<Utc>,
                Option<String>,
                Option<String>,
                String,
                Value,
            ),
        >(&sql);
        if let Some(handle) = filters.actor_handle.as_ref() {
            q = q.bind(format!("%{handle}%"));
        }
        if let Some(action) = filters.action.as_ref() {
            q = q.bind(action);
        }
        if let Some(since) = filters.since {
            q = q.bind(since);
        }
        if let Some(until) = filters.until {
            q = q.bind(until);
        }
        q = q.bind(limit).bind(offset);

        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(
                |(seq, occurred_at, actor_sub, actor_handle, action, payload)| AuditEntryRecord {
                    seq,
                    occurred_at,
                    actor_sub,
                    actor_handle,
                    action,
                    payload,
                },
            )
            .collect())
    }

    async fn share_views_for_owner(
        &self,
        owner_handle: &str,
    ) -> Result<Vec<ShareViewStat>, AuditError> {
        // `payload->>'owner_handle'` matches the canonical key the
        // `share.viewed` audit writer uses. We `lower(...)` both sides
        // to match the rest of the sharing surface, which is
        // case-insensitive on handles.
        let rows = sqlx::query_as::<_, (String, i64, Option<DateTime<Utc>>)>(
            r#"
            SELECT
                payload->>'recipient_handle'        AS recipient_handle,
                COUNT(*)                            AS view_count,
                MAX(occurred_at)                    AS last_viewed_at
            FROM audit_log
            WHERE action = 'share.viewed'
              AND lower(payload->>'owner_handle') = lower($1)
              AND payload->>'recipient_handle' IS NOT NULL
            GROUP BY payload->>'recipient_handle'
            "#,
        )
        .bind(owner_handle)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(recipient_handle, view_count, last_viewed_at)| ShareViewStat {
                    recipient_handle,
                    view_count,
                    last_viewed_at,
                },
            )
            .collect())
    }
}

/// Build canonical bytes for a JSON value: object keys in
/// lexicographic order, no whitespace. `serde_json::to_vec` already
/// produces no whitespace; we reach into the value to sort keys.
fn canonicalize(v: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let sorted = sort_keys(v);
    serde_json::to_vec(&sorted)
}

fn sort_keys(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> =
                map.iter().map(|(k, v)| (k.clone(), sort_keys(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut sorted = serde_json::Map::with_capacity(entries.len());
            for (k, v) in entries {
                sorted.insert(k, v);
            }
            Value::Object(sorted)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_keys).collect()),
        other => other.clone(),
    }
}

// -- Test-only in-memory log -----------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    pub struct MemoryAuditLog {
        entries: Mutex<Vec<AuditEntry>>,
        /// When true, `append` errors. Lets a caller assert that an
        /// unrecordable action is REFUSED rather than performed --
        /// several handlers deliberately abort when the audit write
        /// fails, and that posture needs a test double to exercise.
        fail_append: bool,
        /// Single timestamp stamped on every row this log returns
        /// from `list`. Captured at construction so REPEATED calls
        /// to `list` (e.g. the fan-out grants+revokes queries inside
        /// `check_regrant_cycle`) see a consistent `occurred_at` —
        /// downstream callers sort by `(occurred_at, seq)`, and with
        /// equal timestamps the seq tiebreaker preserves insertion
        /// order. The previous "stamp `Utc::now()` per call"
        /// behaviour produced DIFFERENT timestamps for grants vs
        /// revokes, which broke the merged-temporal-walk in the
        /// regrant-cycle helper.
        created_at: DateTime<Utc>,
    }

    impl Default for MemoryAuditLog {
        fn default() -> Self {
            Self {
                entries: Mutex::new(Vec::new()),
                fail_append: false,
                created_at: Utc::now(),
            }
        }
    }

    impl MemoryAuditLog {
        /// A log whose every `append` fails.
        pub fn failing() -> Self {
            Self {
                fail_append: true,
                ..Self::default()
            }
        }

        pub fn snapshot(&self) -> Vec<AuditEntry> {
            self.entries.lock().expect("audit memlog poisoned").clone()
        }
    }

    #[async_trait]
    impl AuditLog for MemoryAuditLog {
        async fn append(&self, entry: AuditEntry) -> Result<(), AuditError> {
            if self.fail_append {
                return Err(AuditError::Database(sqlx::Error::Protocol(
                    "simulated audit append failure".into(),
                )));
            }
            self.entries
                .lock()
                .expect("audit memlog poisoned")
                .push(entry);
            Ok(())
        }
    }

    /// Test-only `AuditQuery` impl. Honours `action` (exact),
    /// `actor_handle` (case-insensitive substring), and the
    /// `since`/`until` window — enough for higher-level handler tests
    /// that need to count or page filtered slices. Other filter combos
    /// fall through unfiltered, matching the relaxed contract noted
    /// in the trait doc.
    #[async_trait]
    impl AuditQuery for MemoryAuditLog {
        async fn list(&self, filters: AuditFilters) -> Result<Vec<AuditEntryRecord>, AuditError> {
            let snap = self.entries.lock().expect("audit memlog poisoned");
            let limit = filters.limit.clamp(1, 500) as usize;
            let offset = filters.offset.max(0) as usize;
            // Stable per-instance timestamp — see field doc on
            // MemoryAuditLog. The window-filter check below still
            // compares against this `now`, just as before; the only
            // behaviour change is that two list() calls on the same
            // log report the SAME timestamp.
            let now = self.created_at;
            let actor_needle = filters
                .actor_handle
                .as_deref()
                .map(|s| s.to_ascii_lowercase());
            let records: Vec<AuditEntryRecord> = snap
                .iter()
                .enumerate()
                .filter(|(_, e)| match filters.action.as_deref() {
                    Some(a) => e.action == a,
                    None => true,
                })
                .filter(|(_, e)| match actor_needle.as_deref() {
                    Some(needle) => e
                        .actor_handle
                        .as_deref()
                        .map(|h| h.to_ascii_lowercase().contains(needle))
                        .unwrap_or(false),
                    None => true,
                })
                .filter(|_| {
                    // The MemoryAuditLog stores no timestamps; treat
                    // every entry as occurring "now". A since/until
                    // window that includes `now` keeps the row; one
                    // that doesn't drops it.
                    let after_since = filters.since.map(|s| now >= s).unwrap_or(true);
                    let before_until = filters.until.map(|u| now <= u).unwrap_or(true);
                    after_since && before_until
                })
                .rev()
                .skip(offset)
                .take(limit)
                .map(|(idx, e)| AuditEntryRecord {
                    seq: (idx as i64) + 1,
                    occurred_at: now,
                    actor_sub: e.actor_sub.clone(),
                    actor_handle: e.actor_handle.clone(),
                    action: e.action.clone(),
                    payload: e.payload.clone(),
                })
                .collect();
            Ok(records)
        }

        async fn share_views_for_owner(
            &self,
            owner_handle: &str,
        ) -> Result<Vec<ShareViewStat>, AuditError> {
            let snap = self.entries.lock().expect("audit memlog poisoned");
            let owner_lower = owner_handle.to_ascii_lowercase();
            let now = Utc::now();
            let mut by_recipient: std::collections::HashMap<String, (i64, DateTime<Utc>)> =
                std::collections::HashMap::new();
            for e in snap.iter() {
                if e.action != "share.viewed" {
                    continue;
                }
                let payload_owner = e
                    .payload
                    .get("owner_handle")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if payload_owner != owner_lower {
                    continue;
                }
                let Some(recipient) = e.payload.get("recipient_handle").and_then(Value::as_str)
                else {
                    continue;
                };
                let entry = by_recipient
                    .entry(recipient.to_string())
                    .or_insert((0, now));
                entry.0 += 1;
                entry.1 = now;
            }
            Ok(by_recipient
                .into_iter()
                .map(|(recipient_handle, (view_count, last))| ShareViewStat {
                    recipient_handle,
                    view_count,
                    last_viewed_at: Some(last),
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonicalize_orders_keys_deterministically() {
        let a = json!({ "z": 1, "a": 2, "m": { "y": 3, "x": 4 } });
        let b = json!({ "m": { "x": 4, "y": 3 }, "a": 2, "z": 1 });
        assert_eq!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
    }

    #[test]
    fn canonicalize_preserves_array_order() {
        let a = json!([3, 1, 2]);
        let b = json!([1, 2, 3]);
        assert_ne!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
    }

    /// Hash-chain integrity guard for the async writer (env-gated: runs
    /// only against a real Postgres via STARSTATS_TEST_DATABASE_URL,
    /// skipped in offline `cargo test`). Fires N appends concurrently
    /// through the fire-and-forget path, waits for the single background
    /// writer to drain them, then verifies the persisted chain has no
    /// break and that every one of this run's own appends landed.
    ///
    /// Parallel-safe, NOT table-owning: `audit_log` is append-only at
    /// the DB level (0002_audit_log.sql's triggers reject any
    /// UPDATE/DELETE — confirmed empirically, `DELETE` raises
    /// "audit_log is append-only"), and the hash chain is deliberately
    /// ONE global sequence across every writer (that's the entire point
    /// of a tamper-evident ledger), so this test can neither
    /// TRUNCATE-and-own the table nor scope a DELETE the way an
    /// ordinary fixture table can. The old version assumed both
    /// (TRUNCATE first, then "seq starts at 1" and "COUNT(*) == n") —
    /// which broke the moment `cargo test -p starstats-server` ran both
    /// compiled test binaries (`starstats-server`,
    /// `starstats-server-openapi`) concurrently against the same
    /// STARSTATS_TEST_DATABASE_URL: each binary's TRUNCATE could wipe
    /// the other's in-flight rows, and a bare `COUNT(*)` saw both
    /// binaries' appends combined.
    ///
    /// Instead: tag every row this run appends with a marker unique to
    /// this OS process + instant (`actor_sub` already exists on the
    /// table for exactly this purpose), anchor to whatever the tail
    /// already is before appending (instead of assuming an empty
    /// table), then verify two independent things: (1) the chain has
    /// no break anywhere in the observed window — true regardless of
    /// which writer (this run, the sibling binary's copy of this same
    /// test, or an unrelated route emitting its own audit row)
    /// contributed a given row — and (2) all N of this run's own
    /// appends are present, found by marker rather than by position.
    #[tokio::test]
    async fn hash_chain_holds_under_concurrent_appends() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping audit hash-chain test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        // Unique per OS-process-and-instant — the other compiled test
        // binary running this same test concurrently gets a different
        // PID, so the two runs' rows can never be confused.
        let run_marker = format!(
            "hashchain-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );

        // Anchor: whatever the tail already is, before this run
        // appends anything. The first row in the window read back
        // below must chain to THIS, not to an assumed all-zero seed.
        let anchor: Option<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT seq, row_hash FROM audit_log ORDER BY seq DESC LIMIT 1")
                .fetch_optional(&pool)
                .await
                .expect("read anchor tail");
        let (anchor_seq, anchor_hash) = anchor.unwrap_or((0, vec![0u8; 32]));

        let (audit, _writer) = PostgresAuditLog::new(pool.clone(), None);
        let audit = Arc::new(audit);
        let n: i64 = 50;

        // Enqueue concurrently — all funnel to the one writer, which
        // serializes them; the chain must come out intact regardless.
        let mut tasks = Vec::new();
        for i in 0..n {
            let audit = audit.clone();
            let marker = run_marker.clone();
            tasks.push(tokio::spawn(async move {
                audit
                    .append(AuditEntry {
                        actor_sub: Some(format!("{marker}-{i}")),
                        actor_handle: Some(format!("handle-{i}")),
                        action: "test.chain".to_string(),
                        payload: json!({ "i": i, "nested": { "z": 1, "a": 2 } }),
                    })
                    .await
                    .expect("append enqueues");
            }));
        }
        for t in tasks {
            t.await.expect("append task");
        }

        // Wait for the background writer to drain all N of THIS run's
        // own appends — scoped by `run_marker`, so a concurrent
        // sibling process's rows can neither stall nor satisfy this.
        let like_pattern = format!("{run_marker}-%");
        let mut own_count = 0i64;
        for _ in 0..200 {
            own_count =
                sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE actor_sub LIKE $1")
                    .bind(&like_pattern)
                    .fetch_one(&pool)
                    .await
                    .expect("count this run's own rows");
            if own_count >= n {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(
            own_count, n,
            "background writer drained all {n} of this run's own appends"
        );

        // Read the WHOLE window since the anchor — including any
        // concurrent sibling process's rows that landed in between.
        // The chain must hold across every row in that window
        // regardless of who wrote it; that is the actual invariant
        // under test.
        let rows: Vec<(i64, Vec<u8>, Vec<u8>, Value, Option<String>)> = sqlx::query_as(
            "SELECT seq, prev_hash, row_hash, payload, actor_sub FROM audit_log
             WHERE seq > $1 ORDER BY seq ASC",
        )
        .bind(anchor_seq)
        .fetch_all(&pool)
        .await
        .expect("read chain window");

        let mut expected_prev = anchor_hash;
        let mut own_is: Vec<i64> = Vec::new();
        for (seq, prev_hash, row_hash, payload, actor_sub) in &rows {
            assert_eq!(
                prev_hash, &expected_prev,
                "prev_hash links to prior row_hash at seq={seq}"
            );
            let canonical = canonicalize(payload).expect("canonicalize");
            let mut hasher = Sha256::new();
            hasher.update(prev_hash);
            hasher.update(&canonical);
            hasher.update(seq.to_string().as_bytes());
            let recomputed: [u8; 32] = hasher.finalize().into();
            assert_eq!(
                row_hash.as_slice(),
                recomputed.as_slice(),
                "row_hash == SHA256(prev_hash || canonical(payload) || seq) at seq={seq}"
            );
            expected_prev = row_hash.clone();

            if actor_sub
                .as_deref()
                .is_some_and(|s| s.starts_with(&run_marker))
            {
                own_is.push(payload["i"].as_i64().expect("payload.i is an int"));
            }
        }

        own_is.sort_unstable();
        assert_eq!(
            own_is,
            (0..n).collect::<Vec<_>>(),
            "every one of this run's own {n} appended payloads is present in the chain, each exactly once"
        );
    }

    /// Dropping the [`AuditWriterHandle`] must NOT stop the writer.
    ///
    /// The handle exists only so shutdown can EXPLICITLY drain the queue;
    /// a call site that ignores it must still get a working audit log.
    /// Guards the obvious-but-wrong design where the writer keys shutdown
    /// off the handle's `Drop` — that would silently stop persisting audit
    /// rows for every caller that didn't hold the handle. Runs offline: the
    /// pool never connects, so every write fails; what is asserted is the
    /// writer LOOP's liveness (it keeps draining), not the DB write.
    #[tokio::test]
    async fn dropping_the_writer_handle_leaves_the_writer_running() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            // Fail fast instead of the 30s default — every write here is
            // expected to fail, and the test only cares that the loop advances.
            .acquire_timeout(std::time::Duration::from_millis(200))
            .connect_lazy("postgres://does-not-resolve/none")
            .expect("lazy pool builds");

        let (audit, handle) = PostgresAuditLog::new(pool, None);
        drop(handle);

        for i in 0..4 {
            audit
                .append(AuditEntry {
                    actor_sub: None,
                    actor_handle: None,
                    action: "test.after_handle_drop".to_string(),
                    payload: json!({ "i": i }),
                })
                .await
                .expect("append enqueues");
        }

        // A live writer consumes all four; a shut-down one leaves them queued.
        for _ in 0..200 {
            if audit.sender.capacity() == AUDIT_WRITER_CHANNEL_CAPACITY {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(
            audit.sender.capacity(),
            AUDIT_WRITER_CHANNEL_CAPACITY,
            "writer kept draining the queue after its handle was dropped"
        );
    }

    /// Durability guard for the shutdown path (env-gated, as above).
    ///
    /// Moving appends off the request path means a queued entry is only in
    /// memory: without an explicit drain, a deploy/SIGTERM loses every row
    /// still in the channel while `append` has already returned `Ok`.
    /// Asserts `shutdown_and_drain` flushes the whole backlog to Postgres.
    ///
    /// Parallel-safe, NOT table-owning: same reasoning as
    /// `hash_chain_holds_under_concurrent_appends` above — `audit_log`
    /// is append-only (DB triggers reject UPDATE/DELETE) and shared
    /// across both compiled test binaries, so a bare `TRUNCATE` +
    /// unscoped `COUNT(*)` broke under `cargo test -p starstats-server`
    /// running `starstats-server` and `starstats-server-openapi`
    /// concurrently. Tag every row with a marker unique to this OS
    /// process + instant and scope the count to it.
    #[tokio::test]
    async fn shutdown_and_drain_flushes_queued_entries() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping audit drain test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        let run_marker = format!(
            "drain-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );

        let (audit, handle) = PostgresAuditLog::new(pool.clone(), None);
        let n: i64 = 25;
        for i in 0..n {
            audit
                .append(AuditEntry {
                    actor_sub: Some(format!("{run_marker}-{i}")),
                    actor_handle: None,
                    action: "test.drain".to_string(),
                    payload: json!({ "i": i }),
                })
                .await
                .expect("append enqueues");
        }

        // No sleep/poll: the drain itself is the synchronisation point.
        handle
            .shutdown_and_drain(std::time::Duration::from_secs(30))
            .await;

        let persisted: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE actor_sub LIKE $1")
                .bind(format!("{run_marker}-%"))
                .fetch_one(&pool)
                .await
                .expect("count this run's own rows");
        assert_eq!(
            persisted, n,
            "shutdown drained every one of this run's own queued audit entries"
        );
    }
}
