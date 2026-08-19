//! Cross-session entity rollup endpoints with sharing-grant auth.
//!
//! Endpoints:
//!   - `GET /v1/users/{handle}/entities`
//!   - `GET /v1/users/{handle}/entities/{kind}/{id}`
//!
//! The per-session timeline (see [`event_timeline`]) answers "what
//! happened during this session". This module answers the orthogonal
//! question "everything that ever happened to my Cutlass" — the same
//! events, aggregated by `metadata->primary_entity` instead of by the
//! `ProcessInit` boundary.
//!
//! Auth posture mirrors [`event_timeline`] exactly:
//!   * The owner viewing their own data (`auth.preferred_username`
//!     == `handle`, case-insensitive) is always allowed.
//!   * Anyone else must have an active row in `share_metadata`
//!     where `owner_handle = handle`, `recipient_handle = caller`,
//!     `share_event_timeline = TRUE`, and `expires_at` is either
//!     NULL or in the future. Missing / FALSE / expired grant
//!     produces a 403 with `share_event_timeline_not_granted`.
//!
//! ## Index dependencies (migration 0030)
//!
//! Both endpoints rely on `events_metadata_entity`:
//!   `((metadata->'primary_entity'->>'kind'), (metadata->'primary_entity'->>'id'))`
//! Partial WHERE `metadata IS NOT NULL`. This makes the per-entity
//! WHERE clause a B-tree range scan instead of a sequential scan.
//!
//! The list endpoint's GROUP BY is `O(distinct (kind, id))` after the
//! index sort — for a typical user that's a few dozen distinct
//! entities, fine in a single round-trip.
//!
//! ## Why not extract a shared helper with `event_timeline`
//!
//! `event_timeline.rs::caller_may_view_timeline` and `validate_handle`
//! are duplicated below. The shared module is in active churn for the
//! Phase 1/2 backfill work and a shared helper module would force a
//! cross-file touch on every iteration. Duplicate is the path that
//! lets both modules evolve independently; once the audit-v2 sweep
//! closes, fold them.
//!
//! ## Session breakdown cost
//!
//! The per-entity endpoint returns a `session_breakdown` aggregation.
//! Computing it requires deriving session bounds the same way
//! [`event_timeline::list_sessions`] does — a sequential walk of every
//! event row for the user, ordered by `(source_offset, idempotency_key)`,
//! opening a fresh session on each `process_init`. There is no
//! session-id column on the events table, so the walk is the only way
//! to know which session bucket an entity's events fall into.
//!
//! Cost: O(total events for the user) for the session-bounds derivation
//! plus O(events matching the entity) via the JSONB index. For a heavy
//! user this is roughly the same cost as a single
//! [`event_timeline::list_sessions`] call; both endpoints accept this
//! cost today. A future migration could denormalize `session_id` onto
//! each event row at ingest time, collapsing the bounds walk to a
//! direct GROUP BY — see follow-ups for the migration sketch.
//!
//! ## Session-bounds hard cap
//!
//! The session-bounds walk is bounded by [`SESSION_BOUNDS_HARD_CAP`]
//! rows per request. Mirrors the [`ENTITIES_LIST_HARD_CAP`] posture: a
//! pathological event volume must not turn one request into an
//! unbounded sequential scan. On saturation the response degrades
//! silently — `session_count` and `session_breakdown` may be partial,
//! and a `tracing::warn!` records the cap hit so operators can spot
//! affected users.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::location_catalog_cache::LocationCatalogCache;
use async_trait::async_trait;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Extension, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use starstats_core::metadata::EventMetadata;
use starstats_core::wire::{EventEnvelope, LogSource};
use starstats_core::LocationCatalog;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Default cap on the entity-list response. Distinct entity counts
/// per user are small in practice (dozens, not thousands), but we
/// document a default so clients have a predictable page boundary.
pub const ENTITIES_LIST_LIMIT_DEFAULT: i64 = 100;
/// Hard cap on the entity-list response per request. Mirrors the
/// posture of the session-list cap — a heavy user's profile open
/// must stay bounded.
pub const ENTITIES_LIST_LIMIT_MAX: i64 = 200;
/// Absolute cap on entity rows materialised across all pages. If a
/// user somehow has more than this many distinct entities, we warn-log
/// and return `next_after = None` (saturation, not a real cursor).
pub const ENTITIES_LIST_HARD_CAP: i64 = 10_000;

/// Default page size for the per-entity events endpoint. Matches the
/// per-session events endpoint default so the two surfaces feel
/// consistent and share a connection-pool budget.
pub const ENTITY_EVENTS_LIMIT_DEFAULT: i64 = 10_000;
/// Hard cap on the per-entity events endpoint regardless of caller.
pub const ENTITY_EVENTS_LIMIT_MAX: i64 = 10_000;

/// Hard cap on the per-request session-bounds row materialisation. The
/// bounds walk is the only sequential scan in this module — a heavy
/// user with hundreds of thousands of events otherwise turns a single
/// request into a near-full-table scan. On saturation we return what
/// we have and warn-log; `session_count` may under-report and the
/// `session_breakdown` may be truncated rather than failing the
/// endpoint.
pub const SESSION_BOUNDS_HARD_CAP: i64 = 500_000;

/// Closed vocabulary of entity kinds — mirrors `EntityKind` in
/// `starstats-core::metadata`. Kept as a `const` slice (rather than
/// importing the enum) so this module doesn't drag a `Deserialize`
/// dependency through the path validator; the validator just does
/// string membership.
pub const VALID_ENTITY_KINDS: &[&str] = &[
    "player", "vehicle", "item", "location", "shop", "mission", "session", "system",
];

// -- Wire DTOs -------------------------------------------------------

