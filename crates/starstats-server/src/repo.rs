//! Storage abstraction for ingested events.
//!
//! The handler depends on the [`EventStore`] trait, not on Postgres
//! directly, so we can TDD against an in-memory implementation.
//! Production wiring uses [`PostgresStore`].

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use starstats_core::character_life::{derive_lives, LifeConfig, LifeSummary};
use starstats_core::contract_life::{
    derive_contract_runs, ClosedBy, ContractConfig, ContractRun, ContractState, ContractStep,
};
use starstats_core::location_classifier::ResolvedLocation;
use starstats_core::metadata::EventMetadata;
use starstats_core::wire::{EventEnvelope, LogSource};
#[cfg(test)]
use std::sync::Mutex;
use uuid::Uuid;

/// What the server actually stores. Constructed from an
/// [`EventEnvelope`] plus the authenticated identity (claimed handle
/// only for now — `user_id` lands when auth does).
///
/// `metadata` carries the cross-cutting envelope stamped by the
/// client (or synthesised server-side for legacy v1 clients) and is
/// persisted to the `events.metadata JSONB` column added by migration
/// 0030. Per the migration's contract, NULL rows pre-date the column
/// and read sites group-by metadata simply skip them — there is no
/// fan-out or SQL-side default.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: Uuid,
    pub idempotency_key: String,
    pub claimed_handle: String,
    pub event_type: String,
    pub event_timestamp: Option<DateTime<Utc>>,
    pub log_source: LogSource,
    pub source_offset: i64,
    pub raw_line: String,
    pub payload: Value,
    /// Cross-cutting metadata stamped by the client (or synthesised
    /// server-side for v1 clients in the ingest handler). Persisted
    /// to `events.metadata JSONB` by `PostgresStore::insert`. None
    /// only for rows the ingest handler couldn't stamp (envelope had
    /// no parsed event), in which case the column is written as NULL
    /// and downstream readers skip the row.
    pub metadata: Option<EventMetadata>,
    /// Fuzzy-resolved location, stamped by the tray's sync batcher and
    /// carried verbatim on the wire envelope. Persisted to
    /// `events.resolved_location JSONB` (migration 0041). `None` for
    /// placeless events, pre-resolution clients, and rows written before
    /// 0041. NOTE (F4): the event-read endpoints no longer echo this
    /// stored value — because it is client-controlled and a spoofed slug
    /// would render as a KB link, they re-derive the location server-side
    /// at query time (`query::derive_resolved_location`). The column is
    /// still written for fidelity / diagnostics but is authoritative for
    /// nothing. See [`starstats_core::location_classifier::ResolvedLocation`].
    pub resolved_location: Option<ResolvedLocation>,
}

/// Outcome of inserting one event. Lets the handler report how many
/// were accepted vs deduped vs rejected without separate round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

/// An event rejected at ingest, retained for diagnosis instead of being
/// silently dropped (F5). Same trust domain as `events` (the submitter's
/// own data), so the raw line + payload are kept verbatim so a maintainer
/// can see exactly what a misbehaving collector sent.
#[derive(Debug, Clone)]
pub struct QuarantinedEvent {
    pub id: Uuid,
    pub idempotency_key: String,
    pub claimed_handle: String,
    /// Coarse machine-readable bucket, e.g. "validation".
    pub reason: String,
    /// Human-readable specifics, e.g. the validation-error text.
    pub detail: Option<String>,
    pub log_source: LogSource,
    pub source_offset: i64,
    pub raw_line: String,
    pub payload: Value,
}

#[async_trait]
pub trait EventStore: Send + Sync + 'static {
    async fn insert(&self, event: StoredEvent) -> Result<InsertOutcome, RepoError>;

    /// Bulk-insert a batch of events, returning a per-event outcome
    /// (`Inserted`/`Duplicate`) in input order. The default implementation is
    /// sequential (Memory store + tests); `PostgresStore` overrides it with a
    /// single set-based `UNNEST` statement plus in-transaction maintenance of
    /// the `stat_event_counts` rollup, collapsing a batch of N events from
    /// N round-trips + N commits down to one of each.
    ///
    /// Idempotency is unchanged (`ON CONFLICT (claimed_handle, idempotency_key)
    /// DO NOTHING`). Note: a *within-batch* duplicate key (rare — keys are per
    /// source_offset) is reported as `Inserted` for both copies rather than
    /// `Inserted`+`Duplicate`; cross-batch duplicates (the common case) are
    /// classified exactly. See MORNING-REVIEW.md.
    async fn insert_batch(
        &self,
        events: Vec<StoredEvent>,
    ) -> Result<Vec<InsertOutcome>, RepoError> {
        let mut out = Vec::with_capacity(events.len());
        for e in events {
            out.push(self.insert(e).await?);
        }
        Ok(out)
    }

    /// Persist an event that failed validation, for out-of-band
    /// diagnosis. Idempotent on `(claimed_handle, idempotency_key)` so a
    /// retried bad batch doesn't bloat the table. Diagnostic-only: the
    /// ingest handler treats a failure here as best-effort (logs it, and
    /// never fails the batch over a quarantine write).
    async fn quarantine(&self, event: QuarantinedEvent) -> Result<(), RepoError>;

    /// Advance a device's ingest-batch high-water mark to `seq`
    /// (monotonic — a lower `seq` never rewinds it) and return the
    /// PRIOR high-water mark, or `None` if this is the first batch seen
    /// from `device_id`. The ingest handler diffs prior→seq to surface
    /// gaps (lost uploads) and out-of-order arrivals for the F7
    /// `batch_sequence` detector. Best-effort observability: the handler
    /// treats a failure here as non-fatal (logs it, never fails the batch).
    async fn observe_batch_sequence(
        &self,
        device_id: &str,
        seq: i64,
    ) -> Result<Option<i64>, RepoError>;
}

/// Read-side projection of an event row. Subset of the storage table
/// — `raw_line` and `idempotency_key` aren't surfaced by query
/// endpoints (clients have their own copies; the raw line is only
/// useful for re-classification by the server).
#[derive(Debug, Clone, Default)]
pub struct StoredQueryEvent {
    pub seq: i64,
    /// Used by query filters; not surfaced to the API DTO since
    /// the caller already knows their own handle.
    #[allow(dead_code)]
    pub claimed_handle: String,
    pub event_type: String,
    pub event_timestamp: Option<DateTime<Utc>>,
    pub log_source: String,
    pub source_offset: i64,
    pub payload: Value,
    /// Raw tray-stamped location from the JSONB column (migration 0041).
    /// **Deliberately not read on the query path**: the events feed
    /// re-derives `resolved_location` server-side from each event's own
    /// payload (see `query::derive_resolved_location`), because the stored
    /// value is client-controlled and a spoofed slug would otherwise
    /// render as a `/kb/location/{slug}` link (F4). Retained here for DB
    /// fidelity / potential diagnostics.
    #[allow(dead_code)]
    pub resolved_location: Option<Value>,
    /// `Some(ts)` means the owner has marked this row as hidden from
    /// shared/public views (timeline + summary endpoints filter it
    /// out). `None` = visible. Only the owner's own `/v1/me/events`
    /// response surfaces this field — friend/public DTOs are
    /// pre-filtered and never include hidden rows in the first place.
    pub hidden_at: Option<DateTime<Utc>>,
}

/// Direction of cursor pagination on `event_seq`.
///
/// `Before` returns rows older than the cursor in DESC order (the
/// default "newest first" stream walking backwards). `After` returns
/// rows newer than the cursor in ASC order (catch-up tailing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqCursor {
    Before(i64),
    After(i64),
}

/// Filter / pagination spec for [`EventQuery::list_filtered`]. All
/// fields except `limit` are optional and compose with AND semantics.
///
/// Cursor semantics:
///  * `None` -> newest-first (DESC by seq).
///  * `Some(SeqCursor::Before(n))` -> rows with seq < n, DESC by seq.
///  * `Some(SeqCursor::After(n))`  -> rows with seq > n, ASC by seq.
#[derive(Debug, Clone, Default)]
pub struct EventFilters {
    pub cursor: Option<SeqCursor>,
    pub event_type: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,
}

/// Idle gap (in minutes) between two adjacent events that splits a
/// session boundary. Tuned by hand: a typical Star Citizen session has
/// events every few seconds; a 30-minute lull is "they alt-tabbed for a
/// snack and came back" being generous, while still splitting actual
/// distinct play sessions cleanly.
pub const SESSION_IDLE_GAP_MINUTES: i64 = 30;

/// Event types that are NOT active gameplay and must not anchor or
/// bridge a play session. `launcher_activity` comes from the RSI
/// launcher log (a separate log the tray tails) which writes while the
/// launcher runs in the background — even off-hours (auth refresh,
/// update checks) — so counting it stitches distinct play sessions into
/// one giant span. `game_crash` is a post-hoc crash-dir scan, not play.
/// Excluded from every session derivation (the idle-gap queries here
/// AND the process_init walk in `event_timeline`). The SQL uses the
/// literal `NOT IN ('launcher_activity', 'game_crash')` — keep it in
/// sync with this list.
// The canonical non-gameplay list that the SQL `NOT IN ('launcher_activity',
// 'game_crash')` literals mirror. Consumed by the idle-gap mock filters and the
// `derive_sessions` oracle (both test-only now that the timeline sessionizer is
// SQL), so it is dead-code in a non-test build by design — kept as the single
// source of truth the SQL literals must stay in sync with.
#[cfg_attr(not(test), allow(dead_code))]
pub const NON_SESSION_EVENT_TYPES: &[&str] = &["launcher_activity", "game_crash"];

/// One inferred play session — a contiguous run of events where no
/// adjacent pair is more than [`SESSION_IDLE_GAP_MINUTES`] apart.
/// Computed on demand from the events table; not persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredSession {
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub event_count: i64,
}

/// All-time "records" for a handle, computed server-side over the FULL
/// event history rather than the fetch-capped subsets the web widget
/// used to sum client-side (audit F9). Sessions use the same
/// gap-idle clustering as [`InferredSession`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordsAggregate {
    /// Duration of the longest single session, in whole seconds.
    pub longest_session_secs: i64,
    /// Event count of the busiest single session.
    pub busiest_session_events: i64,
    /// Longest gap between two consecutive `player_death` events, in
    /// whole seconds ("longest stretch alive"). 0 with fewer than two
    /// deaths.
    pub longest_survival_streak_secs: i64,
    /// Most `player_death` events in a single session.
    pub deadliest_session_deaths: i64,
}

/// Combined output of [`EventQuery::lives_for_handle`]: the pure
/// character-life FSM's [`LifeSummary`] plus the app's CANONICAL
/// 30-min idle-gap session count from the UNBOUNDED
/// `EventQuery::count_sessions_since(.., None)`.
///
/// The two session counts are computed by genuinely different rules
/// (see [`LifeSummary::sessions`]'s doc comment) and will disagree on
/// real data, so the HTTP response must not surface the FSM's own
/// `sessions`/`deaths_per_session` — the handler recomputes
/// `deaths_per_session` from `summary.deaths` over `sessions` here.
#[derive(Debug, Clone)]
pub struct LivesData {
    pub summary: LifeSummary,
    /// Canonical 30-min idle-gap session count, from
    /// `count_sessions_since(claimed_handle, None)` — the unbounded
    /// full-history count — over the SAME ordered event stream
    /// `summary` was derived from. Deliberately NOT
    /// `event_timeline::derive_sessions(..).len()`, which truncates to
    /// `SESSIONS_LIST_LIMIT` (50) and would silently cap this count.
    pub sessions: u32,
}

/// One row of the `event-types` aggregate. Mirrors
/// [`crate::query::TypeCount`] plus a `last_seen` timestamp; emitted
/// by [`EventQuery::event_type_breakdown`] which is the back-end of the
/// Metrics page's "Event types" tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTypeStats {
    pub event_type: String,
    pub count: i64,
    /// `None` for types whose only rows had no parsed timestamp. The
    /// table column is `event_timestamp`, which is nullable.
    pub last_seen: Option<DateTime<Utc>>,
}

/// One row of the `ingest-history` view: a single batch the caller's
/// desktop client posted. Read straight off `audit_log` filtered to
/// `action = 'ingest.batch_processed'` for the authenticated handle.
/// We deliberately don't retain the raw lines from the batch, so this
/// is metadata only: who shipped what, when, and what the server's
/// per-event accept/dup/reject verdict was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestBatchRow {
    pub seq: i64,
    pub occurred_at: DateTime<Utc>,
    pub batch_id: String,
    pub game_build: Option<String>,
    /// Pairing-flow device that posted the batch. `None` on legacy
    /// rows written before migration 0026 (the field was absent from
    /// the audit payload) and on rows pushed under a user-scoped
    /// bearer token (no device claim). Populated for every batch
    /// posted by a paired tray client going forward.
    pub device_id: Option<Uuid>,
    pub total: i64,
    pub accepted: i64,
    pub duplicate: i64,
    pub rejected: i64,
}

