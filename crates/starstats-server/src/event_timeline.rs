//! Per-session event-timeline endpoints with sharing-grant auth.
//!
//! Endpoints:
//!   - `GET /v1/users/{handle}/sessions`
//!   - `GET /v1/users/{handle}/sessions/{session_id}/events`
//!
//! Auth posture (per audit-v2 Phase 6):
//!   * The owner viewing their own data (`auth.preferred_username`
//!     == `handle`, case-insensitive) is always allowed.
//!   * Anyone else must have an active row in `share_metadata`
//!     where `owner_handle = handle`, `recipient_handle = caller`,
//!     `share_event_timeline = TRUE`, and `expires_at` is either
//!     NULL or in the future. Missing / FALSE / expired grant
//!     produces a 403 with `share_event_timeline_not_granted`.
//!
//! Why a fresh module instead of extending `sharing_routes.rs` or
//! `repo.rs`:
//!   * `repo.rs` and `sharing_routes.rs` are in active churn for the
//!     Phase 1/2 backfill work in `starstats-client`, so threading new
//!     trait methods through them risks merge conflicts.
//!   * The grant predicate here is narrower than the existing
//!     SpiceDB-backed `check_view` (we additionally require the new
//!     `share_event_timeline` column), so it would be a single-caller
//!     extension to that file anyway.
//!
//! Session derivation:
//!   * A session is a `ProcessInit` event plus every event up to (but
//!     not including) the next `ProcessInit` for the same handle, or a
//!     terminating `SessionEnd`. We do not materialise sessions in a
//!     table — both endpoints query the live `events` table on demand,
//!     bounding the response with [`SESSIONS_LIST_LIMIT`] /
//!     [`EVENTS_PAGE_LIMIT`].
//!   * The session_id is derived from the `ProcessInit` event's
//!     `local_session` field via the metadata envelope:
//!     `metadata->'primary_entity'->>'id'`. When metadata is missing
//!     (rows pre-`0030_events_metadata` migration) we fall back to the
//!     event's payload (`payload->>'local_session'`).
//!
//! Metadata column graceful degradation:
//!   * If the `metadata` column doesn't exist (migration 0030 unapplied),
//!     the SELECTs below would fail at parse time. We catch the column-
//!     missing error and return rows with `metadata: None` instead,
//!     so the endpoint stays usable through the deploy window where
//!     server-binary-N+1 runs against database-version-N.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::location_catalog_cache::LocationCatalogCache;
use crate::repo::{EventQuery, PostgresStore};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use starstats_core::metadata::EventMetadata;
use starstats_core::wire::{EventEnvelope, LogSource};
use starstats_core::LocationCatalog;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Hard cap on the session-list response. Sessions are derived live
/// from the events table; capping at 50 keeps the worst-case query
/// (a heavy user opening their profile) bounded.
pub const SESSIONS_LIST_LIMIT: i64 = 50;

/// Default page size for the per-session events endpoint. 10000 is
/// large enough that the typical multi-hour session fits in a single
/// round-trip; the cursor handles the long-tail of multi-day-without-
/// quit sessions.
pub const EVENTS_PAGE_LIMIT_DEFAULT: i64 = 10_000;
/// Hard cap on the per-session events endpoint regardless of caller
/// preference. Mirrors the cap on `/v1/me/events` so a single response
/// can't exhaust the connection pool.
pub const EVENTS_PAGE_LIMIT_MAX: i64 = 10_000;

// -- Wire DTOs -------------------------------------------------------

/// Summary row for the session-list endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SessionSummary {
    /// Synthesised from the originating `ProcessInit` event's
    /// `local_session` field.
    pub id: String,
    /// Timestamp of the `ProcessInit` row that opens the session.
    pub started_at: Option<DateTime<Utc>>,
    /// Timestamp of the closing event — either a `SessionEnd` row or
    /// the next `ProcessInit`. `None` when the session is still open
    /// (last session in the stream).
    pub ended_at: Option<DateTime<Utc>>,
    /// Number of events recorded inside the session bounds.
    pub event_count: u32,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionsListResponse {
    pub sessions: Vec<SessionSummary>,
}

/// OpenAPI mirror of the `EventEnvelope` wire type. The actual
/// serialization on the wire uses the real `EventEnvelope` from
/// `starstats_core::wire` — this is registered separately so the
/// generated TS schema names the shape.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionEventSchema {
    pub idempotency_key: String,
    pub raw_line: String,
    /// Tagged-union `GameEvent`. `None` for envelopes the client
    /// recognised structurally but couldn't classify.
    #[schema(value_type = Option<serde_json::Value>)]
    pub event: Option<serde_json::Value>,
    pub source: String,
    pub source_offset: u64,
    /// Optional cross-cutting metadata stamped by the client or the
    /// server's v1 grace-window back-fill.
    #[schema(value_type = Option<serde_json::Value>)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionEventsResponse {
    pub session_id: String,
    pub events: Vec<EventEnvelope>,
    /// Cursor for the next page; pass back as `after`. `None` when
    /// there are no further pages within this session.
    pub next_after: Option<String>,
}

/// OpenAPI mirror of `SessionEventsResponse`. The runtime response
/// is still the `serde`-derived shape on `SessionEventsResponse`; this
/// type only feeds the schema generator (avoids cross-crate `ToSchema`
/// constraints on the wire `EventEnvelope`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionEventsResponseSchema {
    pub session_id: String,
    pub events: Vec<SessionEventSchema>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct EventsListQuery {
    /// Cursor — return events strictly after this `idempotency_key`
    /// in `(event_timestamp, idempotency_key)` order. Pass the
    /// `next_after` returned by the previous call.
    pub after: Option<String>,
    /// Page size cap. Clamped to [`EVENTS_PAGE_LIMIT_MAX`].
    pub limit: Option<i64>,
}

/// One year — mirrors `query::STATS_MAX_HOURS`, the ceiling the me-scoped
/// stats endpoints enforce. Keeps the trailing-window bound sane so a
/// caller can't request an unbounded scan via a huge `hours`.
const TIMELINE_MAX_HOURS: i64 = 24 * 365;

/// Optional trailing time window (in hours) for the timeline session
/// endpoints (`/v1/users/{handle}/sessions` and
/// `/v1/users/{handle}/stats/playtime`). Absent = all-time (lifetime, no
/// filter), so the endpoints stay backward-compatible when the dashboard
/// range selector is unset; the range-aware Sessions widget passes the
/// selected range's hours to follow it. A present value must be
/// `1..=TIMELINE_MAX_HOURS` or the endpoint 400s with `invalid_hours`.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct TimelineWindowQuery {
    pub hours: Option<i64>,
}