/// One row in the entity list — aggregations across all of `handle`'s
/// events for a particular `(kind, id)`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntitySummary {
    /// `EntityKind` discriminator value, snake_case. See
    /// [`VALID_ENTITY_KINDS`].
    pub kind: String,
    /// Stable identifier (handle, GEID, UUID, …). May be `"unknown"`
    /// when the source line lacked one — that's deliberate so unknown
    /// entities of the same kind collapse into a single rollup row
    /// rather than fanning out.
    pub id: String,
    /// Latest `metadata.primary_entity.display_name` seen for this
    /// entity. Falls back to `id` when no event carried a non-empty
    /// display name.
    pub display_name: String,
    /// Total number of events for this entity across the entire event
    /// history.
    pub event_count: u32,
    /// Earliest `event_timestamp` seen for any event tagged with this
    /// entity. RFC3339 UTC.
    pub first_seen: Option<String>,
    /// Latest `event_timestamp` seen for any event tagged with this
    /// entity. RFC3339 UTC. Sort key — list responses come back
    /// `last_seen DESC`.
    pub last_seen: Option<String>,
    /// Number of distinct sessions the entity appears in. Derived by
    /// the session-bounds walk; see module doc for the cost note.
    pub session_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntitiesListResponse {
    pub entities: Vec<EntitySummary>,
    /// Cursor for the next page; pass back as `after`. `None` when
    /// the response exhausted the entity set OR the hard cap fired
    /// (saturation — see [`ENTITIES_LIST_HARD_CAP`]).
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct EntitiesListQuery {
    /// Cursor — return entities strictly after this opaque token.
    /// Encodes a `(last_seen, kind, id)` tuple as base64; the client
    /// just echoes whatever the server returned.
    pub after: Option<String>,
    /// Page size cap. Clamped to [`ENTITIES_LIST_LIMIT_MAX`]; defaults
    /// to [`ENTITIES_LIST_LIMIT_DEFAULT`] when absent.
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct EntityEventsQuery {
    /// Cursor — return events strictly after this `idempotency_key`
    /// in lexicographic order. Pass the `next_after` returned by
    /// the previous call.
    pub after: Option<String>,
    /// Page size cap. Clamped to [`ENTITY_EVENTS_LIMIT_MAX`].
    pub limit: Option<i64>,
}

/// One row in the per-entity `session_breakdown` array. Lets the UI
/// show which sessions an entity surfaced in without re-running the
/// derivation client-side.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntitySessionBucket {
    /// Session identifier (matches `event_timeline::SessionSummary::id`).
    pub session_id: String,
    /// Timestamp of the `ProcessInit` row that opened the session.
    pub started_at: Option<String>,
    /// Number of events tagged with this entity that fell inside the
    /// session bounds.
    pub event_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityHistoryResponse {
    pub kind: String,
    pub id: String,
    pub display_name: String,
    pub events: Vec<EventEnvelope>,
    pub next_after: Option<String>,
    pub session_breakdown: Vec<EntitySessionBucket>,
}

/// OpenAPI mirror of [`EntityHistoryResponse`]. The wire shape is the
/// `serde`-derived layout on the runtime type; this schema type only
/// exists so utoipa can derive `ToSchema` without pulling the wire
/// `EventEnvelope` from the `starstats-core` crate into a `ToSchema`
/// constraint.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EntityHistoryResponseSchema {
    pub kind: String,
    pub id: String,
    pub display_name: String,
    /// Tagged-union `EventEnvelope` (same shape as
    /// `SessionEventsResponseSchema::events`).
    #[schema(value_type = Vec<serde_json::Value>)]
    pub events: Vec<serde_json::Value>,
    pub next_after: Option<String>,
    pub session_breakdown: Vec<EntitySessionBucket>,
}

// -- Store trait + Postgres impl ------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum EntityRollupStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, EntityRollupStoreError>;

/// Raw row returned by the per-entity event projection. Stays internal
/// — the store hands [`StoredEntityEvent`]s back so the route layer
/// doesn't need to know about the underlying column shape.
///
/// `event_type` is projected because the wire envelope's serde
/// reconstruction reads it via the payload's `type` discriminator, but
/// the column-form is also kept in the row so future logging /
/// debugging paths can use it without re-deserialising the payload.
#[derive(Debug, Clone)]
pub struct StoredEntityEvent {
    pub idempotency_key: String,
    pub raw_line: String,
    #[allow(dead_code)]
    pub event_type: String,
    pub log_source: String,
    pub source_offset: i64,
    pub payload: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    /// Raw tray-stamped location column. Deliberately not read — the
    /// rollup re-derives it server-side from the payload per F4 (see
    /// `envelope_from_row`). Kept for DB fidelity / diagnostics.
    #[allow(dead_code)]
    pub resolved_location: Option<serde_json::Value>,
}

/// Row used by the session-bounds derivation. Same shape as
/// `event_timeline.rs::bounds_rows` but redeclared so the entity
/// module doesn't reach into the timeline module's privates.
///
/// `source_offset` / `idempotency_key` are projected so the in-memory
/// store can sort the input deterministically and so a future
/// debugger can identify the row; the bounds walk doesn't read them
/// directly (it walks the already-sorted slice).
#[derive(Debug, Clone)]
pub struct SessionBoundsRow {
    pub event_type: String,
    #[allow(dead_code)]
    pub source_offset: i64,
    #[allow(dead_code)]
    pub idempotency_key: String,
    pub event_timestamp: Option<DateTime<Utc>>,
    pub meta_id: Option<String>,
    pub payload_local_session: Option<String>,
}

#[async_trait]
pub trait EntityRollupStore: Send + Sync + 'static {
    /// `caller` (case-insensitive) equals `handle` OR has an active
    /// `share_metadata` row with `share_event_timeline = TRUE`.
    async fn caller_may_view(&self, handle: &str, caller: &str) -> Result<bool>;

    /// GROUP BY `(metadata->'primary_entity'->>'kind',
    ///           metadata->'primary_entity'->>'id')` for the handle.
    /// Returns `Vec<EntitySummary>` (NOT paginated — the caller
    /// applies cursor + limit on the in-memory list because the
    /// `session_count` aggregation requires the session-bounds walk
    /// which is keyed on `handle`, not on the cursor position).
    async fn list_entities(&self, handle: &str) -> Result<Vec<EntitySummary>>;

    /// Fetch all events tagged with this `(kind, id)` for the handle,
    /// ordered by `(source_offset, idempotency_key)`, optionally after
    /// a cursor, capped at `limit`.
    async fn entity_events(
        &self,
        handle: &str,
        kind: &str,
        id: &str,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredEntityEvent>>;

    /// Fetch every event row for the handle (just the fields needed
    /// for the session-bounds walk). Same projection as
    /// `event_timeline.rs::list_sessions`.
    async fn session_bounds_rows(&self, handle: &str) -> Result<Vec<SessionBoundsRow>>;

    /// Freshen `entity_rollup_agg` for `handle` (rebuild iff dirty). Delegates
    /// to the shared session-stats rebuild, which materializes the session,
    /// records and entity rollups together under one advisory lock.
    async fn ensure_rollup_fresh(&self, handle: &str) -> Result<()>;

    /// Per-ID distinct-session counts from the materialized `entity_rollup_agg`,
    /// keyed `(String::new(), id)` to match the route fold's id-only lookup.
    /// Empty on a cold/failed rollup so the caller falls back to the live walk.
    async fn entity_session_counts(
        &self,
        handle: &str,
    ) -> Result<std::collections::HashMap<(String, String), u32>>;
}

pub struct PostgresEntityRollupStore {
    pool: PgPool,
}

impl PostgresEntityRollupStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EntityRollupStore for PostgresEntityRollupStore {
    async fn ensure_rollup_fresh(&self, handle: &str) -> Result<()> {
        // The session-stats rebuild (repo.rs) materializes entity_rollup_agg
        // alongside session_summary/character_records. Constructing a
        // PostgresStore over the same pool is cheap (it just wraps the pool).
        crate::repo::PostgresStore::new(self.pool.clone())
            .ensure_session_stats_fresh(handle)
            .await
            .map_err(|e| match e {
                crate::repo::RepoError::Database(dbe) => EntityRollupStoreError::Database(dbe),
            })?;
        Ok(())
    }

    async fn entity_session_counts(
        &self,
        handle: &str,
    ) -> Result<std::collections::HashMap<(String, String), u32>> {
        // entity_rollup_agg stores the id's kind-agnostic total on every
        // (kind, id) row (see the rebuild's LEFT JOIN); MAX per id collapses
        // that to one count. `HAVING > 0` omits entities that never appear in a
        // session, matching derive_entity_session_data (which never records a
        // zero) — the route fold's unwrap_or(0) yields 0 for the omitted ids.
        // Keyed (String::new(), id) for the route fold.
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT entity_id, MAX(session_count)::bigint
             FROM entity_rollup_agg
             WHERE claimed_handle = lower($1)
             GROUP BY entity_id
             HAVING MAX(session_count) > 0",
        )
        .bind(handle)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, c)| ((String::new(), id), c.max(0) as u32))
            .collect())
    }

    async fn caller_may_view(&self, handle: &str, caller: &str) -> Result<bool> {
        if caller.eq_ignore_ascii_case(handle) {
            return Ok(true);
        }
        let row: Option<(bool,)> = sqlx::query_as(
            r#"
            SELECT share_event_timeline
            FROM share_metadata
            WHERE lower(owner_handle) = lower($1)
              AND lower(recipient_handle) = lower($2)
              AND (expires_at IS NULL OR expires_at > NOW())
            LIMIT 1
            "#,
        )
        .bind(handle)
        .bind(caller)
        .fetch_optional(&self.pool)
        .await?;
        Ok(matches!(row, Some((true,))))
    }

    async fn list_entities(&self, handle: &str) -> Result<Vec<EntitySummary>> {
        // GROUP BY on the JSONB-projected (kind, id) tuple. The
        // `events_metadata_entity` partial index (migration 0030)
        // covers both columns; we filter `metadata IS NOT NULL` to
        // match the index's WHERE clause so the planner can use it.
        //
        // `display_name` aggregation: pick the most recently observed
        // value via `(array_agg(... ORDER BY source_offset DESC))[1]`.
        // PostgreSQL doesn't have a `last()` aggregate; the array_agg
        // pattern is the canonical workaround. Result is the latest
        // non-empty display_name, falling back to id at the route
        // layer when every captured display_name was empty.
        //
        // `session_count` is intentionally NOT computed in SQL — see
        // the module doc. We fill it in at the route layer using the
        // session-bounds walk.
        let query_result = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                i64,
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
            ),
        >(
            r#"
            SELECT
                metadata->'primary_entity'->>'kind' AS kind,
                metadata->'primary_entity'->>'id' AS id,
                (array_agg(NULLIF(metadata->'primary_entity'->>'display_name', '')
                           ORDER BY source_offset DESC))[1] AS display_name,
                COUNT(*)::BIGINT AS event_count,
                MIN(event_timestamp) AS first_seen,
                MAX(event_timestamp) AS last_seen
            FROM events
            WHERE claimed_handle = lower($1)
              AND metadata IS NOT NULL
              AND metadata->'primary_entity'->>'kind' IS NOT NULL
              AND metadata->'primary_entity'->>'id' IS NOT NULL
            GROUP BY metadata->'primary_entity'->>'kind',
                     metadata->'primary_entity'->>'id'
            ORDER BY MAX(event_timestamp) DESC NULLS LAST,
                     metadata->'primary_entity'->>'kind' ASC,
                     metadata->'primary_entity'->>'id' ASC
            LIMIT $2
            "#,
        )
        .bind(handle)
        .bind(ENTITIES_LIST_HARD_CAP)
        .fetch_all(&self.pool)
        .await;

        let rows = match query_result {
            Ok(rows) => rows,
            Err(ref e) if is_metadata_column_missing(e) => {
                // Pre-migration database. Match the per-entity path
                // (`event_timeline` mirror): degrade to empty rather
                // than 500.
                tracing::warn!(
                    handle = %handle,
                    "metadata column missing — list_entities returning empty"
                );
                return Ok(Vec::new());
            }
            Err(e) => return Err(e.into()),
        };

        Ok(rows
            .into_iter()
            .map(
                |(kind, id, display_name, event_count, first_seen, last_seen)| {
                    let resolved_name = display_name
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| id.clone());
                    EntitySummary {
                        kind,
                        id,
                        display_name: resolved_name,
                        event_count: event_count.max(0) as u32,
                        first_seen: first_seen.map(|t| t.to_rfc3339()),
                        last_seen: last_seen.map(|t| t.to_rfc3339()),
                        session_count: 0,
                    }
                },
            )
            .collect())
    }

    async fn entity_events(
        &self,
        handle: &str,
        kind: &str,
        id: &str,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredEntityEvent>> {
        // The JSONB index `events_metadata_entity` makes the WHERE
        // clause a range scan; combined with the (kind, id) filter the
        // planner reads only the rows that match the entity. Cursor
        // applies on idempotency_key for stable pagination.
        let rows: Vec<(
            String,
            String,
            String,
            String,
            i64,
            serde_json::Value,
            Option<serde_json::Value>,
            Option<serde_json::Value>,
        )> = sqlx::query_as(
            r#"
            SELECT
                idempotency_key,
                raw_line,
                event_type,
                log_source,
                source_offset,
                payload,
                metadata,
                resolved_location
            FROM events
            WHERE claimed_handle = lower($1)
              AND metadata IS NOT NULL
              AND metadata->'primary_entity'->>'kind' = $2
              AND metadata->'primary_entity'->>'id' = $3
              AND ($4::TEXT IS NULL OR idempotency_key > $4::TEXT)
            ORDER BY source_offset ASC, idempotency_key ASC
            LIMIT $5
            "#,
        )
        .bind(handle)
        .bind(kind)
        .bind(id)
        .bind(after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(idem, raw, ty, src, off, payload, metadata, resolved_location)| {
                    StoredEntityEvent {
                        idempotency_key: idem,
                        raw_line: raw,
                        event_type: ty,
                        log_source: src,
                        source_offset: off,
                        payload,
                        metadata,
                        resolved_location,
                    }
                },
            )
            .collect())
    }

    async fn session_bounds_rows(&self, handle: &str) -> Result<Vec<SessionBoundsRow>> {
        let rows: Vec<(
            String,
            i64,
            String,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT
                event_type,
                source_offset,
                idempotency_key,
                event_timestamp,
                (metadata->'primary_entity'->>'id') AS meta_id,
                (payload->>'local_session') AS payload_local_session
            FROM events
            WHERE claimed_handle = lower($1)
              -- An event with no timestamp cannot be attributed to a
              -- session; dropping it here is honest, and keeps the sort
              -- key index-compatible (see below).
              AND event_timestamp IS NOT NULL
            -- (event_timestamp, idempotency_key), NEVER source_offset:
            -- the tray resets source_offset to 0 on every Game.log
            -- rotation, so offset-ordering interleaves launches and
            -- collapses per-entity session counts. This also makes the
            -- LIMIT below a coherent chronological prefix instead of an
            -- arbitrary cross-launch slice.
            --
            -- Default (NULLS LAST) ordering is deliberate: it matches
            -- events_handle_ts_idx, so this plans as an Index Scan +
            -- Incremental Sort. Measured on PG17 @200k rows: 172 buffers
            -- here vs 5496 for an otherwise-identical NULLS FIRST sort,
            -- which cannot use the ASC index and falls back to a seq
            -- scan + top-N heapsort.
            ORDER BY event_timestamp ASC, idempotency_key ASC
            LIMIT $2
            "#,
        )
        .bind(handle)
        .bind(SESSION_BOUNDS_HARD_CAP)
        .fetch_all(&self.pool)
        .await?;

        if rows.len() == SESSION_BOUNDS_HARD_CAP as usize {
            tracing::warn!(
                handle = %handle,
                cap = SESSION_BOUNDS_HARD_CAP,
                "session_bounds query hit hard cap; session_breakdown may be incomplete"
            );
        }

        Ok(rows
            .into_iter()
            .map(|(ty, so, idem, ts, meta_id, ls)| SessionBoundsRow {
                event_type: ty,
                source_offset: so,
                idempotency_key: idem,
                event_timestamp: ts,
                meta_id,
                payload_local_session: ls,
            })
            .collect())
    }
}