#[async_trait]
pub trait EventQuery: Send + Sync + 'static {
    /// Legacy forward-cursor listing. Kept on the trait so external
    /// callers (and the legacy `?after=` query param path) can still
    /// reach it; the new handler always goes through
    /// [`Self::list_filtered`].
    #[allow(dead_code)]
    async fn list_for_handle(
        &self,
        claimed_handle: &str,
        after: i64,
        limit: i64,
    ) -> Result<Vec<StoredQueryEvent>, RepoError> {
        self.list_filtered(
            claimed_handle,
            EventFilters {
                cursor: if after > 0 {
                    Some(SeqCursor::After(after))
                } else {
                    None
                },
                event_type: None,
                since: None,
                until: None,
                limit,
            },
        )
        .await
    }

    /// Filtered + cursor-paginated listing. Composes optional
    /// `event_type`, `since`, `until`, and a single seq cursor.
    async fn list_filtered(
        &self,
        claimed_handle: &str,
        filters: EventFilters,
    ) -> Result<Vec<StoredQueryEvent>, RepoError>;

    /// Per-day event counts for the trailing `days` window. Returns
    /// only days that had events; the handler is responsible for
    /// zero-padding the bucket series.
    async fn timeline(
        &self,
        claimed_handle: &str,
        days: u32,
    ) -> Result<Vec<(NaiveDate, i64)>, RepoError>;

    /// Same as [`Self::timeline`] but excludes rows the owner has
    /// hidden (`hidden_at IS NOT NULL`). Used by the friend/public
    /// timeline endpoints. Default delegates to `timeline` so the
    /// in-memory test impl stays simple — the production Postgres
    /// impl overrides with the WHERE filter.
    async fn timeline_shared(
        &self,
        claimed_handle: &str,
        days: u32,
    ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
        self.timeline(claimed_handle, days).await
    }

    /// Scope-aware shared timeline. Same as [`Self::timeline_shared`]
    /// but additionally clamps the per-event row stream by event_type
    /// before bucketing. When both `allow_types` and `deny_types` are
    /// `None` this is exactly `timeline_shared`. When `allow_types` is
    /// `Some(&[..])` only those types contribute; when `deny_types` is
    /// `Some(&[..])` those types are excluded. The allowlist wins —
    /// types absent from a non-empty allowlist are dropped before the
    /// denylist is consulted, matching the precedence already used by
    /// the summary `apply_event_type_filter` helper. An empty
    /// allowlist therefore returns an empty timeline (no types match).
    ///
    /// Default impl falls through to `timeline_shared` so a fresh
    /// in-memory test stub doesn't have to think about the per-type
    /// clamp; the MemoryQuery + PostgresStore impls below override to
    /// actually honour the lists.
    async fn timeline_shared_filtered(
        &self,
        claimed_handle: &str,
        days: u32,
        _allow_types: Option<&[String]>,
        _deny_types: Option<&[String]>,
    ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
        self.timeline_shared(claimed_handle, days).await
    }

    /// Returns (total, [(event_type, count)]).
    async fn summary_for_handle(
        &self,
        claimed_handle: &str,
    ) -> Result<(u64, Vec<(String, u64)>), RepoError>;

    /// Same as [`Self::summary_for_handle`] but excludes hidden rows.
    /// Used by the friend/public summary endpoints. Default delegates
    /// to the owner variant — Postgres overrides with WHERE filter.
    async fn summary_for_handle_shared(
        &self,
        claimed_handle: &str,
    ) -> Result<(u64, Vec<(String, u64)>), RepoError> {
        self.summary_for_handle(claimed_handle).await
    }

    /// Set or clear the `hidden_at` flag on one event. `hide=true`
    /// marks the row hidden (`hidden_at = NOW()`); `hide=false`
    /// clears it (`hidden_at = NULL`). Returns `true` when a row
    /// matched and `false` for a no-op (either the event doesn't
    /// belong to the caller or it was already in the requested
    /// state). Idempotent. Default impl returns false — the in-memory
    /// test impl overrides to support tests.
    async fn set_event_hidden(
        &self,
        _claimed_handle: &str,
        _seq: i64,
        _hide: bool,
    ) -> Result<bool, RepoError> {
        Ok(false)
    }

    /// Per-event-type breakdown with `last_seen` for the Metrics page's
    /// "Event types" tab. Returns rows sorted by count DESC. If
    /// `since` is set, only events with `event_timestamp >= since` are
    /// counted; rows whose only matches all had NULL timestamps are
    /// dropped (we can't show "last seen" for them and they aren't
    /// actionable on the time-windowed view).
    async fn event_type_breakdown(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<EventTypeStats>, RepoError>;

    /// Inferred play sessions for the Metrics page's "Sessions" tab.
    /// Groups consecutive events by `event_timestamp` and starts a new
    /// session whenever the gap between two adjacent events exceeds
    /// [`SESSION_IDLE_GAP_MINUTES`]. Events with NULL `event_timestamp`
    /// are excluded — they can't anchor a session window. Returns rows
    /// newest-first; the handler exposes `limit`/`offset` pagination.
    async fn sessions_for_handle(
        &self,
        claimed_handle: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InferredSession>, RepoError>;

    /// Sum of inferred-session durations (seconds) since `since`
    /// (None = all-time). Sessions use the same 30-min-idle-gap
    /// clusters as `sessions_for_handle`.
    async fn total_playtime_secs(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<i64, RepoError>;

    /// Count of inferred sessions whose `start_at >= since` (None =
    /// all-time). Same 30-min-idle-gap clusters as `sessions_for_handle`,
    /// but counts without materializing every session.
    async fn count_sessions_since(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<i64, RepoError>;

    /// Records for a handle, computed over the event history. Backs
    /// `GET /v1/me/stats/records`, replacing the web widget's
    /// fetch-capped, client-side computation (audit F9).
    ///
    /// `since = None` computes over the FULL history (the lifetime
    /// figures). `since = Some(ts)` restricts every underlying scan to
    /// events with `event_timestamp >= ts`, powering the range-windowed
    /// variant surfaced alongside the lifetime fields.
    async fn records_for_handle(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<RecordsAggregate, RepoError>;

    /// Character-life FSM summary for a handle (character-life-fsm
    /// Phase 1). Walks the handle's FULL timestamp-ordered event stream
    /// through `starstats_core::character_life::derive_lives` to
    /// segment it into spawn -> death/crash/gap spans. Backs
    /// `GET /v1/me/stats/lives`.
    ///
    /// CRITICAL: the FSM's own `LifeSummary::sessions` /
    /// `deaths_per_session` are ALSO 30-min idle-gap based internally
    /// (`LifeConfig::session_gap_secs` defaults to 1800s, same as
    /// [`SESSION_IDLE_GAP_MINUTES`]) but walk a different event stream
    /// than the app's CANONICAL session count: `derive_lives` here is
    /// fed the FULL timestamped stream, including `game_crash` (needed
    /// to close a life as `Crash`), while `count_sessions_since`
    /// additionally excludes `launcher_activity`/`game_crash` via
    /// `NON_SESSION_EVENT_TYPES` — so the two WILL disagree on real
    /// data. Implementations must call
    /// `self.count_sessions_since(claimed_handle, None)` —
    /// the UNBOUNDED, full-history gap-idle count — and return
    /// that via [`LivesData::sessions`] instead of the FSM's own
    /// figure, so this endpoint's `sessions` always agrees with
    /// `/v1/users/{handle}/sessions`. Do NOT use
    /// `event_timeline::derive_sessions(..).len()` here — that helper
    /// truncates to `SESSIONS_LIST_LIMIT` (50) for the paginated
    /// sessions LIST and would silently cap this count for any handle
    /// with more than 50 sessions.
    ///
    /// `since = None` walks the FULL history (the lifetime figures).
    /// `since = Some(ts)` restricts the event stream (and the canonical
    /// session count) to events with `event_timestamp >= ts`, powering
    /// the range-windowed variant surfaced alongside the lifetime
    /// fields.
    async fn lives_for_handle(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<LivesData, RepoError>;

    /// Recent ingest batches the caller's clients posted, newest first.
    /// Backs the My logs page (Wave 11). Reads `audit_log` filtered to
    /// the canonical ingest action; the user's desktop client is the
    /// only writer of those rows for that handle, so cross-account
    /// leakage is prevented by the `actor_handle = $1` filter alone.
    async fn ingest_history_for_handle(
        &self,
        actor_handle: &str,
        device_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<IngestBatchRow>, RepoError>;

    /// Returns the most recent location-bearing event for the user
    /// alongside the most recent `join_pu` shard hint, if any.
    /// Backs `GET /v1/me/location/current`.
    ///
    /// Caller passes the list of acceptable event types (canonical
    /// list lives in [`crate::locations::LOCATION_EVENT_TYPES`]) and
    /// gets back two independent reads:
    ///
    /// - `location_event` — the most recent event whose type is in
    ///   the list. The handler funnels its payload through
    ///   [`crate::locations::resolve`] to produce the wire DTO.
    /// - `shard_hint` — the shard string from the most recent
    ///   `join_pu` event regardless of where in the result list it
    ///   sits. Lets the resolver attach shard info even when a more
    ///   recent `planet_terrain_load` is the headline event.
    async fn latest_location(
        &self,
        claimed_handle: &str,
        event_types: &[&str],
    ) -> Result<LatestLocation, RepoError>;

    /// Recent location-bearing events for a user, newest-first, capped
    /// at `limit`. Used by `GET /v1/me/location/current` to compute the
    /// **entered_at** timestamp by walking back from the head event
    /// through the contiguous run of same-location-key events.
    ///
    /// Unlike `location_event_stream` this method has no time window
    /// — the limit is a row count instead. That's the whole point:
    /// the dwell anchor cannot be window-truncated without producing
    /// the exact bug the field exists to fix (the chip rendering
    /// "23h 57m" because a 24h window saturated). When the limit is
    /// hit before the walk finds a key change, the handler marks the
    /// result as a lower bound rather than lying about it.
    async fn recent_location_events(
        &self,
        claimed_handle: &str,
        event_types: &[&str],
        limit: i64,
    ) -> Result<Vec<LatestLocationEvent>, RepoError>;

    /// Aggregate dwell time per `(planet, city)` pair over a window.
    /// Backs `GET /v1/me/location/breakdown`. Returns rows with
    /// `dwell_seconds` derived from the gap between adjacent
    /// location events (capped at the session-idle threshold to
    /// avoid an idle gap inflating one location's dwell). Sorted by
    /// dwell DESC.
    ///
    /// Implementation note: this returns the raw event stream in the
    /// query window — the handler does the gap-walk + dedup +
    /// labelling using [`crate::locations::resolve`]. Keeping it on
    /// the handler side means the dwell aggregation logic stays
    /// pure-Rust (testable without a database) and the repo stays a
    /// thin SQL layer.
    ///
    /// `limit` bounds the fetch to the **most-recent** N raw location
    /// events in the window. Cost is therefore O(limit) regardless of
    /// how wide the window is (or how active the player was) — the
    /// caps on the trace/breakdown endpoints lift to a full year
    /// precisely because this bound keeps the scan from growing with
    /// window × activity. The returned Vec is still **oldest-first**
    /// (ASC) so the dwell-collapse walker can iterate forward in time.
    async fn location_event_stream(
        &self,
        claimed_handle: &str,
        event_types: &[&str],
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<LatestLocationEvent>, RepoError>;

    /// Aggregate stats from the events table for the activity
    /// surface. Returns counts grouped by a JSON-payload field for
    /// the named event_type, sorted by count DESC.
    ///
    /// `payload_field` is a top-level JSON key (no dotted paths).
    /// `since`/`until` bound the window (see [`EventQuery`]'s window
    /// convention below). `payload_filter`, when set, restricts to rows
    /// whose given field equals the expected value (case-sensitive). Used
    /// by the combat stats handler to scope "top weapons" to kills
    /// (killer==caller) vs "deaths by zone" to deaths (victim==caller).
    ///
    /// # Window convention
    ///
    /// Shared by this method, [`Self::payload_numeric_sum`],
    /// [`Self::count_event_type`], [`Self::objective_outcomes`] and
    /// [`Self::has_events_in_window`]: `since` is an INCLUSIVE lower bound
    /// (`>=`), `until` an EXCLUSIVE upper bound (`<`). That asymmetry is
    /// load-bearing — it is what lets the stats handlers ask for a window
    /// and the period before it (`[now-2N, now-N)` then `[now-N, now]`)
    /// without an event on the shared edge being counted in both, or in
    /// neither. `None` on either side means unbounded on that side.
    ///
    /// A row with a NULL `event_timestamp` satisfies an unbounded side but
    /// never a bounded one, so it is excluded as soon as either bound is
    /// set — matching Postgres, where `NULL >= x` is never true.
    // Eight parameters: `until` pushed this one past clippy's ceiling.
    // Bundling the bounds into a window struct would be the alternative,
    // but it would reshape a method with ~20 existing call sites across
    // seven endpoints to satisfy a lint, so it is allowed here as it is
    // elsewhere in the workspace.
    #[allow(clippy::too_many_arguments)]
    async fn payload_field_breakdown(
        &self,
        claimed_handle: &str,
        event_type: &str,
        payload_field: &str,
        payload_filter: Option<PayloadFilter<'_>>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<PayloadFieldBucket>, RepoError>;

    /// Docking occurrences derived from ship-stow telemetry in the optional
    /// `[since, until)` window.
    ///
    /// `vehicle_stowed` fires once per ship when a lobby transition stows a
    /// hangar, so raw rows within the same short temporal episode are one
    /// occurrence, not one occurrence per ship. Newer trays collapse runs of
    /// three or more into a `burst_summary` with `rule_id =
    /// vehicle_stowed_burst`. A summary whose timestamp matches a raw episode
    /// anchor is the same occurrence; the raw row supplies its landing area.
    /// Folding happens before the window is applied so a run that crosses a
    /// boundary belongs only to the window containing its first row.
    async fn docking_occurrences(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<DockingOccurrences, RepoError>;

    /// Distinct-objective outcomes for a handle in the optional
    /// `since`/`until` window (same bound convention as
    /// [`Self::payload_field_breakdown`]). Rows sharing an `objective_id`
    /// fold into one outcome at
    /// the highest-precedence state seen: completed > failed >
    /// unresolved > in_progress. Rows with no `objective_id` (legacy,
    /// predating objective_id capture) count as one objective each.
    ///
    /// Rows whose `state` field is absent entirely (rank 0 — e.g. the
    /// `CMissionLogEntry::UpdateActiveObjective` text-update variant,
    /// which carries no `state`) are excluded from every bucket. This is
    /// intentional: such an objective has no known outcome. It means the
    /// returned counts sum to "objectives with a known state seen", not
    /// "every distinct objective_id observed" — do not assume otherwise.
    /// Folding happens WITHIN the window: an objective whose only
    /// in-window row says `in_progress` counts as `no_outcome` here even
    /// if it completed after `until`. That is the same truncation `since`
    /// already implied, applied to the other end.
    async fn objective_outcomes(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<ObjectiveOutcomes, RepoError>;

    /// Materialised contract runs for `claimed_handle`, optionally scoped
    /// to runs accepted at or after `since`.
    ///
    /// Unlike [`Self::lives_for_handle`]'s `since` (which truncates the
    /// INPUT event stream before the FSM runs), this filters the OUTPUT
    /// of a full-history fold: contract runs are materialised from the
    /// handle's *complete* event stream (`PostgresStore` via
    /// `ensure_contract_runs_fresh`/`contract_runs` table, migration
    /// 0060; `MemoryQuery` derives fresh on every call) and `since` scopes
    /// the already-derived runs by `accepted_at` afterward. Truncating the
    /// input first would risk `derive_contract_runs` mis-closing a run
    /// right at the window boundary (a spurious session-gap/shard-change
    /// abandonment that a longer view would have closed correctly via its
    /// own terminal banner).
    ///
    /// Ordered `accepted_at DESC` (newest first).
    async fn contract_runs(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ContractRunRow>, RepoError>;

    /// Total count of events of the given type for the user, in the
    /// optional `since`/`until` window (same bound convention as
    /// [`Self::payload_field_breakdown`]). `payload_filter` lets the
    /// caller scope to rows whose given JSON field equals the expected
    /// value — that's how `stats_combat` separates kills (killer==caller)
    /// from deaths (victim==caller) using the same `actor_death`
    /// table.
    async fn count_event_type(
        &self,
        claimed_handle: &str,
        event_type: &str,
        payload_filter: Option<PayloadFilter<'_>>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<u64, RepoError>;

    /// Sum a numeric top-level JSON payload field across events of the
    /// given type for the user (optional `since`/`until` window, same
    /// bound convention as [`Self::payload_field_breakdown`]). Rows whose
    /// field is absent or non-numeric contribute 0; returns 0 when there
    /// are no matching rows. Used by `stats_spend` to total
    /// `shop_buy_request.price` (whole aUEC).
    async fn payload_numeric_sum(
        &self,
        claimed_handle: &str,
        event_type: &str,
        numeric_field: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<i64, RepoError>;

    /// Does this handle have at least ONE event, of ANY type, in
    /// `[since, until)`? (Same bound convention as
    /// [`Self::payload_field_breakdown`], but both bounds are required —
    /// the question is only ever asked about a closed window.)
    ///
    /// Exists to separate "played and did nothing" from "was not a user
    /// yet". The previous-period twins on the stats endpoints are gated on
    /// this: a brand-new player's previous window reads as zero because
    /// they had not signed up, and rendering "+47, trending upwards"
    /// against that zero would be a fabricated comparison. A genuine zero
    /// — the handle was active but bought nothing / flew nowhere — still
    /// returns `true` here, because that IS a real comparison.
    ///
    /// Deliberately type-agnostic. Probing the endpoint's own event type
    /// would answer a different (and useless) question: "did they spend in
    /// the previous window", which is exactly the zero we are trying to
    /// interpret.
    async fn has_events_in_window(
        &self,
        claimed_handle: &str,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<bool, RepoError>;

    /// Combat counts for a handle in one pass: `(kills, actor_deaths,
    /// player_deaths)`. `subject` is the caller's handle — kills are
    /// `actor_death` rows where `payload.killer == subject`, actor_deaths are
    /// `actor_death` rows where `payload.victim == subject`, player_deaths are
    /// all `player_death` rows. The default runs the three `count_event_type`
    /// queries (Memory/tests); `PostgresStore` overrides it with a single
    /// `COUNT(*) FILTER` scan instead of three separate index scans, so a
    /// combat-widget render issues one query for the counts, not three.
    async fn combat_counts(
        &self,
        claimed_handle: &str,
        subject: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<(u64, u64, u64), RepoError> {
        let kills = self
            .count_event_type(
                claimed_handle,
                "actor_death",
                Some(PayloadFilter {
                    field: "killer",
                    equals: subject,
                }),
                since,
                None,
            )
            .await?;
        let deaths_actor = self
            .count_event_type(
                claimed_handle,
                "actor_death",
                Some(PayloadFilter {
                    field: "victim",
                    equals: subject,
                }),
                since,
                None,
            )
            .await?;
        let deaths_player = self
            .count_event_type(claimed_handle, "player_death", None, since, None)
            .await?;
        Ok((kills, deaths_actor, deaths_player))
    }
}

/// Filter clause for the activity-stats queries. Both methods that
/// take `Option<PayloadFilter<'_>>` apply it as a `payload->>field =
/// value` predicate on the `events` table. Borrowed form so the
/// caller doesn't have to allocate.
#[derive(Debug, Clone, Copy)]
pub struct PayloadFilter<'a> {
    pub field: &'a str,
    pub equals: &'a str,
}

/// One bucket from [`EventQuery::payload_field_breakdown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadFieldBucket {
    pub value: String,
    pub count: i64,
}

/// Occurrence-level docking aggregate. Structured raw observations retain a
/// representative landing area; collapsed summaries have no structured
/// landing-area field and therefore contribute to `unknown`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockingOccurrences {
    pub landing_areas: Vec<PayloadFieldBucket>,
    pub unknown: i64,
}

impl DockingOccurrences {
    pub fn total(&self) -> i64 {
        self.unknown
            + self
                .landing_areas
                .iter()
                .map(|bucket| bucket.count)
                .sum::<i64>()
    }
}

/// Distinct-objective outcome counts. Each objective is counted exactly
/// ONCE, at its terminal state — unlike `payload_field_breakdown`, which
/// counts state-transition rows and therefore double-counts any objective
/// that progressed (an in_progress -> completed objective lands in both
/// buckets).
///
/// `unresolved`: resolved but not completed. Covers TWO stored spellings.
/// `withdrawn` is what `parse_objective_state` maps `WITHDRAWN` to from
/// v1.8.149 on; `unknown` is the parser's catch-all, which is both what
/// older collectors stored WITHDRAWN as (those payloads are never
/// rewritten) and where a state CIG ships that the parser has no variant
/// for yet still lands. Kept named `unresolved` rather than `withdrawn`
/// precisely because it is that union, not one state.
///
/// `no_outcome`: the only state ever observed for this objective was the
/// raw `in_progress` payload state — no terminal state (completed/
/// failed/unresolved) was ever recorded for it. This is NOT "currently
/// active": it's objectives the parser never saw resolve at all
/// (abandoned missions, app exits, log rotations mid-mission). On real
/// data this bucket can dwarf a player's actual handful of concurrent
/// objectives, so don't read it as a live in-progress count — that's
/// exactly the misreading this field used to invite when it was named
/// `in_progress`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ObjectiveOutcomes {
    pub completed: i64,
    pub failed: i64,
    pub unresolved: i64,
    pub no_outcome: i64,
}

/// One materialised contract run — the DTO [`EventQuery::contract_runs`]
/// returns. `state`/`closed_by` are the fold's `ContractState`/`ClosedBy`
/// enums lowered to their lowercase snake_case TEXT form (see
/// `contract_state_str`/`closed_by_str`), matching exactly what's stored
/// in the `contract_runs.state`/`closed_by` columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRunRow {
    pub mission_id: String,
    pub name: String,
    pub state: String,
    pub closed_by: String,
    pub step_count: i32,
    pub steps_complete: i32,
    pub steps_remaining: i32,
    pub partial_history: bool,
    pub connected_server: Option<String>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub steps: Vec<ContractStep>,
}

/// Lower a `ContractState` to its DB/DTO TEXT form. Written out explicitly
/// (rather than round-tripping through `serde_json`) but kept spelled
/// identically to the enum's own `#[serde(rename_all = "snake_case")]` —
/// don't let the two drift.
fn contract_state_str(s: ContractState) -> &'static str {
    match s {
        ContractState::InProgress => "in_progress",
        ContractState::Completed => "completed",
        ContractState::Failed => "failed",
        ContractState::Withdrawn => "withdrawn",
        ContractState::Abandoned => "abandoned",
        ContractState::Unknown => "unknown",
        ContractState::Superseded => "superseded",
    }
}

/// Lower a `ClosedBy` to its DB/DTO TEXT form — same rationale as
/// [`contract_state_str`].
fn closed_by_str(c: ClosedBy) -> &'static str {
    match c {
        ClosedBy::HudComplete => "hud_complete",
        ClosedBy::HudFailed => "hud_failed",
        ClosedBy::HudWithdrawn => "hud_withdrawn",
        ClosedBy::SessionEnd => "session_end",
        ClosedBy::GameCrash => "game_crash",
        ClosedBy::SessionGap => "session_gap",
        ClosedBy::ShardChange => "shard_change",
        ClosedBy::Superseded => "superseded",
        ClosedBy::None => "none",
    }
}

/// Convert one pure-fold `ContractRun` into its DB/DTO shape: enum fields
/// lowered to TEXT and the fold's `Option<String>` RFC3339 timestamps
/// parsed to `Option<DateTime<Utc>>` via the same `parse_event_timestamp`
/// this module uses for every other event timestamp. A timestamp that
/// fails to parse becomes `None` rather than erroring the whole rebuild —
/// defensive only: the fold only ever stamps a run's timestamps from an
/// event timestamp that already passed ingest validation.
fn contract_run_to_row(run: ContractRun) -> ContractRunRow {
    ContractRunRow {
        mission_id: run.mission_id,
        name: run.name,
        state: contract_state_str(run.state).to_string(),
        closed_by: closed_by_str(run.closed_by).to_string(),
        step_count: run.step_count as i32,
        steps_complete: run.steps_complete as i32,
        steps_remaining: run.steps_remaining as i32,
        partial_history: run.partial_history,
        connected_server: run.connected_server,
        accepted_at: run.accepted_at.as_deref().and_then(parse_event_timestamp),
        closed_at: run.closed_at.as_deref().and_then(parse_event_timestamp),
        last_event_at: run.last_event_at.as_deref().and_then(parse_event_timestamp),
        steps: run.steps,
    }
}

/// Raw `contract_runs` row shape for `sqlx::query_as` — column order
/// matches `PostgresStore::contract_runs`'s SELECT list exactly.
/// `sqlx::types::Json` decodes the `steps JSONB` column straight into
/// `Vec<ContractStep>`, the same idiom `hangar_store.rs`/`profile_store.rs`
/// use for their own JSONB columns. Kept separate from `ContractRunRow`
/// (rather than deriving `FromRow` on the DTO itself) so the DTO's public
/// `steps: Vec<ContractStep>` field doesn't have to be the sqlx wrapper
/// type.
#[derive(sqlx::FromRow)]
struct ContractRunSqlRow {
    mission_id: String,
    name: String,
    state: String,
    closed_by: String,
    step_count: i32,
    steps_complete: i32,
    steps_remaining: i32,
    partial_history: bool,
    connected_server: Option<String>,
    accepted_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    last_event_at: Option<DateTime<Utc>>,
    steps: sqlx::types::Json<Vec<ContractStep>>,
}

impl From<ContractRunSqlRow> for ContractRunRow {
    fn from(r: ContractRunSqlRow) -> Self {
        ContractRunRow {
            mission_id: r.mission_id,
            name: r.name,
            state: r.state,
            closed_by: r.closed_by,
            step_count: r.step_count,
            steps_complete: r.steps_complete,
            steps_remaining: r.steps_remaining,
            partial_history: r.partial_history,
            connected_server: r.connected_server,
            accepted_at: r.accepted_at,
            closed_at: r.closed_at,
            last_event_at: r.last_event_at,
            steps: r.steps.0,
        }
    }
}

/// One run-observed contract name (`contract_runs.name`) with no
/// matching row in the published `contracts` catalog — the DTO
/// [`PostgresStore::contract_catalog_gaps`] returns. Exact,
/// case/whitespace-insensitive match only; fuzzy matching is a
/// separate, later piece of work with its own design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractGapRow {
    pub name: String,
    pub run_count: i64,
    pub distinct_handles: i64,
    /// `None` only in the defensive edge case where every run sharing
    /// this name has a NULL `accepted_at` (an unparseable fold
    /// timestamp) — see `contract_runs.accepted_at`'s nullability.
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

/// Raw row shape for [`PostgresStore::contract_catalog_gaps`]'s
/// `sqlx::query_as` — column names match the SELECT list exactly
/// (`FromRow` matches by name, not position, same rationale as
/// `ContractSummaryRow` in `contracts.rs`).
#[derive(Debug, sqlx::FromRow)]
struct ContractGapSqlRow {
    name: String,
    run_count: i64,
    distinct_handles: i64,
    first_seen: Option<DateTime<Utc>>,
    last_seen: Option<DateTime<Utc>>,
}

impl From<ContractGapSqlRow> for ContractGapRow {
    fn from(r: ContractGapSqlRow) -> Self {
        Self {
            name: r.name,
            run_count: r.run_count,
            distinct_handles: r.distinct_handles,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
        }
    }
}

/// What [`EventQuery::latest_location`] returns. Both fields are
/// independently optional so the handler can distinguish "no events
/// at all yet" (both `None` → 204) from "we know they're online but
/// don't know where" (only `shard_hint` populated → return the
/// `JoinPu`-shaped fallback).
#[derive(Debug, Clone, Default)]
pub struct LatestLocation {
    pub location_event: Option<LatestLocationEvent>,
    pub shard_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LatestLocationEvent {
    pub event_type: String,
    pub event_timestamp: DateTime<Utc>,
    pub payload: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Build a [`StoredEvent`] from the wire envelope plus the
/// authenticated handle. Pulls `event_type` and `event_timestamp` out
/// of the parsed payload when present, falling back to `"unknown"`
/// for unclassified lines so we can still insert them.
///
/// If `serde_json::to_value` ever fails (custom Serialize impl, NaN
/// float, etc.) we still insert the row with `payload: Null` so the
/// idempotency key is recorded — but we log loudly with the
/// idempotency key + raw line preview so the operator notices.
/// Without the warning the row sits in the DB as
/// `event_type=unknown, payload=null` indistinguishable from a real
/// unparseable line, and the bug stays buried.
pub fn from_envelope(env: &EventEnvelope, claimed_handle: &str) -> StoredEvent {
    let payload = match serde_json::to_value(&env.event) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                idempotency_key = %env.idempotency_key,
                claimed_handle = %claimed_handle,
                source_offset = env.source_offset,
                "envelope event payload failed to serialize; storing as null"
            );
            Value::Null
        }
    };
    let (event_type, event_timestamp) = extract_type_and_ts(&payload);
    StoredEvent {
        id: Uuid::now_v7(),
        idempotency_key: env.idempotency_key.clone(),
        claimed_handle: claimed_handle.to_owned(),
        event_type,
        event_timestamp,
        log_source: env.source,
        source_offset: env.source_offset as i64,
        raw_line: env.raw_line.clone(),
        payload,
        metadata: env.metadata.clone(),
        resolved_location: env.resolved_location.clone(),
    }
}

fn extract_type_and_ts(payload: &Value) -> (String, Option<DateTime<Utc>>) {
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let timestamp = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_event_timestamp);
    (event_type, timestamp)
}

/// Parse an event timestamp into UTC, accepting every dialect
/// `core::validators::check_timestamp` tolerates:
///   - `2026-05-02T21:14:23.189Z`  — Game.log canonical ISO-8601
///   - `2026-05-04T21:10:12+00:00` — chrono `to_rfc3339` (GameCrash)
///   - `2026-05-06 12:34:56.789`   — LauncherActivity (Electron): a
///     naive, offset-less form with a space separator.
///
/// The naive launcher form has no timezone, so it is interpreted as
/// UTC. That may be off by the user's local offset for launcher events,
/// but it is far better than the previous behaviour — `parse_from_rfc3339`
/// rejected it outright, yielding a NULL `event_timestamp` that dropped
/// the row out of every time-ordered query (F12). A single fixed
/// assumption keeps ordering among launcher events consistent.
fn parse_event_timestamp(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Fallback: the launcher's naive `YYYY-MM-DD HH:MM:SS(.fff)`, with
    // or without fractional seconds.
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(naive.and_utc());
        }
    }
    None
}

// -- Test-only query stub --------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// In-memory equivalent of the `($n IS NULL OR event_timestamp >= $n)`
    /// / `($n IS NULL OR event_timestamp < $n)` pair the Postgres
    /// aggregates use — see [`EventQuery::payload_field_breakdown`]'s
    /// "Window convention": `since` inclusive, `until` exclusive.
    ///
    /// One function rather than a copy per aggregate, so the four methods
    /// that share the convention cannot drift apart on the boundary.
    ///
    /// A NULL-timestamp row passes only while BOTH bounds are absent,
    /// mirroring Postgres, where `NULL >= x` and `NULL < x` are both never
    /// true but the `IS NULL` short-circuit lets the row through when the
    /// bound itself is absent.
    fn in_window(
        ts: Option<DateTime<Utc>>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> bool {
        if since.is_none() && until.is_none() {
            return true;
        }
        match ts {
            Some(t) => since.map_or(true, |s| t >= s) && until.map_or(true, |u| t < u),
            None => false,
        }
    }

    pub struct MemoryQuery {
        rows: Vec<StoredQueryEvent>,
        /// (actor_handle, row) pairs. The production read path filters
        /// audit_log by `actor_handle`, so the test stub keeps the
        /// handle alongside each row to mirror that scoping.
        ingest_history: Vec<(String, IngestBatchRow)>,
    }

    impl MemoryQuery {
        pub fn new(rows: Vec<StoredQueryEvent>) -> Self {
            Self {
                rows,
                ingest_history: Vec::new(),
            }
        }

        pub fn with_ingest_history(mut self, history: Vec<(String, IngestBatchRow)>) -> Self {
            self.ingest_history = history;
            self
        }
    }

    #[async_trait]
    impl EventQuery for MemoryQuery {
        async fn list_filtered(
            &self,
            claimed_handle: &str,
            filters: EventFilters,
        ) -> Result<Vec<StoredQueryEvent>, RepoError> {
            let mut rows: Vec<StoredQueryEvent> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| match &filters.event_type {
                    Some(t) => &r.event_type == t,
                    None => true,
                })
                .filter(|r| match (filters.since, r.event_timestamp) {
                    (Some(s), Some(ts)) => ts >= s,
                    (Some(_), None) => false,
                    _ => true,
                })
                .filter(|r| match (filters.until, r.event_timestamp) {
                    (Some(u), Some(ts)) => ts <= u,
                    (Some(_), None) => false,
                    _ => true,
                })
                .filter(|r| match filters.cursor {
                    Some(SeqCursor::Before(n)) => r.seq < n,
                    Some(SeqCursor::After(n)) => r.seq > n,
                    None => true,
                })
                .cloned()
                .collect();

            match filters.cursor {
                Some(SeqCursor::After(_)) => rows.sort_by_key(|r| r.seq),
                _ => rows.sort_by(|a, b| b.seq.cmp(&a.seq)),
            }

            rows.truncate(filters.limit.max(0) as usize);
            Ok(rows)
        }

        async fn timeline(
            &self,
            claimed_handle: &str,
            days: u32,
        ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
            // Window: events whose event_timestamp is within the last
            // `days` days. The handler does the zero-padding pass; we
            // only return present days.
            let since = Utc::now() - chrono::Duration::days(days as i64);
            let mut counts: std::collections::BTreeMap<NaiveDate, i64> =
                std::collections::BTreeMap::new();
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle) {
                    continue;
                }
                let Some(ts) = r.event_timestamp else {
                    continue;
                };
                if ts < since {
                    continue;
                }
                *counts.entry(ts.date_naive()).or_default() += 1;
            }
            Ok(counts.into_iter().collect())
        }

        /// In-memory scope-aware timeline. Filters raw rows by the
        /// allow/deny event-type lists BEFORE bucketing — once rows
        /// have been GROUPed into daily counts the per-type identity
        /// is gone, so filtering has to happen on the row stream.
        /// Mirrors the precedence in `apply_event_type_filter`:
        /// allowlist first (absent = dropped), then denylist. Also
        /// honours `hidden_at IS NOT NULL` the same way the production
        /// `timeline_shared` does so test parity is preserved.
        async fn timeline_shared_filtered(
            &self,
            claimed_handle: &str,
            days: u32,
            allow_types: Option<&[String]>,
            deny_types: Option<&[String]>,
        ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
            let since = Utc::now() - chrono::Duration::days(days as i64);
            let mut counts: std::collections::BTreeMap<NaiveDate, i64> =
                std::collections::BTreeMap::new();
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle) {
                    continue;
                }
                if r.hidden_at.is_some() {
                    continue;
                }
                let Some(ts) = r.event_timestamp else {
                    continue;
                };
                if ts < since {
                    continue;
                }
                if let Some(allow) = allow_types {
                    if !allow.iter().any(|t| t == &r.event_type) {
                        continue;
                    }
                }
                if let Some(deny) = deny_types {
                    if deny.iter().any(|t| t == &r.event_type) {
                        continue;
                    }
                }
                *counts.entry(ts.date_naive()).or_default() += 1;
            }
            Ok(counts.into_iter().collect())
        }

        async fn summary_for_handle(
            &self,
            claimed_handle: &str,
        ) -> Result<(u64, Vec<(String, u64)>), RepoError> {
            let mine: Vec<&StoredQueryEvent> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .collect();
            let total = mine.len() as u64;
            let mut counts: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            for e in &mine {
                *counts.entry(e.event_type.clone()).or_default() += 1;
            }
            let mut by_type: Vec<(String, u64)> = counts.into_iter().collect();
            by_type.sort_by(|a, b| b.1.cmp(&a.1));
            Ok((total, by_type))
        }

        async fn event_type_breakdown(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
        ) -> Result<Vec<EventTypeStats>, RepoError> {
            let mut counts: std::collections::HashMap<String, (i64, Option<DateTime<Utc>>)> =
                std::collections::HashMap::new();
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle) {
                    continue;
                }
                if let (Some(s), Some(ts)) = (since, r.event_timestamp) {
                    if ts < s {
                        continue;
                    }
                } else if since.is_some() && r.event_timestamp.is_none() {
                    // The Postgres impl drops timestampless rows when
                    // `since` is set — match it so tests are honest.
                    continue;
                }
                let entry = counts.entry(r.event_type.clone()).or_insert((0, None));
                entry.0 += 1;
                if let Some(ts) = r.event_timestamp {
                    entry.1 = Some(entry.1.map_or(ts, |prev| prev.max(ts)));
                }
            }
            let mut rows: Vec<EventTypeStats> = counts
                .into_iter()
                .map(|(event_type, (count, last_seen))| EventTypeStats {
                    event_type,
                    count,
                    last_seen,
                })
                .collect();
            rows.sort_by(|a, b| {
                b.count
                    .cmp(&a.count)
                    .then_with(|| a.event_type.cmp(&b.event_type))
            });
            Ok(rows)
        }

        async fn sessions_for_handle(
            &self,
            claimed_handle: &str,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<InferredSession>, RepoError> {
            // Replicates the Postgres window-function logic in plain Rust:
            // sort the (timestampful) events ascending, walk them, and
            // start a new session whenever the gap to the previous event
            // exceeds the configured idle threshold.
            let mut timestamps: Vec<DateTime<Utc>> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| !NON_SESSION_EVENT_TYPES.contains(&r.event_type.as_str()))
                .filter_map(|r| r.event_timestamp)
                .collect();
            timestamps.sort();

            let gap = chrono::Duration::minutes(SESSION_IDLE_GAP_MINUTES);
            let mut sessions: Vec<InferredSession> = Vec::new();
            for ts in timestamps {
                match sessions.last_mut() {
                    Some(s) if ts - s.end_at <= gap => {
                        s.end_at = ts;
                        s.event_count += 1;
                    }
                    _ => sessions.push(InferredSession {
                        start_at: ts,
                        end_at: ts,
                        event_count: 1,
                    }),
                }
            }
            sessions.sort_by(|a, b| b.start_at.cmp(&a.start_at));
            let start = offset.max(0) as usize;
            let take = limit.max(0) as usize;
            Ok(sessions.into_iter().skip(start).take(take).collect())
        }

        async fn total_playtime_secs(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
        ) -> Result<i64, RepoError> {
            // Reuse the same session-inference logic as sessions_for_handle,
            // then sum (end_at - start_at).num_seconds() for sessions whose
            // start_at falls within the requested window.
            let mut timestamps: Vec<DateTime<Utc>> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| !NON_SESSION_EVENT_TYPES.contains(&r.event_type.as_str()))
                .filter_map(|r| r.event_timestamp)
                .collect();
            timestamps.sort();

            let gap = chrono::Duration::minutes(SESSION_IDLE_GAP_MINUTES);
            let mut sessions: Vec<InferredSession> = Vec::new();
            for ts in timestamps {
                match sessions.last_mut() {
                    Some(s) if ts - s.end_at <= gap => {
                        s.end_at = ts;
                        s.event_count += 1;
                    }
                    _ => sessions.push(InferredSession {
                        start_at: ts,
                        end_at: ts,
                        event_count: 1,
                    }),
                }
            }

            let total = sessions
                .iter()
                .filter(|s| since.map_or(true, |t| s.start_at >= t))
                .map(|s| (s.end_at - s.start_at).num_seconds())
                .sum();
            Ok(total)
        }

        async fn count_sessions_since(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
        ) -> Result<i64, RepoError> {
            // Reuse the same session-inference logic as total_playtime_secs,
            // but return the COUNT of sessions whose start_at is in-window.
            let mut timestamps: Vec<DateTime<Utc>> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| !NON_SESSION_EVENT_TYPES.contains(&r.event_type.as_str()))
                .filter_map(|r| r.event_timestamp)
                .collect();
            timestamps.sort();

            let gap = chrono::Duration::minutes(SESSION_IDLE_GAP_MINUTES);
            let mut sessions: Vec<InferredSession> = Vec::new();
            for ts in timestamps {
                match sessions.last_mut() {
                    Some(s) if ts - s.end_at <= gap => {
                        s.end_at = ts;
                        s.event_count += 1;
                    }
                    _ => sessions.push(InferredSession {
                        start_at: ts,
                        end_at: ts,
                        event_count: 1,
                    }),
                }
            }

            let count = sessions
                .iter()
                .filter(|s| since.map_or(true, |t| s.start_at >= t))
                .count() as i64;
            Ok(count)
        }

        async fn records_for_handle(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
        ) -> Result<RecordsAggregate, RepoError> {
            // Mirror the Postgres gap-idle sessionization, carrying a
            // per-event "is death" flag so per-session death counts fall
            // out of the same walk. `since` (when set) drops events
            // older than the window before any clustering happens.
            let mut events: Vec<(DateTime<Utc>, bool)> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| !NON_SESSION_EVENT_TYPES.contains(&r.event_type.as_str()))
                .filter_map(|r| {
                    r.event_timestamp
                        .map(|ts| (ts, r.event_type == "player_death"))
                })
                .filter(|(ts, _)| since.map_or(true, |t| *ts >= t))
                .collect();
            events.sort_by_key(|(ts, _)| *ts);

            struct Sess {
                start: DateTime<Utc>,
                end: DateTime<Utc>,
                events: i64,
                deaths: i64,
            }
            let gap = chrono::Duration::minutes(SESSION_IDLE_GAP_MINUTES);
            let mut sessions: Vec<Sess> = Vec::new();
            for (ts, is_death) in events {
                match sessions.last_mut() {
                    Some(s) if ts - s.end <= gap => {
                        s.end = ts;
                        s.events += 1;
                        s.deaths += is_death as i64;
                    }
                    _ => sessions.push(Sess {
                        start: ts,
                        end: ts,
                        events: 1,
                        deaths: is_death as i64,
                    }),
                }
            }

            // Survival streak: max gap between consecutive player_death
            // events across the whole timeline (not session-clustered).
            let mut death_ts: Vec<DateTime<Utc>> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| r.event_type == "player_death")
                .filter_map(|r| r.event_timestamp)
                .filter(|ts| since.map_or(true, |t| *ts >= t))
                .collect();
            death_ts.sort();

            Ok(RecordsAggregate {
                longest_session_secs: sessions
                    .iter()
                    .map(|s| (s.end - s.start).num_seconds())
                    .max()
                    .unwrap_or(0),
                busiest_session_events: sessions.iter().map(|s| s.events).max().unwrap_or(0),
                longest_survival_streak_secs: death_ts
                    .windows(2)
                    .map(|w| (w[1] - w[0]).num_seconds())
                    .max()
                    .unwrap_or(0),
                deadliest_session_deaths: sessions.iter().map(|s| s.deaths).max().unwrap_or(0),
            })
        }

        async fn lives_for_handle(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
        ) -> Result<LivesData, RepoError> {
            // Ordered event stream for the handle. Unlike the other
            // session queries, this one must NOT exclude `game_crash` —
            // `derive_lives` needs GameCrash events to close a life as
            // `Crash` rather than leaving it `StillAlive`. `since` (when
            // set) drops events older than the window first.
            let mut rows: Vec<&StoredQueryEvent> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| r.event_timestamp.is_some())
                .filter(|r| since.map_or(true, |t| r.event_timestamp.is_some_and(|ts| ts >= t)))
                .collect();
            rows.sort_by_key(|r| (r.event_timestamp, r.seq));

            let envs: Vec<EventEnvelope> = rows
                .iter()
                .map(|r| EventEnvelope {
                    idempotency_key: r.seq.to_string(),
                    raw_line: String::new(),
                    event: serde_json::from_value(r.payload.clone()).ok(),
                    source: LogSource::Live,
                    source_offset: r.source_offset.max(0) as u64,
                    metadata: None,
                    resolved_location: None,
                })
                .collect();
            let summary = derive_lives(&envs, &LifeConfig::default());

            // Canonical 30-min idle-gap session count. Deliberately NOT
            // `event_timeline::derive_sessions(..).len()` — that helper
            // is written for the paginated sessions LIST and internally
            // truncates to `SESSIONS_LIST_LIMIT` (50), which would
            // silently cap this endpoint's `sessions` (and inflate
            // `deaths_per_session`) for any handle with more than 50
            // gap-idle sessions. `count_sessions_since(.., since)` scopes
            // the session count to the same window as the FSM stream —
            // `None` yields the unbounded full-history count.
            let sessions = self.count_sessions_since(claimed_handle, since).await? as u32;

            Ok(LivesData { summary, sessions })
        }

        async fn ingest_history_for_handle(
            &self,
            actor_handle: &str,
            device_id: Option<Uuid>,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<IngestBatchRow>, RepoError> {
            let mut rows: Vec<IngestBatchRow> = self
                .ingest_history
                .iter()
                .filter(|(h, _)| h.eq_ignore_ascii_case(actor_handle))
                .filter(|(_, row)| match device_id {
                    Some(want) => row.device_id == Some(want),
                    None => true,
                })
                .map(|(_, row)| row.clone())
                .collect();
            rows.sort_by(|a, b| b.seq.cmp(&a.seq));
            let start = offset.max(0) as usize;
            let take = limit.max(0) as usize;
            Ok(rows.into_iter().skip(start).take(take).collect())
        }

        async fn location_event_stream(
            &self,
            claimed_handle: &str,
            event_types: &[&str],
            since: DateTime<Utc>,
            limit: i64,
        ) -> Result<Vec<LatestLocationEvent>, RepoError> {
            // Same filter as location_trace but newest-LAST so the
            // dwell-walker can iterate forward in time.
            let mut rows: Vec<LatestLocationEvent> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter_map(|r| {
                    let ts = r.event_timestamp?;
                    if ts < since || !event_types.contains(&r.event_type.as_str()) {
                        return None;
                    }
                    Some(LatestLocationEvent {
                        event_type: r.event_type.clone(),
                        event_timestamp: ts,
                        payload: r.payload.clone(),
                    })
                })
                .collect();
            rows.sort_by(|a, b| a.event_timestamp.cmp(&b.event_timestamp));
            // Mirror the Postgres `ORDER BY ts DESC LIMIT` + reverse:
            // keep only the most-recent `limit` raw events by dropping
            // from the FRONT (oldest), preserving oldest-first order.
            if limit >= 0 && rows.len() > limit as usize {
                let drop = rows.len() - limit as usize;
                rows.drain(0..drop);
            }
            Ok(rows)
        }

        // Mirrors the trait's allow — same eight parameters.
        #[allow(clippy::too_many_arguments)]
        async fn payload_field_breakdown(
            &self,
            claimed_handle: &str,
            event_type: &str,
            payload_field: &str,
            payload_filter: Option<PayloadFilter<'_>>,
            since: Option<DateTime<Utc>>,
            until: Option<DateTime<Utc>>,
            limit: i64,
        ) -> Result<Vec<PayloadFieldBucket>, RepoError> {
            use std::collections::HashMap;
            let mut counts: HashMap<String, i64> = HashMap::new();
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle)
                    || r.event_type != event_type
                {
                    continue;
                }
                if !in_window(r.event_timestamp, since, until) {
                    continue;
                }
                if let Some(filter) = payload_filter {
                    let actual = r.payload.get(filter.field).and_then(|v| v.as_str());
                    if actual != Some(filter.equals) {
                        continue;
                    }
                }
                if let Some(value) = r.payload.get(payload_field).and_then(|v| v.as_str()) {
                    *counts.entry(value.to_string()).or_insert(0) += 1;
                }
            }
            let mut rows: Vec<PayloadFieldBucket> = counts
                .into_iter()
                .map(|(value, count)| PayloadFieldBucket { value, count })
                .collect();
            rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
            rows.truncate(limit.max(0) as usize);
            Ok(rows)
        }

        async fn docking_occurrences(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
            until: Option<DateTime<Utc>>,
        ) -> Result<DockingOccurrences, RepoError> {
            use std::collections::HashMap;

            const RAW_RUN_GAP_SECONDS: i64 = 120;

            let mut rows: Vec<&StoredQueryEvent> = self
                .rows
                .iter()
                .filter(|row| row.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .collect();
            rows.sort_by_key(|row| (row.event_timestamp, row.seq));

            let mut counts: HashMap<String, i64> = HashMap::new();
            let mut unknown = 0;
            let mut previous_raw_timestamp: Option<DateTime<Utc>> = None;
            let mut raw_starts_by_timestamp: HashMap<DateTime<Utc>, i64> = HashMap::new();
            let mut bursts_by_timestamp: HashMap<DateTime<Utc>, i64> = HashMap::new();
            for row in rows {
                let is_stow_burst = row.event_type == "burst_summary"
                    && row.payload.get("rule_id").and_then(Value::as_str)
                        == Some("vehicle_stowed_burst");
                let is_raw_stow = row.event_type == "vehicle_stowed";
                if is_raw_stow {
                    let same_raw_episode = match (previous_raw_timestamp, row.event_timestamp) {
                        (Some(before), Some(after)) => {
                            let gap = (after - before).num_seconds();
                            (0..=RAW_RUN_GAP_SECONDS).contains(&gap)
                        }
                        _ => false,
                    };
                    if !same_raw_episode && in_window(row.event_timestamp, since, until) {
                        if let Some(area) = row.payload.get("landing_area").and_then(Value::as_str)
                        {
                            *counts.entry(area.to_string()).or_insert(0) += 1;
                        } else {
                            unknown += 1;
                        }
                        if let Some(timestamp) = row.event_timestamp {
                            *raw_starts_by_timestamp.entry(timestamp).or_insert(0) += 1;
                        }
                    }
                    previous_raw_timestamp = row.event_timestamp;
                } else if is_stow_burst && in_window(row.event_timestamp, since, until) {
                    if let Some(timestamp) = row.event_timestamp {
                        *bursts_by_timestamp.entry(timestamp).or_insert(0) += 1;
                    } else {
                        unknown += 1;
                    }
                }
            }
            for (timestamp, burst_count) in bursts_by_timestamp {
                let represented_by_raw = raw_starts_by_timestamp
                    .get(&timestamp)
                    .copied()
                    .unwrap_or(0);
                unknown += (burst_count - represented_by_raw).max(0);
            }

            let mut landing_areas: Vec<PayloadFieldBucket> = counts
                .into_iter()
                .map(|(value, count)| PayloadFieldBucket { value, count })
                .collect();
            landing_areas.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
            Ok(DockingOccurrences {
                landing_areas,
                unknown,
            })
        }

        async fn objective_outcomes(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
            until: Option<DateTime<Utc>>,
        ) -> Result<ObjectiveOutcomes, RepoError> {
            use std::collections::HashMap;
            fn rank(state: &str) -> u8 {
                match state {
                    "completed" => 4,
                    "failed" => 3,
                    // Two spellings, one bucket. Current collectors store
                    // WITHDRAWN as "withdrawn"; collectors older than
                    // v1.8.149 stored it as the parser's "unknown"
                    // catch-all, and those rows are never rewritten.
                    // "unknown" also still covers a state CIG ships that
                    // the parser has no variant for yet. All are resolved
                    // but not completed, so they rank the same.
                    "withdrawn" | "unknown" => 2,
                    "in_progress" => 1,
                    _ => 0,
                }
            }
            let mut best: HashMap<String, u8> = HashMap::new();
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle)
                    || r.event_type != "mission_objective"
                {
                    continue;
                }
                if !in_window(r.event_timestamp, since, until) {
                    continue;
                }
                let key = r
                    .payload
                    .get("objective_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("seq:{}", r.seq));
                let state = r
                    .payload
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let slot = best.entry(key).or_insert(0);
                *slot = (*slot).max(rank(state));
            }
            let mut out = ObjectiveOutcomes::default();
            for v in best.values() {
                match v {
                    4 => out.completed += 1,
                    3 => out.failed += 1,
                    2 => out.unresolved += 1,
                    1 => out.no_outcome += 1,
                    _ => {}
                }
            }
            Ok(out)
        }

        async fn contract_runs(
            &self,
            claimed_handle: &str,
            since: Option<DateTime<Utc>>,
        ) -> Result<Vec<ContractRunRow>, RepoError> {
            // Full ordered event stream -- `since` scopes the OUTPUT runs
            // by `accepted_at` below, NOT this input window (see the
            // trait doc): truncating the input here would risk the fold
            // mis-closing a run right at the window boundary.
            let mut rows: Vec<&StoredQueryEvent> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter(|r| r.event_timestamp.is_some())
                .collect();
            rows.sort_by_key(|r| (r.event_timestamp, r.seq));

            let envs: Vec<EventEnvelope> = rows
                .iter()
                .map(|r| EventEnvelope {
                    idempotency_key: r.seq.to_string(),
                    raw_line: String::new(),
                    event: serde_json::from_value(r.payload.clone()).ok(),
                    source: LogSource::Live,
                    source_offset: r.source_offset.max(0) as u64,
                    metadata: None,
                    resolved_location: None,
                })
                .collect();

            let mut out: Vec<ContractRunRow> =
                derive_contract_runs(&envs, &ContractConfig::default())
                    .into_iter()
                    .map(contract_run_to_row)
                    // A timestampless (unparseable) accepted_at is dropped
                    // once `since` is set, same as the Postgres read's
                    // `accepted_at >= $2` (NULL compares false there too).
                    .filter(|r| since.map_or(true, |s| r.accepted_at.is_some_and(|ts| ts >= s)))
                    .collect();
            out.sort_by(|a, b| b.accepted_at.cmp(&a.accepted_at));
            Ok(out)
        }

        async fn payload_numeric_sum(
            &self,
            claimed_handle: &str,
            event_type: &str,
            numeric_field: &str,
            since: Option<DateTime<Utc>>,
            until: Option<DateTime<Utc>>,
        ) -> Result<i64, RepoError> {
            let mut sum: i64 = 0;
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle)
                    || r.event_type != event_type
                {
                    continue;
                }
                if !in_window(r.event_timestamp, since, until) {
                    continue;
                }
                // Accept both the JSON-number form (`price: 15000`) and a
                // numeric-string form (`price: "15000"`).
                if let Some(v) = r.payload.get(numeric_field) {
                    if let Some(n) = v.as_i64() {
                        sum += n;
                    } else if let Some(n) = v.as_str().and_then(|s| s.parse::<i64>().ok()) {
                        sum += n;
                    }
                }
            }
            Ok(sum)
        }

        async fn count_event_type(
            &self,
            claimed_handle: &str,
            event_type: &str,
            payload_filter: Option<PayloadFilter<'_>>,
            since: Option<DateTime<Utc>>,
            until: Option<DateTime<Utc>>,
        ) -> Result<u64, RepoError> {
            let mut n = 0u64;
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle)
                    || r.event_type != event_type
                {
                    continue;
                }
                if !in_window(r.event_timestamp, since, until) {
                    continue;
                }
                if let Some(filter) = payload_filter {
                    let actual = r.payload.get(filter.field).and_then(|v| v.as_str());
                    if actual != Some(filter.equals) {
                        continue;
                    }
                }
                n += 1;
            }
            Ok(n)
        }

        async fn has_events_in_window(
            &self,
            claimed_handle: &str,
            since: DateTime<Utc>,
            until: DateTime<Utc>,
        ) -> Result<bool, RepoError> {
            Ok(self.rows.iter().any(|r| {
                r.claimed_handle.eq_ignore_ascii_case(claimed_handle)
                    && in_window(r.event_timestamp, Some(since), Some(until))
            }))
        }

        async fn latest_location(
            &self,
            claimed_handle: &str,
            event_types: &[&str],
        ) -> Result<LatestLocation, RepoError> {
            // Two passes over the same in-memory vec: one for the
            // most-recent location-bearing event, one for the most-
            // recent join_pu shard hint regardless of position.
            let mut location_event: Option<LatestLocationEvent> = None;
            let mut shard_hint: Option<(DateTime<Utc>, String)> = None;
            for r in &self.rows {
                if !r.claimed_handle.eq_ignore_ascii_case(claimed_handle) {
                    continue;
                }
                let Some(ts) = r.event_timestamp else {
                    continue;
                };
                if event_types.contains(&r.event_type.as_str()) {
                    let better = match &location_event {
                        Some(le) => ts > le.event_timestamp,
                        None => true,
                    };
                    if better {
                        location_event = Some(LatestLocationEvent {
                            event_type: r.event_type.clone(),
                            event_timestamp: ts,
                            payload: r.payload.clone(),
                        });
                    }
                }
                if r.event_type == "join_pu" {
                    if let Some(s) = r.payload.get("shard").and_then(|v| v.as_str()) {
                        if shard_hint
                            .as_ref()
                            .map_or(true, |(prev_ts, _)| ts > *prev_ts)
                        {
                            shard_hint = Some((ts, s.to_string()));
                        }
                    }
                }
            }
            Ok(LatestLocation {
                location_event,
                shard_hint: shard_hint.map(|(_, s)| s),
            })
        }

        async fn recent_location_events(
            &self,
            claimed_handle: &str,
            event_types: &[&str],
            limit: i64,
        ) -> Result<Vec<LatestLocationEvent>, RepoError> {
            let mut rows: Vec<LatestLocationEvent> = self
                .rows
                .iter()
                .filter(|r| r.claimed_handle.eq_ignore_ascii_case(claimed_handle))
                .filter_map(|r| {
                    let ts = r.event_timestamp?;
                    if !event_types.contains(&r.event_type.as_str()) {
                        return None;
                    }
                    Some(LatestLocationEvent {
                        event_type: r.event_type.clone(),
                        event_timestamp: ts,
                        payload: r.payload.clone(),
                    })
                })
                .collect();
            // Newest-first to match the Postgres impl's ORDER BY DESC.
            // The handler relies on this ordering for its walk-back.
            rows.sort_by(|a, b| b.event_timestamp.cmp(&a.event_timestamp));
            rows.truncate(limit.max(0) as usize);
            Ok(rows)
        }
    }
}