/// Validate a [`TimelineWindowQuery`] into an optional lower bound on
/// `event_timestamp` / session `start_at`. `None` (absent `hours`) means
/// lifetime; `Some(ts)` restricts to the trailing window. Mirrors the
/// `since` half of `query::parse_stats_range` (this surface has no
/// previous-period twin, so it does not need the other half).
fn timeline_window_since(params: &TimelineWindowQuery) -> Result<Option<DateTime<Utc>>, Response> {
    match params.hours {
        Some(hours) => {
            if hours <= 0 || hours > TIMELINE_MAX_HOURS {
                return Err(err(StatusCode::BAD_REQUEST, "invalid_hours"));
            }
            Ok(Some(Utc::now() - chrono::Duration::hours(hours)))
        }
        None => Ok(None),
    }
}

// -- Router ----------------------------------------------------------

pub fn routes(pool: PgPool) -> Router {
    Router::new()
        .route("/v1/users/:handle/sessions", get(list_sessions))
        .route(
            "/v1/users/:handle/sessions/:session_id/events",
            get(list_session_events),
        )
        .route("/v1/users/:handle/stats/playtime", get(user_playtime))
        .with_state(Arc::new(pool))
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

// M-S3: single source of truth in `crate::validation` (a small helper module,
// so importing it doesn't drag a large surface into this compile unit — the
// original reason this was duplicated from the big `users` module).
use crate::validation::validate_handle;

/// Caller may view `handle`'s timeline iff:
///   * `caller` (case-insensitive) equals `handle`, or
///   * an active row in `share_metadata` exists with
///     `share_event_timeline = TRUE`.
///
/// "Active" = `expires_at IS NULL OR expires_at > NOW()`.
///
/// Returns `Ok(true)` on allow, `Ok(false)` on deny, `Err(_)` for
/// any database error so the caller can report a 500 instead of
/// silently masquerading a query failure as a 403.
async fn caller_may_view_timeline(
    pool: &PgPool,
    handle: &str,
    caller: &str,
) -> Result<bool, sqlx::Error> {
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
    .fetch_optional(pool)
    .await?;
    Ok(matches!(row, Some((true,))))
}

/// Detects the "metadata column missing" error so callers can fall
/// back to the metadata-less query plan. We only treat the column
/// being missing as recoverable — any other DB error bubbles.
fn is_metadata_column_missing(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db_err) => {
            // Postgres SQLSTATE 42703 = undefined_column.
            db_err.code().as_deref() == Some("42703")
        }
        _ => false,
    }
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/users/{handle}/sessions",
    tag = "event-timeline",
    operation_id = "event_timeline_list_sessions",
    params(
        ("handle" = String, Path, description = "Owner RSI handle"),
        TimelineWindowQuery,
    ),
    responses(
        (status = 200, description = "Session summaries (newest first, capped at 50)", body = SessionsListResponse),
        (status = 400, description = "Malformed handle or invalid hours window", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller has no share_event_timeline grant", body = ApiErrorBody),
        (status = 500, description = "Query failed", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_sessions(
    State(pool): State<Arc<PgPool>>,
    auth: AuthenticatedUser,
    Path(handle): Path<String>,
    Query(params): Query<TimelineWindowQuery>,
) -> Response {
    if !validate_handle(&handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    let since = match timeline_window_since(&params) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match caller_may_view_timeline(pool.as_ref(), &handle, &auth.preferred_username).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "share_event_timeline_not_granted"),
        Err(e) => {
            tracing::error!(error = %e, "share grant lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "grant_lookup_failed");
        }
    }

    // Session derivation now runs entirely in SQL: `sessions_via_sql` is a
    // gaps-and-islands `process_init` sessionizer that reproduces the Rust
    // `derive_sessions` oracle exactly, so we no longer load every event row
    // into Rust and truncate in memory — the LIMIT 50 is pushed into the query.
    // `derive_sessions` is retained as the parity oracle (its unit tests + the
    // STARSTATS_TEST_DATABASE_URL parity integration test in this module).
    // (Migration 0030 added `metadata`; it exists on every deployed DB, so the
    // old pre-0030 metadata-missing fallback is gone.)
    let sessions = match sessions_via_sql(pool.as_ref(), &handle, since).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "sessions list query failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };
    (StatusCode::OK, Json(SessionsListResponse { sessions })).into_response()
}

/// SQL `process_init` sessionizer — reproduces [`derive_sessions`] entirely in
/// Postgres so the session list no longer materialises every event row in Rust.
///
/// A gaps-and-islands query: `session_ord` is a running count of `process_init`
/// rows over the `(event_timestamp, idempotency_key)` order (rows before the
/// first init get ord 0 and are dropped). Within each session, rows after the
/// first `session_end` are dropped (orphans), the id comes only from the init
/// row (`metadata->'primary_entity'->>'id'`, empty→`local_session`→`'unknown'`),
/// `ended_at` is the `session_end` timestamp or, if none, the next session's
/// init timestamp (NULL for a trailing open session), and the newest 50 are
/// returned (ordered by `session_ord DESC`, matching the Rust `reverse()` +
/// `truncate(50)` — the tie-break is stream order, not timestamp).
///
/// Keep this in sync with [`derive_sessions`]; the parity integration test
/// asserts they agree on every fixture.
async fn sessions_via_sql(
    pool: &PgPool,
    handle: &str,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<SessionSummary>, sqlx::Error> {
    let rows: Vec<(String, Option<DateTime<Utc>>, Option<DateTime<Utc>>, i64)> = sqlx::query_as(
        r#"
        WITH filtered AS (
            SELECT
                event_type,
                event_timestamp,
                idempotency_key,
                NULLIF(metadata->'primary_entity'->>'id', '') AS meta_id,
                NULLIF(payload->>'local_session', '')         AS local_session,
                SUM(CASE WHEN event_type = 'process_init' THEN 1 ELSE 0 END)
                    OVER (ORDER BY event_timestamp ASC, idempotency_key ASC) AS session_ord
            FROM events
            WHERE claimed_handle = lower($1)
              AND event_timestamp IS NOT NULL
              AND event_type NOT IN ('launcher_activity', 'game_crash')
              AND ($2::timestamptz IS NULL OR event_timestamp >= $2)
        ),
        in_stream AS (
            SELECT *,
                   ROW_NUMBER() OVER (
                       PARTITION BY session_ord
                       ORDER BY event_timestamp ASC, idempotency_key ASC
                   ) AS rn_in_group
            FROM filtered
            WHERE session_ord > 0
        ),
        first_end AS (
            SELECT session_ord,
                   MIN(rn_in_group) FILTER (WHERE event_type = 'session_end') AS first_end_rn
            FROM in_stream
            GROUP BY session_ord
        ),
        kept AS (
            SELECT s.*
            FROM in_stream s
            JOIN first_end f USING (session_ord)
            WHERE f.first_end_rn IS NULL OR s.rn_in_group <= f.first_end_rn
        ),
        agg AS (
            SELECT
                session_ord,
                MIN(event_timestamp) AS started_at,
                MAX(CASE WHEN event_type = 'process_init'
                         THEN COALESCE(meta_id, local_session, 'unknown') END) AS id,
                COUNT(*)::bigint AS event_count,
                MAX(event_timestamp) FILTER (WHERE event_type = 'session_end') AS session_end_ts
            FROM kept
            GROUP BY session_ord
        )
        SELECT
            id,
            started_at,
            COALESCE(session_end_ts, LEAD(started_at) OVER (ORDER BY session_ord)) AS ended_at,
            event_count
        FROM agg
        ORDER BY session_ord DESC
        LIMIT $3
        "#,
    )
    .bind(handle)
    .bind(since)
    .bind(SESSIONS_LIST_LIMIT)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, started_at, ended_at, event_count)| SessionSummary {
            id,
            started_at,
            ended_at,
            event_count: event_count.max(0) as u32,
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/v1/users/{handle}/sessions/{session_id}/events",
    tag = "event-timeline",
    operation_id = "event_timeline_list_session_events",
    params(
        ("handle" = String, Path, description = "Owner RSI handle"),
        ("session_id" = String, Path, description = "Session identifier from /v1/users/{handle}/sessions"),
        EventsListQuery,
    ),
    responses(
        (status = 200, description = "Page of events inside the session", body = SessionEventsResponseSchema),
        (status = 400, description = "Malformed handle or session_id", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller has no share_event_timeline grant", body = ApiErrorBody),
        (status = 500, description = "Query failed", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_session_events(
    State(pool): State<Arc<PgPool>>,
    auth: AuthenticatedUser,
    Extension(catalog_cache): Extension<LocationCatalogCache>,
    Path((handle, session_id)): Path<(String, String)>,
    Query(params): Query<EventsListQuery>,
) -> Response {
    // Snapshot the location catalog up front so both event-projection
    // branches below can re-derive `resolved_location` server-side (F4).
    let catalog = catalog_cache.snapshot().await;
    if !validate_handle(&handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    if session_id.is_empty() || session_id.len() > 128 {
        return err(StatusCode::BAD_REQUEST, "invalid_session_id");
    }
    match caller_may_view_timeline(pool.as_ref(), &handle, &auth.preferred_username).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "share_event_timeline_not_granted"),
        Err(e) => {
            tracing::error!(error = %e, "share grant lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "grant_lookup_failed");
        }
    }

    let limit = params
        .limit
        .unwrap_or(EVENTS_PAGE_LIMIT_DEFAULT)
        .clamp(1, EVENTS_PAGE_LIMIT_MAX);

    // We need to find the [start, end) (event_timestamp, idempotency_key)
    // bounds of this session before we can stream its events. The bounds
    // are computed from the same derivation as `list_sessions` — by
    // walking the projection of (event_type, event_timestamp,
    // idempotency_key, session-id source), ordered by event_timestamp.
    // Keying on event_timestamp (not source_offset) is essential: the
    // tray resets source_offset to 0 on every Game.log rotation, so two
    // different-day sessions can hold overlapping offset ranges and an
    // offset-bounded fetch would pull the wrong session's events.
    let bounds_rows: Result<
        Vec<(
            String,
            DateTime<Utc>,
            String,
            Option<String>,
            Option<String>,
        )>,
        sqlx::Error,
    > = sqlx::query_as(
        r#"
        SELECT
            event_type,
            event_timestamp,
            idempotency_key,
            (metadata->'primary_entity'->>'id') AS meta_id,
            (payload->>'local_session') AS payload_local_session
        FROM events
        WHERE claimed_handle = lower($1)
          AND event_timestamp IS NOT NULL
          AND event_type NOT IN ('launcher_activity', 'game_crash')
        ORDER BY event_timestamp ASC, idempotency_key ASC
        "#,
    )
    .bind(&handle)
    .fetch_all(pool.as_ref())
    .await;

    let bounds_rows = match bounds_rows {
        Ok(rs) => rs,
        Err(e) if is_metadata_column_missing(&e) => {
            match sqlx::query_as::<_, (String, DateTime<Utc>, String, Option<String>)>(
                r#"
                SELECT
                    event_type,
                    event_timestamp,
                    idempotency_key,
                    (payload->>'local_session') AS payload_local_session
                FROM events
                WHERE claimed_handle = lower($1)
                  AND event_timestamp IS NOT NULL
                ORDER BY event_timestamp ASC, idempotency_key ASC
                "#,
            )
            .bind(&handle)
            .fetch_all(pool.as_ref())
            .await
            {
                Ok(rs) => rs
                    .into_iter()
                    .map(|(ty, ts, idem, ls)| (ty, ts, idem, None, ls))
                    .collect(),
                Err(e2) => {
                    tracing::error!(error = %e2, "session bounds fallback query failed");
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "session bounds query failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };

    let Some(bounds) = find_session_bounds(&bounds_rows, &session_id) else {
        // Session_id doesn't match any process_init — treat as empty
        // session (cleaner UX than 404 because the caller already
        // proved access).
        return (
            StatusCode::OK,
            Json(SessionEventsResponse {
                session_id,
                events: Vec::new(),
                next_after: None,
            }),
        )
            .into_response();
    };

    // Stream events for the bounds. The cursor is the previous page's
    // last idempotency_key.
    let after = params.after.clone();
    let metadata_supported = bounds_rows
        .iter()
        .any(|(_ty, _ts, _idem, meta_id, _ls)| meta_id.is_some())
        || metadata_column_exists(pool.as_ref()).await;

    let events_result = if metadata_supported {
        fetch_session_events_with_metadata(
            pool.as_ref(),
            &handle,
            &bounds,
            after.as_deref(),
            limit,
            &catalog,
        )
        .await
    } else {
        fetch_session_events_without_metadata(
            pool.as_ref(),
            &handle,
            &bounds,
            after.as_deref(),
            limit,
            &catalog,
        )
        .await
    };

    let events = match events_result {
        Ok(evs) => evs,
        Err(e) if is_metadata_column_missing(&e) => {
            match fetch_session_events_without_metadata(
                pool.as_ref(),
                &handle,
                &bounds,
                after.as_deref(),
                limit,
                &catalog,
            )
            .await
            {
                Ok(evs) => evs,
                Err(e2) => {
                    tracing::error!(error = %e2, "session events fallback query failed");
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "session events query failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };

    // If we returned exactly `limit` rows, expose the last one's
    // idempotency_key as the next cursor. The client decides whether
    // to fetch more.
    let next_after = if events.len() as i64 >= limit {
        events.last().map(|e| e.idempotency_key.clone())
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(SessionEventsResponse {
            session_id,
            events,
            next_after,
        }),
    )
        .into_response()
}

// -- Pure-fn derivation (testable without a DB) ----------------------

/// Bounds of one session inside the event stream, keyed on
/// `event_timestamp` (the real wall-clock of each event). `start_ts` /
/// `start_idem` are inclusive; `end_ts` / `end_idem` are exclusive.
/// When the session is still open (last in the stream), the `end_*`
/// fields are `None` and the events query reads open-ended.
///
/// NOTE: we deliberately key on `event_timestamp`, NOT `source_offset`.
/// Game.log rotates on every launch, so `source_offset` resets to 0
/// each session and different-day sessions get overlapping offset
/// ranges — slicing by offset would fetch the wrong session's events.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionBounds {
    start_ts: DateTime<Utc>,
    start_idem: String,
    /// Exclusive upper bound — timestamp of the first row that does NOT
    /// belong to the session.
    end_ts: Option<DateTime<Utc>>,
    end_idem: Option<String>,
}

/// Derive a [`SessionSummary`] list from a stream of
/// `(event_type, event_timestamp, meta_id, payload_local_session)` rows
/// ordered by `(event_timestamp, idempotency_key)`. Pure function.
///
/// Retained as the parity ORACLE for [`sessions_via_sql`], the SQL `process_init`
/// sessionizer that replaced it on the timeline: its own unit tests lock the
/// boundary logic, and the `STARSTATS_TEST_DATABASE_URL`-gated parity test
/// asserts the SQL reproduces this exactly. No production caller remains since
/// the timeline moved to SQL, so it is dead-code in a non-test build by design.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn derive_sessions(
    rows: &[(
        String,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
    )],
) -> Vec<SessionSummary> {
    let mut sessions: Vec<SessionSummary> = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_started: Option<DateTime<Utc>> = None;
    let mut current_count: u32 = 0;
    let mut last_ts: Option<DateTime<Utc>> = None;

    for (event_type, ts, meta_id, payload_local_session) in rows {
        // Non-gameplay events (launcher log, crash scans) never anchor,
        // extend, or bridge a session — they fire off-hours and would
        // otherwise keep a session "open" long after the player quit.
        // The SQL already excludes them; this mirrors it so the pure
        // walk is correct on any input.
        if crate::repo::NON_SESSION_EVENT_TYPES.contains(&event_type.as_str()) {
            continue;
        }
        let id = session_id_from_row(event_type, meta_id, payload_local_session);
        if event_type == "process_init" {
            // Close the previous session (if any) before opening this one.
            if let Some(prev_id) = current_id.take() {
                sessions.push(SessionSummary {
                    id: prev_id,
                    started_at: current_started.take(),
                    ended_at: *ts,
                    event_count: current_count,
                });
            }
            current_id = Some(id.unwrap_or_else(|| "unknown".to_string()));
            current_started = *ts;
            current_count = 1;
            last_ts = *ts;
            continue;
        }
        if event_type == "session_end" {
            current_count = current_count.saturating_add(1);
            if let Some(open_id) = current_id.take() {
                sessions.push(SessionSummary {
                    id: open_id,
                    started_at: current_started.take(),
                    ended_at: *ts,
                    event_count: current_count,
                });
            }
            current_count = 0;
            last_ts = *ts;
            continue;
        }
        // Mid-session event. Only count if a session is open; events
        // that arrive before any process_init are dropped (legacy data
        // from clients that didn't emit process_init).
        if current_id.is_some() {
            current_count = current_count.saturating_add(1);
            if ts.is_some() {
                last_ts = *ts;
            }
        }
    }

    // Flush the open session (if any). `ended_at = None` because the
    // session never closed cleanly in the stream.
    if let Some(open_id) = current_id.take() {
        sessions.push(SessionSummary {
            id: open_id,
            started_at: current_started.take(),
            ended_at: None,
            event_count: current_count,
        });
    }
    let _ = last_ts;

    // Newest first, capped.
    sessions.reverse();
    sessions.truncate(SESSIONS_LIST_LIMIT as usize);
    sessions
}

fn session_id_from_row(
    event_type: &str,
    meta_id: &Option<String>,
    payload_local_session: &Option<String>,
) -> Option<String> {
    if event_type != "process_init" {
        return None;
    }
    if let Some(id) = meta_id.as_ref().filter(|s| !s.is_empty()) {
        return Some(id.clone());
    }
    payload_local_session
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
}

/// Pure helper — locate the [start, end) (event_timestamp, idempotency_key)
/// bounds of `target_session_id` inside a stream ordered by
/// `(event_timestamp, idempotency_key)`. Returns `None` when no
/// `process_init` row matches the target id.
fn find_session_bounds(
    rows: &[(
        String,
        DateTime<Utc>,
        String,
        Option<String>,
        Option<String>,
    )],
    target_session_id: &str,
) -> Option<SessionBounds> {
    let mut bounds: Option<SessionBounds> = None;
    for (event_type, ts, idem, meta_id, payload_local_session) in rows {
        let id = session_id_from_row(event_type, meta_id, payload_local_session);
        if event_type == "process_init" {
            if let Some(open) = bounds.as_mut() {
                if open.end_ts.is_none() {
                    // Close the active match — this process_init starts
                    // the next session.
                    open.end_ts = Some(*ts);
                    open.end_idem = Some(idem.clone());
                    return bounds;
                }
            }
            if id.as_deref() == Some(target_session_id) {
                bounds = Some(SessionBounds {
                    start_ts: *ts,
                    start_idem: idem.clone(),
                    end_ts: None,
                    end_idem: None,
                });
            }
            continue;
        }
        if event_type == "session_end" {
            if let Some(open) = bounds.as_mut() {
                if open.end_ts.is_none() {
                    // session_end is inclusive in the session; use the
                    // NEXT row as the exclusive upper bound.
                    open.end_ts = Some(*ts);
                    open.end_idem = Some(idem.clone());
                    // bump past the session_end by overwriting in the
                    // next iteration if there are no more rows.
                    // To keep semantics inclusive of session_end we
                    // record a sentinel that the bounds-fetcher
                    // interprets as "strictly less than or equal".
                    // Simpler approach: leave end_offset pointing AT
                    // session_end and let the events query include it
                    // with a `<=` predicate keyed on the same offset.
                }
            }
        }
    }
    bounds
}

// -- DB streaming for the per-session events endpoint ---------------

/// Test whether the `metadata` column exists on `events`. Used as a
/// one-shot probe so the events fetch can pick the right SQL up front
/// rather than catching an error inside the inner loop.
async fn metadata_column_exists(pool: &PgPool) -> bool {
    let row: Result<Option<(i64,)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT 1::BIGINT
        FROM information_schema.columns
        WHERE table_name = 'events' AND column_name = 'metadata'
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await;
    matches!(row, Ok(Some(_)))
}

/// Fetch raw event rows inside the bounds, projecting through the
/// metadata column. Returns deserialised [`EventEnvelope`]s ready
/// for serialisation back to the caller.
async fn fetch_session_events_with_metadata(
    pool: &PgPool,
    handle: &str,
    bounds: &SessionBounds,
    after: Option<&str>,
    limit: i64,
    catalog: &LocationCatalog,
) -> Result<Vec<EventEnvelope>, sqlx::Error> {
    // We always filter by `(event_timestamp >= start)` and, when bounded,
    // by `(event_timestamp <= end)`. Within a single timestamp,
    // idempotency_key tiebreaks; with `after` set we additionally skip
    // past the cursor. Keying on event_timestamp (not source_offset) is
    // what keeps a session's slice correct across Game.log rotations.
    let end_ts = bounds.end_ts;
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
          AND event_timestamp IS NOT NULL
          AND (event_timestamp, idempotency_key) >= ($2::TIMESTAMPTZ, $3::TEXT)
          AND ($4::TIMESTAMPTZ IS NULL OR event_timestamp <= $4::TIMESTAMPTZ)
          AND ($5::TEXT IS NULL OR idempotency_key > $5::TEXT)
        ORDER BY event_timestamp ASC, idempotency_key ASC
        LIMIT $6
        "#,
    )
    .bind(handle)
    .bind(bounds.start_ts)
    .bind(&bounds.start_idem)
    .bind(end_ts)
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let envelopes = rows
        .into_iter()
        .map(
            |(
                idem,
                raw_line,
                event_type,
                log_source,
                source_offset,
                payload,
                metadata,
                resolved_location,
            )| {
                envelope_from_columns(
                    idem,
                    raw_line,
                    event_type,
                    log_source,
                    source_offset,
                    payload,
                    metadata,
                    resolved_location,
                    catalog,
                )
            },
        )
        .collect();
    Ok(envelopes)
}

/// Fetch raw event rows inside the bounds without selecting the
/// metadata column. Used when migration 0030 hasn't run yet.
async fn fetch_session_events_without_metadata(
    pool: &PgPool,
    handle: &str,
    bounds: &SessionBounds,
    after: Option<&str>,
    limit: i64,
    catalog: &LocationCatalog,
) -> Result<Vec<EventEnvelope>, sqlx::Error> {
    let end_ts = bounds.end_ts;
    let rows: Vec<(String, String, String, String, i64, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT
            idempotency_key,
            raw_line,
            event_type,
            log_source,
            source_offset,
            payload
        FROM events
        WHERE claimed_handle = lower($1)
          AND event_timestamp IS NOT NULL
          AND (event_timestamp, idempotency_key) >= ($2::TIMESTAMPTZ, $3::TEXT)
          AND ($4::TIMESTAMPTZ IS NULL OR event_timestamp <= $4::TIMESTAMPTZ)
          AND ($5::TEXT IS NULL OR idempotency_key > $5::TEXT)
        ORDER BY event_timestamp ASC, idempotency_key ASC
        LIMIT $6
        "#,
    )
    .bind(handle)
    .bind(bounds.start_ts)
    .bind(&bounds.start_idem)
    .bind(end_ts)
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let envelopes = rows
        .into_iter()
        .map(
            |(idem, raw_line, event_type, log_source, source_offset, payload)| {
                envelope_from_columns(
                    idem,
                    raw_line,
                    event_type,
                    log_source,
                    source_offset,
                    payload,
                    None,
                    None,
                    catalog,
                )
            },
        )
        .collect();
    Ok(envelopes)
}

/// Reconstruct an `EventEnvelope` from the projected DB columns.
/// `event_type` is implicit in the payload tag (`GameEvent`), but we
/// don't rely on it here — we deserialise the payload through serde
/// which already has the `type` discriminator.
#[allow(clippy::too_many_arguments)]
fn envelope_from_columns(
    idempotency_key: String,
    raw_line: String,
    event_type: String,
    log_source: String,
    source_offset: i64,
    payload: serde_json::Value,
    metadata: Option<serde_json::Value>,
    // The stored column is accepted for signature symmetry with the
    // query but deliberately IGNORED — see below.
    _resolved_location: Option<serde_json::Value>,
    catalog: &LocationCatalog,
) -> EventEnvelope {
    // Re-derive the location server-side from the event's own payload
    // (F4). The stored `resolved_location` is untrusted — a client could
    // stamp any KB slug on it — so we never echo it; we classify against
    // the catalog exactly like the current-location / trace paths.
    // Computed before `payload` is moved into `from_value`.
    let resolved_location =
        crate::query::derive_resolved_location(&event_type, &payload, None, catalog);
    let parsed_event = serde_json::from_value(payload).ok();
    let parsed_source = match log_source.as_str() {
        "live" => LogSource::Live,
        "ptu" => LogSource::Ptu,
        "eptu" => LogSource::Eptu,
        "hotfix" => LogSource::Hotfix,
        "tech" => LogSource::Tech,
        _ => LogSource::Other,
    };
    let parsed_metadata: Option<EventMetadata> = metadata
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    EventEnvelope {
        idempotency_key,
        raw_line,
        event: parsed_event,
        source: parsed_source,
        // source_offset is stored as i64 (BIGINT) in PG; the wire type
        // is u64. Negative values aren't producible by the parser, so
        // the cast is safe in practice.
        source_offset: source_offset.max(0) as u64,
        metadata: parsed_metadata,
        resolved_location,
    }
}

// -- Tests -----------------------------------------------------------

/// Response for `GET /v1/users/{handle}/stats/playtime`. Deliberately
/// NOT in the OpenAPI spec (no `#[utoipa::path]`): the web client
/// hand-types this shape (mirroring `stats_records`), so adding it would
/// trip the OpenAPI codegen-drift gate for no generated-consumer benefit.
#[derive(Debug, Serialize)]
pub struct HandlePlaytimeResponse {
    pub total_playtime_secs: i64,
    pub session_count: i64,
}

/// `GET /v1/users/{handle}/stats/playtime` — the all-time playtime +
/// session-count aggregate for `handle`, behind the SAME
/// `share_event_timeline` gate as [`list_sessions`]. Lets a visitor's
/// Sessions widget show true lifetime totals instead of undercounting
/// from the 50-capped session list (F9). Reuses the exact owner-side
/// aggregate (`total_playtime_secs` / `count_sessions_since`, `since =
/// None` = all-time) so a visitor's totals match what the owner sees via
/// `/v1/me/stats/playtime?all_time=true`.
pub async fn user_playtime(
    State(pool): State<Arc<PgPool>>,
    auth: AuthenticatedUser,
    Path(handle): Path<String>,
    Query(params): Query<TimelineWindowQuery>,
) -> Response {
    if !validate_handle(&handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    // Absent `hours` = all-time (matches the historical lifetime behaviour
    // this endpoint shipped with); a present window is validated + applied
    // so the range-aware Sessions widget's summary line follows the
    // dashboard range instead of contradicting a windowed session list.
    let since = match timeline_window_since(&params) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match caller_may_view_timeline(pool.as_ref(), &handle, &auth.preferred_username).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::FORBIDDEN, "share_event_timeline_not_granted"),
        Err(e) => {
            tracing::error!(error = %e, "share grant lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "grant_lookup_failed");
        }
    }

    let query = PostgresStore::new(pool.as_ref().clone());
    let total_playtime_secs = match query.total_playtime_secs(&handle, since).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "user_playtime total_playtime_secs failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };
    let session_count = match query.count_sessions_since(&handle, since).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "user_playtime count_sessions_since failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };

    (
        StatusCode::OK,
        Json(HandlePlaytimeResponse {
            total_playtime_secs,
            session_count,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Extension;
    use tower::ServiceExt;

    fn router_for_test(verifier: Arc<AuthVerifier>) -> Router {
        // Lazy pool — only opens a connection if a handler actually
        // queries. The negative-path tests (auth, validation) finish
        // before any query runs, so an unreachable URL is fine.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/starstats_test_unused")
            .expect("connect_lazy is infallible for a syntactically valid URL");
        routes(pool).layer(Extension(verifier)).layer(Extension(
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

    #[tokio::test]
    async fn sessions_list_rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let app = router_for_test(Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/sessions")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn session_events_rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let app = router_for_test(Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/sessions/abc/events")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn user_playtime_rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let app = router_for_test(Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice/stats/playtime")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Auth is extracted before any DB query, so this rejects on the
        // lazy (unconnected) pool — same as the sessions endpoints.
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn user_playtime_rejects_malformed_handle() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice%20bob/stats/playtime")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // validate_handle runs before the grant lookup / aggregate query.
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_handle");
    }

    #[tokio::test]
    async fn sessions_list_rejects_malformed_handle() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/users/alice%20bob/sessions")
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
    async fn session_events_rejects_empty_session_id() {
        // Axum's path matcher splits on `/`, so `//events` is unreachable
        // from a real client — instead we exercise the length cap.
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let app = router_for_test(Arc::new(verifier));
        let oversize = "x".repeat(129);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/v1/users/alice/sessions/{oversize}/events"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = body_bytes(resp).await;
        let err: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err.error, "invalid_session_id");
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn derive_sessions_groups_by_process_init() {
        // Two sessions: s1 (process_init -> ... -> session_end) and
        // s2 (process_init only).
        let rows = vec![
            (
                "process_init".to_string(),
                Some(ts("2026-05-17T00:00:00Z")),
                Some("s1".to_string()),
                None,
            ),
            (
                "player_death".to_string(),
                Some(ts("2026-05-17T00:05:00Z")),
                None,
                None,
            ),
            (
                "session_end".to_string(),
                Some(ts("2026-05-17T00:10:00Z")),
                None,
                None,
            ),
            (
                "process_init".to_string(),
                Some(ts("2026-05-17T01:00:00Z")),
                Some("s2".to_string()),
                None,
            ),
            (
                "player_death".to_string(),
                Some(ts("2026-05-17T01:05:00Z")),
                None,
                None,
            ),
        ];
        let sessions = derive_sessions(&rows);
        // Newest first.
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, "s2");
        assert!(sessions[0].ended_at.is_none(), "s2 is open");
        assert_eq!(sessions[0].event_count, 2);
        assert_eq!(sessions[1].id, "s1");
        assert!(sessions[1].ended_at.is_some(), "s1 closed");
        assert_eq!(sessions[1].event_count, 3); // init + death + end
    }

    #[test]
    fn derive_sessions_ignores_launcher_activity() {
        // Launcher-log events fire off-hours (launcher open in the
        // background) hours/days after the player quit. They must not
        // count toward the session or keep it artificially "open".
        let rows = vec![
            (
                "process_init".to_string(),
                Some(ts("2026-05-17T00:00:00Z")),
                Some("s1".to_string()),
                None,
            ),
            (
                "player_death".to_string(),
                Some(ts("2026-05-17T00:05:00Z")),
                None,
                None,
            ),
            (
                "launcher_activity".to_string(),
                Some(ts("2026-05-17T06:00:00Z")),
                None,
                None,
            ),
            (
                "launcher_activity".to_string(),
                Some(ts("2026-05-18T09:00:00Z")),
                None,
                None,
            ),
        ];
        let sessions = derive_sessions(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(
            sessions[0].event_count, 2,
            "launcher_activity must be excluded from the session"
        );
    }

    #[test]
    fn derive_sessions_falls_back_to_payload_local_session_without_metadata() {
        let rows = vec![(
            "process_init".to_string(),
            Some(ts("2026-05-17T00:00:00Z")),
            None,
            Some("s-fallback".to_string()),
        )];
        let sessions = derive_sessions(&rows);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s-fallback");
    }

    #[test]
    fn derive_sessions_drops_events_before_first_process_init() {
        let rows = vec![
            (
                "player_death".to_string(),
                Some(ts("2026-05-17T00:00:00Z")),
                None,
                None,
            ),
            (
                "process_init".to_string(),
                Some(ts("2026-05-17T00:05:00Z")),
                Some("s1".to_string()),
                None,
            ),
        ];
        let sessions = derive_sessions(&rows);
        // One session, count = 1 (just the process_init; the stray
        // player_death is discarded because no session was open).
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert_eq!(sessions[0].event_count, 1);
    }

    #[test]
    fn derive_sessions_caps_at_limit() {
        // Build LIMIT + 5 sessions; assert truncation to LIMIT and
        // newest-first ordering.
        let mut rows: Vec<(
            String,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
        )> = Vec::new();
        for i in 0..(SESSIONS_LIST_LIMIT + 5) {
            rows.push((
                "process_init".to_string(),
                Some(ts("2026-05-17T00:00:00Z")),
                Some(format!("s{i}")),
                None,
            ));
        }
        let sessions = derive_sessions(&rows);
        assert_eq!(sessions.len() as i64, SESSIONS_LIST_LIMIT);
        // Newest = highest index.
        assert_eq!(sessions[0].id, format!("s{}", SESSIONS_LIST_LIMIT + 5 - 1));
    }

    #[test]
    fn find_session_bounds_locates_inclusive_start_exclusive_end() {
        let rows = vec![
            (
                "process_init".to_string(),
                ts("2026-05-17T00:00:00Z"),
                "k-a".to_string(),
                Some("s1".to_string()),
                None,
            ),
            (
                "player_death".to_string(),
                ts("2026-05-17T00:05:00Z"),
                "k-b".to_string(),
                None,
                None,
            ),
            (
                "process_init".to_string(),
                ts("2026-05-17T01:00:00Z"),
                "k-c".to_string(),
                Some("s2".to_string()),
                None,
            ),
        ];
        let b = find_session_bounds(&rows, "s1").expect("s1 bounds");
        assert_eq!(b.start_ts, ts("2026-05-17T00:00:00Z"));
        assert_eq!(b.start_idem, "k-a");
        assert_eq!(b.end_ts, Some(ts("2026-05-17T01:00:00Z")));
        assert_eq!(b.end_idem, Some("k-c".to_string()));
    }

    #[test]
    fn find_session_bounds_open_session_has_no_end() {
        let rows = vec![(
            "process_init".to_string(),
            ts("2026-05-17T00:00:00Z"),
            "k-a".to_string(),
            Some("s-open".to_string()),
            None,
        )];
        let b = find_session_bounds(&rows, "s-open").expect("open bounds");
        assert!(b.end_ts.is_none());
        assert!(b.end_idem.is_none());
    }

    #[test]
    fn find_session_bounds_returns_none_for_unknown_id() {
        let rows = vec![(
            "process_init".to_string(),
            ts("2026-05-17T00:00:00Z"),
            "k-a".to_string(),
            Some("s1".to_string()),
            None,
        )];
        assert!(find_session_bounds(&rows, "missing").is_none());
    }

    #[test]
    fn find_session_bounds_keys_on_timestamp_not_log_offset() {
        // Regression for "last played 10 days ago": Game.log rotates on
        // every launch, so `source_offset` resets to 0 each session and
        // a chronologically-LATER session can carry SMALLER offsets than
        // an earlier one. Bounds must be keyed on `event_timestamp` (the
        // real clock), never on `source_offset`. Here the rows are fed in
        // true chronological order (as the timestamp-ordered query now
        // yields them) and the older session "s1" must be bounded by the
        // newer session "s2"'s start timestamp — independent of any log
        // offset.
        let rows = vec![
            (
                "process_init".to_string(),
                ts("2026-05-10T20:00:00Z"),
                "k-old-init".to_string(),
                Some("s1".to_string()),
                None,
            ),
            (
                "player_death".to_string(),
                ts("2026-05-10T20:30:00Z"),
                "k-old-death".to_string(),
                None,
                None,
            ),
            (
                "process_init".to_string(),
                ts("2026-05-20T20:00:00Z"),
                "k-new-init".to_string(),
                Some("s2".to_string()),
                None,
            ),
        ];
        let b = find_session_bounds(&rows, "s1").expect("s1 bounds");
        assert_eq!(b.start_ts, ts("2026-05-10T20:00:00Z"));
        assert_eq!(b.end_ts, Some(ts("2026-05-20T20:00:00Z")));
    }

    #[test]
    fn envelope_from_columns_parses_metadata_when_present() {
        let metadata = serde_json::json!({
            "primary_entity": {
                "kind": "player",
                "id": "alice",
                "display_name": "alice",
            },
            "source": "observed",
            "confidence": 1.0,
            "group_key": "player_death:player:alice",
        });
        let payload = serde_json::json!({
            "type": "player_death",
            "timestamp": "2026-05-17T00:00:00.000Z",
            "body_class": "body_01_noMagicPocket",
            "body_id": "1",
            "zone": null,
        });
        let env = envelope_from_columns(
            "idem-1".to_string(),
            "<line>".to_string(),
            "player_death".to_string(),
            "live".to_string(),
            42,
            payload,
            Some(metadata),
            None,
            &starstats_core::LocationCatalog::from_entries(vec![]),
        );
        assert_eq!(env.idempotency_key, "idem-1");
        assert!(env.event.is_some());
        assert!(env.metadata.is_some());
        let md = env.metadata.unwrap();
        assert_eq!(md.primary_entity.id, "alice");
        assert_eq!(md.group_key, "player_death:player:alice");
    }

    #[test]
    fn envelope_from_columns_handles_missing_metadata_gracefully() {
        let payload = serde_json::json!({
            "type": "player_death",
            "timestamp": "2026-05-17T00:00:00.000Z",
            "body_class": "body_01_noMagicPocket",
            "body_id": "1",
            "zone": null,
        });
        let env = envelope_from_columns(
            "idem-1".to_string(),
            "<line>".to_string(),
            "player_death".to_string(),
            "live".to_string(),
            42,
            payload,
            None,
            None,
            &starstats_core::LocationCatalog::from_entries(vec![]),
        );
        assert!(env.metadata.is_none());
        assert!(env.event.is_some());
    }

    #[test]
    fn envelope_from_columns_rederives_location_and_ignores_stored_slug() {
        // F4: the stored `resolved_location` column is UNTRUSTED — a
        // client can stamp any KB slug on it, which the web renders as a
        // `/kb/location/{slug}` link. `envelope_from_columns` must
        // re-derive the location from the payload and NEVER echo the
        // stored slug. With an empty catalog the derivation yields no
        // slug, so the spoofed one is gone either way.
        let payload = serde_json::json!({
            "type": "planet_terrain_load",
            "timestamp": "2026-06-03T00:00:00.000Z",
            "planet": "Lorville",
        });
        let spoofed = serde_json::json!({
            "display_name": "Totally Legit Place",
            "slug": "phishing-target",
            "system": "Stanton",
            "tier": "landing_zone",
            "source": "catalog",
        });
        let env = envelope_from_columns(
            "idem-loc".to_string(),
            "<line>".to_string(),
            "planet_terrain_load".to_string(),
            "live".to_string(),
            7,
            payload,
            None,
            Some(spoofed),
            &starstats_core::LocationCatalog::from_entries(vec![]),
        );
        assert_ne!(
            env.resolved_location.and_then(|l| l.slug),
            Some("phishing-target".to_string()),
            "stored resolved_location slug must never be echoed to the web"
        );
    }

    // ---- Item B: SQL sessionizer parity (env-gated integration test) ----
    // Runs ONLY when STARSTATS_TEST_DATABASE_URL points at a real Postgres;
    // offline `cargo test` skips it (early return). Asserts `sessions_via_sql`
    // reproduces the `derive_sessions` oracle across every derive_sessions
    // fixture (plus orphan-drop + empty-meta cases). This is the safety net for
    // the SQL rewrite — the timeline has no live fallback.
    #[tokio::test]
    async fn sessions_via_sql_matches_derive_sessions_parity() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping SQL sessionizer parity test");
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

        let handle = "sessionizer_parity_test";
        type Row = (
            String,
            Option<DateTime<Utc>>,
            Option<String>,
            Option<String>,
        );
        let r = |ty: &str, t: &str, meta: Option<&str>, ls: Option<&str>| -> Row {
            (
                ty.to_string(),
                Some(ts(t)),
                meta.map(str::to_string),
                ls.map(str::to_string),
            )
        };

        let mut fixtures: Vec<(&str, Vec<Row>)> = vec![
            (
                "two_sessions_close_then_open",
                vec![
                    r("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    r("player_death", "2026-05-17T00:05:00Z", None, None),
                    r("session_end", "2026-05-17T00:10:00Z", None, None),
                    r("process_init", "2026-05-17T01:00:00Z", Some("s2"), None),
                    r("player_death", "2026-05-17T01:05:00Z", None, None),
                ],
            ),
            (
                "ignores_non_session_events",
                vec![
                    r("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    r("player_death", "2026-05-17T00:05:00Z", None, None),
                    r("game_crash", "2026-05-17T00:06:00Z", None, None),
                    r("launcher_activity", "2026-05-18T00:00:00Z", None, None),
                ],
            ),
            (
                "payload_local_session_fallback",
                vec![
                    r("process_init", "2026-05-17T00:00:00Z", None, Some("s-fb")),
                    r("player_death", "2026-05-17T00:05:00Z", None, None),
                ],
            ),
            (
                "empty_meta_id_falls_back_to_local_session",
                vec![r(
                    "process_init",
                    "2026-05-17T00:00:00Z",
                    Some(""),
                    Some("s-empty"),
                )],
            ),
            (
                "drops_events_before_first_process_init",
                vec![
                    r("player_death", "2026-05-16T23:00:00Z", None, None),
                    r("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                ],
            ),
            (
                "orphans_after_session_end_dropped",
                vec![
                    r("process_init", "2026-05-17T00:00:00Z", Some("s1"), None),
                    r("session_end", "2026-05-17T00:10:00Z", None, None),
                    r("player_death", "2026-05-17T00:11:00Z", None, None),
                    r("process_init", "2026-05-17T01:00:00Z", Some("s2"), None),
                ],
            ),
        ];
        // caps_at_limit: SESSIONS_LIST_LIMIT+5 inits at one timestamp; the
        // idempotency_key tiebreak must make the newest 50 survive (id "s54"..).
        let mut cap_rows: Vec<Row> = Vec::new();
        for i in 0..(SESSIONS_LIST_LIMIT + 5) {
            cap_rows.push((
                "process_init".to_string(),
                Some(ts("2026-05-17T00:00:00Z")),
                Some(format!("s{i}")),
                None,
            ));
        }
        fixtures.push(("caps_at_limit", cap_rows));

        for (name, rows) in &fixtures {
            sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
                .bind(handle)
                .execute(&pool)
                .await
                .expect("clean events");

            // idempotency_key = zero-padded index → (event_timestamp,
            // idempotency_key) order == fixture order, so the oracle and the SQL
            // sessionizer walk the identical stream even on timestamp ties.
            for (i, (ty, t, meta, ls)) in rows.iter().enumerate() {
                let payload = match ls {
                    Some(s) => serde_json::json!({ "local_session": s }),
                    None => serde_json::json!({}),
                };
                let metadata: Option<serde_json::Value> = meta
                    .as_ref()
                    .map(|id| serde_json::json!({ "primary_entity": { "id": id } }));
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
                .bind(*t)
                .bind(i as i64)
                .bind(&payload)
                .bind(&metadata)
                .execute(&pool)
                .await
                .expect("insert fixture row");
            }

            // Oracle: load rows exactly as the old list_sessions did.
            let oracle_rows: Vec<(
                String,
                Option<DateTime<Utc>>,
                Option<String>,
                Option<String>,
            )> = sqlx::query_as(
                r#"
                SELECT
                    event_type,
                    event_timestamp,
                    (metadata->'primary_entity'->>'id') AS meta_id,
                    (payload->>'local_session') AS payload_local_session
                FROM events
                WHERE claimed_handle = lower($1)
                  AND event_timestamp IS NOT NULL
                  AND event_type NOT IN ('launcher_activity', 'game_crash')
                ORDER BY event_timestamp ASC, idempotency_key ASC
                "#,
            )
            .bind(handle)
            .fetch_all(&pool)
            .await
            .expect("load oracle rows");
            let oracle = derive_sessions(&oracle_rows);

            let candidate = sessions_via_sql(&pool, handle, None)
                .await
                .expect("sessions_via_sql");

            assert_eq!(
                candidate, oracle,
                "SQL sessionizer diverged from derive_sessions on fixture `{name}`"
            );
        }

        sqlx::query("DELETE FROM events WHERE claimed_handle = $1")
            .bind(handle)
            .execute(&pool)
            .await
            .ok();
    }
}