// -- Router ----------------------------------------------------------

pub fn routes(store: Arc<PostgresEntityRollupStore>) -> Router {
    let store_dyn: Arc<dyn EntityRollupStore> = store;
    Router::new()
        .route("/v1/users/:handle/entities", get(list_entities))
        .route("/v1/users/:handle/entities/:kind/:id", get(entity_history))
        .with_state(store_dyn)
}

// -- Helpers ---------------------------------------------------------

fn err(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

// M-S3: single source of truth in `crate::validation`.
use crate::validation::validate_handle;

fn validate_kind(kind: &str) -> bool {
    VALID_ENTITY_KINDS.contains(&kind)
}

/// Mirrors `event_timeline::is_metadata_column_missing`. Detects the
/// pre-migration "metadata column does not exist" error so the list
/// query can degrade to empty instead of returning 500. Per the
/// module doc, we duplicate rather than share with `event_timeline`.
fn is_metadata_column_missing(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db_err) => {
            // Postgres SQLSTATE 42703 = undefined_column.
            db_err.code().as_deref() == Some("42703")
        }
        _ => false,
    }
}

/// Entity ids are free-form but bounded — a 256-char cap rejects URL
/// abuse and accidental dumps without rejecting any legitimate GEID
/// or UUID identifier.
fn validate_entity_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256
}

/// Encode a `(last_seen_iso, kind, id)` tuple as an opaque base64
/// cursor. The wire format is `"<last_seen>|<kind>|<id>"` URL-safe
/// base64-encoded with no padding — the separator is `|` which is
/// not in the base64 alphabet, so the decode is unambiguous.
fn encode_cursor(last_seen: &str, kind: &str, id: &str) -> String {
    let payload = format!("{}|{}|{}", last_seen, kind, id);
    URL_SAFE_NO_PAD.encode(payload.as_bytes())
}

/// Inverse of [`encode_cursor`]. Returns `None` for any malformed
/// cursor — callers treat that as "no cursor" rather than a 400, so
/// a stale client doesn't get stuck at a hard-error after a deploy.
fn decode_cursor(token: &str) -> Option<(String, String, String)> {
    let bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
    let s = String::from_utf8(bytes).ok()?;
    let mut parts = s.splitn(3, '|');
    let last_seen = parts.next()?.to_owned();
    let kind = parts.next()?.to_owned();
    let id = parts.next()?.to_owned();
    Some((last_seen, kind, id))
}

/// Apply the cursor to an already-sorted entity list. The list comes
/// back from the store ordered by `(last_seen DESC, kind ASC, id ASC)`;
/// the cursor is the LAST entity returned by the previous page, so we
/// skip past it on the next call.
fn apply_cursor(entities: Vec<EntitySummary>, after: Option<&str>) -> Vec<EntitySummary> {
    let Some(token) = after else {
        return entities;
    };
    let Some((cur_last, cur_kind, cur_id)) = decode_cursor(token) else {
        return entities;
    };
    entities
        .into_iter()
        .skip_while(|e| {
            let last = e.last_seen.clone().unwrap_or_default();
            // Match the SQL ordering tuple. "Strictly after" the
            // cursor means: (last < cur_last) OR (last == cur_last
            // AND (kind, id) <= (cur_kind, cur_id)).
            //
            // We compare strings; ISO timestamps sort correctly as
            // text. The store handed back DESC by last_seen, ASC by
            // (kind, id), so the in-memory walk must match.
            if last > cur_last {
                return true;
            }
            if last < cur_last {
                return false;
            }
            // Equal last_seen: compare (kind, id) ASC.
            match e.kind.as_str().cmp(cur_kind.as_str()) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => e.id.as_str() <= cur_id.as_str(),
            }
        })
        .collect()
}