// -- Postgres store --------------------------------------------------

pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Recompute session_summary + character_records + entity_rollup_agg for
    /// `handle` from events under a per-handle advisory lock, then clear
    /// stat_rollup_state.sessions_dirty. Called by
    /// [`Self::ensure_session_stats_fresh`] on a dirty/missing rollup.
    ///
    /// Concurrency: a per-handle `pg_advisory_xact_lock` serializes rebuilds of
    /// the same handle so two concurrent first-reads can't collide on the rollup
    /// PKs, and a double-check under the lock skips the recompute when a sibling
    /// rebuild already cleared the flag (herd-safe at deploy, when 0058 marks
    /// every handle dirty). The dirty-clear (step 4) is conditional on the
    /// `updated_at` captured under the lock: if an ingest batch commits during
    /// the rebuild (bumping updated_at + re-setting dirty=TRUE), the clear no-ops
    /// and the handle stays dirty so the next read re-materializes the batch's
    /// events instead of dropping them.
    pub(crate) async fn rebuild_handle_session_stats(&self, handle: &str) -> Result<(), RepoError> {
        let gap_minutes = SESSION_IDLE_GAP_MINUTES as i32;
        let mut tx = self.pool.begin().await?;

        // Serialize per-handle rebuilds for the duration of this tx (namespace
        // 21331 = 'SS', a key space distinct from the audit advisory lock).
        sqlx::query("SELECT pg_advisory_xact_lock(21331, hashtext(LOWER($1)))")
            .bind(handle)
            .execute(&mut *tx)
            .await?;

        // Double-checked under the lock: read the current dirty flag + its
        // updated_at in one snapshot. A missing row counts as dirty (pre-0058 /
        // never-seen handle). If a sibling rebuild already cleared it while we
        // waited on the lock, skip the recompute (the tx rolls back on return,
        // releasing the advisory lock).
        let state: Option<(bool, DateTime<Utc>)> = sqlx::query_as(
            "SELECT sessions_dirty, updated_at FROM stat_rollup_state
             WHERE claimed_handle = LOWER($1)",
        )
        .bind(handle)
        .fetch_optional(&mut *tx)
        .await?;
        let (still_dirty, captured_updated_at) = match state {
            Some((dirty, ts)) => (dirty, Some(ts)),
            None => (true, None),
        };
        if !still_dirty {
            return Ok(());
        }

        // (1) session_summary: DELETE then re-INSERT from the gap-sessionized events.
        //     session_id is the running-sum ordinal cast to TEXT (the column is TEXT).
        sqlx::query("DELETE FROM session_summary WHERE claimed_handle = LOWER($1)")
            .bind(handle)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"
            WITH gaps AS (
                SELECT event_timestamp, event_type,
                       LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS prev_ts
                FROM events
                WHERE claimed_handle = LOWER($1) AND event_timestamp IS NOT NULL
                  AND event_type NOT IN ('launcher_activity','game_crash')
            ), labeled AS (
                SELECT event_timestamp, event_type,
                       SUM(CASE WHEN prev_ts IS NULL
                                 OR event_timestamp - prev_ts > make_interval(mins => $2)
                                THEN 1 ELSE 0 END)
                         OVER (ORDER BY event_timestamp ASC) AS session_id
                FROM gaps
            )
            INSERT INTO session_summary
                (claimed_handle, session_id, started_at, ended_at, event_count, death_count)
            SELECT LOWER($1), session_id::text,
                   MIN(event_timestamp), MAX(event_timestamp),
                   COUNT(*)::bigint,
                   COUNT(*) FILTER (WHERE event_type = 'player_death')::bigint
            FROM labeled GROUP BY session_id
            "#,
        )
        .bind(handle)
        .bind(gap_minutes)
        .execute(&mut *tx)
        .await?;

        // (2) character_records: reproduce records_for_handle(handle, None) exactly from
        //     the freshly-written session_summary (per-session maxes) + a death-gap scan.
        //     kills/pvp_deaths are NOT materialized (combat stays on the live combat_counts
        //     scan; no 'kill' event_type exists) — they keep DEFAULT 0.
        sqlx::query(
            r#"
            WITH sess AS (
                SELECT started_at, ended_at, event_count, death_count
                FROM session_summary WHERE claimed_handle = LOWER($1)
            ), streak AS (
                SELECT MAX(EXTRACT(EPOCH FROM gap))::bigint AS longest_gap
                FROM (
                    SELECT event_timestamp
                           - LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS gap
                    FROM events
                    WHERE claimed_handle = LOWER($1)
                      AND event_type = 'player_death' AND event_timestamp IS NOT NULL
                ) g
            )
            INSERT INTO character_records
                (claimed_handle, total_deaths, total_sessions, longest_session_secs,
                 busiest_session_events, deadliest_session_deaths,
                 longest_survival_gap_secs, first_event_at, last_event_at, updated_at)
            SELECT LOWER($1),
                   (SELECT COALESCE(SUM(death_count),0) FROM sess),
                   (SELECT COUNT(*) FROM sess),
                   (SELECT COALESCE(MAX(EXTRACT(EPOCH FROM (ended_at - started_at))::bigint),0) FROM sess),
                   (SELECT COALESCE(MAX(event_count),0) FROM sess),
                   (SELECT COALESCE(MAX(death_count),0) FROM sess),
                   (SELECT COALESCE(longest_gap,0) FROM streak),
                   (SELECT MIN(started_at) FROM sess),
                   (SELECT MAX(ended_at) FROM sess),
                   now()
            ON CONFLICT (claimed_handle) DO UPDATE SET
                total_deaths = EXCLUDED.total_deaths,
                total_sessions = EXCLUDED.total_sessions,
                longest_session_secs = EXCLUDED.longest_session_secs,
                busiest_session_events = EXCLUDED.busiest_session_events,
                deadliest_session_deaths = EXCLUDED.deadliest_session_deaths,
                longest_survival_gap_secs = EXCLUDED.longest_survival_gap_secs,
                first_event_at = EXCLUDED.first_event_at,
                last_event_at = EXCLUDED.last_event_at,
                updated_at = now()
            "#,
        )
        .bind(handle)
        .execute(&mut *tx)
        .await?;

        // (3) entity_rollup_agg (A4): display fields mirror the list_entities
        //     GROUP BY; session_count is the per-ID distinct-session count under
        //     the PROCESS_INIT sessionizer, reproducing
        //     entity_rollup::derive_entity_session_data (NOT the 30-min gap
        //     sessionizer used for session_summary above). Ordered by
        //     (event_timestamp, idempotency_key) — source_offset scrambles
        //     cross-day log rotations; no NON_SESSION filter; session_end closes
        //     without tallying; a null-timestamp row is dropped (it can't be
        //     placed in a session). The count is per-ID (kind-agnostic), applied
        //     to every (kind, id) row via LEFT JOIN so an ID that spans kinds
        //     gets the same total — matching the route fold's id-only lookup.
        sqlx::query("DELETE FROM entity_rollup_agg WHERE claimed_handle = LOWER($1)")
            .bind(handle)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            r#"
            WITH ordered AS (
                SELECT event_timestamp, idempotency_key, event_type,
                       metadata->'primary_entity'->>'id' AS meta_id,
                       SUM(CASE WHEN event_type = 'process_init' THEN 1 ELSE 0 END)
                           OVER (ORDER BY event_timestamp ASC, idempotency_key ASC) AS session_ord
                FROM events
                WHERE claimed_handle = LOWER($1) AND event_timestamp IS NOT NULL
            ), in_stream AS (
                SELECT *,
                       ROW_NUMBER() OVER (PARTITION BY session_ord
                                          ORDER BY event_timestamp ASC, idempotency_key ASC) AS rn
                FROM ordered WHERE session_ord > 0
            ), first_end AS (
                SELECT session_ord,
                       MIN(rn) FILTER (WHERE event_type = 'session_end') AS first_end_rn
                FROM in_stream GROUP BY session_ord
            ), tally AS (
                SELECT s.session_ord, s.meta_id
                FROM in_stream s JOIN first_end f USING (session_ord)
                WHERE (f.first_end_rn IS NULL OR s.rn <= f.first_end_rn)
                  AND s.event_type <> 'session_end'
                  AND s.meta_id IS NOT NULL
            ), id_counts AS (
                SELECT meta_id AS entity_id, COUNT(DISTINCT session_ord)::bigint AS session_count
                FROM tally GROUP BY meta_id
            ), entities AS (
                SELECT metadata->'primary_entity'->>'kind' AS entity_kind,
                       metadata->'primary_entity'->>'id'   AS entity_id,
                       (array_agg(NULLIF(metadata->'primary_entity'->>'display_name','')
                                  ORDER BY source_offset DESC))[1] AS display_name,
                       COUNT(*)::bigint     AS event_count,
                       MIN(event_timestamp) AS first_seen_at,
                       MAX(event_timestamp) AS last_seen_at
                FROM events
                WHERE claimed_handle = LOWER($1) AND metadata IS NOT NULL
                  AND metadata->'primary_entity'->>'kind' IS NOT NULL
                  AND metadata->'primary_entity'->>'id'   IS NOT NULL
                GROUP BY 1, 2
            )
            INSERT INTO entity_rollup_agg
                (claimed_handle, entity_kind, entity_id, display_name,
                 event_count, session_count, first_seen_at, last_seen_at, updated_at)
            SELECT LOWER($1), e.entity_kind, e.entity_id, e.display_name,
                   e.event_count, COALESCE(c.session_count, 0),
                   e.first_seen_at, e.last_seen_at, now()
            FROM entities e
            LEFT JOIN id_counts c ON c.entity_id = e.entity_id
            "#,
        )
        .bind(handle)
        .execute(&mut *tx)
        .await?;

        // (4) clear the dirty flag — but ONLY if no ingest batch bumped
        //     updated_at since we captured it under the lock. If a batch
        //     committed mid-rebuild, its events post-date this recompute's
        //     snapshot and its dirty=TRUE must survive so the next read
        //     re-materializes them. When the row was missing at capture
        //     (captured_updated_at = NULL) the INSERT path creates a FALSE row;
        //     a racing batch that created the row first has a non-NULL
        //     updated_at, so the WHERE is false and its dirty=TRUE is preserved.
        sqlx::query(
            "INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, rebuilt_at, updated_at)
             VALUES (LOWER($1), FALSE, now(), now())
             ON CONFLICT (claimed_handle) DO UPDATE
                 SET sessions_dirty = FALSE, rebuilt_at = now(), updated_at = now()
                 WHERE stat_rollup_state.updated_at = $2",
        )
        .bind(handle)
        .bind(captured_updated_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Rebuild session rollups for `handle` iff dirty or the state row is
    /// missing (a missing row is treated as dirty so pre-0058 handles and
    /// never-seen handles both recompute on first read). Cheap when clean:
    /// one point SELECT against `stat_rollup_state`'s primary key.
    pub(crate) async fn ensure_session_stats_fresh(&self, handle: &str) -> Result<(), RepoError> {
        let dirty: bool = sqlx::query_scalar(
            "SELECT COALESCE(
                (SELECT sessions_dirty FROM stat_rollup_state WHERE claimed_handle = LOWER($1)),
                TRUE)",
        )
        .bind(handle)
        .fetch_one(&self.pool)
        .await?;
        if dirty {
            self.rebuild_handle_session_stats(handle).await?;
        }
        Ok(())
    }

    /// Recompute `contract_runs` for `handle` from the handle's FULL
    /// ordered event history under a per-handle advisory lock, then clear
    /// `stat_rollup_state.contracts_dirty`. Called by
    /// [`Self::ensure_contract_runs_fresh`] on a dirty/missing rollup.
    ///
    /// Same concurrency shape as [`Self::rebuild_handle_session_stats`]: a
    /// distinct advisory-lock namespace (21332, vs. 21331 for the session
    /// rebuild — the two rollups are independent tables and must not
    /// serialize on each other's lock) serializes rebuilds of the same
    /// handle, a double-check under the lock skips the recompute when a
    /// sibling rebuild already cleared the flag, and the dirty-clear is
    /// conditional on the `updated_at` captured under the lock: if an
    /// ingest batch commits mid-rebuild (bumping `updated_at` and
    /// re-setting `contracts_dirty = TRUE`), the clear no-ops and the
    /// handle stays dirty so the next read re-materializes the batch's
    /// events instead of dropping them.
    pub(crate) async fn rebuild_contract_runs_for_handle(
        &self,
        handle: &str,
    ) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("SELECT pg_advisory_xact_lock(21332, hashtext(LOWER($1)))")
            .bind(handle)
            .execute(&mut *tx)
            .await?;

        // Double-checked under the lock, same shape as
        // rebuild_handle_session_stats: a missing row counts as dirty.
        let state: Option<(bool, DateTime<Utc>)> = sqlx::query_as(
            "SELECT contracts_dirty, updated_at FROM stat_rollup_state
             WHERE claimed_handle = LOWER($1)",
        )
        .bind(handle)
        .fetch_optional(&mut *tx)
        .await?;
        let (still_dirty, captured_updated_at) = match state {
            Some((dirty, ts)) => (dirty, Some(ts)),
            None => (true, None),
        };
        if !still_dirty {
            return Ok(());
        }

        // Full ordered event stream -- ORDER BY event_timestamp, NEVER
        // source_offset (which resets to 0 on every log rotation and would
        // interleave sessions across log files, firing a spurious
        // session-gap close_all and abandoning every open run). Same
        // ordering as `lives_for_handle`'s query.
        let rows: Vec<(String, Value)> = sqlx::query_as(
            "SELECT idempotency_key, payload FROM events
             WHERE claimed_handle = LOWER($1) AND event_timestamp IS NOT NULL
             ORDER BY event_timestamp ASC, idempotency_key ASC",
        )
        .bind(handle)
        .fetch_all(&mut *tx)
        .await?;

        let envs: Vec<EventEnvelope> = rows
            .iter()
            .map(|(idempotency_key, payload)| EventEnvelope {
                idempotency_key: idempotency_key.clone(),
                raw_line: String::new(),
                event: serde_json::from_value(payload.clone()).ok(),
                source: LogSource::Live,
                source_offset: 0,
                metadata: None,
                resolved_location: None,
            })
            .collect();
        let runs: Vec<ContractRunRow> = derive_contract_runs(&envs, &ContractConfig::default())
            .into_iter()
            .map(contract_run_to_row)
            .collect();

        // DELETE then re-INSERT as two statements in one transaction --
        // deliberately NOT a writable-CTE `DELETE ... INSERT`, which
        // self-collides on this repo's Postgres (both would see the same
        // pre-statement snapshot, so the INSERT would race the
        // not-yet-visible deletes). Same trap `retention.rs` documents for
        // `stat_event_counts`.
        sqlx::query("DELETE FROM contract_runs WHERE claimed_handle = LOWER($1)")
            .bind(handle)
            .execute(&mut *tx)
            .await?;

        if !runs.is_empty() {
            let n = runs.len();
            let mut mission_ids = Vec::with_capacity(n);
            let mut names = Vec::with_capacity(n);
            let mut states = Vec::with_capacity(n);
            let mut closed_bys = Vec::with_capacity(n);
            let mut step_counts = Vec::with_capacity(n);
            let mut steps_completes = Vec::with_capacity(n);
            let mut steps_remainings = Vec::with_capacity(n);
            let mut partial_histories = Vec::with_capacity(n);
            let mut connected_servers: Vec<Option<String>> = Vec::with_capacity(n);
            let mut accepted_ats: Vec<Option<DateTime<Utc>>> = Vec::with_capacity(n);
            let mut closed_ats: Vec<Option<DateTime<Utc>>> = Vec::with_capacity(n);
            let mut last_event_ats: Vec<Option<DateTime<Utc>>> = Vec::with_capacity(n);
            let mut steps_json: Vec<Value> = Vec::with_capacity(n);
            for r in &runs {
                mission_ids.push(r.mission_id.clone());
                names.push(r.name.clone());
                states.push(r.state.clone());
                closed_bys.push(r.closed_by.clone());
                step_counts.push(r.step_count);
                steps_completes.push(r.steps_complete);
                steps_remainings.push(r.steps_remaining);
                partial_histories.push(r.partial_history);
                connected_servers.push(r.connected_server.clone());
                accepted_ats.push(r.accepted_at);
                closed_ats.push(r.closed_at);
                last_event_ats.push(r.last_event_at);
                steps_json.push(
                    serde_json::to_value(&r.steps).expect("Vec<ContractStep> serialises to JSON"),
                );
            }
            let handles = vec![handle.to_ascii_lowercase(); n];

            sqlx::query(
                r#"
                INSERT INTO contract_runs (
                    claimed_handle, mission_id, accepted_at, name, state, closed_by,
                    step_count, steps_complete, steps_remaining, partial_history,
                    connected_server, closed_at, last_event_at, steps
                )
                SELECT * FROM UNNEST(
                    $1::text[], $2::text[], $3::timestamptz[], $4::text[], $5::text[], $6::text[],
                    $7::integer[], $8::integer[], $9::integer[], $10::boolean[],
                    $11::text[], $12::timestamptz[], $13::timestamptz[], $14::jsonb[]
                )
                "#,
            )
            .bind(&handles)
            .bind(&mission_ids)
            .bind(&accepted_ats)
            .bind(&names)
            .bind(&states)
            .bind(&closed_bys)
            .bind(&step_counts)
            .bind(&steps_completes)
            .bind(&steps_remainings)
            .bind(&partial_histories)
            .bind(&connected_servers)
            .bind(&closed_ats)
            .bind(&last_event_ats)
            .bind(&steps_json)
            .execute(&mut *tx)
            .await?;
        }

        // Clear the dirty flag -- conditional on `updated_at`, same
        // concurrency reasoning as rebuild_handle_session_stats's own
        // dirty-clear.
        sqlx::query(
            "INSERT INTO stat_rollup_state (claimed_handle, contracts_dirty, updated_at)
             VALUES (LOWER($1), FALSE, now())
             ON CONFLICT (claimed_handle) DO UPDATE
                 SET contracts_dirty = FALSE, updated_at = now()
                 WHERE stat_rollup_state.updated_at = $2",
        )
        .bind(handle)
        .bind(captured_updated_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Rebuild `contract_runs` for `handle` iff dirty or the state row is
    /// missing (a missing row counts as dirty, matching
    /// [`Self::ensure_session_stats_fresh`]). Cheap when clean: one point
    /// SELECT against `stat_rollup_state`'s primary key.
    pub(crate) async fn ensure_contract_runs_fresh(&self, handle: &str) -> Result<(), RepoError> {
        let dirty: bool = sqlx::query_scalar(
            "SELECT COALESCE(
                (SELECT contracts_dirty FROM stat_rollup_state WHERE claimed_handle = LOWER($1)),
                TRUE)",
        )
        .bind(handle)
        .fetch_one(&self.pool)
        .await?;
        if dirty {
            self.rebuild_contract_runs_for_handle(handle).await?;
        }
        Ok(())
    }

    /// Run-observed contract names (`contract_runs.name`) with no
    /// matching row in the published `contracts` catalog, ranked by
    /// OCCURRENCE — not distinct name count. Measured on a 280-log
    /// corpus against the live 266-row catalog: Combat Gauntlet is one
    /// catalog row ("Scenario #6") but the corpus ran scenarios #1–#8,
    /// 202 occurrences / 37% of all runs, while being only 8 of 147
    /// distinct unmatched names (~5%). A name-ranked list would bury
    /// that single biggest win under a long tail of one-off names —
    /// this ranks by `run_count DESC` for exactly that reason.
    ///
    /// `state <> 'superseded'` excludes re-accept bookkeeping rows (69
    /// of 609 runs in the same corpus, 11%) from the count — they're
    /// not distinct play, and counting them would inflate the ranking.
    /// A name whose every run is `Superseded` produces no group at all
    /// here, so it is correctly not surfaced as a gap.
    ///
    /// Matching against the catalog is exact (`LOWER(BTRIM(..))` only,
    /// no fuzzy normalisation) — deliberately so; a fuzzy matching
    /// engine is later, separate work.
    ///
    /// `limit` caps the ranked page returned; see
    /// [`Self::contract_catalog_gaps_total`] for the un-limited grand
    /// total across every gap name, independent of page size.
    pub async fn contract_catalog_gaps(
        &self,
        limit: i64,
    ) -> Result<Vec<ContractGapRow>, RepoError> {
        let rows: Vec<ContractGapSqlRow> = sqlx::query_as(
            "SELECT r.name,
                    COUNT(*)::BIGINT                          AS run_count,
                    COUNT(DISTINCT r.claimed_handle)::BIGINT  AS distinct_handles,
                    MIN(r.accepted_at)                        AS first_seen,
                    MAX(r.accepted_at)                        AS last_seen
             FROM contract_runs r
             WHERE r.state <> 'superseded'
               AND NOT EXISTS (
                   SELECT 1 FROM contracts c
                   WHERE LOWER(BTRIM(c.display_name)) = LOWER(BTRIM(r.name))
               )
             GROUP BY r.name
             ORDER BY run_count DESC, r.name ASC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(ContractGapRow::from).collect())
    }

    /// Grand total unmatched run occurrences across EVERY gap name —
    /// not just the `limit`-capped page [`Self::contract_catalog_gaps`]
    /// returns. The admin surface's headline "how big is the whole
    /// gap" number, so it stays accurate regardless of how many rows
    /// the ranked list shows. Same exclusions as
    /// [`Self::contract_catalog_gaps`] (non-superseded, catalog-exact-
    /// match `NOT EXISTS`), just ungrouped.
    pub async fn contract_catalog_gaps_total(&self) -> Result<i64, RepoError> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT
             FROM contract_runs r
             WHERE r.state <> 'superseded'
               AND NOT EXISTS (
                   SELECT 1 FROM contracts c
                   WHERE LOWER(BTRIM(c.display_name)) = LOWER(BTRIM(r.name))
               )",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(total)
    }
}

#[async_trait]
impl EventQuery for PostgresStore {
    async fn list_filtered(
        &self,
        claimed_handle: &str,
        filters: EventFilters,
    ) -> Result<Vec<StoredQueryEvent>, RepoError> {
        // Build the filter set with QueryBuilder so every value
        // hits the wire as a bound parameter — no string interpolation.
        let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
            "SELECT seq, claimed_handle, event_type, event_timestamp,
                    log_source, source_offset, payload, resolved_location, hidden_at
             FROM events
             WHERE claimed_handle = LOWER(",
        );
        qb.push_bind(claimed_handle);
        qb.push(")");

        if let Some(t) = &filters.event_type {
            qb.push(" AND event_type = ");
            qb.push_bind(t.clone());
        }
        if let Some(s) = filters.since {
            qb.push(" AND event_timestamp >= ");
            qb.push_bind(s);
        }
        if let Some(u) = filters.until {
            qb.push(" AND event_timestamp <= ");
            qb.push_bind(u);
        }
        match filters.cursor {
            Some(SeqCursor::Before(n)) => {
                qb.push(" AND seq < ");
                qb.push_bind(n);
                qb.push(" ORDER BY seq DESC");
            }
            Some(SeqCursor::After(n)) => {
                qb.push(" AND seq > ");
                qb.push_bind(n);
                qb.push(" ORDER BY seq ASC");
            }
            None => {
                qb.push(" ORDER BY seq DESC");
            }
        }
        qb.push(" LIMIT ");
        qb.push_bind(filters.limit);

        let rows = qb
            .build_query_as::<(
                i64,
                String,
                String,
                Option<DateTime<Utc>>,
                String,
                i64,
                Value,
                Option<Value>,
                Option<DateTime<Utc>>,
            )>()
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    seq,
                    claimed_handle,
                    event_type,
                    event_timestamp,
                    log_source,
                    source_offset,
                    payload,
                    resolved_location,
                    hidden_at,
                )| {
                    StoredQueryEvent {
                        seq,
                        claimed_handle,
                        event_type,
                        event_timestamp,
                        log_source,
                        source_offset,
                        payload,
                        resolved_location,
                        hidden_at,
                    }
                },
            )
            .collect())
    }

    async fn timeline(
        &self,
        claimed_handle: &str,
        days: u32,
    ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
        // `make_interval(days => $2)` keeps the day count as a real bound
        // parameter rather than being baked into the literal string.
        let rows: Vec<(NaiveDate, i64)> = sqlx::query_as(
            "SELECT (date_trunc('day', event_timestamp) AT TIME ZONE 'UTC')::date AS day,
                    COUNT(*)::BIGINT
             FROM events
             WHERE claimed_handle = LOWER($1)
               AND event_timestamp IS NOT NULL
               AND event_timestamp >= NOW() - make_interval(days => $2)
             GROUP BY day
             ORDER BY day ASC",
        )
        .bind(claimed_handle)
        .bind(days as i32)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn summary_for_handle(
        &self,
        claimed_handle: &str,
    ) -> Result<(u64, Vec<(String, u64)>), RepoError> {
        // Hard-cut to the stat_event_counts rollup (maintained in the ingest
        // transaction): a PK-prefix point lookup instead of two full scans of
        // the handle's entire event history.
        let by_type: Vec<(String, i64)> = sqlx::query_as(
            "SELECT event_type, event_count
             FROM stat_event_counts
             WHERE claimed_handle = LOWER($1)
             ORDER BY event_count DESC",
        )
        .bind(claimed_handle)
        .fetch_all(&self.pool)
        .await?;

        if !by_type.is_empty() {
            let total: i64 = by_type.iter().map(|(_, c)| *c).sum();
            return Ok((
                total.max(0) as u64,
                by_type
                    .into_iter()
                    .map(|(t, c)| (t, c.max(0) as u64))
                    .collect(),
            ));
        }

        // Live fallback: no rollup row for this handle (cold user, or events
        // predating the rollup). Compute from source; the rollup self-heals on
        // the next ingest for this handle.
        let by_type: Vec<(String, i64)> = sqlx::query_as(
            "SELECT event_type, COUNT(*)::BIGINT
             FROM events
             WHERE claimed_handle = LOWER($1)
             GROUP BY event_type
             ORDER BY 2 DESC",
        )
        .bind(claimed_handle)
        .fetch_all(&self.pool)
        .await?;
        let total: i64 = by_type.iter().map(|(_, c)| *c).sum();
        Ok((
            total.max(0) as u64,
            by_type
                .into_iter()
                .map(|(t, c)| (t, c.max(0) as u64))
                .collect(),
        ))
    }

    /// Shared-perspective variant: excludes rows the owner has hidden.
    /// Same shape as [`Self::timeline`] otherwise.
    async fn timeline_shared(
        &self,
        claimed_handle: &str,
        days: u32,
    ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
        let rows: Vec<(NaiveDate, i64)> = sqlx::query_as(
            "SELECT (date_trunc('day', event_timestamp) AT TIME ZONE 'UTC')::date AS day,
                    COUNT(*)::BIGINT
             FROM events
             WHERE claimed_handle = LOWER($1)
               AND event_timestamp IS NOT NULL
               AND event_timestamp >= NOW() - make_interval(days => $2)
               AND hidden_at IS NULL
             GROUP BY day
             ORDER BY day ASC",
        )
        .bind(claimed_handle)
        .bind(days as i32)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Scope-aware shared timeline. Adds two optional `event_type`
    /// predicates to the `timeline_shared` query:
    ///
    ///   - `allow_types: Some(&[...])` -> `event_type = ANY($N)`
    ///   - `deny_types:  Some(&[...])` -> `event_type <> ALL($N)`
    ///
    /// Allowlist wins by precedence: when both are set, the allowlist
    /// is applied first (an "absent from allow" type can never reach
    /// the deny check), matching the in-memory + summary helper
    /// semantics. Empty allowlists therefore return zero buckets.
    /// Bound as Postgres `text[]` so the planner uses the existing
    /// `events_event_ts_idx` index the same way `latest_location`
    /// does for its `event_type = ANY($2)` clause.
    async fn timeline_shared_filtered(
        &self,
        claimed_handle: &str,
        days: u32,
        allow_types: Option<&[String]>,
        deny_types: Option<&[String]>,
    ) -> Result<Vec<(NaiveDate, i64)>, RepoError> {
        // Fast path: no per-type clamp -> same SQL as `timeline_shared`,
        // so reuse it instead of duplicating the bind sites.
        if allow_types.is_none() && deny_types.is_none() {
            return self.timeline_shared(claimed_handle, days).await;
        }
        // Build the WHERE clause + bind list dynamically so a missing
        // allow/deny doesn't burn a bind slot. `$1` = handle, `$2` =
        // days; the optional arrays slot in as `$3`/`$4` only when set
        // and the placeholder index is computed accordingly.
        let mut sql = String::from(
            "SELECT (date_trunc('day', event_timestamp) AT TIME ZONE 'UTC')::date AS day,
                    COUNT(*)::BIGINT
             FROM events
             WHERE claimed_handle = LOWER($1)
               AND event_timestamp IS NOT NULL
               AND event_timestamp >= NOW() - make_interval(days => $2)
               AND hidden_at IS NULL",
        );
        let mut next_param: usize = 3;
        let allow_owned = allow_types.map(<[String]>::to_vec);
        let deny_owned = deny_types.map(<[String]>::to_vec);
        if allow_owned.is_some() {
            sql.push_str(&format!(
                "\n               AND event_type = ANY(${next_param}::text[])"
            ));
            next_param += 1;
        }
        if deny_owned.is_some() {
            sql.push_str(&format!(
                "\n               AND event_type <> ALL(${next_param}::text[])"
            ));
            // next_param += 1; // unused — kept for symmetry / future binds
        }
        sql.push_str("\n             GROUP BY day\n             ORDER BY day ASC");

        let mut q = sqlx::query_as::<_, (NaiveDate, i64)>(&sql)
            .bind(claimed_handle)
            .bind(days as i32);
        if let Some(allow) = allow_owned.as_ref() {
            q = q.bind(allow);
        }
        if let Some(deny) = deny_owned.as_ref() {
            q = q.bind(deny);
        }
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    /// Shared-perspective variant: excludes rows the owner has hidden
    /// from both the total and the by-type breakdown. Friend + public
    /// summary endpoints call this; the owner's `/v1/me/summary`
    /// keeps the un-filtered count.
    async fn summary_for_handle_shared(
        &self,
        claimed_handle: &str,
    ) -> Result<(u64, Vec<(String, u64)>), RepoError> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events
             WHERE claimed_handle = LOWER($1) AND hidden_at IS NULL",
        )
        .bind(claimed_handle)
        .fetch_one(&self.pool)
        .await?;

        let by_type: Vec<(String, i64)> = sqlx::query_as(
            "SELECT event_type, COUNT(*)::BIGINT
             FROM events
             WHERE claimed_handle = LOWER($1) AND hidden_at IS NULL
             GROUP BY event_type
             ORDER BY 2 DESC",
        )
        .bind(claimed_handle)
        .fetch_all(&self.pool)
        .await?;

        Ok((
            total.max(0) as u64,
            by_type
                .into_iter()
                .map(|(t, c)| (t, c.max(0) as u64))
                .collect(),
        ))
    }

    /// Toggle the `hidden_at` flag for one event. `hide=true` sets
    /// it to `NOW()`; `hide=false` nulls it. The `claimed_handle`
    /// filter is the ownership check — an event belonging to a
    /// different user is invisible and the UPDATE matches zero
    /// rows. Returns `true` when a row actually changed (not "row
    /// exists but state was already as requested").
    async fn set_event_hidden(
        &self,
        claimed_handle: &str,
        seq: i64,
        hide: bool,
    ) -> Result<bool, RepoError> {
        // Two paths keep the SQL simple. The `IS DISTINCT FROM NULL`
        // / `IS DISTINCT FROM hidden_at` predicate suppresses no-op
        // updates so the boolean return is meaningful.
        let result = if hide {
            sqlx::query(
                "UPDATE events SET hidden_at = NOW()
                 WHERE claimed_handle = LOWER($1) AND seq = $2
                   AND hidden_at IS NULL",
            )
            .bind(claimed_handle)
            .bind(seq)
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query(
                "UPDATE events SET hidden_at = NULL
                 WHERE claimed_handle = LOWER($1) AND seq = $2
                   AND hidden_at IS NOT NULL",
            )
            .bind(claimed_handle)
            .bind(seq)
            .execute(&self.pool)
            .await?
        };
        Ok(result.rows_affected() > 0)
    }

    async fn event_type_breakdown(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<EventTypeStats>, RepoError> {
        // Two queries lets us keep the SQL simple. The first scopes
        // counts to the optional `since` window; the second lifts the
        // last_seen for each (unscoped) event_type so the column is
        // meaningful even if the type had zero rows in the window.
        let rows: Vec<(String, i64, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT event_type,
                    COUNT(*)::BIGINT AS count,
                    MAX(event_timestamp) AS last_seen
             FROM events
             WHERE claimed_handle = LOWER($1)
               AND ($2::TIMESTAMPTZ IS NULL OR event_timestamp >= $2)
             GROUP BY event_type
             ORDER BY count DESC, event_type ASC",
        )
        .bind(claimed_handle)
        .bind(since)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(event_type, count, last_seen)| EventTypeStats {
                event_type,
                count,
                last_seen,
            })
            .collect())
    }

    async fn sessions_for_handle(
        &self,
        claimed_handle: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InferredSession>, RepoError> {
        // Hard-cut to the session_summary rollup (Task A3): freshen iff
        // dirty/missing, then a point read on the
        // `session_summary_handle_start_idx` index instead of a full
        // events window-function scan.
        self.ensure_session_stats_fresh(claimed_handle).await?;
        // started_at/ended_at are nullable columns, but a session always
        // has a non-null MIN/MAX (the rebuild filters
        // `event_timestamp IS NOT NULL`); the `started_at IS NOT NULL`
        // guard keeps the DateTime<Utc> decode total.
        let rows: Vec<(DateTime<Utc>, DateTime<Utc>, i64)> = sqlx::query_as(
            "SELECT started_at, ended_at, event_count
             FROM session_summary
             WHERE claimed_handle = LOWER($1)
               AND started_at IS NOT NULL AND ended_at IS NOT NULL
             ORDER BY started_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(claimed_handle)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(start_at, end_at, event_count)| InferredSession {
                start_at,
                end_at,
                event_count,
            })
            .collect())
    }

    async fn total_playtime_secs(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<i64, RepoError> {
        // Hard-cut to session_summary (Task A3). Mirrors the live query's
        // semantics exactly: sum of per-session (ended_at - started_at)
        // durations for sessions whose start_at >= since (NULL = all-time).
        self.ensure_session_stats_fresh(claimed_handle).await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(EXTRACT(EPOCH FROM (ended_at - started_at))::BIGINT), 0)::BIGINT
             FROM session_summary
             WHERE claimed_handle = LOWER($1)
               AND ($2::timestamptz IS NULL OR started_at >= $2)",
        )
        .bind(claimed_handle)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(total)
    }

    async fn count_sessions_since(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<i64, RepoError> {
        // Hard-cut to session_summary (Task A3). Mirrors the live query's
        // semantics exactly: count of sessions whose start_at >= since
        // (NULL = all-time).
        self.ensure_session_stats_fresh(claimed_handle).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM session_summary
             WHERE claimed_handle = LOWER($1)
               AND ($2::timestamptz IS NULL OR started_at >= $2)",
        )
        .bind(claimed_handle)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    async fn records_for_handle(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<RecordsAggregate, RepoError> {
        // Hard-cut to character_records for the lifetime (since=None) call
        // (Task A3): freshen iff dirty/missing, then a point read. The
        // windowed variant (since=Some) is NOT materialized (YAGNI — the
        // rollup is lifetime-only) and keeps the live two-scan path below,
        // which also serves as the fallback on a rollup miss (cold user
        // with a stat_rollup_state row but no character_records row yet,
        // which should not happen post-rebuild but is handled defensively).
        if since.is_none() {
            self.ensure_session_stats_fresh(claimed_handle).await?;
            // busiest_session_events is NOT NULL (0059); the others are
            // nullable -> COALESCE-equivalent `.unwrap_or(0)` on read to
            // match the live path's semantics.
            let row: Option<(Option<i64>, Option<i64>, Option<i64>, i64)> = sqlx::query_as(
                "SELECT longest_session_secs, deadliest_session_deaths,
                        longest_survival_gap_secs, busiest_session_events
                 FROM character_records WHERE claimed_handle = LOWER($1)",
            )
            .bind(claimed_handle)
            .fetch_optional(&self.pool)
            .await?;
            if let Some((longest, deadliest, streak, busiest)) = row {
                return Ok(RecordsAggregate {
                    longest_session_secs: longest.unwrap_or(0),
                    busiest_session_events: busiest,
                    longest_survival_streak_secs: streak.unwrap_or(0),
                    deadliest_session_deaths: deadliest.unwrap_or(0),
                });
            }
        }
        // windowed (since = Some) OR rollup-miss: fall through to the
        // original live two-scan computation, preserved verbatim below.
        //
        // Same gap-idle sessionization as the other session queries,
        // extended to carry `event_type` so the longest/busiest session
        // and the per-session death count all fall out of one pass.
        //
        // `since` (when set) adds an `event_timestamp >= $N` clamp INSIDE
        // each scan's WHERE so the window is applied before clustering.
        // The clause is only appended (and the timestamp only bound) when
        // `since` is `Some`; a bound NULL would silently drop every row.
        let gap_minutes = SESSION_IDLE_GAP_MINUTES;
        let since_filter = if since.is_some() {
            " AND event_timestamp >= $3"
        } else {
            ""
        };
        let records_sql = format!(
            "WITH gaps AS (
                    SELECT event_timestamp, event_type,
                           LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS prev_ts
                    FROM events
                    WHERE claimed_handle = LOWER($1)
                      AND event_timestamp IS NOT NULL
                      AND event_type NOT IN ('launcher_activity', 'game_crash'){since_filter}
                ), labeled AS (
                    SELECT event_timestamp, event_type,
                           SUM(CASE
                                 WHEN prev_ts IS NULL
                                   OR event_timestamp - prev_ts > make_interval(mins => $2)
                                 THEN 1 ELSE 0
                               END) OVER (ORDER BY event_timestamp ASC) AS session_id
                    FROM gaps
                ), per_session AS (
                    SELECT
                        EXTRACT(EPOCH FROM (MAX(event_timestamp) - MIN(event_timestamp)))::BIGINT
                            AS dur_secs,
                        COUNT(*) AS event_count,
                        COUNT(*) FILTER (WHERE event_type = 'player_death') AS death_count
                    FROM labeled
                    GROUP BY session_id
                )
                SELECT MAX(dur_secs), MAX(event_count), MAX(death_count) FROM per_session"
        );
        let mut records_query = sqlx::query_as(&records_sql)
            .bind(claimed_handle)
            .bind(gap_minutes as i32);
        if let Some(ts) = since {
            records_query = records_query.bind(ts);
        }
        let (dur_secs, event_count, death_count): (Option<i64>, Option<i64>, Option<i64>) =
            records_query.fetch_one(&self.pool).await?;

        // Longest stretch alive = the largest gap between two consecutive
        // player_death events, over the whole timeline (independent of
        // session clustering).
        let streak_since_filter = if since.is_some() {
            " AND event_timestamp >= $2"
        } else {
            ""
        };
        let streak_sql = format!(
            "WITH death_gaps AS (
                SELECT event_timestamp
                       - LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS gap
                FROM events
                WHERE claimed_handle = LOWER($1)
                  AND event_type = 'player_death'
                  AND event_timestamp IS NOT NULL{streak_since_filter}
            )
            SELECT MAX(EXTRACT(EPOCH FROM gap))::BIGINT FROM death_gaps"
        );
        let mut streak_query = sqlx::query_scalar(&streak_sql).bind(claimed_handle);
        if let Some(ts) = since {
            streak_query = streak_query.bind(ts);
        }
        let streak_secs: Option<i64> = streak_query.fetch_one(&self.pool).await?;

        Ok(RecordsAggregate {
            longest_session_secs: dur_secs.unwrap_or(0),
            busiest_session_events: event_count.unwrap_or(0),
            longest_survival_streak_secs: streak_secs.unwrap_or(0),
            deadliest_session_deaths: death_count.unwrap_or(0),
        })
    }

    async fn lives_for_handle(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<LivesData, RepoError> {
        // Full ordered event stream for the handle -- deliberately NOT
        // filtered by event_type (unlike the session-list queries):
        // `derive_lives` needs `game_crash` rows to close a life as
        // `Crash` instead of leaving it `StillAlive`. Same ORDER BY as
        // `event_timeline`'s ordered-events query so both derivations
        // see events in the same sequence. `since` (when set) clamps the
        // stream to the window via `event_timestamp >= $2`; the clause
        // and its bind are omitted for the all-time (`None`) case.
        let since_filter = if since.is_some() {
            " AND event_timestamp >= $2"
        } else {
            ""
        };
        let lives_sql = format!(
            "SELECT
                idempotency_key,
                event_timestamp,
                payload
            FROM events
            WHERE claimed_handle = lower($1)
              AND event_timestamp IS NOT NULL{since_filter}
            ORDER BY event_timestamp ASC, idempotency_key ASC"
        );
        let mut lives_query = sqlx::query_as(&lives_sql).bind(claimed_handle);
        if let Some(ts) = since {
            lives_query = lives_query.bind(ts);
        }
        let rows: Vec<(String, Option<DateTime<Utc>>, Value)> =
            lives_query.fetch_all(&self.pool).await?;

        let envs: Vec<EventEnvelope> = rows
            .iter()
            .map(|(idempotency_key, _ts, payload)| EventEnvelope {
                idempotency_key: idempotency_key.clone(),
                raw_line: String::new(),
                event: serde_json::from_value(payload.clone()).ok(),
                source: LogSource::Live,
                source_offset: 0,
                metadata: None,
                resolved_location: None,
            })
            .collect();
        let summary = derive_lives(&envs, &LifeConfig::default());

        // Canonical 30-min idle-gap session count. Deliberately NOT
        // `event_timeline::derive_sessions(..).len()` — that helper is
        // written for the paginated sessions LIST and internally
        // truncates to `SESSIONS_LIST_LIMIT` (50), which would silently
        // cap this endpoint's `sessions` (and inflate
        // `deaths_per_session`) for any handle with more than 50
        // gap-idle sessions. `count_sessions_since(.., since)` scopes the
        // session count to the same window as the FSM stream — `None`
        // yields the unbounded full-history count.
        let sessions = self.count_sessions_since(claimed_handle, since).await? as u32;

        Ok(LivesData { summary, sessions })
    }

    async fn ingest_history_for_handle(
        &self,
        actor_handle: &str,
        device_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<IngestBatchRow>, RepoError> {
        // We pull each known field with a JSON path operator; `->>` on
        // a missing key yields NULL which we coerce to 0/None below.
        // Defensive — a future schema bump that drops fields shouldn't
        // crash the read endpoint.
        //
        // When `device_id` is set we add a `payload->>'device_id' =
        // $N` clamp. Pre-0026 rows have no device_id key in their
        // payload (`->>` yields NULL) so they are correctly excluded
        // from any device-scoped filter. The partial functional index
        // from migration 0026 keeps this cheap.
        let device_filter_sql = if device_id.is_some() {
            " AND payload->>'device_id' = $4"
        } else {
            ""
        };
        let sql = format!(
            "SELECT seq,
                    occurred_at,
                    payload->>'batch_id'   AS batch_id,
                    payload->>'game_build' AS game_build,
                    payload->>'device_id'  AS device_id,
                    NULLIF(payload->>'total','')::BIGINT     AS total,
                    NULLIF(payload->>'accepted','')::BIGINT  AS accepted,
                    NULLIF(payload->>'duplicate','')::BIGINT AS duplicate,
                    NULLIF(payload->>'rejected','')::BIGINT  AS rejected
             FROM audit_log
             WHERE action = 'ingest.batch_processed'
               AND LOWER(actor_handle) = LOWER($1){device_filter_sql}
             ORDER BY seq DESC
             LIMIT $2 OFFSET $3"
        );
        let mut q = sqlx::query_as::<
            _,
            (
                i64,
                DateTime<Utc>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
            ),
        >(&sql)
        .bind(actor_handle)
        .bind(limit)
        .bind(offset);
        if let Some(d) = device_id {
            q = q.bind(d.to_string());
        }
        let rows = q.fetch_all(&self.pool).await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    seq,
                    occurred_at,
                    batch_id,
                    game_build,
                    device_id,
                    total,
                    accepted,
                    duplicate,
                    rejected,
                )| {
                    IngestBatchRow {
                        seq,
                        occurred_at,
                        batch_id: batch_id.unwrap_or_default(),
                        game_build,
                        device_id: device_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
                        total: total.unwrap_or(0),
                        accepted: accepted.unwrap_or(0),
                        duplicate: duplicate.unwrap_or(0),
                        rejected: rejected.unwrap_or(0),
                    }
                },
            )
            .collect())
    }

    async fn latest_location(
        &self,
        claimed_handle: &str,
        event_types: &[&str],
    ) -> Result<LatestLocation, RepoError> {
        // Two cheap reads. Both walk the existing
        // `events_event_ts_idx` index (claimed_handle + event_timestamp
        // DESC), filtered to the small set of event types we care about,
        // so the planner can stop after the first matching row.
        //
        // We pass the type list as a Postgres array — `event_type =
        // ANY($2)` is the canonical shape for "match against this small
        // set without bloating the query plan with an OR chain".
        let event_types_owned: Vec<String> = event_types.iter().map(|s| s.to_string()).collect();

        let location_row: Option<(String, DateTime<Utc>, Value)> = sqlx::query_as(
            "SELECT event_type, event_timestamp, payload
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = ANY($2)
                AND event_timestamp IS NOT NULL
              ORDER BY event_timestamp DESC
              LIMIT 1",
        )
        .bind(claimed_handle)
        .bind(&event_types_owned)
        .fetch_optional(&self.pool)
        .await?;

        let location_event = location_row.map(|(event_type, ts, payload)| LatestLocationEvent {
            event_type,
            event_timestamp: ts,
            payload,
        });

        // Independent fetch for the most recent shard hint. We don't
        // care about timestamp ordering relative to the location_event
        // — the resolver attaches the shard as contextual extra info
        // alongside whatever planet/city the location_event resolved.
        let shard_hint: Option<String> = sqlx::query_scalar(
            "SELECT payload->>'shard'
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = 'join_pu'
                AND payload ? 'shard'
              ORDER BY event_timestamp DESC NULLS LAST
              LIMIT 1",
        )
        .bind(claimed_handle)
        .fetch_optional(&self.pool)
        .await?
        .flatten();

        Ok(LatestLocation {
            location_event,
            shard_hint,
        })
    }

    async fn recent_location_events(
        &self,
        claimed_handle: &str,
        event_types: &[&str],
        limit: i64,
    ) -> Result<Vec<LatestLocationEvent>, RepoError> {
        // Same index walk as `latest_location` (events_event_ts_idx),
        // but returns up to `limit` rows newest-first instead of just
        // the head. The handler uses the result to compute the
        // entered_at anchor — see `query::location_current`.
        let event_types_owned: Vec<String> = event_types.iter().map(|s| s.to_string()).collect();
        let rows: Vec<(String, DateTime<Utc>, Value)> = sqlx::query_as(
            "SELECT event_type, event_timestamp, payload
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = ANY($2)
                AND event_timestamp IS NOT NULL
              ORDER BY event_timestamp DESC
              LIMIT $3",
        )
        .bind(claimed_handle)
        .bind(&event_types_owned)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(event_type, event_timestamp, payload)| LatestLocationEvent {
                    event_type,
                    event_timestamp,
                    payload,
                },
            )
            .collect())
    }

    async fn location_event_stream(
        &self,
        claimed_handle: &str,
        event_types: &[&str],
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<LatestLocationEvent>, RepoError> {
        let event_types_owned: Vec<String> = event_types.iter().map(|s| s.to_string()).collect();
        // Fetch only the most-recent `limit` raw location events in the
        // window (ORDER BY DESC + LIMIT), then reverse to oldest-first
        // so the dwell-collapse walker sees ascending time. This bounds
        // cost at O(limit) instead of scanning every location event in
        // the window — which is what lets the endpoint caps extend to a
        // full year without an unbounded query.
        let rows: Vec<(String, DateTime<Utc>, Value)> = sqlx::query_as(
            "SELECT event_type, event_timestamp, payload
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = ANY($2)
                AND event_timestamp IS NOT NULL
                AND event_timestamp >= $3
              ORDER BY event_timestamp DESC
              LIMIT $4",
        )
        .bind(claimed_handle)
        .bind(&event_types_owned)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out: Vec<LatestLocationEvent> = rows
            .into_iter()
            .map(
                |(event_type, event_timestamp, payload)| LatestLocationEvent {
                    event_type,
                    event_timestamp,
                    payload,
                },
            )
            .collect();
        // Undo the DESC fetch so the walker gets oldest-first (ASC).
        out.reverse();
        Ok(out)
    }

    // Mirrors the trait's allow — same eight parameters.
    #[allow(clippy::too_many_arguments)]
    async fn payload_field_breakdown(
        &self,
        claimed_handle: &str,
        event_type: &str,
        payload_field: &str,
        payload_filter: Option<PayloadFilter<'_>>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<PayloadFieldBucket>, RepoError> {
        // `payload->>$N` keeps the field name a parameter rather than
        // an interpolation, so even a hostile caller can't escape into
        // a different column. Filter is similarly parameterised — the
        // `$6 IS NULL OR ...` shape collapses to a no-op when the
        // caller passes None.
        //
        // `>= $4` / `< $8`: inclusive lower, EXCLUSIVE upper (see the
        // trait's "Window convention"). Adjacent windows must not both
        // claim an event on their shared edge.
        let (filter_field, filter_value) = match payload_filter {
            Some(f) => (Some(f.field.to_string()), Some(f.equals.to_string())),
            None => (None, None),
        };
        let rows: Vec<(Option<String>, i64)> = sqlx::query_as(
            "SELECT payload->>$2 AS value, COUNT(*)::BIGINT AS count
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = $3
                AND ($4::timestamptz IS NULL OR event_timestamp >= $4)
                AND ($8::timestamptz IS NULL OR event_timestamp < $8)
                AND payload ? $2
                AND ($5::text IS NULL OR payload->>$5 = $6)
              GROUP BY payload->>$2
              ORDER BY count DESC, value ASC
              LIMIT $7",
        )
        .bind(claimed_handle)
        .bind(payload_field)
        .bind(event_type)
        .bind(since)
        .bind(filter_field)
        .bind(filter_value)
        .bind(limit)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(value, count)| value.map(|v| PayloadFieldBucket { value: v, count }))
            .collect())
    }

    async fn docking_occurrences(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<DockingOccurrences, RepoError> {
        // The tray's retroactive burst pass uses 120 seconds as its hard
        // ceiling between adjacent members. Mirror that ceiling here for
        // legacy raw rows by comparing consecutive raw stows. Server ingest
        // sequence is not Game.log source order, so unrelated telemetry must
        // not split one temporal stow episode.
        //
        // A reparse can insert a burst summary after its raw member rows have
        // already synced. Deleting those rows locally cannot delete their
        // server copies. Reconcile a summary against a raw episode with the
        // same anchor timestamp and keep the raw landing area, rather than
        // counting both representations.
        //
        // Window predicates are applied AFTER `is_start` is derived from
        // the full ordered history. A raw run crossing a window boundary is
        // therefore counted once, in the window containing its first row.
        let rows: Vec<(Option<String>, i64)> = sqlx::query_as(
            "WITH relevant AS (
                 SELECT event_type, event_timestamp, seq, payload
                   FROM events
                  WHERE claimed_handle = LOWER($1)
                    AND (event_type = 'vehicle_stowed'
                         OR (event_type = 'burst_summary'
                             AND payload->>'rule_id' = 'vehicle_stowed_burst'))
             ),
             raw_ordered AS (
                 SELECT event_timestamp,
                        seq,
                        payload->>'landing_area' AS landing_area,
                        LAG(event_timestamp) OVER (
                            ORDER BY event_timestamp ASC NULLS LAST, seq ASC
                        ) AS previous_raw_timestamp
                   FROM relevant
                  WHERE event_type = 'vehicle_stowed'
             ),
             raw_starts AS (
                 SELECT event_timestamp, landing_area
                   FROM raw_ordered
                  WHERE previous_raw_timestamp IS NULL
                     OR event_timestamp IS NULL
                     OR event_timestamp - previous_raw_timestamp > INTERVAL '120 seconds'
             ),
             raw_start_counts AS (
                 SELECT event_timestamp, COUNT(*)::BIGINT AS count
                   FROM raw_starts
                  WHERE event_timestamp IS NOT NULL
                  GROUP BY event_timestamp
             ),
             ranked_bursts AS (
                 SELECT event_timestamp,
                        ROW_NUMBER() OVER (
                            PARTITION BY event_timestamp ORDER BY seq ASC
                        ) AS duplicate_ordinal
                   FROM relevant
                  WHERE event_type = 'burst_summary'
             ),
             occurrences AS (
                 SELECT event_timestamp, landing_area
                   FROM raw_starts
                 UNION ALL
                 SELECT burst.event_timestamp, NULL AS landing_area
                   FROM ranked_bursts burst
                   LEFT JOIN raw_start_counts raw
                     ON raw.event_timestamp = burst.event_timestamp
                  WHERE burst.event_timestamp IS NULL
                     OR burst.duplicate_ordinal > COALESCE(raw.count, 0)
             )
             SELECT landing_area, COUNT(*)::BIGINT AS count
               FROM occurrences
              WHERE ($2::timestamptz IS NULL OR event_timestamp >= $2)
                AND ($3::timestamptz IS NULL OR event_timestamp < $3)
              GROUP BY landing_area
              ORDER BY count DESC, landing_area ASC NULLS LAST",
        )
        .bind(claimed_handle)
        .bind(since)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;

        let mut result = DockingOccurrences::default();
        for (landing_area, count) in rows {
            match landing_area {
                Some(value) => result
                    .landing_areas
                    .push(PayloadFieldBucket { value, count }),
                None => result.unknown = count,
            }
        }
        Ok(result)
    }

    async fn objective_outcomes(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<ObjectiveOutcomes, RepoError> {
        // Fold to one row per objective first, then count the folded
        // rows. MAX(rank) picks the terminal state because the ranks are
        // monotonic in lifecycle order, so an objective that reported
        // in_progress and later completed resolves to completed.
        //
        // COALESCE keys legacy rows (no objective_id, predating capture)
        // by seq so they count as one objective each rather than
        // collapsing into a single NULL bucket.
        //
        // 'withdrawn' and 'unknown' both rank as unresolved. Current
        // collectors store WITHDRAWN as "withdrawn"; collectors older
        // than v1.8.149 stored it as the parser's "unknown" catch-all and
        // those payloads are never rewritten, so both spellings live in
        // this table side by side. 'unknown' additionally covers any
        // state CIG ships that the parser has no variant for yet. We rank
        // the STORED string, never the in-game name — a state missing
        // from this CASE scores 0 and vanishes from every bucket.
        let rows: Vec<(i32, i64)> = sqlx::query_as(
            "WITH per_objective AS (
                 SELECT COALESCE(payload->>'objective_id', 'seq:' || seq::text) AS key,
                        MAX(CASE payload->>'state'
                              WHEN 'completed'   THEN 4
                              WHEN 'failed'      THEN 3
                              WHEN 'withdrawn'   THEN 2
                              WHEN 'unknown'     THEN 2
                              WHEN 'in_progress' THEN 1
                              ELSE 0
                            END) AS rank
                   FROM events
                  WHERE claimed_handle = LOWER($1)
                    AND event_type = 'mission_objective'
                    AND ($2::timestamptz IS NULL OR event_timestamp >= $2)
                    AND ($3::timestamptz IS NULL OR event_timestamp < $3)
                  GROUP BY 1
             )
             SELECT rank, COUNT(*)::BIGINT AS count
               FROM per_objective
              GROUP BY rank",
        )
        .bind(claimed_handle)
        .bind(since)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;

        let mut out = ObjectiveOutcomes::default();
        for (rank, count) in rows {
            match rank {
                4 => out.completed = count,
                3 => out.failed = count,
                2 => out.unresolved = count,
                1 => out.no_outcome = count,
                _ => {}
            }
        }
        Ok(out)
    }

    async fn contract_runs(
        &self,
        claimed_handle: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<ContractRunRow>, RepoError> {
        self.ensure_contract_runs_fresh(claimed_handle).await?;

        // `since` filters the materialised OUTPUT by `accepted_at` (see
        // the trait doc) — NOT re-derived from a truncated event window.
        // A NULL `accepted_at` (defensive-only: an unparseable fold
        // timestamp) never satisfies `>=` in Postgres, so it's excluded
        // once `since` is set, matching `MemoryQuery`'s equivalent filter.
        let since_filter = if since.is_some() {
            " AND accepted_at >= $2"
        } else {
            ""
        };
        let sql = format!(
            "SELECT mission_id, name, state, closed_by, step_count, steps_complete,
                    steps_remaining, partial_history, connected_server,
                    accepted_at, closed_at, last_event_at, steps
             FROM contract_runs
             WHERE claimed_handle = LOWER($1){since_filter}
             ORDER BY accepted_at DESC, mission_id ASC"
        );
        let mut q = sqlx::query_as::<_, ContractRunSqlRow>(&sql).bind(claimed_handle);
        if let Some(ts) = since {
            q = q.bind(ts);
        }
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(ContractRunRow::from).collect())
    }

    async fn payload_numeric_sum(
        &self,
        claimed_handle: &str,
        event_type: &str,
        numeric_field: &str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<i64, RepoError> {
        // Cast the JSON text field to numeric and sum. The `~ '^[0-9]+$'`
        // guard keeps a stray non-numeric value from erroring the whole
        // aggregate (price is whole aUEC, serialised as a JSON number →
        // `->>'price'` yields its digit string). COALESCE → 0 for no rows.
        //
        // `>= $4` / `< $5`: inclusive lower, EXCLUSIVE upper — see the
        // trait's "Window convention".
        let sum: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM((payload->>$2)::numeric), 0)::BIGINT
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = $3
                AND ($4::timestamptz IS NULL OR event_timestamp >= $4)
                AND ($5::timestamptz IS NULL OR event_timestamp < $5)
                AND payload->>$2 ~ '^[0-9]+$'",
        )
        .bind(claimed_handle)
        .bind(numeric_field)
        .bind(event_type)
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await?;
        Ok(sum)
    }

    async fn count_event_type(
        &self,
        claimed_handle: &str,
        event_type: &str,
        payload_filter: Option<PayloadFilter<'_>>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<u64, RepoError> {
        // `>= $3` / `< $6`: inclusive lower, EXCLUSIVE upper — see the
        // trait's "Window convention".
        let (filter_field, filter_value) = match payload_filter {
            Some(f) => (Some(f.field.to_string()), Some(f.equals.to_string())),
            None => (None, None),
        };
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type = $2
                AND ($3::timestamptz IS NULL OR event_timestamp >= $3)
                AND ($6::timestamptz IS NULL OR event_timestamp < $6)
                AND ($4::text IS NULL OR payload->>$4 = $5)",
        )
        .bind(claimed_handle)
        .bind(event_type)
        .bind(since)
        .bind(filter_field)
        .bind(filter_value)
        .bind(until)
        .fetch_one(&self.pool)
        .await?;
        Ok(count.max(0) as u64)
    }

    async fn has_events_in_window(
        &self,
        claimed_handle: &str,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        // EXISTS stops at the first matching row, so this is an index
        // probe rather than a count — it is issued once per windowed stats
        // request purely to tell "no activity" apart from "not a user
        // yet", and must not cost a scan.
        //
        // No `event_type` predicate on purpose (see the trait doc): ANY
        // event proves the handle was live in that window.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                   FROM events
                  WHERE claimed_handle = LOWER($1)
                    AND event_timestamp >= $2
                    AND event_timestamp < $3
             )",
        )
        .bind(claimed_handle)
        .bind(since)
        .bind(until)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    async fn combat_counts(
        &self,
        claimed_handle: &str,
        subject: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<(u64, u64, u64), RepoError> {
        // One scan of the handle's actor_death + player_death rows, splitting
        // kills / actor-deaths / player-deaths with FILTER — replaces three
        // separate count_event_type index scans with identical results.
        let (kills, deaths_actor, deaths_player): (i64, i64, i64) = sqlx::query_as(
            "SELECT
                 COUNT(*) FILTER (WHERE event_type = 'actor_death' AND payload->>'killer' = $2)::BIGINT,
                 COUNT(*) FILTER (WHERE event_type = 'actor_death' AND payload->>'victim' = $2)::BIGINT,
                 COUNT(*) FILTER (WHERE event_type = 'player_death')::BIGINT
               FROM events
              WHERE claimed_handle = LOWER($1)
                AND event_type IN ('actor_death', 'player_death')
                AND ($3::timestamptz IS NULL OR event_timestamp >= $3)",
        )
        .bind(claimed_handle)
        .bind(subject)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok((
            kills.max(0) as u64,
            deaths_actor.max(0) as u64,
            deaths_player.max(0) as u64,
        ))
    }
}