/// Derive `session_count` for every entity AND the per-entity session
/// breakdown from a single walk of the bounds-rows. Returns a map
/// keyed on `(kind, id)` → `Vec<EntitySessionBucket>` plus a separate
/// map of `(kind, id)` → distinct-session-count. Pure function so the
/// unit tests cover the boundary logic without a live database.
fn derive_entity_session_data(
    rows: &[SessionBoundsRow],
) -> (
    std::collections::HashMap<(String, String), u32>,
    std::collections::HashMap<(String, String), Vec<EntitySessionBucket>>,
) {
    use std::collections::HashMap;
    let mut counts: HashMap<(String, String), u32> = HashMap::new();
    let mut breakdowns: HashMap<(String, String), Vec<EntitySessionBucket>> = HashMap::new();

    // Walk in (event_timestamp, idempotency_key) order — NEVER by
    // source_offset. The tray resets source_offset to 0 on every
    // Game.log rotation, so offset-ordering a handle's whole history
    // interleaves launches (every launch's offset-0 row sorts together)
    // and collapses per-entity session counts. Same rule the timeline's
    // bounds query documents. Sorted here rather than trusted from the
    // caller's ORDER BY so the precondition holds for every caller.
    //
    // `None` timestamps sort FIRST (Rust's `Option` ordering): an event
    // with no timestamp can't be attributed to a session, and landing
    // before the first `process_init` means it is skipped rather than
    // mis-attributed to whichever session happens to be open at the end.
    // The SQL drops such rows outright, so this only guards other
    // callers — the two agree that a timestamp-less event never counts.
    let mut ordered: Vec<&SessionBoundsRow> = rows.iter().collect();
    ordered.sort_by(|a, b| {
        a.event_timestamp
            .cmp(&b.event_timestamp)
            .then_with(|| a.idempotency_key.cmp(&b.idempotency_key))
    });

    // Track the currently-open session id + started_at + per-entity
    // tally for the active session.
    let mut current_id: Option<String> = None;
    let mut current_started: Option<DateTime<Utc>> = None;
    let mut current_tally: HashMap<(String, String), u32> = HashMap::new();

    for row in ordered {
        let row_kind_id = row.meta_id.clone();
        // Open / close session bookkeeping.
        if row.event_type == "process_init" {
            // Close prior session.
            if let Some(prev_id) = current_id.take() {
                flush_session(
                    &prev_id,
                    current_started.take(),
                    std::mem::take(&mut current_tally),
                    &mut counts,
                    &mut breakdowns,
                );
            }
            // Open new session. ID comes from metadata first, then
            // payload->>local_session fallback.
            let new_id = row
                .meta_id
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| row.payload_local_session.clone().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "unknown".to_string());
            current_id = Some(new_id);
            current_started = row.event_timestamp;
            // The process_init row IS an event tagged with the
            // session entity (kind=session, id=<the session id>). The
            // entity rollup counts it — fall through to the generic
            // tally below.
        } else if row.event_type == "session_end" {
            // session_end belongs to the active session; tally it
            // before flushing.
            // We don't know which entity tag the row carries without
            // re-querying — the bounds row projection only includes
            // meta_id. Skip tagging and just flush.
            if let Some(prev_id) = current_id.take() {
                flush_session(
                    &prev_id,
                    current_started.take(),
                    std::mem::take(&mut current_tally),
                    &mut counts,
                    &mut breakdowns,
                );
            }
            continue;
        }

        // Mid-session event. Tally it against the active session iff
        // we know which entity it was tagged for AND a session is open.
        if current_id.is_some() {
            if let Some(meta_id) = row_kind_id {
                // The SessionBoundsRow projection only carries the
                // entity id (not the kind). For the session-count
                // aggregation we need both — but the only consumers
                // are the entity list (which already has kind+id from
                // the GROUP BY) and the per-entity history endpoint
                // (which takes kind+id as path params). We tally on
                // id alone and let the caller key by (kind, id) using
                // their kind context.
                let key = (String::new(), meta_id);
                *current_tally.entry(key).or_insert(0) += 1;
            }
        }
    }

    // Flush final session.
    if let Some(prev_id) = current_id.take() {
        flush_session(
            &prev_id,
            current_started.take(),
            current_tally,
            &mut counts,
            &mut breakdowns,
        );
    }

    (counts, breakdowns)
}

fn flush_session(
    session_id: &str,
    started_at: Option<DateTime<Utc>>,
    tally: std::collections::HashMap<(String, String), u32>,
    counts: &mut std::collections::HashMap<(String, String), u32>,
    breakdowns: &mut std::collections::HashMap<(String, String), Vec<EntitySessionBucket>>,
) {
    for (key, n) in tally {
        if n == 0 {
            continue;
        }
        *counts.entry(key.clone()).or_insert(0) += 1;
        let bucket = EntitySessionBucket {
            session_id: session_id.to_string(),
            started_at: started_at.map(|t| t.to_rfc3339()),
            event_count: n,
        };
        breakdowns.entry(key).or_default().push(bucket);
    }
}