#[async_trait]
impl EventStore for PostgresStore {
    async fn insert(&self, event: StoredEvent) -> Result<InsertOutcome, RepoError> {
        // ON CONFLICT lets Postgres tell us whether the row was new.
        // RETURNING (xmax = 0) is a Postgres-ism: xmax is 0 on a fresh
        // insert and non-zero when an existing row was updated; we
        // don't update on conflict so it stays 0 only for inserts.
        // Serialise the EventMetadata once so the bind is a plain
        // serde_json::Value — sqlx maps Value → JSONB without an
        // explicit cast. None flattens to a NULL bind so Postgres
        // stores SQL NULL (distinguishable from JSON null), which is
        // what read sites assume for "no metadata".
        let metadata_json: Option<Value> = event
            .metadata
            .as_ref()
            .map(|m| serde_json::to_value(m).expect("EventMetadata serialises to JSON"));

        // Same NULL-vs-JSON-null discipline as metadata: `None` →
        // SQL NULL (migration 0041's "no resolution" sentinel), `Some`
        // → the serialised ResolvedLocation the tray stamped.
        let resolved_location_json: Option<Value> = event
            .resolved_location
            .as_ref()
            .map(|r| serde_json::to_value(r).expect("ResolvedLocation serialises to JSON"));

        let inserted: bool = sqlx::query_scalar(
            r#"
            INSERT INTO events (
                id, idempotency_key, claimed_handle, event_type,
                event_timestamp, log_source, source_offset, raw_line, payload, metadata,
                resolved_location
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (claimed_handle, idempotency_key) DO NOTHING
            RETURNING TRUE
            "#,
        )
        .bind(event.id)
        .bind(&event.idempotency_key)
        .bind(&event.claimed_handle)
        .bind(&event.event_type)
        .bind(event.event_timestamp)
        .bind(log_source_to_str(event.log_source))
        .bind(event.source_offset)
        .bind(&event.raw_line)
        .bind(&event.payload)
        .bind(metadata_json)
        .bind(resolved_location_json)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false);

        Ok(if inserted {
            InsertOutcome::Inserted
        } else {
            InsertOutcome::Duplicate
        })
    }

    async fn insert_batch(
        &self,
        events: Vec<StoredEvent>,
    ) -> Result<Vec<InsertOutcome>, RepoError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        // Decompose the batch into column arrays for a single UNNEST insert.
        let n = events.len();
        let mut ids = Vec::with_capacity(n);
        let mut idem = Vec::with_capacity(n);
        let mut handles = Vec::with_capacity(n);
        let mut types = Vec::with_capacity(n);
        let mut timestamps: Vec<Option<DateTime<Utc>>> = Vec::with_capacity(n);
        let mut sources = Vec::with_capacity(n);
        let mut offsets = Vec::with_capacity(n);
        let mut raw_lines = Vec::with_capacity(n);
        let mut payloads = Vec::with_capacity(n);
        let mut metadatas: Vec<Option<Value>> = Vec::with_capacity(n);
        let mut resolved: Vec<Option<Value>> = Vec::with_capacity(n);
        for e in &events {
            ids.push(e.id);
            idem.push(e.idempotency_key.clone());
            handles.push(e.claimed_handle.clone());
            types.push(e.event_type.clone());
            timestamps.push(e.event_timestamp);
            sources.push(log_source_to_str(e.log_source).to_string());
            offsets.push(e.source_offset);
            raw_lines.push(e.raw_line.clone());
            payloads.push(e.payload.clone());
            metadatas.push(
                e.metadata
                    .as_ref()
                    .map(|m| serde_json::to_value(m).expect("EventMetadata serialises to JSON")),
            );
            resolved.push(
                e.resolved_location
                    .as_ref()
                    .map(|r| serde_json::to_value(r).expect("ResolvedLocation serialises to JSON")),
            );
        }

        // One atomic statement does all three writes:
        //   ins   — set-based UNNEST insert; ON CONFLICT DO NOTHING keeps
        //           idempotency; RETURNING classifies Inserted vs Duplicate.
        //   roll  — maintain stat_event_counts from JUST the inserted rows,
        //           aggregation in SQL (GROUP BY) not Rust.
        //   dirty — stamp stat_rollup_state so the session/records rollups
        //           AND the contract_runs rollup know each touched handle
        //           needs a recompute on next read.
        // A single wCTE is atomic on its own — no explicit transaction needed,
        // and the rollup can never drift from the events it summarises.
        let inserted_keys: Vec<String> = sqlx::query_scalar(
            r#"
            WITH ins AS (
                INSERT INTO events (
                    id, idempotency_key, claimed_handle, event_type, event_timestamp,
                    log_source, source_offset, raw_line, payload, metadata, resolved_location
                )
                SELECT * FROM UNNEST(
                    $1::uuid[], $2::text[], $3::text[], $4::text[], $5::timestamptz[],
                    $6::text[], $7::bigint[], $8::text[], $9::jsonb[], $10::jsonb[], $11::jsonb[]
                )
                ON CONFLICT (claimed_handle, idempotency_key) DO NOTHING
                RETURNING idempotency_key, claimed_handle, event_type, event_timestamp
            ),
            roll AS (
                INSERT INTO stat_event_counts
                    (claimed_handle, event_type, event_count, first_seen_at, last_seen_at)
                SELECT claimed_handle, event_type, COUNT(*),
                       MIN(event_timestamp), MAX(event_timestamp)
                FROM ins
                GROUP BY claimed_handle, event_type
                ON CONFLICT (claimed_handle, event_type) DO UPDATE SET
                    event_count   = stat_event_counts.event_count + EXCLUDED.event_count,
                    first_seen_at = LEAST(stat_event_counts.first_seen_at, EXCLUDED.first_seen_at),
                    last_seen_at  = GREATEST(stat_event_counts.last_seen_at, EXCLUDED.last_seen_at),
                    updated_at    = now()
            ),
            dirty AS (
                INSERT INTO stat_rollup_state
                    (claimed_handle, sessions_dirty, contracts_dirty, counts_last_seq)
                SELECT DISTINCT claimed_handle, TRUE, TRUE, 0 FROM ins
                ON CONFLICT (claimed_handle) DO UPDATE SET
                    sessions_dirty = TRUE,
                    contracts_dirty = TRUE,
                    updated_at = now()
            )
            SELECT idempotency_key FROM ins
            "#,
        )
        .bind(&ids)
        .bind(&idem)
        .bind(&handles)
        .bind(&types)
        .bind(&timestamps)
        .bind(&sources)
        .bind(&offsets)
        .bind(&raw_lines)
        .bind(&payloads)
        .bind(&metadatas)
        .bind(&resolved)
        .fetch_all(&self.pool)
        .await?;
        let inserted_set: std::collections::HashSet<&str> =
            inserted_keys.iter().map(String::as_str).collect();

        Ok(events
            .iter()
            .map(|e| {
                if inserted_set.contains(e.idempotency_key.as_str()) {
                    InsertOutcome::Inserted
                } else {
                    InsertOutcome::Duplicate
                }
            })
            .collect())
    }

    async fn quarantine(&self, event: QuarantinedEvent) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO quarantined_events (
                id, idempotency_key, claimed_handle, reason, detail,
                log_source, source_offset, raw_line, payload
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (claimed_handle, idempotency_key) DO NOTHING
            "#,
        )
        .bind(event.id)
        .bind(&event.idempotency_key)
        .bind(&event.claimed_handle)
        .bind(&event.reason)
        .bind(&event.detail)
        .bind(log_source_to_str(event.log_source))
        .bind(event.source_offset)
        .bind(&event.raw_line)
        .bind(&event.payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn observe_batch_sequence(
        &self,
        device_id: &str,
        seq: i64,
    ) -> Result<Option<i64>, RepoError> {
        // Read the prior high-water mark and advance it in one statement.
        // The `prior` CTE snapshots the pre-upsert value; the upsert then
        // advances the mark monotonically via GREATEST, so an out-of-order
        // (lower) arrival still records its observation without rewinding
        // the mark. `fetch_optional` returns None on first-seen (the CTE
        // matched no existing row).
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            WITH prior AS (
                SELECT last_batch_sequence AS prev
                FROM device_batch_progress
                WHERE device_id = $1
            ), upsert AS (
                INSERT INTO device_batch_progress (device_id, last_batch_sequence)
                VALUES ($1, $2)
                ON CONFLICT (device_id) DO UPDATE SET
                    last_batch_sequence = GREATEST(
                        device_batch_progress.last_batch_sequence,
                        EXCLUDED.last_batch_sequence
                    ),
                    updated_at = NOW()
            )
            SELECT prev FROM prior
            "#,
        )
        .bind(device_id)
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(prev,)| prev))
    }
}

fn log_source_to_str(s: LogSource) -> &'static str {
    match s {
        LogSource::Live => "live",
        LogSource::Ptu => "ptu",
        LogSource::Eptu => "eptu",
        LogSource::Hotfix => "hotfix",
        LogSource::Tech => "tech",
        LogSource::Other => "other",
    }
}

// -- In-memory store (test-only) -------------------------------------

#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    rows: Mutex<Vec<StoredEvent>>,
    quarantined: Mutex<Vec<QuarantinedEvent>>,
    batch_progress: Mutex<std::collections::HashMap<String, i64>>,
}

#[cfg(test)]
impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<StoredEvent> {
        self.rows.lock().expect("memory store poisoned").clone()
    }

    pub fn quarantined_snapshot(&self) -> Vec<QuarantinedEvent> {
        self.quarantined
            .lock()
            .expect("memory store poisoned")
            .clone()
    }
}

#[cfg(test)]
#[async_trait]
impl EventStore for MemoryStore {
    async fn insert(&self, event: StoredEvent) -> Result<InsertOutcome, RepoError> {
        let mut rows = self.rows.lock().expect("memory store poisoned");
        let dup = rows.iter().any(|r| {
            r.claimed_handle == event.claimed_handle && r.idempotency_key == event.idempotency_key
        });
        if dup {
            return Ok(InsertOutcome::Duplicate);
        }
        rows.push(event);
        Ok(InsertOutcome::Inserted)
    }

    async fn quarantine(&self, event: QuarantinedEvent) -> Result<(), RepoError> {
        let mut q = self.quarantined.lock().expect("memory store poisoned");
        // Mirror the Postgres unique index: idempotent per (handle, key).
        let dup = q.iter().any(|r| {
            r.claimed_handle == event.claimed_handle && r.idempotency_key == event.idempotency_key
        });
        if !dup {
            q.push(event);
        }
        Ok(())
    }