/// Reconstruct an `EventEnvelope` from a [`StoredEntityEvent`]. Same
/// shape as `event_timeline.rs::envelope_from_columns`; duplicated
/// rather than imported per the module-doc note.
fn envelope_from_row(row: StoredEntityEvent, catalog: &LocationCatalog) -> EventEnvelope {
    // Re-derive the location server-side from the event's own payload;
    // never echo the untrusted stored `resolved_location`, which the web
    // renders as a KB link and a client could spoof (F4). Computed before
    // `payload` is moved into `from_value` below. Rollup rows don't select
    // a timestamp, but the classification doesn't need one.
    let resolved_location =
        crate::query::derive_resolved_location(&row.event_type, &row.payload, None, catalog);
    let parsed_event = serde_json::from_value(row.payload).ok();
    let parsed_source = match row.log_source.as_str() {
        "live" => LogSource::Live,
        "ptu" => LogSource::Ptu,
        "eptu" => LogSource::Eptu,
        "hotfix" => LogSource::Hotfix,
        "tech" => LogSource::Tech,
        _ => LogSource::Other,
    };
    let parsed_metadata: Option<EventMetadata> = row
        .metadata
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    EventEnvelope {
        idempotency_key: row.idempotency_key,
        raw_line: row.raw_line,
        event: parsed_event,
        source: parsed_source,
        source_offset: row.source_offset.max(0) as u64,
        metadata: parsed_metadata,
        resolved_location,
    }
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/users/{handle}/entities",
    tag = "entity-rollup",
    operation_id = "entity_rollup_list_entities",
    params(
        ("handle" = String, Path, description = "Owner RSI handle"),
        EntitiesListQuery,
    ),
    responses(
        (status = 200, description = "Cross-session entity rollup for the user", body = EntitiesListResponse),
        (status = 400, description = "Malformed handle", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller has no share_event_timeline grant", body = ApiErrorBody),
        (status = 500, description = "Query failed", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_entities(
    State(store): State<Arc<dyn EntityRollupStore>>,
    auth: AuthenticatedUser,
    Path(handle): Path<String>,
    Query(params): Query<EntitiesListQuery>,
) -> Response {
    if !validate_handle(&handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    match store
        .caller_may_view(&handle, &auth.preferred_username)
        .await
    {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "share_event_timeline_not_granted"),
        Err(e) => {
            tracing::error!(error = %e, "share grant lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "grant_lookup_failed");
        }
    }

    let limit = params
        .limit
        .unwrap_or(ENTITIES_LIST_LIMIT_DEFAULT)
        .clamp(1, ENTITIES_LIST_LIMIT_MAX);

    let entities = match store.list_entities(&handle).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "entities list query failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };
    let saturated = entities.len() as i64 >= ENTITIES_LIST_HARD_CAP;
    if saturated {
        tracing::warn!(
            handle = %handle,
            cap = ENTITIES_LIST_HARD_CAP,
            "entity rollup hit hard cap; cursor will report no further pages"
        );
    }

    // Fold session counts in (A4): read the materialized entity_rollup_agg
    // (freshened on dirty) instead of the O(history) session-bounds walk. The
    // walk stays the fallback on a cold/empty/failed rollup, so the endpoint is
    // never wrong — just slower on the first read after a change.
    if let Err(e) = store.ensure_rollup_fresh(&handle).await {
        tracing::warn!(error = %e, handle = %handle, "entity rollup freshen failed; using live fallback");
    }
    let session_counts = match store.entity_session_counts(&handle).await {
        Ok(counts) if !counts.is_empty() => counts,
        _ => match store.session_bounds_rows(&handle).await {
            Ok(rows) => {
                let (counts, _) = derive_entity_session_data(&rows);
                counts
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    handle = %handle,
                    "session bounds fallback failed; entity rollup returning session_count=0"
                );
                std::collections::HashMap::new()
            }
        },
    };
    let mut entities: Vec<EntitySummary> = entities
        .into_iter()
        .map(|mut e| {
            // The session-counts map is keyed on (String::new(), id)
            // because the bounds projection lacks `kind`. We attach
            // the count irrespective of kind — collisions across kinds
            // for the same id are vanishingly rare in practice (a
            // shop id and a vehicle id never coincide), and the
            // alternative (a second SQL pass per entity) blows up the
            // request budget.
            let key = (String::new(), e.id.clone());
            e.session_count = session_counts.get(&key).copied().unwrap_or(0);
            e
        })
        .collect();

    // Apply opaque cursor, then truncate to limit. We fetch the full
    // entity list (cap = ENTITIES_LIST_HARD_CAP) so cursor + limit
    // composition is purely in-memory; the cost is bounded by the
    // hard cap above.
    entities = apply_cursor(entities, params.after.as_deref());

    let next_after = if !saturated && entities.len() as i64 > limit {
        entities.truncate(limit as usize);
        entities.last().map(|e| {
            let last_seen = e.last_seen.clone().unwrap_or_default();
            encode_cursor(&last_seen, &e.kind, &e.id)
        })
    } else {
        entities.truncate(limit as usize);
        None
    };

    (
        StatusCode::OK,
        Json(EntitiesListResponse {
            entities,
            next_after,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/users/{handle}/entities/{kind}/{id}",
    tag = "entity-rollup",
    operation_id = "entity_rollup_entity_history",
    params(
        ("handle" = String, Path, description = "Owner RSI handle"),
        ("kind" = String, Path, description = "EntityKind discriminator (player|vehicle|item|location|shop|mission|session|system)"),
        ("id" = String, Path, description = "Entity identifier (URL-decoded)"),
        EntityEventsQuery,
    ),
    responses(
        (status = 200, description = "All events tagged with this entity across all sessions", body = EntityHistoryResponseSchema),
        (status = 400, description = "Malformed handle / kind / id", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller has no share_event_timeline grant", body = ApiErrorBody),
        (status = 404, description = "Entity has no events for this user", body = ApiErrorBody),
        (status = 500, description = "Query failed", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn entity_history(
    State(store): State<Arc<dyn EntityRollupStore>>,
    auth: AuthenticatedUser,
    Extension(catalog_cache): Extension<LocationCatalogCache>,
    Path((handle, kind, id)): Path<(String, String, String)>,
    Query(params): Query<EntityEventsQuery>,
) -> Response {
    if !validate_handle(&handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    if !validate_kind(&kind) {
        return err(StatusCode::BAD_REQUEST, "invalid_kind");
    }
    if !validate_entity_id(&id) {
        return err(StatusCode::BAD_REQUEST, "invalid_entity_id");
    }
    match store
        .caller_may_view(&handle, &auth.preferred_username)
        .await
    {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "share_event_timeline_not_granted"),
        Err(e) => {
            tracing::error!(error = %e, "share grant lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "grant_lookup_failed");
        }
    }

    let limit = params
        .limit
        .unwrap_or(ENTITY_EVENTS_LIMIT_DEFAULT)
        .clamp(1, ENTITY_EVENTS_LIMIT_MAX);

    let stored_events = match store
        .entity_events(&handle, &kind, &id, params.after.as_deref(), limit)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "entity events query failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };

    // 404 when the entity has no events AND no cursor was supplied —
    // a cursor implies the caller already saw events, so empty-tail
    // is a 200 with empty events instead.
    if stored_events.is_empty() && params.after.is_none() {
        return err(StatusCode::NOT_FOUND, "entity_not_found");
    }

    // Pick a display_name from the most recent event with a non-empty
    // one. Fall back to `id`.
    let display_name = stored_events
        .iter()
        .rev()
        .find_map(|row| {
            row.metadata
                .as_ref()
                .and_then(|m| m.get("primary_entity"))
                .and_then(|pe| pe.get("display_name"))
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| id.clone());

    // Session breakdown — best effort, same posture as the list
    // endpoint. We re-derive from the full bounds walk and filter to
    // the requested entity id.
    let breakdown: Vec<EntitySessionBucket> = match store.session_bounds_rows(&handle).await {
        Ok(rows) => {
            let (_, breakdowns) = derive_entity_session_data(&rows);
            let key = (String::new(), id.clone());
            breakdowns.get(&key).cloned().unwrap_or_default()
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                handle = %handle,
                kind = %kind,
                id = %id,
                "session bounds query failed; entity history returning empty breakdown"
            );
            Vec::new()
        }
    };

    let next_after = if stored_events.len() as i64 >= limit {
        stored_events.last().map(|e| e.idempotency_key.clone())
    } else {
        None
    };

    let catalog = catalog_cache.snapshot().await;
    let events: Vec<EventEnvelope> = stored_events
        .into_iter()
        .map(|row| envelope_from_row(row, &catalog))
        .collect();

    (
        StatusCode::OK,
        Json(EntityHistoryResponse {
            kind,
            id,
            display_name,
            events,
            next_after,
            session_breakdown: breakdown,
        }),
    )
        .into_response()
}

// -- Test support + tests -------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory implementation. Mirrors the Postgres semantics for
    /// the route-layer tests: handle-scoped storage, grant lookup,
    /// GROUP BY with last-seen ordering, JSONB-indexed entity filter,
    /// session-bounds walk.
    #[derive(Default)]
    pub struct MemoryEntityRollupStore {
        /// Events keyed by handle (lowercased), in insertion order.
        events: Mutex<HashMap<String, Vec<StoredEvent>>>,
        /// Active grants: (owner_lower, recipient_lower) -> bool.
        grants: Mutex<HashMap<(String, String), bool>>,
        /// Test-only override for the session-bounds row cap. Defaults
        /// to [`SESSION_BOUNDS_HARD_CAP`]. Lowering this lets a unit
        /// test exercise the saturation branch without inserting half
        /// a million rows.
        session_bounds_cap_override: Mutex<Option<i64>>,
    }

    #[derive(Debug, Clone)]
    pub struct StoredEvent {
        pub idempotency_key: String,
        pub raw_line: String,
        pub event_type: String,
        pub log_source: String,
        pub source_offset: i64,
        pub event_timestamp: Option<DateTime<Utc>>,
        pub payload: serde_json::Value,
        pub metadata: Option<serde_json::Value>,
    }

    impl MemoryEntityRollupStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn insert_event(&self, handle: &str, event: StoredEvent) {
            self.events
                .lock()
                .unwrap()
                .entry(handle.to_ascii_lowercase())
                .or_default()
                .push(event);
        }

        pub fn grant(&self, owner: &str, recipient: &str, value: bool) {
            self.grants.lock().unwrap().insert(
                (owner.to_ascii_lowercase(), recipient.to_ascii_lowercase()),
                value,
            );
        }

        /// Test-only: override the session-bounds row cap so a unit
        /// test can hit the saturation branch with a tiny fixture.
        pub fn set_session_bounds_cap(&self, cap: i64) {
            *self.session_bounds_cap_override.lock().unwrap() = Some(cap);
        }
    }

    #[async_trait]
    impl EntityRollupStore for MemoryEntityRollupStore {
        async fn ensure_rollup_fresh(&self, _handle: &str) -> Result<()> {
            Ok(())
        }

        async fn entity_session_counts(
            &self,
            _handle: &str,
        ) -> Result<std::collections::HashMap<(String, String), u32>> {
            // Empty -> list_entities falls back to the in-memory session-bounds
            // walk, keeping the existing mock-based tests unchanged.
            Ok(std::collections::HashMap::new())
        }

        async fn caller_may_view(&self, handle: &str, caller: &str) -> Result<bool> {
            if caller.eq_ignore_ascii_case(handle) {
                return Ok(true);
            }
            let grants = self.grants.lock().unwrap();
            Ok(grants
                .get(&(handle.to_ascii_lowercase(), caller.to_ascii_lowercase()))
                .copied()
                .unwrap_or(false))
        }

        async fn list_entities(&self, handle: &str) -> Result<Vec<EntitySummary>> {
            let key = handle.to_ascii_lowercase();
            let events = self.events.lock().unwrap();
            let user_events = events.get(&key).cloned().unwrap_or_default();
            // GROUP BY (kind, id) — replicate the Postgres aggregation.
            let mut by_entity: HashMap<(String, String), Vec<StoredEvent>> = HashMap::new();
            for ev in user_events {
                let Some(metadata) = ev.metadata.as_ref() else {
                    continue;
                };
                let kind = metadata
                    .get("primary_entity")
                    .and_then(|pe| pe.get("kind"))
                    .and_then(|k| k.as_str())
                    .map(|s| s.to_string());
                let id = metadata
                    .get("primary_entity")
                    .and_then(|pe| pe.get("id"))
                    .and_then(|i| i.as_str())
                    .map(|s| s.to_string());
                if let (Some(kind), Some(id)) = (kind, id) {
                    by_entity.entry((kind, id)).or_default().push(ev);
                }
            }
            let mut summaries: Vec<EntitySummary> = by_entity
                .into_iter()
                .map(|((kind, id), mut group)| {
                    group.sort_by_key(|e| e.source_offset);
                    let event_count = group.len() as u32;
                    let first_seen = group.iter().filter_map(|e| e.event_timestamp).min();
                    let last_seen = group.iter().filter_map(|e| e.event_timestamp).max();
                    let display_name = group
                        .iter()
                        .rev()
                        .find_map(|e| {
                            e.metadata
                                .as_ref()
                                .and_then(|m| m.get("primary_entity"))
                                .and_then(|pe| pe.get("display_name"))
                                .and_then(|n| n.as_str())
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_else(|| id.clone());
                    EntitySummary {
                        kind,
                        id,
                        display_name,
                        event_count,
                        first_seen: first_seen.map(|t| t.to_rfc3339()),
                        last_seen: last_seen.map(|t| t.to_rfc3339()),
                        session_count: 0,
                    }
                })
                .collect();
            // ORDER BY last_seen DESC NULLS LAST, kind ASC, id ASC.
            summaries.sort_by(|a, b| {
                match (b.last_seen.as_ref(), a.last_seen.as_ref()) {
                    (Some(b_ts), Some(a_ts)) => b_ts.cmp(a_ts),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| a.kind.cmp(&b.kind))
                .then_with(|| a.id.cmp(&b.id))
            });
            summaries.truncate(ENTITIES_LIST_HARD_CAP as usize);
            Ok(summaries)
        }

        async fn entity_events(
            &self,
            handle: &str,
            kind: &str,
            id: &str,
            after: Option<&str>,
            limit: i64,
        ) -> Result<Vec<StoredEntityEvent>> {
            let key = handle.to_ascii_lowercase();
            let events = self.events.lock().unwrap();
            let mut matching: Vec<StoredEvent> = events
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|ev| {
                    let Some(metadata) = ev.metadata.as_ref() else {
                        return false;
                    };
                    let ev_kind = metadata
                        .get("primary_entity")
                        .and_then(|pe| pe.get("kind"))
                        .and_then(|k| k.as_str());
                    let ev_id = metadata
                        .get("primary_entity")
                        .and_then(|pe| pe.get("id"))
                        .and_then(|i| i.as_str());
                    ev_kind == Some(kind) && ev_id == Some(id)
                })
                .filter(|ev| match after {
                    None => true,
                    Some(cursor) => ev.idempotency_key.as_str() > cursor,
                })
                .collect();
            matching.sort_by(|a, b| {
                a.source_offset
                    .cmp(&b.source_offset)
                    .then_with(|| a.idempotency_key.cmp(&b.idempotency_key))
            });
            matching.truncate(limit as usize);
            Ok(matching
                .into_iter()
                .map(|ev| StoredEntityEvent {
                    idempotency_key: ev.idempotency_key,
                    raw_line: ev.raw_line,
                    event_type: ev.event_type,
                    log_source: ev.log_source,
                    source_offset: ev.source_offset,
                    payload: ev.payload,
                    metadata: ev.metadata,
                    resolved_location: None,
                })
                .collect())
        }

        async fn session_bounds_rows(&self, handle: &str) -> Result<Vec<SessionBoundsRow>> {
            let key = handle.to_ascii_lowercase();
            let events = self.events.lock().unwrap();
            let mut rows: Vec<SessionBoundsRow> = events
                .get(&key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|ev| {
                    let meta_id = ev
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("primary_entity"))
                        .and_then(|pe| pe.get("id"))
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string());
                    let payload_local_session = ev
                        .payload
                        .get("local_session")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    SessionBoundsRow {
                        event_type: ev.event_type,
                        source_offset: ev.source_offset,
                        idempotency_key: ev.idempotency_key,
                        event_timestamp: ev.event_timestamp,
                        meta_id,
                        payload_local_session,
                    }
                })
                .collect();
            rows.sort_by(|a, b| {
                a.source_offset
                    .cmp(&b.source_offset)
                    .then_with(|| a.idempotency_key.cmp(&b.idempotency_key))
            });
            // Apply the same hard cap the Postgres impl uses. Tests
            // can lower this via `set_session_bounds_cap` to hit the
            // saturation branch without inserting cap+ rows.
            let cap = self
                .session_bounds_cap_override
                .lock()
                .unwrap()
                .unwrap_or(SESSION_BOUNDS_HARD_CAP);
            if (rows.len() as i64) > cap {
                rows.truncate(cap as usize);
            }
            if rows.len() as i64 == cap {
                tracing::warn!(
                    handle = %handle,
                    cap = cap,
                    "session_bounds query hit hard cap; session_breakdown may be incomplete"
                );
            }
            Ok(rows)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{MemoryEntityRollupStore, StoredEvent};
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Extension;
    use tower::ServiceExt;

    fn router_for_test(store: Arc<MemoryEntityRollupStore>, verifier: Arc<AuthVerifier>) -> Router {
        let store_dyn: Arc<dyn EntityRollupStore> = store;
        Router::new()
            .route("/v1/users/:handle/entities", get(list_entities))
            .route("/v1/users/:handle/entities/:kind/:id", get(entity_history))
            .with_state(store_dyn)
            .layer(Extension(verifier))
            .layer(Extension(
                crate::location_catalog_cache::LocationCatalogCache::empty(),
            ))
    }

    fn issue_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&uuid::Uuid::now_v7().to_string(), handle)
            .expect("sign user token")
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn make_event(
        idem: &str,
        offset: i64,
        ts_str: &str,
        kind: &str,
        id: &str,
        name: &str,
        ev_type: &str,
    ) -> StoredEvent {
        StoredEvent {
            idempotency_key: idem.to_string(),
            raw_line: format!("<{ts_str}> {ev_type}"),
            event_type: ev_type.to_string(),
            log_source: "live".to_string(),
            source_offset: offset,
            event_timestamp: Some(ts(ts_str)),
            payload: serde_json::json!({
                "type": ev_type,
                "timestamp": ts_str,
            }),
            metadata: Some(serde_json::json!({
                "primary_entity": {
                    "kind": kind,
                    "id": id,
                    "display_name": name,
                },
                "source": "observed",
                "confidence": 1.0,
                "group_key": format!("{ev_type}:{kind}:{id}"),
            })),
        }
    }

    // ----- 401 / 403 / 400 paths -----

    #[tokio::test]
    async fn entities_list_rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn entity_history_rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities/vehicle/CUTLASS_GEID")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn entities_list_rejects_malformed_handle() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice%20bob/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_handle");
    }

    #[tokio::test]
    async fn entity_history_rejects_invalid_kind() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities/spaceship/CUTLASS")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_kind");
    }

    #[tokio::test]
    async fn entities_list_denies_when_no_grant() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "bob");
        let store = Arc::new(MemoryEntityRollupStore::new());
        // No grant for alice -> bob.
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "share_event_timeline_not_granted");
    }

    #[tokio::test]
    async fn entities_list_allows_when_grant_present() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "bob");
        let store = Arc::new(MemoryEntityRollupStore::new());
        store.grant("alice", "bob", true);
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn entities_list_allows_self_access_without_grant() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        // Notice: NO grant registered.
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ----- Happy paths -----

    #[tokio::test]
    async fn entities_list_returns_aggregated_summaries() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        // Two events for vehicle Cutlass, one for vehicle Aurora.
        store.insert_event(
            "alice",
            make_event(
                "k1",
                10,
                "2026-05-17T00:00:00Z",
                "vehicle",
                "CUTLASS_GEID",
                "Cutlass Black",
                "vehicle_destruction",
            ),
        );
        store.insert_event(
            "alice",
            make_event(
                "k2",
                20,
                "2026-05-17T00:05:00Z",
                "vehicle",
                "CUTLASS_GEID",
                "Cutlass Black",
                "vehicle_destruction",
            ),
        );
        store.insert_event(
            "alice",
            make_event(
                "k3",
                30,
                "2026-05-17T01:00:00Z",
                "vehicle",
                "AURORA_GEID",
                "Aurora",
                "vehicle_stowed",
            ),
        );
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = body_bytes(resp).await;
        let body: EntitiesListResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.entities.len(), 2);
        // last_seen DESC: Aurora (01:00) before Cutlass (00:05).
        assert_eq!(body.entities[0].id, "AURORA_GEID");
        assert_eq!(body.entities[0].event_count, 1);
        assert_eq!(body.entities[1].id, "CUTLASS_GEID");
        assert_eq!(body.entities[1].event_count, 2);
        assert!(body.next_after.is_none());
    }

    #[tokio::test]
    async fn entities_list_paginates_via_cursor() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        // Three entities, ascending source_offset gives a deterministic
        // (last_seen DESC) order: e3, e2, e1.
        for (i, id) in ["e1", "e2", "e3"].iter().enumerate() {
            store.insert_event(
                "alice",
                make_event(
                    &format!("k{i}"),
                    (i as i64) * 10,
                    &format!("2026-05-17T0{i}:00:00Z"),
                    "vehicle",
                    id,
                    id,
                    "vehicle_stowed",
                ),
            );
        }
        let app = router_for_test(store, Arc::new(verifier.clone()));
        // First page, limit=2.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities?limit=2")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let bytes = body_bytes(resp).await;
        let body: EntitiesListResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.entities.len(), 2);
        assert_eq!(body.entities[0].id, "e3");
        assert_eq!(body.entities[1].id, "e2");
        let cursor = body.next_after.expect("cursor present");
        // Second page using the cursor.
        let app2 = router_for_test(Arc::new(rebuild_store_for_pagination()), Arc::new(verifier));
        let req2 = Request::builder()
            .method("GET")
            .uri(format!("/v1/users/alice/entities?limit=2&after={cursor}"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        let bytes2 = body_bytes(resp2).await;
        let body2: EntitiesListResponse = serde_json::from_slice(&bytes2).unwrap();
        assert_eq!(body2.entities.len(), 1);
        assert_eq!(body2.entities[0].id, "e1");
        assert!(body2.next_after.is_none());
    }

    fn rebuild_store_for_pagination() -> MemoryEntityRollupStore {
        let store = MemoryEntityRollupStore::new();
        for (i, id) in ["e1", "e2", "e3"].iter().enumerate() {
            store.insert_event(
                "alice",
                make_event(
                    &format!("k{i}"),
                    (i as i64) * 10,
                    &format!("2026-05-17T0{i}:00:00Z"),
                    "vehicle",
                    id,
                    id,
                    "vehicle_stowed",
                ),
            );
        }
        store
    }

    #[tokio::test]
    async fn entity_history_returns_filtered_events() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        store.insert_event(
            "alice",
            make_event(
                "k1",
                10,
                "2026-05-17T00:00:00Z",
                "vehicle",
                "CUTLASS",
                "Cutlass",
                "vehicle_destruction",
            ),
        );
        store.insert_event(
            "alice",
            make_event(
                "k2",
                20,
                "2026-05-17T00:05:00Z",
                "vehicle",
                "AURORA",
                "Aurora",
                "vehicle_stowed",
            ),
        );
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities/vehicle/CUTLASS")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = body_bytes(resp).await;
        let body: EntityHistoryResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.kind, "vehicle");
        assert_eq!(body.id, "CUTLASS");
        assert_eq!(body.display_name, "Cutlass");
        assert_eq!(body.events.len(), 1);
        assert_eq!(body.events[0].idempotency_key, "k1");
    }

    #[tokio::test]
    async fn entity_history_returns_404_for_unknown_entity() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities/vehicle/GHOST")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "entity_not_found");
    }

    #[tokio::test]
    async fn entity_history_paginates_via_idempotency_cursor() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        for i in 0..3 {
            store.insert_event(
                "alice",
                make_event(
                    &format!("k{i}"),
                    (i as i64) * 10,
                    &format!("2026-05-17T0{i}:00:00Z"),
                    "vehicle",
                    "CUTLASS",
                    "Cutlass",
                    "vehicle_destruction",
                ),
            );
        }
        let app = router_for_test(store, Arc::new(verifier));
        // limit=2; expect events k0, k1 with cursor=k1.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities/vehicle/CUTLASS?limit=2")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let bytes = body_bytes(resp).await;
        let body: EntityHistoryResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.events.len(), 2);
        assert_eq!(body.events[0].idempotency_key, "k0");
        assert_eq!(body.events[1].idempotency_key, "k1");
        assert_eq!(body.next_after.as_deref(), Some("k1"));
    }

    // ----- Pure-fn derivation tests -----

    #[test]
    fn cursor_round_trips() {
        let enc = encode_cursor("2026-05-17T00:00:00+00:00", "vehicle", "CUTLASS_GEID");
        let (last, kind, id) = decode_cursor(&enc).unwrap();
        assert_eq!(last, "2026-05-17T00:00:00+00:00");
        assert_eq!(kind, "vehicle");
        assert_eq!(id, "CUTLASS_GEID");
    }

    #[test]
    fn cursor_decode_rejects_garbage() {
        assert!(decode_cursor("not a cursor").is_none());
        assert!(decode_cursor("aGVsbG8").is_none()); // valid b64 but missing pipes
    }

    #[test]
    fn validate_kind_accepts_all_eight_variants() {
        for k in VALID_ENTITY_KINDS {
            assert!(validate_kind(k), "rejected: {k}");
        }
        assert!(!validate_kind("spaceship"));
        assert!(!validate_kind(""));
    }

    #[test]
    fn derive_entity_session_data_counts_sessions_per_entity() {
        // Two sessions; vehicle CUTLASS appears in both, AURORA only
        // in the second. Expected counts: CUTLASS=2, AURORA=1.
        let rows = vec![
            SessionBoundsRow {
                event_type: "process_init".to_string(),
                source_offset: 0,
                idempotency_key: "k0".to_string(),
                event_timestamp: Some(ts("2026-05-17T00:00:00Z")),
                meta_id: Some("s1".to_string()),
                payload_local_session: None,
            },
            SessionBoundsRow {
                event_type: "vehicle_destruction".to_string(),
                source_offset: 10,
                idempotency_key: "k1".to_string(),
                event_timestamp: Some(ts("2026-05-17T00:05:00Z")),
                meta_id: Some("CUTLASS".to_string()),
                payload_local_session: None,
            },
            SessionBoundsRow {
                event_type: "process_init".to_string(),
                source_offset: 20,
                idempotency_key: "k2".to_string(),
                event_timestamp: Some(ts("2026-05-17T01:00:00Z")),
                meta_id: Some("s2".to_string()),
                payload_local_session: None,
            },
            SessionBoundsRow {
                event_type: "vehicle_destruction".to_string(),
                source_offset: 30,
                idempotency_key: "k3".to_string(),
                event_timestamp: Some(ts("2026-05-17T01:05:00Z")),
                meta_id: Some("CUTLASS".to_string()),
                payload_local_session: None,
            },
            SessionBoundsRow {
                event_type: "vehicle_stowed".to_string(),
                source_offset: 40,
                idempotency_key: "k4".to_string(),
                event_timestamp: Some(ts("2026-05-17T01:10:00Z")),
                meta_id: Some("AURORA".to_string()),
                payload_local_session: None,
            },
        ];
        let (counts, breakdowns) = derive_entity_session_data(&rows);
        let cutlass_key = (String::new(), "CUTLASS".to_string());
        let aurora_key = (String::new(), "AURORA".to_string());
        assert_eq!(counts.get(&cutlass_key).copied(), Some(2));
        assert_eq!(counts.get(&aurora_key).copied(), Some(1));
        assert_eq!(breakdowns.get(&cutlass_key).map(|v| v.len()), Some(2));
        assert_eq!(breakdowns.get(&aurora_key).map(|v| v.len()), Some(1));
    }

    /// Regression: session attribution must survive a Game.log rotation.
    ///
    /// The tray resets `source_offset` to 0 on every rotation (see the
    /// same warning on `event_timeline`'s bounds query), so ordering a
    /// handle's whole history by `source_offset` INTERLEAVES launches:
    /// every launch's offset-0 row sorts together, ahead of every
    /// launch's offset-10 row. The walk then sees back-to-back
    /// `process_init`s, flushes empty sessions, and lumps every later
    /// event into whichever session happened to be open last.
    ///
    /// Rows here arrive in that (source_offset, idempotency_key) order —
    /// exactly what the DB used to return — for two launches on
    /// different days, each of which used CUTLASS once. The truth is
    /// CUTLASS = 2 sessions; the offset-ordered walk reported 1.
    #[test]
    fn derive_entity_session_data_survives_log_rotation() {
        let rows = vec![
            // --- both launches' offset-0 rows sort first ---
            SessionBoundsRow {
                event_type: "process_init".to_string(),
                source_offset: 0,
                idempotency_key: "a0".to_string(),
                event_timestamp: Some(ts("2026-05-17T00:00:00Z")),
                meta_id: Some("s1".to_string()),
                payload_local_session: None,
            },
            SessionBoundsRow {
                event_type: "process_init".to_string(),
                source_offset: 0,
                idempotency_key: "b0".to_string(),
                event_timestamp: Some(ts("2026-05-18T00:00:00Z")),
                meta_id: Some("s2".to_string()),
                payload_local_session: None,
            },
            // --- then both launches' offset-10 rows ---
            SessionBoundsRow {
                event_type: "vehicle_destruction".to_string(),
                source_offset: 10,
                idempotency_key: "a1".to_string(),
                event_timestamp: Some(ts("2026-05-17T00:05:00Z")),
                meta_id: Some("CUTLASS".to_string()),
                payload_local_session: None,
            },
            SessionBoundsRow {
                event_type: "vehicle_destruction".to_string(),
                source_offset: 10,
                idempotency_key: "b1".to_string(),
                event_timestamp: Some(ts("2026-05-18T00:05:00Z")),
                meta_id: Some("CUTLASS".to_string()),
                payload_local_session: None,
            },
        ];

        let (counts, breakdowns) = derive_entity_session_data(&rows);
        let cutlass_key = (String::new(), "CUTLASS".to_string());
        assert_eq!(
            counts.get(&cutlass_key).copied(),
            Some(2),
            "CUTLASS was used in two separate launches, so it spans two sessions"
        );
        let buckets = breakdowns.get(&cutlass_key).expect("CUTLASS breakdown");
        assert_eq!(buckets.len(), 2, "one bucket per session, not one merged");
        let session_ids: Vec<&str> = buckets.iter().map(|b| b.session_id.as_str()).collect();
        assert!(
            session_ids.contains(&"s1") && session_ids.contains(&"s2"),
            "buckets attribute to both launches, got {session_ids:?}"
        );
    }

    #[test]
    fn apply_cursor_skips_past_token() {
        let entities = vec![
            EntitySummary {
                kind: "vehicle".to_string(),
                id: "e3".to_string(),
                display_name: "e3".to_string(),
                event_count: 1,
                first_seen: Some("2026-05-17T02:00:00+00:00".to_string()),
                last_seen: Some("2026-05-17T02:00:00+00:00".to_string()),
                session_count: 0,
            },
            EntitySummary {
                kind: "vehicle".to_string(),
                id: "e2".to_string(),
                display_name: "e2".to_string(),
                event_count: 1,
                first_seen: Some("2026-05-17T01:00:00+00:00".to_string()),
                last_seen: Some("2026-05-17T01:00:00+00:00".to_string()),
                session_count: 0,
            },
            EntitySummary {
                kind: "vehicle".to_string(),
                id: "e1".to_string(),
                display_name: "e1".to_string(),
                event_count: 1,
                first_seen: Some("2026-05-17T00:00:00+00:00".to_string()),
                last_seen: Some("2026-05-17T00:00:00+00:00".to_string()),
                session_count: 0,
            },
        ];
        let cursor = encode_cursor("2026-05-17T01:00:00+00:00", "vehicle", "e2");
        let remaining = apply_cursor(entities, Some(&cursor));
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "e1");
    }

    // ----- Coverage additions (Phase 6 synthesis) -----

    #[tokio::test]
    async fn entities_list_returns_empty_for_user_with_no_events() {
        // Empty Memory store: a self-access caller should get a 200
        // with an empty list and no cursor, not a 404 or 500.
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "lonely_user");
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/lonely_user/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = body_bytes(resp).await;
        let body: EntitiesListResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(body.entities.is_empty());
        assert!(body.next_after.is_none());
    }

    #[test]
    fn validate_entity_id_rejects_empty() {
        assert!(!validate_entity_id(""));
    }

    #[test]
    fn validate_entity_id_rejects_overlong() {
        let id = "x".repeat(257);
        assert!(!validate_entity_id(&id));
    }

    #[test]
    fn validate_entity_id_accepts_boundary() {
        // 256 is the upper bound; one shorter must also pass.
        let id_max = "x".repeat(256);
        assert!(validate_entity_id(&id_max));
        let id_one = "x";
        assert!(validate_entity_id(id_one));
    }

    #[tokio::test]
    async fn entity_history_rejects_overlong_id() {
        // Round-trip the path validator through the handler so the
        // 400 carries the correct error code.
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store = Arc::new(MemoryEntityRollupStore::new());
        let app = router_for_test(store, Arc::new(verifier));
        let overlong = "x".repeat(257);
        let uri = format!("/v1/users/alice/entities/vehicle/{overlong}");
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_entity_id");
    }

    #[tokio::test]
    async fn entities_list_denies_when_grant_flag_false() {
        // A grant row exists but `share_event_timeline = FALSE`.
        // Memory `caller_may_view` mirrors Postgres: false flag → 403.
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "bob");
        let store = Arc::new(MemoryEntityRollupStore::new());
        store.grant("alice", "bob", false);
        let app = router_for_test(store, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/entities")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "share_event_timeline_not_granted");
    }

    #[tokio::test]
    async fn session_bounds_rows_hits_cap_via_test_override() {
        // Lower the cap to 3 via the test-only override; insert 5
        // rows; verify the Memory impl truncates to 3.
        let store = MemoryEntityRollupStore::new();
        store.set_session_bounds_cap(3);
        for i in 0..5 {
            store.insert_event(
                "alice",
                make_event(
                    &format!("k{i}"),
                    (i as i64) * 10,
                    &format!("2026-05-17T0{i}:00:00Z"),
                    "vehicle",
                    "CUTLASS",
                    "Cutlass",
                    "vehicle_destruction",
                ),
            );
        }
        let rows = store.session_bounds_rows("alice").await.unwrap();
        assert_eq!(rows.len(), 3, "cap override must truncate the walk");
    }

    /// Item A4: the materialized `entity_rollup_agg.session_count` must
    /// reproduce `derive_entity_session_data` (the live walk) exactly. Env-gated
    /// (STARSTATS_TEST_DATABASE_URL); offline `cargo test` skips it. Seeds
    /// events, rebuilds the rollups, then compares the per-id count map read
    /// from `entity_rollup_agg` against the Rust oracle on the same events.
    #[tokio::test]
    async fn entity_session_counts_match_derive_entity_session_data_parity() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!(
                "STARSTATS_TEST_DATABASE_URL unset — skipping entity session-count parity test"
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

        let handle = "entity_parity_test";
        // (event_type, ts, meta_id, local_session)
        type Row = (
            &'static str,
            &'static str,
            Option<&'static str>,
            Option<&'static str>,
        );
        let fixtures: Vec<(&str, Vec<Row>)> = vec![
            (
                "cutlass_two_sessions_aurora_one",
                vec![
                    ("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T00:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                    ("process_init", "2026-05-17T01:00:00Z", Some("s2"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T01:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                    (
                        "vehicle_stowed",
                        "2026-05-17T01:10:00Z",
                        Some("AURORA"),
                        None,
                    ),
                ],
            ),
            (
                "survives_log_rotation_two_days",
                vec![
                    ("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T00:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                    ("process_init", "2026-05-18T00:00:00Z", Some("s2"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-18T00:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                ],
            ),
            (
                "session_end_orphan_not_counted",
                vec![
                    ("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T00:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                    ("session_end", "2026-05-17T00:10:00Z", None, None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T00:11:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                    ("process_init", "2026-05-17T01:00:00Z", Some("s2"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T01:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                ],
            ),
            (
                "events_before_first_process_init_dropped",
                vec![
                    (
                        "vehicle_destruction",
                        "2026-05-16T23:00:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                    ("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    (
                        "vehicle_destruction",
                        "2026-05-17T00:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                ],
            ),
            (
                "local_session_fallback_id",
                vec![
                    (
                        "process_init",
                        "2026-05-17T00:00:00Z",
                        None,
                        Some("s-fallback"),
                    ),
                    (
                        "vehicle_destruction",
                        "2026-05-17T00:05:00Z",
                        Some("CUTLASS"),
                        None,
                    ),
                ],
            ),
        ];

        let cleanup_tables = [
            "events",
            "entity_rollup_agg",
            "session_summary",
            "character_records",
            "stat_rollup_state",
        ];

        for (name, rows) in &fixtures {
            for tbl in cleanup_tables {
                sqlx::query(&format!("DELETE FROM {tbl} WHERE claimed_handle = $1"))
                    .bind(handle)
                    .execute(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("clean {tbl}: {e}"));
            }

            for (i, row) in rows.iter().enumerate() {
                let (ty, t, meta_id, ls) = *row;
                let payload = match ls {
                    Some(s) => serde_json::json!({ "local_session": s }),
                    None => serde_json::json!({}),
                };
                let metadata: Option<serde_json::Value> = meta_id.map(
                    |id| serde_json::json!({ "primary_entity": { "kind": "test", "id": id } }),
                );
                sqlx::query(
                    r#"
                    INSERT INTO events
                        (id, idempotency_key, claimed_handle, event_type,
                         event_timestamp, log_source, source_offset, raw_line,
                         payload, metadata)
                    VALUES (gen_random_uuid(), $1, $2, $3, $4, 'test', $5, '', $6, $7)
                    "#,
                )
                .bind(format!("{i:05}"))
                .bind(handle)
                .bind(ty)
                .bind(ts(t))
                .bind(i as i64)
                .bind(&payload)
                .bind(&metadata)
                .execute(&pool)
                .await
                .expect("insert fixture event");
            }

            // Force a rebuild: mark dirty, then run it directly.
            sqlx::query(
                "INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, contracts_dirty)
                 VALUES ($1, TRUE, TRUE)
                 ON CONFLICT (claimed_handle) DO UPDATE SET sessions_dirty = TRUE, contracts_dirty = TRUE",
            )
            .bind(handle)
            .execute(&pool)
            .await
            .expect("mark dirty");
            crate::repo::PostgresStore::new(pool.clone())
                .rebuild_handle_session_stats(handle)
                .await
                .expect("rebuild");

            let entity_store = PostgresEntityRollupStore::new(pool.clone());
            let candidate = entity_store
                .entity_session_counts(handle)
                .await
                .expect("entity_session_counts");
            let bounds = entity_store
                .session_bounds_rows(handle)
                .await
                .expect("session_bounds_rows");
            let (oracle, _) = derive_entity_session_data(&bounds);

            assert_eq!(
                candidate, oracle,
                "entity_rollup_agg session_count diverged from derive_entity_session_data on `{name}`"
            );
        }

        for tbl in cleanup_tables {
            sqlx::query(&format!("DELETE FROM {tbl} WHERE claimed_handle = $1"))
                .bind(handle)
                .execute(&pool)
                .await
                .ok();
        }
    }
}