    async fn observe_batch_sequence(
        &self,
        device_id: &str,
        seq: i64,
    ) -> Result<Option<i64>, RepoError> {
        let mut m = self.batch_progress.lock().expect("memory store poisoned");
        let prev = m.get(device_id).copied();
        // Monotonic, mirroring the Postgres GREATEST upsert.
        let next = prev.map_or(seq, |p| p.max(seq));
        m.insert(device_id.to_string(), next);
        Ok(prev)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryQuery;
    use super::*;
    use chrono::DateTime;
    use serde_json::json;
    use starstats_core::contract_life::StepState;

    #[tokio::test]
    async fn observe_batch_sequence_tracks_high_water_mark_and_returns_prior() {
        let store = MemoryStore::new();
        // First batch from a device: no prior mark.
        assert_eq!(
            store.observe_batch_sequence("dev-a", 1).await.unwrap(),
            None
        );
        // Next contiguous batch sees the prior (1).
        assert_eq!(
            store.observe_batch_sequence("dev-a", 2).await.unwrap(),
            Some(1)
        );
        // A forward jump returns the last-seen (2); the caller derives the gap.
        assert_eq!(
            store.observe_batch_sequence("dev-a", 5).await.unwrap(),
            Some(2)
        );
        // An out-of-order (lower) arrival returns the current mark (5) and
        // must NOT rewind it.
        assert_eq!(
            store.observe_batch_sequence("dev-a", 3).await.unwrap(),
            Some(5)
        );
        assert_eq!(
            store.observe_batch_sequence("dev-a", 6).await.unwrap(),
            Some(5),
            "the stale lower arrival must not have rewound the mark"
        );
        // A second device is tracked independently.
        assert_eq!(
            store.observe_batch_sequence("dev-b", 1).await.unwrap(),
            None
        );
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("valid RFC 3339 timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn parse_event_timestamp_accepts_all_validator_dialects() {
        // These three shapes are exactly what core::validators::
        // check_timestamp accepts: Game.log canonical, GameCrash chrono
        // offset, and the LauncherActivity space-separated form. The
        // server must parse all three — silently NULLing the launcher
        // form dropped launcher events out of every event_timestamp-
        // ordered query (F12).
        assert!(
            parse_event_timestamp("2026-05-02T21:14:23.189Z").is_some(),
            "canonical ISO-8601"
        );
        assert!(
            parse_event_timestamp("2026-05-04T21:10:12+00:00").is_some(),
            "chrono to_rfc3339 offset form"
        );
        assert!(
            parse_event_timestamp("2026-05-06 12:34:56.789").is_some(),
            "launcher space-separated form"
        );
        assert!(
            parse_event_timestamp("not-a-date").is_none(),
            "garbage must still be rejected"
        );
    }

    fn test_store_with_events(events: &[(&str, &str, DateTime<Utc>)]) -> MemoryQuery {
        let rows: Vec<StoredQueryEvent> = events
            .iter()
            .enumerate()
            .map(|(i, (handle, ty, timestamp))| StoredQueryEvent {
                seq: i as i64 + 1,
                claimed_handle: handle.to_string(),
                event_type: ty.to_string(),
                event_timestamp: Some(*timestamp),
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({"type": ty}),
                resolved_location: None,
                hidden_at: None,
            })
            .collect();
        MemoryQuery::new(rows)
    }

    #[tokio::test]
    async fn records_for_handle_computes_all_time_records() {
        let store = test_store_with_events(&[
            // Session A: 10:00 -> 10:10 (600s), 3 events, 2 deaths.
            ("alice", "join_pu", ts("2026-06-01T10:00:00Z")),
            ("alice", "player_death", ts("2026-06-01T10:05:00Z")),
            ("alice", "player_death", ts("2026-06-01T10:10:00Z")),
            // 31-min idle gap -> new session B: 1 event, 0 deaths.
            ("alice", "join_pu", ts("2026-06-01T10:41:00Z")),
        ]);
        let r = store.records_for_handle("alice", None).await.unwrap();
        assert_eq!(r.longest_session_secs, 600, "session A spans 10 min");
        assert_eq!(r.busiest_session_events, 3, "session A has 3 events");
        assert_eq!(r.deadliest_session_deaths, 2, "session A has 2 deaths");
        // Two deaths 5 min apart -> 300s longest stretch alive.
        assert_eq!(r.longest_survival_streak_secs, 300);
    }

    #[tokio::test]
    async fn records_for_handle_empty_is_all_zero() {
        let store = test_store_with_events(&[]);
        let r = store.records_for_handle("nobody", None).await.unwrap();
        assert_eq!(r, RecordsAggregate::default());
    }

    #[tokio::test]
    async fn records_for_handle_window_excludes_events_before_since() {
        let now = Utc::now();
        let old = now - chrono::Duration::days(40);
        let recent = now - chrono::Duration::hours(1);
        let store = test_store_with_events(&[
            // Old session (40 days ago): two deaths 5 min apart.
            ("alice", "player_death", old),
            ("alice", "player_death", old + chrono::Duration::minutes(5)),
            // Recent session (1h ago): a single non-death event.
            ("alice", "join_pu", recent),
        ]);

        // Lifetime (since = None) sees the old session's records.
        let all = store.records_for_handle("alice", None).await.unwrap();
        assert_eq!(
            all.deadliest_session_deaths, 2,
            "lifetime sees the old deaths"
        );
        assert_eq!(
            all.longest_survival_streak_secs, 300,
            "old death pair, 5 min apart"
        );
        assert_eq!(all.busiest_session_events, 2, "old session is the busiest");

        // A 24h window drops the 40-day-old session entirely; only the
        // recent single-event session remains.
        let since = Some(now - chrono::Duration::hours(24));
        let win = store.records_for_handle("alice", since).await.unwrap();
        assert_eq!(
            win.deadliest_session_deaths, 0,
            "old deaths excluded by window"
        );
        assert_eq!(
            win.longest_survival_streak_secs, 0,
            "no death pair falls inside the window"
        );
        assert_eq!(
            win.busiest_session_events, 1,
            "only the recent join_pu remains"
        );
    }

    /// Build a `StoredQueryEvent` with a payload that actually
    /// round-trips through `serde_json::from_value::<GameEvent>` --
    /// unlike `test_store_with_events`'s `{"type": ty}` stub, which is
    /// enough for the gap-idle SQL-mirroring tests above but NOT enough
    /// for `derive_lives`, which parses the real `GameEvent` variant.
    fn life_evt(
        seq: i64,
        handle: &str,
        ts: DateTime<Utc>,
        payload: serde_json::Value,
    ) -> StoredQueryEvent {
        let event_type = payload["type"].as_str().unwrap().to_string();
        StoredQueryEvent {
            seq,
            claimed_handle: handle.to_string(),
            event_type,
            event_timestamp: Some(ts),
            log_source: "live".into(),
            source_offset: 0,
            payload,
            resolved_location: None,
            hidden_at: None,
        }
    }

    fn spawn_payload(ts: &str) -> serde_json::Value {
        json!({
            "type": "resolve_spawn",
            "timestamp": ts,
            "player_geid": "Jim",
            "fallback": false,
        })
    }

    fn death_payload(ts: &str) -> serde_json::Value {
        json!({
            "type": "player_death",
            "timestamp": ts,
            "body_class": "body_01",
            "body_id": "body_id_1",
            "zone": null,
        })
    }

    #[tokio::test]
    async fn lives_for_handle_summarizes_a_multi_life_stream() {
        // spawn -> death -> spawn -> death -> spawn (still alive):
        // total_lives 3, deaths 2.
        let store = MemoryQuery::new(vec![
            life_evt(
                1,
                "alice",
                ts("2026-06-01T10:00:00Z"),
                spawn_payload("2026-06-01T10:00:00Z"),
            ),
            life_evt(
                2,
                "alice",
                ts("2026-06-01T10:05:00Z"),
                death_payload("2026-06-01T10:05:00Z"),
            ),
            life_evt(
                3,
                "alice",
                ts("2026-06-01T10:06:00Z"),
                spawn_payload("2026-06-01T10:06:00Z"),
            ),
            life_evt(
                4,
                "alice",
                ts("2026-06-01T10:20:00Z"),
                death_payload("2026-06-01T10:20:00Z"),
            ),
            life_evt(
                5,
                "alice",
                ts("2026-06-01T10:21:00Z"),
                spawn_payload("2026-06-01T10:21:00Z"),
            ),
        ]);
        let data = store.lives_for_handle("alice", None).await.unwrap();
        assert_eq!(data.summary.total_lives, 3);
        assert_eq!(data.summary.deaths, 2);
        // All 5 events fall within one 30-min idle-gap window, so the
        // unbounded `count_sessions_since` count (what `sessions` must
        // come from) clusters them into a single session.
        let expected_sessions = store.count_sessions_since("alice", None).await.unwrap() as u32;
        assert_eq!(data.sessions, expected_sessions);
        assert_eq!(
            data.sessions, 1,
            "all 5 events are within the 30-min idle gap -> 1 canonical session"
        );
    }

    #[tokio::test]
    async fn lives_for_handle_session_count_is_unbounded_gap_idle() {
        // resolve_spawn/player_death pairs clustered by >30-min idle
        // gaps into distinct sessions -- proves `sessions` is wired to
        // the UNBOUNDED `count_sessions_since(.., None)` path rather
        // than the FSM's own gap-idle `LifeSummary::sessions` or the
        // 50-capped `event_timeline::derive_sessions`.
        let mut events = Vec::new();
        let mut seq = 1;
        let starts = [
            "2026-06-01T10:00:00Z",
            "2026-06-01T12:00:00Z",
            "2026-06-01T14:00:00Z",
            "2026-06-01T16:00:00Z",
        ];
        for start in starts {
            events.push(life_evt(seq, "bob", ts(start), spawn_payload(start)));
            seq += 1;
            events.push(life_evt(seq, "bob", ts(start), death_payload(start)));
            seq += 1;
        }
        let store = MemoryQuery::new(events);

        let expected_sessions = store.count_sessions_since("bob", None).await.unwrap() as u32;
        assert_eq!(
            expected_sessions, 4,
            "sanity: 4 gap-idle clusters, >30min apart each"
        );

        let data = store.lives_for_handle("bob", None).await.unwrap();
        assert_eq!(data.sessions, expected_sessions);
        assert_eq!(data.summary.deaths, 4);
        assert_eq!(
            data.sessions as f32, 4.0,
            "deaths_per_session denominator must be the unbounded count"
        );

        let deaths_per_session =
            (data.sessions > 0).then(|| data.summary.deaths as f32 / data.sessions as f32);
        assert_eq!(deaths_per_session, Some(1.0));
    }

    #[tokio::test]
    async fn lives_for_handle_window_excludes_lives_before_since() {
        let now = Utc::now();
        let old = now - chrono::Duration::days(40);
        let recent = now - chrono::Duration::hours(1);
        let old_spawn = old.to_rfc3339();
        let old_death = (old + chrono::Duration::minutes(5)).to_rfc3339();
        let rec_spawn = recent.to_rfc3339();
        let rec_death = (recent + chrono::Duration::minutes(5)).to_rfc3339();
        let store = MemoryQuery::new(vec![
            // Old life: spawn -> death, 40 days ago.
            life_evt(1, "alice", old, spawn_payload(&old_spawn)),
            life_evt(
                2,
                "alice",
                old + chrono::Duration::minutes(5),
                death_payload(&old_death),
            ),
            // Recent life: spawn -> death, 1h ago.
            life_evt(3, "alice", recent, spawn_payload(&rec_spawn)),
            life_evt(
                4,
                "alice",
                recent + chrono::Duration::minutes(5),
                death_payload(&rec_death),
            ),
        ]);

        // Lifetime (since = None): both lives counted.
        let all = store.lives_for_handle("alice", None).await.unwrap();
        assert_eq!(all.summary.total_lives, 2);
        assert_eq!(all.summary.deaths, 2);

        // 24h window: the 40-day-old life is dropped before the FSM runs.
        let since = Some(now - chrono::Duration::hours(24));
        let win = store.lives_for_handle("alice", since).await.unwrap();
        assert_eq!(win.summary.total_lives, 1, "old life excluded by window");
        assert_eq!(win.summary.deaths, 1, "old death excluded by window");
    }

    #[tokio::test]
    async fn total_playtime_sums_session_spans() {
        let store = test_store_with_events(&[
            // Session A: 10:00 -> 10:20 (1200s)
            ("alice", "join_pu", ts("2026-06-01T10:00:00Z")),
            ("alice", "actor_death", ts("2026-06-01T10:20:00Z")),
            // 31 min idle gap -> new session
            // Session B: 10:51 -> 11:01 (600s)
            ("alice", "join_pu", ts("2026-06-01T10:51:00Z")),
            ("alice", "actor_death", ts("2026-06-01T11:01:00Z")),
        ]);
        let since = ts("2026-06-01T00:00:00Z");
        let total = store
            .total_playtime_secs("alice", Some(since))
            .await
            .unwrap();
        assert_eq!(total, 1200 + 600);
    }

    #[tokio::test]
    async fn total_playtime_zero_when_no_events() {
        let store = test_store_with_events(&[]);
        let total = store.total_playtime_secs("alice", None).await.unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn total_playtime_excludes_sessions_before_since() {
        let store = test_store_with_events(&[
            // Session A: before the window
            ("alice", "join_pu", ts("2026-06-01T08:00:00Z")),
            ("alice", "actor_death", ts("2026-06-01T08:30:00Z")),
            // Session B: within the window (600s)
            ("alice", "join_pu", ts("2026-06-01T10:00:00Z")),
            ("alice", "actor_death", ts("2026-06-01T10:10:00Z")),
        ]);
        // since = 09:00, so only session B counts
        let since = ts("2026-06-01T09:00:00Z");
        let total = store
            .total_playtime_secs("alice", Some(since))
            .await
            .unwrap();
        assert_eq!(total, 600);
    }

    #[tokio::test]
    async fn sessions_for_handle_is_case_insensitive() {
        // Events ingested under mixed-case handle (e.g. after a re-pair).
        let store = test_store_with_events(&[
            ("TheCodeSaiyan", "join_pu", ts("2026-06-01T10:00:00Z")),
            ("TheCodeSaiyan", "actor_death", ts("2026-06-01T10:20:00Z")),
        ]);

        // Query with a different case of the same handle.
        let sessions = store
            .sessions_for_handle("thecodesaiyan", 50, 0)
            .await
            .unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "sessions_for_handle must match claimed_handle case-insensitively"
        );

        // Also verify count and playtime follow the same rule.
        let count = store
            .count_sessions_since("thecodesaiyan", None)
            .await
            .unwrap();
        assert_eq!(
            count, 1,
            "count_sessions_since must match claimed_handle case-insensitively"
        );

        let playtime = store
            .total_playtime_secs("thecodesaiyan", None)
            .await
            .unwrap();
        assert!(
            playtime > 0,
            "total_playtime_secs must match claimed_handle case-insensitively"
        );
    }

    #[tokio::test]
    async fn list_filtered_and_timeline_are_case_insensitive() {
        // Events stored under "TheCodeSaiyan" (mixed case). Anchor the
        // timestamps to `now` rather than a fixed calendar date: this
        // test asserts `timeline(_, 30)` counts them regardless of
        // handle case, and a hardcoded date silently ages out of the
        // trailing-30-day window (the window is incidental to what's
        // under test).
        let recent = Utc::now() - chrono::Duration::hours(2);
        let store = test_store_with_events(&[
            ("TheCodeSaiyan", "join_pu", recent),
            (
                "TheCodeSaiyan",
                "actor_death",
                recent + chrono::Duration::minutes(20),
            ),
            // A different user — must NOT appear in results.
            (
                "other_user",
                "join_pu",
                recent + chrono::Duration::minutes(5),
            ),
        ]);

        // list_filtered queried with all-lowercase handle.
        let filters = EventFilters {
            event_type: None,
            since: None,
            until: None,
            cursor: None,
            limit: 50,
        };
        let results = store.list_filtered("thecodesaiyan", filters).await.unwrap();
        assert_eq!(
            results.len(),
            2,
            "list_filtered must return events for all handle-case variants"
        );
        assert!(
            results
                .iter()
                .all(|r| r.claimed_handle.eq_ignore_ascii_case("thecodesaiyan")),
            "list_filtered must not return events for other users"
        );

        // timeline queried with all-uppercase handle.
        let buckets = store.timeline("THECODESAIYAN", 30).await.unwrap();
        let total: i64 = buckets.iter().map(|(_, c)| c).sum();
        assert_eq!(
            total, 2,
            "timeline must count events regardless of handle case"
        );
    }

    #[tokio::test]
    async fn sessions_exclude_launcher_activity_from_bridging() {
        // launcher_activity comes from the RSI launcher log and fires
        // while the launcher runs in the background — even off-hours.
        // Counting it would bridge the idle gap between two real play
        // bursts into a single session. Here two play events are 60 min
        // apart (a clean >30-min split) with launcher noise every 20 min
        // in the gap; excluding the noise must yield TWO sessions.
        let base = ts("2026-06-01T10:00:00Z");
        let store = test_store_with_events(&[
            ("h", "player_death", base),
            (
                "h",
                "launcher_activity",
                base + chrono::Duration::minutes(20),
            ),
            (
                "h",
                "launcher_activity",
                base + chrono::Duration::minutes(40),
            ),
            ("h", "actor_death", base + chrono::Duration::minutes(60)),
        ]);
        let sessions = store.sessions_for_handle("h", 50, 0).await.unwrap();
        assert_eq!(
            sessions.len(),
            2,
            "launcher_activity must not bridge two real play sessions"
        );
    }

    #[tokio::test]
    async fn count_sessions_since_counts_sessions() {
        let store = test_store_with_events(&[
            // Session A: 10:00 -> 10:20
            ("alice", "join_pu", ts("2026-06-01T10:00:00Z")),
            ("alice", "actor_death", ts("2026-06-01T10:20:00Z")),
            // 31 min idle gap -> new session
            // Session B: 10:51 -> 11:01
            ("alice", "join_pu", ts("2026-06-01T10:51:00Z")),
            ("alice", "actor_death", ts("2026-06-01T11:01:00Z")),
        ]);

        // None = all-time: both sessions
        let count = store.count_sessions_since("alice", None).await.unwrap();
        assert_eq!(count, 2, "all-time should count both sessions");

        // since after session A ends but before session B starts: only session B
        let since = ts("2026-06-01T10:30:00Z");
        let count = store
            .count_sessions_since("alice", Some(since))
            .await
            .unwrap();
        assert_eq!(
            count, 1,
            "since after session A should count only session B"
        );
    }

    fn evt_with_payload(
        seq: i64,
        handle: &str,
        ty: &str,
        ts: DateTime<Utc>,
        payload: serde_json::Value,
    ) -> StoredQueryEvent {
        StoredQueryEvent {
            seq,
            claimed_handle: handle.into(),
            event_type: ty.into(),
            event_timestamp: Some(ts),
            log_source: "live".into(),
            source_offset: 0,
            payload,
            resolved_location: None,
            hidden_at: None,
        }
    }

    #[tokio::test]
    async fn objective_outcomes_counts_each_objective_once() {
        // One objective that goes in_progress -> completed is ONE completed
        // objective, not one in-progress plus one completed.
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-a","state":"in_progress"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-a","state":"completed"}),
            ),
        ]);
        let out = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out.completed, 1);
        assert_eq!(out.no_outcome, 0);
        assert_eq!(out.failed, 0);
        assert_eq!(out.unresolved, 0);
    }

    #[tokio::test]
    async fn objective_outcomes_maps_withdrawn_state_to_unresolved() {
        // Collectors from v1.8.149 on store WITHDRAWN as
        // `{"state":"withdrawn"}` (see `parse_objective_state`). The rank
        // table MUST know that string: an unranked state scores 0 and is
        // dropped by the `_ => {}` arm, so a rank-table miss would delete
        // every withdrawn objective from the aggregate AND from `total` —
        // the counts would simply shrink, with nothing reporting an error.
        // It must also outrank in_progress, exactly as "unknown" does.
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-w","state":"in_progress"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-w","state":"withdrawn"}),
            ),
        ]);
        let out = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out.unresolved, 1);
        assert_eq!(out.no_outcome, 0);
        // Present in `total`, not silently deleted from it.
        assert_eq!(
            out.completed + out.failed + out.unresolved + out.no_outcome,
            1
        );
    }

    #[tokio::test]
    async fn objective_outcomes_ranks_legacy_unknown_alongside_withdrawn() {
        // Pre-v1.8.149 collectors stored WITHDRAWN as `{"state":"unknown"}`
        // and those rows are never rewritten, so BOTH spellings must rank
        // identically and land in the same bucket — otherwise the same
        // objective counts differently depending on which tray version
        // uploaded it.
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-old","state":"unknown"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-new","state":"withdrawn"}),
            ),
        ]);
        let out = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out.unresolved, 2);
        assert_eq!(out.no_outcome, 0);
    }

    #[tokio::test]
    async fn objective_outcomes_maps_unknown_state_to_unresolved() {
        // A state the parser genuinely does not recognise is stored as
        // `{"state":"unknown"}` (as WITHDRAWN itself was, pre-v1.8.149).
        // This must land in `unresolved`, not silently disappear into the
        // `_ => 0` (unranked) bucket, and it must NOT be mistaken for
        // `no_outcome` (the regression this guards: MAX(rank) must pick
        // unresolved(2) over in_progress(1)).
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-b","state":"in_progress"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-b","state":"unknown"}),
            ),
        ]);
        let out = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out.unresolved, 1);
        assert_eq!(out.no_outcome, 0);
    }

    #[tokio::test]
    async fn objective_outcomes_without_objective_id_counts_per_row() {
        // Legacy rows predating objective_id capture must not all collapse
        // into a single bucket.
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"state":"completed"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"state":"completed"}),
            ),
        ]);
        let out = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out.completed, 2);
    }

    #[tokio::test]
    async fn objective_outcomes_since_window_filters_old_objectives() {
        // One objective completed "now", one completed 60 days ago. Pins the
        // `since` comparison direction: flipping `>=` to `<=`, or dropping
        // the predicate entirely, would still pass the `None` case here but
        // silently return the wrong window — nothing else in this file
        // exercises `since` on `objective_outcomes`.
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now,
                serde_json::json!({"objective_id":"obj-recent","state":"completed"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                old,
                serde_json::json!({"objective_id":"obj-old","state":"completed"}),
            ),
        ]);

        let out_all = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out_all.completed, 2, "no since — both objectives count");

        let out_recent = q
            .objective_outcomes("alice", Some(now - chrono::Duration::hours(24)), None)
            .await
            .unwrap();
        assert_eq!(
            out_recent.completed, 1,
            "since 24h ago — only the recent objective counts"
        );
    }

    /// The four range-scoped aggregates must agree on window arithmetic:
    /// `since` INCLUSIVE, `until` EXCLUSIVE. An event landing exactly on
    /// the shared edge of two adjacent windows (`[a,b)` then `[b,c)`)
    /// belongs to the LATER one and is counted exactly once across the
    /// pair — that is what makes a previous-vs-current comparison sound.
    ///
    /// Asserted per method rather than once, because each carries its own
    /// copy of the predicate (four `MemoryQuery` loops, four SQL strings):
    /// a `<=` slipping into any one of them double-counts the boundary in
    /// that endpoint alone.
    #[tokio::test]
    async fn until_is_exclusive_so_adjacent_windows_count_the_edge_once() {
        let edge = Utc::now() - chrono::Duration::hours(24);
        let older = edge - chrono::Duration::hours(1);
        let newer = edge + chrono::Duration::hours(1);
        let start = edge - chrono::Duration::hours(24);
        let end = edge + chrono::Duration::hours(24);

        // One event in the earlier window, one exactly ON the edge, one in
        // the later window — for each of the four aggregates.
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                older,
                serde_json::json!({"price":100,"shop_name":"Older"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                edge,
                serde_json::json!({"price":200,"shop_name":"Edge"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "shop_buy_request",
                newer,
                serde_json::json!({"price":400,"shop_name":"Newer"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "mission_objective",
                older,
                serde_json::json!({"objective_id":"obj-older","state":"completed"}),
            ),
            evt_with_payload(
                5,
                "alice",
                "mission_objective",
                edge,
                serde_json::json!({"objective_id":"obj-edge","state":"completed"}),
            ),
            evt_with_payload(
                6,
                "alice",
                "mission_objective",
                newer,
                serde_json::json!({"objective_id":"obj-newer","state":"completed"}),
            ),
        ]);

        // -- count_event_type
        let earlier = q
            .count_event_type("alice", "shop_buy_request", None, Some(start), Some(edge))
            .await
            .unwrap();
        let later = q
            .count_event_type("alice", "shop_buy_request", None, Some(edge), Some(end))
            .await
            .unwrap();
        assert_eq!(earlier, 1, "the edge event must NOT fall in [start, edge)");
        assert_eq!(later, 2, "the edge event must fall in [edge, end)");
        assert_eq!(earlier + later, 3, "every event counted exactly once");

        // -- payload_numeric_sum
        let earlier = q
            .payload_numeric_sum(
                "alice",
                "shop_buy_request",
                "price",
                Some(start),
                Some(edge),
            )
            .await
            .unwrap();
        let later = q
            .payload_numeric_sum("alice", "shop_buy_request", "price", Some(edge), Some(end))
            .await
            .unwrap();
        assert_eq!(earlier, 100, "edge price must not land in the earlier sum");
        assert_eq!(later, 600, "edge price belongs to the later sum");
        assert_eq!(earlier + later, 700, "no aUEC counted twice");

        // -- payload_field_breakdown
        let earlier = q
            .payload_field_breakdown(
                "alice",
                "shop_buy_request",
                "shop_name",
                None,
                Some(start),
                Some(edge),
                100,
            )
            .await
            .unwrap();
        let later = q
            .payload_field_breakdown(
                "alice",
                "shop_buy_request",
                "shop_name",
                None,
                Some(edge),
                Some(end),
                100,
            )
            .await
            .unwrap();
        assert_eq!(
            earlier.iter().map(|b| b.value.as_str()).collect::<Vec<_>>(),
            vec!["Older"],
            "the edge bucket must not appear in [start, edge)"
        );
        let mut later_values: Vec<&str> = later.iter().map(|b| b.value.as_str()).collect();
        later_values.sort_unstable();
        assert_eq!(later_values, vec!["Edge", "Newer"]);

        // -- objective_outcomes
        let earlier = q
            .objective_outcomes("alice", Some(start), Some(edge))
            .await
            .unwrap();
        let later = q
            .objective_outcomes("alice", Some(edge), Some(end))
            .await
            .unwrap();
        assert_eq!(
            earlier.completed, 1,
            "the edge objective must NOT fall in [start, edge)"
        );
        assert_eq!(
            later.completed, 2,
            "the edge objective belongs to [edge, end)"
        );
        assert_eq!(
            earlier.completed + later.completed,
            3,
            "every objective counted exactly once"
        );
    }

    /// `has_events_in_window` answers "did this handle exist yet", so it
    /// must not care WHICH event type — any row inside the window counts.
    /// Its bounds match the aggregates above: `since` inclusive, `until`
    /// exclusive.
    #[tokio::test]
    async fn has_events_in_window_matches_any_event_type_and_excludes_until() {
        let edge = Utc::now() - chrono::Duration::hours(24);
        let start = edge - chrono::Duration::hours(24);
        // A type none of the stats aggregates ever reads.
        let q = MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "join_pu",
            edge,
            serde_json::json!({"shard":"1a"}),
        )]);

        assert!(
            q.has_events_in_window("alice", edge, edge + chrono::Duration::hours(1))
                .await
                .unwrap(),
            "an event of ANY type inside the window proves the handle existed"
        );
        assert!(
            !q.has_events_in_window("alice", start, edge).await.unwrap(),
            "`until` is exclusive — the edge event is outside [start, edge)"
        );
        assert!(
            !q.has_events_in_window("bob", start, edge + chrono::Duration::hours(1))
                .await
                .unwrap(),
            "another handle's events must not count"
        );
    }

    #[tokio::test]
    async fn objective_outcomes_completed_outranks_failed() {
        // Pins completed(4) > failed(3): a single objective_id reporting
        // both a failed row and a completed row must resolve to completed.
        // Nothing else in this file has one objective hit both states, so
        // swapping the two ranks in `rank()` would break no other test.
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-e","state":"failed"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                Utc::now(),
                serde_json::json!({"objective_id":"obj-e","state":"completed"}),
            ),
        ]);
        let out = q.objective_outcomes("alice", None, None).await.unwrap();
        assert_eq!(out.completed, 1);
        assert_eq!(out.failed, 0);
    }

    // ---- contract_runs ----

    fn hud_payload(
        ts: &str,
        text: &str,
        notification_id: u64,
        mission_id: &str,
    ) -> serde_json::Value {
        json!({
            "type": "hud_notification",
            "timestamp": ts,
            "text": text,
            "notification_id": notification_id,
            "mission_id": mission_id,
            "objective_id": null,
        })
    }

    #[tokio::test]
    async fn contract_runs_derives_accept_then_complete_pair() {
        // Mirrors starstats_core::contract_life's own
        // `accepted_then_completed_yields_one_completed_run` fixture, fed
        // through the repo layer instead of calling the fold directly.
        // RED test for `EventQuery::contract_runs` (Task 2 step 1) — fails
        // to compile until the trait method + MemoryQuery impl exist.
        let mid = "7de35808-d909-4a6d-affe-edadf3e6fe77";
        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "hud_notification",
                ts("2026-07-26T13:57:59Z"),
                hud_payload(
                    "2026-07-26T13:57:59Z",
                    "Contract Accepted:  Combat Gauntlet - Scenario #5: ",
                    1,
                    mid,
                ),
            ),
            evt_with_payload(
                2,
                "alice",
                "hud_notification",
                ts("2026-07-26T14:03:42Z"),
                hud_payload(
                    "2026-07-26T14:03:42Z",
                    "Contract Complete: Combat Gauntlet - Scenario #5: ",
                    2,
                    mid,
                ),
            ),
        ]);

        let runs = q.contract_runs("alice", None).await.unwrap();
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.mission_id, mid);
        assert_eq!(r.name, "Combat Gauntlet - Scenario #5");
        assert_eq!(r.state, "completed");
        assert_eq!(r.closed_by, "hud_complete");
        assert!(r.accepted_at.is_some());
        assert!(r.closed_at.is_some());
        assert_eq!(r.step_count, 0);
    }

    #[tokio::test]
    async fn contract_runs_since_filters_output_not_input_window() {
        // Two independent contracts 60 days apart, each closed by its own
        // HUD banner. Pins that `since` scopes the DERIVED runs by
        // `accepted_at` (per the trait doc) rather than truncating the
        // event stream before the fold runs — the opposite of
        // `lives_for_handle`'s `since`, which truncates the input. Getting
        // this backwards (filtering input instead of output) would still
        // pass a naive version of this test only by accident; the 60-day
        // gap is large enough that either implementation excludes the old
        // contract, so what actually pins the direction is the `since:
        // None` assertion below returning BOTH runs.
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let mid_old = "11111111-1111-1111-1111-111111111111";
        let mid_new = "22222222-2222-2222-2222-222222222222";
        let old_ts = old.to_rfc3339();
        let old_close_ts = (old + chrono::Duration::minutes(5)).to_rfc3339();
        let new_ts = now.to_rfc3339();
        let new_close_ts = (now + chrono::Duration::minutes(5)).to_rfc3339();

        let q = MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "bob",
                "hud_notification",
                old,
                hud_payload(&old_ts, "Contract Accepted:  Old Job: ", 1, mid_old),
            ),
            evt_with_payload(
                2,
                "bob",
                "hud_notification",
                old + chrono::Duration::minutes(5),
                hud_payload(&old_close_ts, "Contract Complete: Old Job: ", 2, mid_old),
            ),
            evt_with_payload(
                3,
                "bob",
                "hud_notification",
                now,
                hud_payload(&new_ts, "Contract Accepted:  New Job: ", 3, mid_new),
            ),
            evt_with_payload(
                4,
                "bob",
                "hud_notification",
                now + chrono::Duration::minutes(5),
                hud_payload(&new_close_ts, "Contract Complete: New Job: ", 4, mid_new),
            ),
        ]);

        let all = q.contract_runs("bob", None).await.unwrap();
        assert_eq!(all.len(), 2, "no since — both contracts derive");

        let recent = q
            .contract_runs("bob", Some(now - chrono::Duration::hours(24)))
            .await
            .unwrap();
        assert_eq!(
            recent.len(),
            1,
            "since 24h ago excludes the 60-day-old contract"
        );
        assert_eq!(recent[0].mission_id, mid_new);
    }

    // Postgres-gated round-trip (env-gated integration test, same pattern
    // as `objective_outcomes_postgres_round_trip` below). Runs ONLY when
    // STARSTATS_TEST_DATABASE_URL points at a real Postgres; offline
    // `cargo test` skips it. Goes through `PostgresStore::insert_batch`
    // (the real ingest path) rather than a hand-crafted INSERT, so this is
    // also the only test exercising the `contracts_dirty = TRUE` edit made
    // to the bulk-ingest CTE alongside this test, plus the rebuild's UNNEST
    // insert + JSONB `steps` decode + advisory-lock dirty-clear — none of
    // which any MemoryQuery test above can catch.
    #[tokio::test]
    async fn contract_runs_postgres_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres round-trip test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        let handle = "contractrun_roundtrip_probe";
        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe events");
        sqlx::query("DELETE FROM contract_runs WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe contract_runs");
        sqlx::query("DELETE FROM stat_rollup_state WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe rollup state");

        let store = PostgresStore::new(pool.clone());
        let mid = "7de35808-d909-4a6d-affe-edadf3e6fe77";
        let obj_id = "2432e890-93a3-0c46-a8a5-c7bb4915881f";
        let accept_ts = "2026-07-26T13:57:59Z";
        let step_ts = "2026-07-26T14:00:11Z";
        let complete_ts = "2026-07-26T14:03:42Z";
        let events = vec![
            StoredEvent {
                id: Uuid::new_v4(),
                idempotency_key: "contractrun-probe-1".to_string(),
                claimed_handle: handle.to_string(),
                event_type: "hud_notification".to_string(),
                event_timestamp: Some(ts(accept_ts)),
                log_source: LogSource::Live,
                source_offset: 0,
                raw_line: String::new(),
                payload: hud_payload(
                    accept_ts,
                    "Contract Accepted:  Combat Gauntlet - Scenario #5: ",
                    1,
                    mid,
                ),
                metadata: None,
                resolved_location: None,
            },
            StoredEvent {
                id: Uuid::new_v4(),
                idempotency_key: "contractrun-probe-1b".to_string(),
                claimed_handle: handle.to_string(),
                event_type: "hud_notification".to_string(),
                event_timestamp: Some(ts(step_ts)),
                log_source: LogSource::Live,
                source_offset: 1,
                raw_line: String::new(),
                // A step banner so `steps` decodes as non-empty JSONB below
                // -- the fixture previously only ever exercised the
                // `'[]'::jsonb` default, which left the "JSONB steps
                // decode" claim in this test's own doc comment untrue.
                payload: json!({
                    "type": "hud_notification",
                    "timestamp": step_ts,
                    "text": "New Objective: Go to Checkmate Station: ",
                    "notification_id": 2,
                    "mission_id": mid,
                    "objective_id": obj_id,
                }),
                metadata: None,
                resolved_location: None,
            },
            StoredEvent {
                id: Uuid::new_v4(),
                idempotency_key: "contractrun-probe-2".to_string(),
                claimed_handle: handle.to_string(),
                event_type: "hud_notification".to_string(),
                event_timestamp: Some(ts(complete_ts)),
                log_source: LogSource::Live,
                source_offset: 2,
                raw_line: String::new(),
                payload: hud_payload(
                    complete_ts,
                    "Contract Complete: Combat Gauntlet - Scenario #5: ",
                    3,
                    mid,
                ),
                metadata: None,
                resolved_location: None,
            },
        ];

        let outcomes = store
            .insert_batch(events)
            .await
            .expect("insert_batch must not error");
        assert!(outcomes.iter().all(|o| *o == InsertOutcome::Inserted));

        let dirty_after_ingest: bool = sqlx::query_scalar(
            "SELECT contracts_dirty FROM stat_rollup_state WHERE claimed_handle = $1",
        )
        .bind(handle)
        .fetch_one(&pool)
        .await
        .expect("stat_rollup_state row must exist after ingest");
        assert!(
            dirty_after_ingest,
            "bulk ingest must mark contracts_dirty — else the rollup freezes forever"
        );

        let runs = store.contract_runs(handle, None).await.expect(
            "contract_runs must not error — exercises the UNNEST insert + JSONB steps decode",
        );
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.mission_id, mid);
        assert_eq!(r.name, "Combat Gauntlet - Scenario #5");
        assert_eq!(r.state, "completed");
        assert_eq!(r.closed_by, "hud_complete");
        assert!(r.accepted_at.is_some());
        assert!(r.closed_at.is_some());
        assert_eq!(
            r.steps.len(),
            1,
            "the New Objective banner must round-trip through JSONB"
        );
        assert_eq!(r.steps[0].objective_id.as_deref(), Some(obj_id));
        assert_eq!(r.steps[0].text.as_deref(), Some("Go to Checkmate Station"));
        assert_eq!(r.steps[0].state, StepState::InProgress);

        let dirty_after_read: bool = sqlx::query_scalar(
            "SELECT contracts_dirty FROM stat_rollup_state WHERE claimed_handle = $1",
        )
        .bind(handle)
        .fetch_one(&pool)
        .await
        .expect("stat_rollup_state row must still exist");
        assert!(!dirty_after_read, "the rebuild must clear contracts_dirty");

        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean up probe events");
        sqlx::query("DELETE FROM contract_runs WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean up probe contract_runs");
        sqlx::query("DELETE FROM stat_rollup_state WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean up probe rollup state");
    }

    // ---- contract_catalog_gaps ----
    //
    // Parallel-safe per the Task 1 convention (contracts.rs's
    // `list_filters_contract_type_across_separator_and_boundary_variants_on_real_postgres`):
    // unique `t6gap_` prefix on every name/canonical_id this test
    // seeds, scoped `DELETE ... WHERE ... LIKE 't6gap_%'` on entry AND
    // exit (never TRUNCATE), assertions scoped to rows this test owns.
    // `contract_runs` is a rollup table other tests touch concurrently
    // (two test binaries against the same DB, per this task's brief),
    // so the contract `name` values below are ALSO prefixed — not just
    // `mission_id` — because `contract_catalog_gaps` groups by `name`;
    // reusing a generic name like "Combat Gauntlet - Scenario #5"
    // (used verbatim by `contract_runs_postgres_round_trip` above)
    // would let a concurrently-running sibling test's rows leak into
    // this test's aggregate.

    /// Escape a prefix for use as a `LIKE 'prefix%'` pattern. Postgres's
    /// `_` is a single-character LIKE wildcard, so an unescaped prefix
    /// ending in `_` (both prefixes in this file do) matches a sibling
    /// prefix too — unescaped, `t6gap_%` also matches `t6gapsup_...`
    /// (any single character stands in for the `_`), which let the two
    /// gap tests' cleanup delete each other's rows out from under a
    /// concurrently-running sibling. Escaping (Postgres's default LIKE
    /// escape character is backslash) makes the match exact.
    fn escape_like_prefix(prefix: &str) -> String {
        format!(
            "{}%",
            prefix
                .replace('\\', "\\\\")
                .replace('_', "\\_")
                .replace('%', "\\%")
        )
    }

    async fn clear_gap_scoped_rows(pool: &PgPool, prefix: &str) {
        let pattern = escape_like_prefix(prefix);
        sqlx::query("DELETE FROM contract_runs WHERE name LIKE $1")
            .bind(&pattern)
            .execute(pool)
            .await
            .expect("delete this test's scoped contract_runs rows");
        sqlx::query("DELETE FROM contracts WHERE canonical_id LIKE $1")
            .bind(&pattern)
            .execute(pool)
            .await
            .expect("delete this test's scoped contracts rows");
    }

    async fn seed_gap_catalog(pool: &PgPool, canonical_id: &str, display_name: &str) {
        sqlx::query(
            "INSERT INTO contracts (canonical_id, schema_version, display_name, record)
             VALUES ($1, '1', $2, '{}'::jsonb)
             ON CONFLICT (canonical_id) DO NOTHING",
        )
        .bind(canonical_id)
        .bind(display_name)
        .execute(pool)
        .await
        .expect("seed catalog row");
    }

    /// Hand-crafted INSERT rather than going through the ingest+fold
    /// pipeline (unlike `contract_runs_postgres_round_trip` above) --
    /// this test exercises the read-only gap-aggregate SQL, not the
    /// materialisation path, so a direct row is the right level.
    async fn seed_gap_run(
        pool: &PgPool,
        mission_id: &str,
        name: &str,
        claimed_handle: &str,
        state: &str,
        accepted_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO contract_runs
                (claimed_handle, mission_id, accepted_at, name, state, closed_by)
             VALUES (LOWER($1), $2, $3, $4, $5, 'none')",
        )
        .bind(claimed_handle)
        .bind(mission_id)
        .bind(accepted_at)
        .bind(name)
        .bind(state)
        .execute(pool)
        .await
        .expect("seed contract_runs row");
    }

    #[tokio::test]
    async fn gaps_rank_by_occurrence_not_distinct_name_on_real_postgres() {
        // The load-bearing behaviour this task exists for: Combat
        // Gauntlet is a handful of distinct names but the overwhelming
        // majority of unmatched RUNS (measured on a 280-log corpus
        // against the live catalog: 8 of 147 distinct unmatched names,
        // ~5%, but 37% of all runs). A name-ranked list buries it; an
        // occurrence-ranked one doesn't.
        //
        // `t6gap_aaa_alpha_rare` is deliberately the alphabetically
        // FIRST unmatched name, seeded with only 1 run. If the query's
        // `ORDER BY run_count DESC, name ASC` ever regresses to a
        // name-only sort, THIS name -- not the 40-run Combat Gauntlet
        // stand-in -- would land at `gaps[0]`, so the assertion below
        // actually distinguishes the two orderings instead of passing
        // under both (a same-alphabetical-winner fixture would not).
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!(
                "STARSTATS_TEST_DATABASE_URL unset — skipping contract_catalog_gaps ranking PG test"
            );
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        let prefix = "t6gap_";
        // Self-heal: a previous crashed run may have left rows behind.
        clear_gap_scoped_rows(&pool, prefix).await;

        let store = PostgresStore::new(pool.clone());
        let base = ts("2026-07-27T12:00:00Z");

        // Catalog: one published name. Its 99 runs must be excluded
        // from the gap list (and the total) entirely.
        let published_name = format!("{prefix}published_contract");
        seed_gap_catalog(&pool, &format!("{prefix}cat1"), &published_name).await;
        for i in 0..99 {
            seed_gap_run(
                &pool,
                &format!("{prefix}pub_mission_{i}"),
                &published_name,
                "t6gap_h_pub",
                "completed",
                base + chrono::Duration::seconds(i),
            )
            .await;
        }

        // The single biggest gap: 40 runs under one name, split across
        // 2 handles, spanning a known first/last window.
        let combat_name = format!("{prefix}combat_gauntlet");
        for i in 0..40 {
            let handle = if i % 2 == 0 { "t6gap_ha" } else { "t6gap_hb" };
            seed_gap_run(
                &pool,
                &format!("{prefix}combat_mission_{i}"),
                &combat_name,
                handle,
                "completed",
                base + chrono::Duration::minutes(i),
            )
            .await;
        }

        // A one-run name that sorts alphabetically before `combat_name`
        // -- see the doc comment above for why this matters.
        let alpha_name = format!("{prefix}aaa_alpha_rare");
        assert!(
            alpha_name < combat_name,
            "fixture bug: alpha_name must sort before combat_name for this test to be load-bearing"
        );
        seed_gap_run(
            &pool,
            &format!("{prefix}alpha_mission"),
            &alpha_name,
            "t6gap_hc",
            "completed",
            base,
        )
        .await;

        // Three more one-off unmatched names, so the many-run name is
        // ranked against several single-run names, not just one.
        for suffix in ["rare_b", "rare_c", "rare_d"] {
            seed_gap_run(
                &pool,
                &format!("{prefix}{suffix}_mission"),
                &format!("{prefix}{suffix}"),
                "t6gap_hc",
                "completed",
                base,
            )
            .await;
        }

        // A large, effectively-unbounded limit -- NOT 10 -- so this
        // holds regardless of what else lives in the DB. A small limit
        // plus an unscoped `gaps[0]` check would only be valid against
        // a scratch/empty Postgres: a busier shared DB (CI's eventual
        // shared instance) could have some unrelated unmatched name
        // with >40 runs rank ahead of this test's own fixtures — or
        // even push them out of a 10-row page entirely. Filtering to
        // this test's own `t6gap_` prefix afterward (order-preserving,
        // since filtering a sorted `Vec` never reorders it) scopes
        // every assertion below to rows this test owns, per the Task 1
        // convention.
        let gaps = store
            .contract_catalog_gaps(100_000)
            .await
            .expect("contract_catalog_gaps must not error");
        let scoped: Vec<&ContractGapRow> =
            gaps.iter().filter(|g| g.name.starts_with(prefix)).collect();

        assert_eq!(
            scoped[0].name, combat_name,
            "the 40-run name must rank first among this test's own unmatched names, \
             even though it's one of five unmatched distinct names"
        );
        assert_eq!(scoped[0].run_count, 40);
        assert_eq!(scoped[0].distinct_handles, 2);
        assert_eq!(scoped[0].first_seen, Some(base));
        assert_eq!(
            scoped[0].last_seen,
            Some(base + chrono::Duration::minutes(39))
        );
        assert!(
            !gaps.iter().any(|g| g.name == published_name),
            "a name present in the catalog is not a gap"
        );

        // Global (ungrouped) total -- asserted as a lower bound, not
        // exact equality: `contract_catalog_gaps_total` deliberately
        // has no scoping, so a concurrently-running sibling test's own
        // unmatched rows may legitimately add to it. Our own 44 (40 +
        // 1 + 1 + 1 + 1: combat_gauntlet + alpha + rare_b/c/d) rows
        // are guaranteed present at this point, so the total can never
        // be LESS than that.
        let total = store
            .contract_catalog_gaps_total()
            .await
            .expect("contract_catalog_gaps_total must not error");
        assert!(
            total >= 44,
            "total_unmatched_runs must count at least this test's 44 unmatched rows, got {total}"
        );

        clear_gap_scoped_rows(&pool, prefix).await;
    }

    #[tokio::test]
    async fn gaps_excludes_superseded_runs_on_real_postgres() {
        // `superseded` runs are re-accept bookkeeping, not distinct
        // play (measured: 69/609 runs in a 280-log corpus, 11%) --
        // counting them would inflate the ranking. A name whose every
        // run is `superseded` must not appear as a gap at all; a name
        // with a MIX of superseded and non-superseded runs must count
        // only the non-superseded ones.
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!(
                "STARSTATS_TEST_DATABASE_URL unset — skipping contract_catalog_gaps superseded-exclusion PG test"
            );
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        let prefix = "t6gapsup_";
        clear_gap_scoped_rows(&pool, prefix).await;

        let store = PostgresStore::new(pool.clone());
        let base = ts("2026-07-27T12:00:00Z");

        // Entirely-superseded name: three superseded rows, zero live
        // ones -- must produce no gap row at all.
        let superseded_only = format!("{prefix}superseded_only");
        for i in 0..3 {
            seed_gap_run(
                &pool,
                &format!("{prefix}sup_only_mission_{i}"),
                &superseded_only,
                "t6gapsup_h1",
                "superseded",
                base + chrono::Duration::seconds(i),
            )
            .await;
        }

        // Mixed name: 2 superseded + 3 non-superseded -- run_count
        // must reflect only the 3 non-superseded runs.
        let mixed = format!("{prefix}mixed_state");
        for i in 0..2 {
            seed_gap_run(
                &pool,
                &format!("{prefix}mixed_superseded_{i}"),
                &mixed,
                "t6gapsup_h1",
                "superseded",
                base + chrono::Duration::seconds(i),
            )
            .await;
        }
        for i in 0..3 {
            seed_gap_run(
                &pool,
                &format!("{prefix}mixed_live_{i}"),
                &mixed,
                "t6gapsup_h1",
                "completed",
                base + chrono::Duration::seconds(i),
            )
            .await;
        }

        // A large, effectively-unbounded limit -- NOT 50 -- so finding
        // `mixed` below doesn't depend on nothing else in the DB
        // outranking its 3 runs. A small limit is only safe against a
        // scratch/empty Postgres; against a busier shared instance, 50+
        // unrelated unmatched names could each have more than 3 runs
        // and push `mixed` off the page entirely, turning an unrelated
        // fact about the rest of the DB into a false failure here.
        let gaps = store
            .contract_catalog_gaps(100_000)
            .await
            .expect("contract_catalog_gaps must not error");

        assert!(
            !gaps.iter().any(|g| g.name == superseded_only),
            "a name whose every run is superseded is not a gap"
        );
        let mixed_row = gaps
            .iter()
            .find(|g| g.name == mixed)
            .expect("the mixed-state name must still surface as a gap");
        assert_eq!(
            mixed_row.run_count, 3,
            "run_count must count only the non-superseded runs"
        );

        clear_gap_scoped_rows(&pool, prefix).await;
    }

    // ---- Postgres round-trip (env-gated integration test) ----
    // Runs ONLY when STARSTATS_TEST_DATABASE_URL points at a real Postgres;
    // offline `cargo test` skips it (early return). This is the ONLY test
    // that exercises the sqlx tuple binding in
    // `PostgresStore::objective_outcomes` — `MAX(CASE ...)` yields Postgres
    // `integer` and `COUNT(*)` is cast to `bigint`, so a wrong Rust tuple
    // type here fails at runtime while every MemoryQuery test above stays
    // green.
    #[tokio::test]
    async fn objective_outcomes_postgres_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Postgres round-trip test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        // Isolated fixture: a unique handle so the assertions hold regardless
        // of what else lives in this database — including whatever the
        // sessionizer parity test in event_timeline.rs inserts concurrently
        // under --test-threads=4 against this same Postgres instance.
        let handle = "objoutcomes_roundtrip_probe";
        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe rows");

        // obj-a: in_progress -> completed  => 1 completed
        // obj-b: in_progress -> unknown    => 1 unresolved (MAX picks 2 over 1)
        // obj-f: in_progress -> withdrawn  => 1 unresolved (pins the SQL `WHEN
        //        'withdrawn' THEN 2` arm; obj-b pins the legacy 'unknown'
        //        spelling of the same bucket. A state missing from the CASE
        //        scores 0 and is dropped by `_ => {}`, so without this the
        //        withdrawn rows would vanish from the aggregate silently)
        // obj-c: in_progress only          => 1 no_outcome
        // obj-e: in_progress -> failed     => 1 failed (pins the SQL `WHEN
        //        'failed' THEN 3` arm — the MemoryQuery rank-3 path is
        //        covered elsewhere, but this is the only test that exercises
        //        it through the actual CASE expression)
        // two rows with NO objective_id    => 2 separate completed
        // one row with NO state at all     => rank 0, excluded from all buckets
        //
        // `seq` is left to its BIGSERIAL default rather than bound to a
        // literal: events_seq_uq is a table-wide unique index, and a
        // hardcoded 1..8 could collide with rows another gated test inserts
        // concurrently in the same run. The query only needs each id-less
        // row to land on a *distinct* seq (for the `'seq:' || seq::text`
        // fallback key), which the default guarantees regardless of value.
        let payloads = [
            r#"{"objective_id":"obj-a","state":"in_progress"}"#,
            r#"{"objective_id":"obj-a","state":"completed"}"#,
            r#"{"objective_id":"obj-b","state":"in_progress"}"#,
            r#"{"objective_id":"obj-b","state":"unknown"}"#,
            r#"{"objective_id":"obj-f","state":"in_progress"}"#,
            r#"{"objective_id":"obj-f","state":"withdrawn"}"#,
            r#"{"objective_id":"obj-c","state":"in_progress"}"#,
            r#"{"objective_id":"obj-e","state":"in_progress"}"#,
            r#"{"objective_id":"obj-e","state":"failed"}"#,
            r#"{"state":"completed"}"#,
            r#"{"state":"completed"}"#,
            r#"{"objective_id":"obj-d","text":"Go to ~mission(Location)"}"#,
        ];
        for (i, payload) in payloads.iter().enumerate() {
            sqlx::query(
                "INSERT INTO events
                     (id, idempotency_key, claimed_handle, event_type,
                      event_timestamp, log_source, source_offset, raw_line, payload)
                 VALUES (gen_random_uuid(), $1, $2, 'mission_objective', NOW(),
                         'live', $3, '', $4::jsonb)",
            )
            .bind(format!("objoutcomes-probe-{i}"))
            .bind(handle)
            .bind(i as i64)
            .bind(*payload)
            .execute(&pool)
            .await
            .expect("insert probe row");
        }

        let store = PostgresStore::new(pool.clone());
        let out = store
            .objective_outcomes(handle, None, None)
            .await
            .expect("objective_outcomes must not error — a tuple-binding mismatch surfaces here");

        assert_eq!(out.completed, 3, "obj-a + 2 id-less completed rows");
        assert_eq!(
            out.unresolved, 2,
            "obj-b ('unknown') + obj-f ('withdrawn'): both spellings rank as \
             unresolved, and MAX must pick that over in_progress"
        );
        assert_eq!(out.no_outcome, 1, "obj-c");
        assert_eq!(
            out.failed, 1,
            "obj-e: MAX must pick failed over in_progress — exercises the SQL `WHEN 'failed' THEN 3` arm"
        );

        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean up probe rows");
    }

    #[tokio::test]
    async fn docking_occurrences_postgres_round_trip() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping docking Postgres test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        let handle = "docking_occurrences_roundtrip_probe";
        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clear docking probe rows");

        let base = Utc::now() - chrono::Duration::minutes(10);
        let rows = [
            (
                "vehicle_stowed",
                base,
                serde_json::json!({"landing_area":"Hangar_Large_01"}),
            ),
            (
                "vehicle_stowed",
                base + chrono::Duration::seconds(1),
                serde_json::json!({"landing_area":"Hangar_Small_02"}),
            ),
            (
                "join_pu",
                base + chrono::Duration::seconds(2),
                serde_json::json!({"shard":"1a"}),
            ),
            (
                "vehicle_stowed",
                base + chrono::Duration::seconds(3),
                serde_json::json!({"landing_area":"Pad_Medium_01"}),
            ),
            (
                "burst_summary",
                base,
                serde_json::json!({
                    "rule_id":"vehicle_stowed_burst",
                    "size":3,
                    "end_timestamp":(base + chrono::Duration::seconds(3)).to_rfc3339(),
                }),
            ),
            (
                "vehicle_stowed",
                base + chrono::Duration::seconds(124),
                serde_json::json!({"landing_area":"Pad_Medium_01"}),
            ),
        ];
        for (index, (event_type, timestamp, payload)) in rows.iter().enumerate() {
            sqlx::query(
                "INSERT INTO events
                     (id, idempotency_key, claimed_handle, event_type,
                      event_timestamp, log_source, source_offset, raw_line, payload)
                 VALUES (gen_random_uuid(), $1, $2, $3, $4, 'live', $5, '', $6)",
            )
            .bind(format!("docking-occurrence-probe-{index}"))
            .bind(handle)
            .bind(*event_type)
            .bind(*timestamp)
            .bind(index as i64)
            .bind(payload)
            .execute(&pool)
            .await
            .expect("insert docking probe row");
        }

        let store = PostgresStore::new(pool.clone());
        let all = store
            .docking_occurrences(handle, None, None)
            .await
            .expect("aggregate docking occurrences");
        assert_eq!(
            all.total(),
            2,
            "one summarised raw episode + one later stow"
        );
        assert_eq!(
            all.unknown, 0,
            "a matching raw anchor supplies the burst's landing area"
        );
        assert_eq!(
            all.landing_areas
                .iter()
                .find(|bucket| bucket.value == "Hangar_Large_01")
                .map(|bucket| bucket.count),
            Some(1),
            "the raw run keeps its first landing area"
        );
        assert_eq!(
            all.landing_areas
                .iter()
                .find(|bucket| bucket.value == "Pad_Medium_01")
                .map(|bucket| bucket.count),
            Some(1),
            "the stow after the 120-second episode ceiling starts a second occurrence"
        );

        let tail_only = store
            .docking_occurrences(
                handle,
                Some(base + chrono::Duration::milliseconds(500)),
                Some(base + chrono::Duration::seconds(2)),
            )
            .await
            .expect("aggregate boundary-crossing raw run");
        assert_eq!(
            tail_only.total(),
            0,
            "a window containing only the tail member must not count the occurrence again"
        );

        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean up docking probe rows");
    }

    /// Executes the `until` upper bound through REAL Postgres for the
    /// three query methods no other gated test reaches, plus the
    /// `has_events_in_window` EXISTS probe.
    ///
    /// Why this exists: `until` was threaded into four hand-written SQL
    /// statements, and only `objective_outcomes` had an existing
    /// round-trip test. The other three were exercised solely against
    /// `MemoryQuery`, which shares none of the SQL — so a wrong `$n`
    /// placeholder, a bound parameter in the wrong order, or a predicate
    /// attached to the wrong clause would compile, pass the whole
    /// offline suite, and fail only in production.
    ///
    /// The bound is asserted as HALF-OPEN `[since, until)` on every
    /// method: an event exactly on `until` must be excluded and one
    /// exactly on `since` included, so two adjacent windows tile without
    /// double-counting the shared edge. That property is what makes
    /// `current + previous` sum to the true total.
    #[tokio::test]
    async fn window_bounds_round_trip_on_real_postgres() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping window-bounds round-trip test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        // Unique handle: other gated tests share this database and may run
        // concurrently.
        let handle = "windowbounds_roundtrip_probe";
        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean probe rows");

        // Three fixed instants. `edge` is the boundary the two windows
        // share: older = [start, edge), newer = [edge, end).
        let start = Utc::now() - chrono::Duration::hours(10);
        let edge = Utc::now() - chrono::Duration::hours(5);
        let end = Utc::now() + chrono::Duration::hours(1);

        // (timestamp, amount) — one row strictly before the edge, one
        // exactly ON it. The on-edge row is the whole point: it must land
        // in the NEWER window and never in both.
        let rows: [(DateTime<Utc>, i64); 2] = [(start, 100), (edge, 40)];
        for (i, (ts, amount)) in rows.iter().enumerate() {
            sqlx::query(
                "INSERT INTO events
                     (id, idempotency_key, claimed_handle, event_type,
                      event_timestamp, log_source, source_offset, raw_line, payload)
                 VALUES (gen_random_uuid(), $1, $2, 'shop_purchase', $3,
                         'live', $4, '', $5::jsonb)",
            )
            .bind(format!("windowbounds-probe-{i}"))
            .bind(handle)
            .bind(*ts)
            .bind(i as i64)
            .bind(format!(
                r#"{{"shop_name":"SCShop_Probe","amount":{amount}}}"#
            ))
            .execute(&pool)
            .await
            .expect("insert probe row");
        }

        let store = PostgresStore::new(pool.clone());

        // --- count_event_type -------------------------------------------
        let older = store
            .count_event_type(handle, "shop_purchase", None, Some(start), Some(edge))
            .await
            .expect("count older");
        let newer = store
            .count_event_type(handle, "shop_purchase", None, Some(edge), Some(end))
            .await
            .expect("count newer");
        assert_eq!(older, 1, "count: [start, edge) excludes the on-edge row");
        assert_eq!(newer, 1, "count: [edge, end) includes the on-edge row");
        let total = store
            .count_event_type(handle, "shop_purchase", None, Some(start), Some(end))
            .await
            .expect("count total");
        assert_eq!(older + newer, total, "count: adjacent windows must tile");

        // --- payload_numeric_sum ----------------------------------------
        let older_sum = store
            .payload_numeric_sum(handle, "shop_purchase", "amount", Some(start), Some(edge))
            .await
            .expect("sum older");
        let newer_sum = store
            .payload_numeric_sum(handle, "shop_purchase", "amount", Some(edge), Some(end))
            .await
            .expect("sum newer");
        assert_eq!(
            older_sum, 100,
            "sum: [start, edge) excludes the on-edge row"
        );
        assert_eq!(newer_sum, 40, "sum: [edge, end) includes the on-edge row");

        // --- payload_field_breakdown ------------------------------------
        let older_buckets = store
            .payload_field_breakdown(
                handle,
                "shop_purchase",
                "shop_name",
                None,
                Some(start),
                Some(edge),
                100,
            )
            .await
            .expect("breakdown older");
        let newer_buckets = store
            .payload_field_breakdown(
                handle,
                "shop_purchase",
                "shop_name",
                None,
                Some(edge),
                Some(end),
                100,
            )
            .await
            .expect("breakdown newer");
        assert_eq!(
            older_buckets.iter().map(|b| b.count).sum::<i64>(),
            1,
            "breakdown: [start, edge) excludes the on-edge row"
        );
        assert_eq!(
            newer_buckets.iter().map(|b| b.count).sum::<i64>(),
            1,
            "breakdown: [edge, end) includes the on-edge row"
        );

        // --- has_events_in_window (the previous-period activity probe) ---
        assert!(
            store
                .has_events_in_window(handle, start, edge)
                .await
                .expect("probe older"),
            "probe: must see the row in [start, edge)"
        );
        assert!(
            !store
                .has_events_in_window(
                    handle,
                    start - chrono::Duration::hours(48),
                    start - chrono::Duration::hours(24),
                )
                .await
                .expect("probe empty"),
            "probe: a window with no events must be false — this is what \
             distinguishes 'was not a user yet' from 'did nothing', and a \
             probe stuck on true would fabricate a comparison for every \
             brand-new player"
        );

        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .expect("clean up probe rows");
    }
}
