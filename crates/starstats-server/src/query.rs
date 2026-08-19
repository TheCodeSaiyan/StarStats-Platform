//! Read-side query API.
//!
//! Endpoints:
//!  - `GET /v1/me/events`  — paginated event stream for the caller.
//!  - `GET /v1/me/summary` — aggregated counts by `event_type`.
//!
//! Scoping: every query filters by the authenticated user's
//! `preferred_username`. Cross-user reads land in a separate
//! authorisation slice (SpiceDB) and route prefix.

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::AuthenticatedUser;
use crate::location_catalog_cache::LocationCatalogCache;
use crate::locations::{self, ResolvedLocation, LOCATION_EVENT_TYPES};
use crate::repo::{
    DockingOccurrences, EventFilters, EventQuery, EventTypeStats, InferredSession, IngestBatchRow,
    LivesData, PayloadFieldBucket, PayloadFilter, PostgresStore, SeqCursor,
};
use crate::spicedb::SpicedbClient;
use crate::validation::{build_timeline_buckets, is_valid_event_type, resolve_timeline_days};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Extension,
};
#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use starstats_core::character_life::LifeEnd;
use starstats_core::contract_life::{ContractStep, StepState};
use starstats_core::location_catalog::LocationCatalog;
use starstats_core::location_classifier::{
    classify, ClassificationSource, ResolvedLocation as ClassifiedLocation,
};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Hard caps for the events list endpoint.
const LIST_LIMIT_MAX: u32 = 500;
const LIST_LIMIT_MIN: u32 = 1;

/// Hard caps for the sessions list endpoint. Sessions are aggregates,
/// so even a heavy player won't have many — 500 trips covers years of
/// nightly play.
const SESSIONS_LIMIT_MAX: u32 = 500;
const SESSIONS_LIMIT_MIN: u32 = 1;
const SESSIONS_LIMIT_DEFAULT: u32 = 100;

/// Hard caps for the ingest-history list. Each row is one batch the
/// desktop client posted; an active player at the default 60s sync
/// interval generates ~1440/day, so 500 covers ~8 hours of play
/// without paginating. Heavier than that the user can page.
const INGEST_HISTORY_LIMIT_MAX: u32 = 500;
const INGEST_HISTORY_LIMIT_MIN: u32 = 1;
const INGEST_HISTORY_LIMIT_DEFAULT: u32 = 100;

/// Allowed `range` values on `GET /v1/me/metrics/event-types`, mapped to
/// a `since = NOW() - days` filter.
///
/// `24h` exists because the UI offers it. Without it the client silently
/// widened a 24h pick to 7d and rendered a week under a "24h" label —
/// a confidently wrong number, which is worse than a missing one.
///
/// `all` is 365 days, NOT unbounded. 365 days is the hard retention
/// limit, so "everything we have" and "the last year" are the same set;
/// saying 365 explicitly stops the API promising a depth the data does
/// not have.
const RANGE_OPTIONS: &[(&str, Option<i64>)] = &[
    ("24h", Some(1)),
    ("7d", Some(7)),
    ("30d", Some(30)),
    ("90d", Some(90)),
    ("all", Some(365)),
];
const RANGE_DEFAULT: &str = "30d";

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Legacy forward cursor: return events with `seq > after`.
    /// Kept for backwards compatibility; prefer `after_seq` /
    /// `before_seq` for new clients.
    #[serde(default)]
    pub after: Option<i64>,
    /// Cursor for "older" pagination: return events with
    /// `event_seq < before_seq`, ordered DESC by seq.
    #[serde(default)]
    pub before_seq: Option<i64>,
    /// Cursor for "newer" pagination: return events with
    /// `event_seq > after_seq`, ordered ASC by seq.
    #[serde(default)]
    pub after_seq: Option<i64>,
    /// Filter by exact event type. Validated as `[a-z0-9_]{1,64}`.
    #[serde(default)]
    pub event_type: Option<String>,
    /// Filter to events whose `event_timestamp` is at or after this
    /// instant.
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    /// Filter to events whose `event_timestamp` is at or before this
    /// instant.
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    100
}

fn err(status: StatusCode, code: &str) -> axum::response::Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineParams {
    /// Number of trailing days to bucket. Defaults to 30, max 90.
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimelineResponse {
    pub days: u32,
    pub buckets: Vec<TimelineBucket>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TimelineBucket {
    /// ISO date `YYYY-MM-DD` in UTC.
    pub date: String,
    pub count: u64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventsListResponse {
    pub events: Vec<EventDto>,
    pub next_after: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventDto {
    pub seq: i64,
    pub event_type: String,
    pub event_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub log_source: String,
    pub source_offset: i64,
    /// Free-form JSON — variant of `starstats_core::events::GameEvent`,
    /// internally tagged on `type`.
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    /// Tray-stamped fuzzy-resolved location (migration 0041), passed
    /// through verbatim from the JSONB column. `null` for placeless
    /// events / pre-resolution rows. The web links the slug to
    /// `/kb/location/{slug}` ahead of the exact catalog lookup.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<crate::ingest::ResolvedLocationSchema>)]
    pub resolved_location: Option<serde_json::Value>,
    /// `Some(ts)` means the owner has hidden this row from shared/public
    /// views; `None` means visible. Only surfaced on the owner's own
    /// `/v1/me/events` response so the UI can render a "hidden" badge
    /// + a re-show control. Friend/public endpoints don't expose
    /// `EventDto` (they return per-day counts only).
    pub hidden_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SummaryResponse {
    pub claimed_handle: String,
    pub total: u64,
    pub by_type: Vec<TypeCount>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TypeCount {
    pub event_type: String,
    pub count: u64,
}

#[utoipa::path(
    get,
    path = "/v1/me/events",
    tag = "query",
    params(ListParams),
    responses(
        (status = 200, description = "Paginated event stream for the caller", body = EventsListResponse),
        (status = 400, description = "Invalid filter or cursor combination", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_events<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Extension(catalog_cache): Extension<LocationCatalogCache>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    // Cap limit so a malicious client can't ask for everything in
    // one request and exhaust the connection pool.
    let limit = params.limit.clamp(LIST_LIMIT_MIN, LIST_LIMIT_MAX);

    // Validate event_type before we touch the DB.
    if let Some(t) = &params.event_type {
        if !is_valid_event_type(t) {
            return err(StatusCode::BAD_REQUEST, "invalid_event_type");
        }
    }

    // At most one cursor variant. If both new-style cursors are set,
    // 400. The legacy `after` is treated as `after_seq` only when
    // neither new-style cursor is set, so old clients still work.
    if params.before_seq.is_some() && params.after_seq.is_some() {
        return err(StatusCode::BAD_REQUEST, "conflicting_cursors");
    }

    let cursor = match (params.before_seq, params.after_seq, params.after) {
        (Some(_), Some(_), _) => unreachable!("conflict caught above"),
        (Some(b), None, _) => Some(SeqCursor::Before(b)),
        (None, Some(a), _) => Some(SeqCursor::After(a)),
        (None, None, Some(a)) if a > 0 => Some(SeqCursor::After(a)),
        _ => None,
    };

    let filters = EventFilters {
        cursor,
        event_type: params.event_type.clone(),
        since: params.since,
        until: params.until,
        limit: limit as i64,
    };

    match query.list_filtered(&user.preferred_username, filters).await {
        Ok(events) => {
            // `next_after` = the last seq the caller has now seen, so
            // they can pass it back as `after_seq` (or legacy `after`)
            // to fetch the next forward page.
            let next_after = events.iter().map(|e| e.seq).max();
            // Re-derive the location server-side from each event's own
            // payload — never echo the untrusted collector-supplied
            // `resolved_location` column, which the web renders as a KB
            // link and a client could spoof (F4).
            let catalog = catalog_cache.snapshot().await;
            let dtos = events
                .into_iter()
                .map(|e| {
                    let resolved_location = derive_resolved_location(
                        &e.event_type,
                        &e.payload,
                        e.event_timestamp,
                        &catalog,
                    )
                    .and_then(|c| serde_json::to_value(c).ok());
                    EventDto {
                        seq: e.seq,
                        event_type: e.event_type,
                        event_timestamp: e.event_timestamp,
                        log_source: e.log_source,
                        source_offset: e.source_offset,
                        payload: e.payload,
                        resolved_location,
                        hidden_at: e.hidden_at,
                    }
                })
                .collect();
            (
                StatusCode::OK,
                Json(EventsListResponse {
                    events: dtos,
                    next_after,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list_events failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/timeline",
    tag = "query",
    params(TimelineParams),
    responses(
        (status = 200, description = "Per-day event counts for the trailing window", body = TimelineResponse),
        (status = 400, description = "Invalid `days` parameter", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn timeline<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<TimelineParams>,
) -> impl IntoResponse {
    let Ok(days) = resolve_timeline_days(params.days) else {
        return err(StatusCode::BAD_REQUEST, "invalid_days");
    };

    match query.timeline(&user.preferred_username, days).await {
        Ok(rows) => {
            let buckets = build_timeline_buckets(rows, days)
                .into_iter()
                .map(|(date, count)| TimelineBucket { date, count })
                .collect();
            (StatusCode::OK, Json(TimelineResponse { days, buckets })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "timeline failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/summary",
    tag = "query",
    responses(
        (status = 200, description = "Aggregated counts by event type", body = SummaryResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "SpiceDB denied access to this stats record"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn summary<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Extension(_spicedb): Extension<Arc<Option<SpicedbClient>>>,
) -> impl IntoResponse {
    // Self-view is always allowed. The original implementation gated
    // this endpoint behind a SpiceDB CheckPermission for
    // `stats_record:<handle>#view@user:<handle>`, claiming "every user
    // implicitly has `view` on their own `stats_record`, so the happy
    // path is a no-op". That claim is only true if a
    // `stats_record:<handle>#owner@user:<handle>` relation has been
    // written for the caller — and nothing in the signup or first-
    // ingest path writes one, so on first deploy every authenticated
    // user's own dashboard hit 403 with "SpiceDB denied self-summary
    // access".
    //
    // The right shape for self-view is to skip SpiceDB entirely: the
    // `AuthenticatedUser` extractor has already verified the caller's
    // bearer token, so we know the caller IS this user — SpiceDB
    // can't add anything here. Cross-user reads (`/u/<handle>` public
    // profile, share-recipient timelines) keep their SpiceDB checks
    // because for those endpoints the subject != resource owner and
    // the relationship store is genuinely load-bearing.
    //
    // We still take the `Extension<SpicedbClient>` for handler-
    // signature compatibility with the router wiring in `main.rs`;
    // intentionally bound to `_spicedb` because nothing consumes it.

    match query.summary_for_handle(&user.preferred_username).await {
        Ok((total, by_type)) => (
            StatusCode::OK,
            Json(SummaryResponse {
                claimed_handle: user.preferred_username,
                total,
                by_type: by_type
                    .into_iter()
                    .map(|(event_type, count)| TypeCount { event_type, count })
                    .collect(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "summary failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

/// Response body for the hide/unhide toggles. `changed=true` means
/// the row's `hidden_at` actually flipped this call; `false` is a
/// no-op (already in the requested state, or the seq doesn't match
/// any event the caller owns). Same body shape for both POST and
/// DELETE so the web client only has one type to deal with.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HideToggleResponse {
    pub changed: bool,
}

#[utoipa::path(
    post,
    path = "/v1/me/events/{seq}/hide",
    tag = "query",
    params(("seq" = i64, Path, description = "Event seq cursor of the row to hide")),
    responses(
        (status = 200, description = "Toggle result (no-op or applied)", body = HideToggleResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Update failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn hide_event<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(seq): Path<i64>,
) -> impl IntoResponse {
    match query
        .set_event_hidden(&user.preferred_username, seq, true)
        .await
    {
        Ok(changed) => {
            if changed {
                // Best-effort audit. The hide is already committed at
                // the DB level; a logging hiccup doesn't reverse it.
                if let Err(e) = audit
                    .append(AuditEntry {
                        actor_sub: Some(user.sub.clone()),
                        actor_handle: Some(user.preferred_username.clone()),
                        action: "event.hidden".to_string(),
                        payload: serde_json::json!({ "seq": seq }),
                    })
                    .await
                {
                    tracing::warn!(error = %e, "audit append failed (event.hidden)");
                }
            }
            (StatusCode::OK, Json(HideToggleResponse { changed })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, seq, "hide_event failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1/me/events/{seq}/hide",
    tag = "query",
    params(("seq" = i64, Path, description = "Event seq cursor of the row to unhide")),
    responses(
        (status = 200, description = "Toggle result (no-op or applied)", body = HideToggleResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Update failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn unhide_event<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(seq): Path<i64>,
) -> impl IntoResponse {
    match query
        .set_event_hidden(&user.preferred_username, seq, false)
        .await
    {
        Ok(changed) => {
            if changed {
                if let Err(e) = audit
                    .append(AuditEntry {
                        actor_sub: Some(user.sub.clone()),
                        actor_handle: Some(user.preferred_username.clone()),
                        action: "event.unhidden".to_string(),
                        payload: serde_json::json!({ "seq": seq }),
                    })
                    .await
                {
                    tracing::warn!(error = %e, "audit append failed (event.unhidden)");
                }
            }
            (StatusCode::OK, Json(HideToggleResponse { changed })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, seq, "unhide_event failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

// -- Metrics aggregates ---------------------------------------------
//
// Powers the web app's Metrics page (4 tabs: Overview, Event types,
// Sessions, Raw stream). Overview + Raw stream reuse the existing
// `/v1/me/{summary,events}` endpoints; the two routes below are the
// new aggregates the design needs.

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventTypeBreakdownParams {
    /// Time window: `7d`, `30d`, `90d`, or `all`. Defaults to `30d`.
    #[serde(default)]
    pub range: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventTypeBreakdownResponse {
    /// Echo of the resolved range — clients use it for the column
    /// header without a second round-trip.
    pub range: String,
    pub types: Vec<EventTypeStatsDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventTypeStatsDto {
    pub event_type: String,
    pub count: i64,
    pub last_seen: Option<DateTime<Utc>>,
}

impl From<EventTypeStats> for EventTypeStatsDto {
    fn from(s: EventTypeStats) -> Self {
        Self {
            event_type: s.event_type,
            count: s.count,
            last_seen: s.last_seen,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionsParams {
    #[serde(default = "default_sessions_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_sessions_limit() -> u32 {
    SESSIONS_LIMIT_DEFAULT
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SessionDto {
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub event_count: i64,
}

impl From<InferredSession> for SessionDto {
    fn from(s: InferredSession) -> Self {
        Self {
            start_at: s.start_at,
            end_at: s.end_at,
            event_count: s.event_count,
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/metrics/event-types",
    tag = "metrics",
    params(EventTypeBreakdownParams),
    responses(
        (status = 200, description = "Per-event-type counts + last_seen for the chosen range", body = EventTypeBreakdownResponse),
        (status = 400, description = "Unknown range value", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn metrics_event_types<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<EventTypeBreakdownParams>,
) -> impl IntoResponse {
    let range_key = params.range.as_deref().unwrap_or(RANGE_DEFAULT);
    let Some(&(_, days)) = RANGE_OPTIONS.iter().find(|(k, _)| *k == range_key) else {
        return err(StatusCode::BAD_REQUEST, "invalid_range");
    };
    let since = days.map(|d| Utc::now() - chrono::Duration::days(d));

    match query
        .event_type_breakdown(&user.preferred_username, since)
        .await
    {
        Ok(rows) => (
            StatusCode::OK,
            Json(EventTypeBreakdownResponse {
                range: range_key.to_string(),
                types: rows.into_iter().map(EventTypeStatsDto::from).collect(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "metrics_event_types failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

// -- Ingest history --------------------------------------------------
//
// Powers the My logs page (Wave 11). Per the project's "no raw
// retention" decision, this is metadata-only — there are no
// per-line drill-down or batch-retry endpoints.

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IngestHistoryParams {
    #[serde(default = "default_ingest_history_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
    /// Scope the result to a single paired device. Omit for the
    /// account-wide stream (current default). Pre-0026 batches have
    /// no device_id stamped and are correctly excluded from any
    /// device-scoped filter.
    #[serde(default)]
    pub device_id: Option<uuid::Uuid>,
}

fn default_ingest_history_limit() -> u32 {
    INGEST_HISTORY_LIMIT_DEFAULT
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IngestHistoryResponse {
    pub batches: Vec<IngestBatchDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct IngestBatchDto {
    pub seq: i64,
    pub occurred_at: DateTime<Utc>,
    pub batch_id: String,
    pub game_build: Option<String>,
    /// Paired-device id, when the batch was posted by a tray client
    /// holding a device JWT. `None` on legacy rows (pre-0026) and on
    /// rows posted via user-scoped tokens. The Devices page reads
    /// this to filter batches to the active device tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<uuid::Uuid>,
    pub total: i64,
    pub accepted: i64,
    pub duplicate: i64,
    pub rejected: i64,
}

impl From<IngestBatchRow> for IngestBatchDto {
    fn from(r: IngestBatchRow) -> Self {
        Self {
            seq: r.seq,
            occurred_at: r.occurred_at,
            batch_id: r.batch_id,
            game_build: r.game_build,
            device_id: r.device_id,
            total: r.total,
            accepted: r.accepted,
            duplicate: r.duplicate,
            rejected: r.rejected,
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/ingest-history",
    tag = "metrics",
    params(IngestHistoryParams),
    responses(
        (status = 200, description = "Recent ingest batches the caller's clients posted", body = IngestHistoryResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn ingest_history<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<IngestHistoryParams>,
) -> impl IntoResponse {
    let limit = params
        .limit
        .clamp(INGEST_HISTORY_LIMIT_MIN, INGEST_HISTORY_LIMIT_MAX) as i64;
    let offset = params.offset as i64;

    match query
        .ingest_history_for_handle(&user.preferred_username, params.device_id, limit, offset)
        .await
    {
        Ok(rows) => (
            StatusCode::OK,
            Json(IngestHistoryResponse {
                batches: rows.into_iter().map(IngestBatchDto::from).collect(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "ingest_history failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/metrics/sessions",
    tag = "metrics",
    params(SessionsParams),
    responses(
        (status = 200, description = "Inferred play sessions, newest first", body = SessionsResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn metrics_sessions<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<SessionsParams>,
) -> impl IntoResponse {
    let limit = params.limit.clamp(SESSIONS_LIMIT_MIN, SESSIONS_LIMIT_MAX) as i64;
    let offset = params.offset as i64;

    match query
        .sessions_for_handle(&user.preferred_username, limit, offset)
        .await
    {
        Ok(sessions) => (
            StatusCode::OK,
            Json(SessionsResponse {
                sessions: sessions.into_iter().map(SessionDto::from).collect(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "metrics_sessions failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response()
        }
    }
}

// -- Location: "you are here" resolver -----------------------------
//
// Backs `GET /v1/me/location/current`. Returns 204 (No Content) when
// no resolvable real place (city OR planet) exists in the user's
// recent event history — i.e. brand-new users or users whose entire
// recent window is shard-only / INVALID readings.
//
// "Current location" is the LAST REAL PLACE, regardless of age. A
// player who logged off 15h ago at Orison sees Orison with an honest
// "15h ago" age label — the pill never headlines a raw shard id, and
// it never hides a real place just because it is stale. Staleness is
// communicated via `last_seen_at` in the response, not via 204.
//
// 204 is reserved for the "we genuinely have no spatial information"
// case: no events at all, or every event in the window is a bare
// shard / INVALID reading. In those cases the web hides the pill
// rather than display a meaningless shard string as a headline.

/// Event types that carry real spatial information (city or planet).
/// `join_pu` is intentionally excluded — it only confirms "online in
/// shard X" with no spatial resolution and must never be the sole
/// source for the "current place" headline.
const PLACE_EVENT_TYPES: &[&str] = &["location_inventory_requested", "planet_terrain_load"];

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CurrentLocationResponse {
    pub location: ResolvedLocation,
}

#[utoipa::path(
    get,
    path = "/v1/me/location/current",
    tag = "metrics",
    operation_id = "location_current",
    responses(
        (status = 200, description = "Most recent location reading", body = CurrentLocationResponse),
        (status = 204, description = "No resolvable real place found (no events, or only shard/INVALID readings)"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn location_current<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    Extension(catalog_cache): Extension<LocationCatalogCache>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    // How recently the engine's `INVALID_LOCATION_ID` ("in transit")
    // reading must have synced for it to take over the headline. Within
    // this window the user is actively moving, so we show "In transit";
    // past it a stale loading-screen reading reverts to the last
    // confirmed place.
    const IN_TRANSIT_WINDOW_MINUTES: i64 = 10;

    // Fetch the shard hint (most-recent join_pu) regardless of the
    // place-picking path so the shard rides along as context on the
    // response even when the headline is an older real place.
    let latest = match query
        .latest_location(&user.preferred_username, LOCATION_EVENT_TYPES)
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "latest_location failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response();
        }
    };

    // Fetch the recent window of PLACE-type events (no join_pu).
    // Used both to pick the current place and to compute `entered_at`.
    // join_pu is excluded so shard-spam cannot crowd out the last real
    // place — the shard comes from `latest.shard_hint` instead.
    let recent_places = match query
        .recent_location_events(
            &user.preferred_username,
            PLACE_EVENT_TYPES,
            locations::ENTERED_AT_RUN_LIMIT,
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            // Soft-fail: dwell anchor is decorative; headline still
            // renders via `latest_location`'s location_event below.
            tracing::warn!(error = %e, "recent_location_events failed; using latest only");
            Vec::new()
        }
    };

    // Fresh "in transit" override. If the NEWEST place-type reading is
    // the engine's `INVALID_LOCATION_ID` sentinel (loading screen / deep
    // space / mid-quantum) AND it synced within `IN_TRANSIT_WINDOW`, the
    // user is moving *right now* — surface "In transit" (a placeless
    // reading the UI renders as such) instead of their last confirmed
    // stop. Once it goes stale we fall through to the last confirmed
    // place below, so a transient loading screen never sticks.
    if let Some(newest) = recent_places.first() {
        let fresh = newest.event_timestamp
            >= Utc::now() - chrono::Duration::minutes(IN_TRANSIT_WINDOW_MINUTES);
        if fresh {
            if let Some(r) = locations::resolve(
                &newest.event_type,
                &newest.payload,
                newest.event_timestamp,
                latest.shard_hint.clone(),
            ) {
                let in_transit = r.city.is_none()
                    && r.planet.is_none()
                    && r.raw_city_key.as_deref() == Some("INVALID_LOCATION_ID");
                if in_transit {
                    return (
                        StatusCode::OK,
                        Json(CurrentLocationResponse { location: r }),
                    )
                        .into_response();
                }
            }
        }
    }

    // Pick the "current place": the most recent PLACE event that
    // resolves to a real location (city OR planet present). INVALID
    // readings (loading screen / deep space) are skipped. The chosen
    // place may be hours or days old — that is intentional. Its honest
    // age is surfaced via `last_seen_at`; we never return 204 just
    // because the place is old.
    //
    // Only 204 when we have NO resolvable real place at all (brand-new
    // user, or every place-type event was INVALID / unrecognised). In
    // that case the UI hides the pill rather than show a shard string.
    let chosen = recent_places
        .iter()
        .find_map(|row| {
            let r = locations::resolve(
                &row.event_type,
                &row.payload,
                row.event_timestamp,
                latest.shard_hint.clone(),
            )?;
            (r.city.is_some() || r.planet.is_some()).then_some((row.event_timestamp, r))
        })
        .or_else(|| {
            // No real place in the place-event window (empty window due
            // to soft-fail, or every event was INVALID). Fall back to
            // the most recent place-type event from `latest_location`
            // — this covers the soft-fail path and handles brand-new
            // sessions where `recent_location_events` returns nothing.
            let event = latest.location_event.as_ref()?;
            // Only consider events that carry real spatial info.
            if !PLACE_EVENT_TYPES.contains(&event.event_type.as_str()) {
                return None;
            }
            let r = locations::resolve(
                &event.event_type,
                &event.payload,
                event.event_timestamp,
                latest.shard_hint.clone(),
            )?;
            (r.city.is_some() || r.planet.is_some()).then_some((event.event_timestamp, r))
        });

    let Some((chosen_ts, mut resolved)) = chosen else {
        // No resolvable real place found at all — 204 so the UI hides
        // the pill rather than headline a raw shard id or INVALID.
        return StatusCode::NO_CONTENT.into_response();
    };

    // No staleness gate here. The chosen place may be arbitrarily old;
    // `last_seen_at` in the response conveys its age honestly and the
    // UI displays "Xh ago" / "Xd ago" next to the place name.

    // Walk back through recent place events to find when the user first
    // entered the *current* location. The previous 24h-trace-on-the-
    // client approach silently capped dwell at the trace window — a
    // user who'd been at one place for >24h saw "here 23h 57m" frozen
    // near the boundary. Anchoring on the server, with a row-count
    // limit instead of a time window, eliminates the cap. When the walk
    // exhausts its limit without a key change we surface
    // `entered_at_is_lower_bound = true` so the UI can render "+"
    // rather than lie about the precision.
    //
    // `recent_places` already excludes join_pu, so the walk only sees
    // real spatial events — no skip-while needed for shard rows.
    let head_key = locations::location_key(&resolved);
    if !recent_places.is_empty() {
        let mut anchor = chosen_ts;
        let mut matched_count = 0usize;
        // Skip rows newer than the chosen place (INVALID readings that
        // appeared between the chosen event and now).
        for row in recent_places
            .iter()
            .skip_while(|row| row.event_timestamp > chosen_ts)
        {
            // Re-resolve each row to compare keys. The resolver is
            // pure and cheap; shard hint doesn't affect the key.
            let row_resolved = locations::resolve(
                &row.event_type,
                &row.payload,
                row.event_timestamp,
                latest.shard_hint.clone(),
            );
            let row_key = row_resolved
                .as_ref()
                .map(locations::location_key)
                .unwrap_or_default();
            if row_key != head_key {
                break;
            }
            anchor = row.event_timestamp;
            matched_count += 1;
        }
        // Only populate entered_at when the run is at least two rows
        // — a single matching row tells us nothing about when the
        // user arrived (the anchor would just echo last_seen_at).
        if matched_count >= 2 {
            resolved.entered_at = Some(anchor);
            // Lower bound iff we matched every row we fetched and
            // hit the limit. If the run ended naturally inside the
            // batch, the anchor is exact.
            resolved.entered_at_is_lower_bound = matched_count >= recent_places.len()
                && (recent_places.len() as i64) >= locations::ENTERED_AT_RUN_LIMIT;
        }
    }

    // Classify the current location's raw engine key at query time so
    // the web "you are here" surface shows a friendly name + KB link
    // instead of the raw identifier. Set on the location object itself
    // so the existing `getCurrentLocation` web shape carries it. Also
    // backfill the planet/system hierarchy from the catalog so the
    // breadcrumb doesn't collapse for any place outside the naive
    // parser's 8-row city table.
    let catalog = catalog_cache.snapshot().await;
    if let Some(classification) = classify_full(&resolved, &catalog) {
        apply_catalog_hierarchy(&mut resolved, &classification);
        resolved.resolved_location = Some(ClassifiedLocation::from(classification));
    }

    (
        StatusCode::OK,
        Json(CurrentLocationResponse { location: resolved }),
    )
        .into_response()
}

// -- Location: journey trace --------------------------------------
//
// Backs `GET /v1/me/location/trace`. Returns ordered location
// transitions in a window (default = last 24h). The handler resolves
// each raw event through `locations::resolve` and collapses adjacent
// rows that share the same (planet, city) into single "dwell" entries
// — so a player who pinged `LocationInventoryRequested` ten times in
// Lorville lands as one Lorville entry, not ten.

const TRACE_DEFAULT_HOURS: i64 = 24;
// One full year — matches `STATS_MAX_HOURS`. Safe to extend this far
// only because `TRACE_RAW_LIMIT` bounds the DB fetch to O(K) raw events
// regardless of window width; the window no longer drives query cost.
const TRACE_MAX_HOURS: i64 = 24 * 365;
const TRACE_LIMIT_DEFAULT: i64 = 200;
/// Cap on the raw location events fetched from the DB before the dwell
/// collapse. Only the most-recent K events in the window are read, then
/// collapsed to dwell entries and truncated to `TRACE_LIMIT_DEFAULT`
/// (200). This is the bound that keeps a year-wide window cheap. If
/// full-history trails beyond the K most-recent events are ever needed,
/// the next lever is a materialized dwell view rather than raising K.
const TRACE_RAW_LIMIT: i64 = 6000;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TraceParams {
    /// Hours to look back. Defaults to 24h; capped at one week.
    #[serde(default)]
    pub hours: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TraceEntry {
    pub planet: Option<String>,
    pub city: Option<String>,
    pub system: Option<String>,
    pub shard: Option<String>,
    pub source_event_type: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub event_count: u32,
    /// Classifier output for this dwell entry's most-specific raw engine
    /// key, derived at query time from the location catalog. Supplies the
    /// friendly display name + `/kb/location/{slug}` link for the web
    /// journey chain. `None` when no raw key was available to classify
    /// (e.g. a shard-only reading).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<crate::ingest::ResolvedLocationSchema>)]
    pub resolved_location: Option<ClassifiedLocation>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TraceResponse {
    pub hours: i64,
    pub entries: Vec<TraceEntry>,
}

#[utoipa::path(
    get,
    path = "/v1/me/location/trace",
    tag = "metrics",
    operation_id = "location_trace",
    params(TraceParams),
    responses(
        (status = 200, description = "Ordered location trace, oldest-first (most-recent N kept on truncation); clients reverse for newest-first display", body = TraceResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn location_trace<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    Extension(catalog_cache): Extension<LocationCatalogCache>,
    user: AuthenticatedUser,
    Query(params): Query<TraceParams>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(TRACE_DEFAULT_HOURS);
    if hours <= 0 || hours > TRACE_MAX_HOURS {
        return err(StatusCode::BAD_REQUEST, "invalid_hours");
    }
    let since = Utc::now() - chrono::Duration::hours(hours);
    let catalog = catalog_cache.snapshot().await;

    // Walk the stream forward in time so adjacent same-location
    // events collapse cleanly into dwell entries.
    let stream = match query
        .location_event_stream(
            &user.preferred_username,
            LOCATION_EVENT_TYPES,
            since,
            TRACE_RAW_LIMIT,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "location_event_stream failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response();
        }
    };

    // Returned OLDEST-FIRST. The web client's `toDistinctStops` dwell
    // collapse keys off oldest-first ordering (enteredAt = first event
    // in a run, lastSeenAt = last) and the timeline reverses for
    // newest-first DISPLAY, so handing back newest-first here both
    // inverted the dwell times and rendered the journey oldest-first.
    // On overflow keep the most-recent N by dropping from the FRONT
    // (oldest), preserving oldest-first order.
    let mut entries = collapse_to_trace(stream, &catalog);
    if entries.len() > TRACE_LIMIT_DEFAULT as usize {
        let drop = entries.len() - TRACE_LIMIT_DEFAULT as usize;
        entries.drain(0..drop);
    }

    (StatusCode::OK, Json(TraceResponse { hours, entries })).into_response()
}

/// Collapse a chronologically-ordered (oldest-first) stream of raw
/// location events into dwell entries. Two adjacent events that
/// resolve to the same `(planet, city)` pair become one entry whose
/// `started_at`/`ended_at` span both — and whose `event_count`
/// totals the underlying rows. A change in either field starts a
/// new entry.
/// Run the catalog classifier over the most-specific raw engine key of a
/// resolved location (`raw_city_key` ?? `raw_planet_key`). Returns the
/// FULL [`LocationClassification`] — including `system` / `parent_body`,
/// which the wire-projected [`ClassifiedLocation`] drops — so callers can
/// backfill the resolved hierarchy from the catalog before projecting.
/// `None` when no raw key is available (e.g. a shard-only `join_pu`).
fn classify_full(
    resolved: &ResolvedLocation,
    catalog: &LocationCatalog,
) -> Option<starstats_core::location_classifier::LocationClassification> {
    let raw = resolved
        .raw_city_key
        .as_deref()
        .or(resolved.raw_planet_key.as_deref())?;
    Some(classify(raw, catalog))
}

/// Server-authoritative re-derivation of an event's location
/// classification from its OWN payload + the shared catalog. The
/// collector-supplied `resolved_location` column is an UNTRUSTED hint:
/// a malicious client can stamp any KB slug on it, which the web renders
/// as a `/kb/location/{slug}` link (F4). The event read surfaces (events
/// feed, entity rollup, session timeline) therefore must never echo the
/// stored value — they call this instead, mirroring the current-location
/// / trace paths that already re-derive. `None` when the event carries
/// no classifiable location key.
///
/// `event_timestamp` only fills unrelated fields on the intermediate
/// `ResolvedLocation`; the derived classification (display_name / slug /
/// tier) depends solely on the raw location key parsed from the payload.
/// It is therefore optional — callers without a timestamp (the entity
/// rollup, whose query doesn't select one) pass `None`.
pub(crate) fn derive_resolved_location(
    event_type: &str,
    payload: &serde_json::Value,
    event_timestamp: Option<DateTime<Utc>>,
    catalog: &LocationCatalog,
) -> Option<ClassifiedLocation> {
    let ts = event_timestamp
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid"));
    let resolved = locations::resolve(event_type, payload, ts, None)?;
    classify_full(&resolved, catalog).map(ClassifiedLocation::from)
}

/// Backfill / correct a resolved location's hierarchy from the catalog
/// classification. The naive `locations::resolve` parser only knows the
/// 8-row hand-rolled city table and underscore-splitting; the catalog
/// carries the real wiki taxonomy. Two rules, both conservative:
///
///   * `system` — fill when the naive parse left it `None`. The catalog
///     system is canonical, so also correct a *differing* naive value.
///   * `planet` — fill ONLY when the naive parse left it `None`. The
///     classifier's `parent_body` is the body the place sits on / orbits,
///     i.e. the breadcrumb's "planet". We prefer a confidently-parsed
///     naive planet over the catalog to avoid overriding a known-good
///     attribution; catalog wins only when naive is absent.
fn apply_catalog_hierarchy(
    resolved: &mut ResolvedLocation,
    classification: &starstats_core::location_classifier::LocationClassification,
) {
    if let Some(system) = &classification.system {
        if resolved.system.as_deref() != Some(system.as_str()) {
            resolved.system = Some(system.clone());
        }
    }
    if resolved.planet.is_none() {
        if let Some(parent_body) = &classification.parent_body {
            resolved.planet = Some(parent_body.clone());
        }
    }
    // Override the primary headline field with the classifier's
    // display_name whenever the classifier made a confident match.
    // On Fallback the display is just the humanized raw — no better
    // than what the naive parser already produced, so we leave it.
    if classification.source != ClassificationSource::Fallback {
        if resolved.city.is_some() {
            resolved.city = Some(classification.display_name.clone());
        } else if resolved.planet.is_some() {
            resolved.planet = Some(classification.display_name.clone());
        }
    }
}

fn collapse_to_trace(
    stream: Vec<crate::repo::LatestLocationEvent>,
    catalog: &LocationCatalog,
) -> Vec<TraceEntry> {
    let mut out: Vec<TraceEntry> = Vec::new();
    for ev in stream {
        let Some(mut resolved) =
            locations::resolve(&ev.event_type, &ev.payload, ev.event_timestamp, None)
        else {
            continue;
        };
        // Correct the naive planet/system hierarchy from the catalog
        // BEFORE the same-location comparison — otherwise a catalog-
        // corrected entry and a naive one with the same raw key would
        // compare unequal and fail to collapse.
        let classification = classify_full(&resolved, catalog);
        if let Some(c) = &classification {
            apply_catalog_hierarchy(&mut resolved, c);
        }
        let same_as_last = out.last().map_or(false, |prev| {
            prev.planet == resolved.planet
                && prev.city == resolved.city
                && prev.system == resolved.system
        });
        if same_as_last {
            let last = out.last_mut().unwrap();
            last.ended_at = ev.event_timestamp;
            last.event_count += 1;
        } else {
            // Derive the friendly classification once per dwell entry —
            // adjacent same-location rows collapse above, so this runs
            // only on a genuine location change.
            let resolved_location = classification.map(ClassifiedLocation::from);
            out.push(TraceEntry {
                planet: resolved.planet,
                city: resolved.city,
                system: resolved.system,
                shard: resolved.shard,
                source_event_type: resolved.source_event_type,
                started_at: ev.event_timestamp,
                ended_at: ev.event_timestamp,
                event_count: 1,
                resolved_location,
            });
        }
    }
    out
}

// -- Location: aggregate breakdown ---------------------------------
//
// Backs `GET /v1/me/location/breakdown`. Sums dwell time per
// `(planet, city)` over a window. Dwell time is the gap between
// adjacent events at the same location, capped at the
// session-idle threshold so a logout doesn't bloat the dwell of the
// last-known place.

const BREAKDOWN_DEFAULT_HOURS: i64 = 24 * 7;
// One full year — matches `STATS_MAX_HOURS`. Like the trace endpoint,
// safe to extend this far only because `BREAKDOWN_RAW_LIMIT` bounds the
// underlying fetch (this handler streams the same location events and
// aggregates dwell in Rust).
const BREAKDOWN_MAX_HOURS: i64 = 24 * 365;
/// Cap on the raw location events fetched before dwell aggregation.
/// Same bound as [`TRACE_RAW_LIMIT`] — keeps the year-wide window O(K).
const BREAKDOWN_RAW_LIMIT: i64 = 6000;
/// Cap a single inter-event gap at this many minutes when summing
/// dwell. Anything longer is treated as "logged out" and contributes
/// only the cap — not the full gap.
const DWELL_CAP_MINUTES: i64 = 30;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BreakdownParams {
    #[serde(default)]
    pub hours: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BreakdownEntry {
    pub planet: Option<String>,
    pub city: Option<String>,
    pub system: Option<String>,
    pub dwell_seconds: i64,
    pub visit_count: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BreakdownResponse {
    pub hours: i64,
    pub entries: Vec<BreakdownEntry>,
}

#[utoipa::path(
    get,
    path = "/v1/me/location/breakdown",
    tag = "metrics",
    operation_id = "location_breakdown",
    params(BreakdownParams),
    responses(
        (status = 200, description = "Aggregate dwell by location", body = BreakdownResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn location_breakdown<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<BreakdownParams>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(BREAKDOWN_DEFAULT_HOURS);
    if hours <= 0 || hours > BREAKDOWN_MAX_HOURS {
        return err(StatusCode::BAD_REQUEST, "invalid_hours");
    }
    let since = Utc::now() - chrono::Duration::hours(hours);

    let stream = match query
        .location_event_stream(
            &user.preferred_username,
            LOCATION_EVENT_TYPES,
            since,
            BREAKDOWN_RAW_LIMIT,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "location_event_stream failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response();
        }
    };

    let entries = aggregate_dwell(stream);
    (StatusCode::OK, Json(BreakdownResponse { hours, entries })).into_response()
}

/// Walk an oldest-first stream and accumulate dwell time per
/// `(planet, city)` key. Each transition contributes the gap to the
/// PRIOR location, capped at [`DWELL_CAP_MINUTES`]. The terminal
/// location gets one cap-worth of dwell since we don't know when the
/// player left.
fn aggregate_dwell(stream: Vec<crate::repo::LatestLocationEvent>) -> Vec<BreakdownEntry> {
    use std::collections::BTreeMap;
    let cap = chrono::Duration::minutes(DWELL_CAP_MINUTES);
    let mut buckets: BTreeMap<(Option<String>, Option<String>, Option<String>), (i64, u32)> =
        BTreeMap::new();
    let mut prev: Option<(ResolvedLocation, DateTime<Utc>)> = None;
    for ev in stream {
        let Some(resolved) =
            locations::resolve(&ev.event_type, &ev.payload, ev.event_timestamp, None)
        else {
            continue;
        };
        if let Some((prev_loc, prev_ts)) = prev.take() {
            let gap = (ev.event_timestamp - prev_ts).min(cap);
            let secs = gap.num_seconds().max(0);
            let key = (
                prev_loc.planet.clone(),
                prev_loc.city.clone(),
                prev_loc.system.clone(),
            );
            let entry = buckets.entry(key).or_insert((0, 0));
            entry.0 += secs;
            entry.1 += 1;
        }
        prev = Some((resolved, ev.event_timestamp));
    }
    // Tail: contribute one cap-window of dwell to the terminal location.
    if let Some((last_loc, _)) = prev {
        let key = (last_loc.planet, last_loc.city, last_loc.system);
        let entry = buckets.entry(key).or_insert((0, 0));
        entry.0 += cap.num_seconds();
        entry.1 += 1;
    }

    let mut entries: Vec<BreakdownEntry> = buckets
        .into_iter()
        .map(|((planet, city, system), (dwell, visits))| BreakdownEntry {
            planet,
            city,
            system,
            dwell_seconds: dwell,
            visit_count: visits,
        })
        .collect();
    entries.sort_by(|a, b| b.dwell_seconds.cmp(&a.dwell_seconds));
    entries
}

// -- Activity stats: combat / travel / loadout / stability ---------
//
// One handler per stat family. Each is a thin wrapper around two
// repo calls: a `count_event_type` for the headline number plus a
// `payload_field_breakdown` for the secondary list. Bundling them
// here (rather than in their own modules) avoids fragmenting the
// query.rs surface — every read endpoint already lives in this file.

const STATS_DEFAULT_HOURS: i64 = 24 * 30;
const STATS_MAX_HOURS: i64 = 24 * 365;
/// Cap on raw rows returned per stats breakdown. The web app now
/// performs client-side hierarchical roll-up (manufacturer → family →
/// size for weapons / items; system → body → place for locations),
/// so a small limit silently truncates the long tail before it can
/// be aggregated. 100 covers the practical ceiling of Star Citizen's
/// active class catalogue while keeping the JSON response tight.
const STATS_BUCKET_LIMIT: i64 = 100;
/// Cap on the raw location events `stats_locations` fetches before it
/// counts distinct places. `STATS_MAX_HOURS` already allows a year-wide
/// window; this bound keeps that fetch O(K) (same rationale as
/// [`TRACE_RAW_LIMIT`]) rather than scanning every location event.
const STATS_LOCATIONS_RAW_LIMIT: i64 = 6000;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatsParams {
    #[serde(default)]
    pub hours: Option<i64>,
}

/// Params for `/v1/me/stats/playtime`. Superset of [`StatsParams`] with
/// an `all_time` flag so the sessions widget can show true lifetime
/// totals (the bounded `hours` window can't span a multi-year history).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PlaytimeParams {
    #[serde(default)]
    pub hours: Option<i64>,
    /// When true, aggregate over all recorded history and ignore
    /// `hours`. The response `hours` field is `0` in this mode.
    #[serde(default)]
    pub all_time: Option<bool>,
}

/// Params for `/v1/me/stats/contracts`. Superset of [`StatsParams`] with
/// an `include_steps` flag.
///
/// This endpoint is called on every `/me` page load by the contracts
/// widget, which renders eight integers and a contract name — it never
/// needs the per-step breakdown. Steps roughly double the response size
/// (~2.3 steps/run on average; measured at ~237 KB of added uncompressed
/// JSON for a 609-run heavy-player lifetime window, on top of the
/// existing unbounded `runs` list). Defaulting `include_steps` to
/// off keeps that per-load cost at today's baseline; only a
/// contract-history detail view (which actually reads `steps`) should
/// pass `include_steps=true`. Do NOT flip this default without also
/// addressing the endpoint's missing `LIMIT` — see `ContractRunRow`'s
/// doc.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ContractsParams {
    #[serde(default)]
    pub hours: Option<i64>,
    #[serde(default)]
    pub include_steps: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StatsBucket {
    pub value: String,
    pub count: i64,
}

impl From<PayloadFieldBucket> for StatsBucket {
    fn from(b: PayloadFieldBucket) -> Self {
        Self {
            value: b.value,
            count: b.count,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CombatStatsResponse {
    pub hours: i64,
    /// Times the user appeared as the killer in `actor_death`.
    pub kills: u64,
    /// Times the user (or their character) appeared as the victim.
    pub deaths: u64,
    /// How many of `deaths` were INFERRED rather than observed.
    ///
    /// CIG removed the Actor Death log lines, so a death is frequently
    /// reconstructed from a `Corpse` line and arrives as a
    /// `player_death` carrying `body_class = "inferred"`. Summing the
    /// two sources hides that, so the split travels with the total.
    ///
    /// Always `<= deaths`.
    pub deaths_inferred: u64,
    pub top_weapons: Vec<StatsBucket>,
    pub deaths_by_zone: Vec<StatsBucket>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TravelStatsResponse {
    pub hours: i64,
    pub quantum_jumps: u64,
    pub top_destinations: Vec<StatsBucket>,
    pub planets_visited: Vec<StatsBucket>,
}

/// One ship the caller flies, from quantum_target_selected.vehicle_class.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FleetShipRow {
    pub vehicle_class: String,
    pub trip_count: i64,
}

/// Lifetime twin for a windowed [`FleetResponse`].
///
/// UX Rule 2: a bare number means nothing. "3 ships, 40 trips" tells a
/// player nothing until they know whether that is most of what they fly
/// or a corner of it.
///
/// Deliberately the INVERSE of `LivesWindow`. There the top-level fields
/// are lifetime and `window` is the slice. Here the top level is already
/// the requested window — widgets pass `hours` and depend on it BEING the
/// scoped figure — so the twin adds the lifetime baseline instead.
///
/// `FleetResponse` has no scalar of its own; it is a ranked list. So the
/// baseline is that list's two MAGNITUDES — how much flying, across how
/// many ships. Their windowed counterparts are the sum of
/// `ships[].trip_count` and `ships.len()`, from the very same breakdown,
/// so the comparison is like-for-like. The `ships` ranking itself is
/// deliberately NOT mirrored: a second list names components, not a
/// magnitude, exactly as `SpendLifetime` omits `top_shop`.
///
/// Both figures inherit the top-`STATS_BUCKET_LIMIT` truncation of the
/// breakdown they are derived from — the same truncation the windowed
/// list already has.
///
/// `None` when no window was requested: with no `hours` the response
/// already IS lifetime and a twin would merely repeat it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FleetLifetime {
    /// Quantum trips summed across every ship, lifetime.
    pub total_trips: i64,
    /// Distinct ships flown, lifetime.
    pub ships_flown: i64,
}

/// Previous-period twin for a windowed [`FleetResponse`]: the same two
/// magnitudes over the same-length window shifted back once.
///
/// Where [`FleetLifetime`] answers "how much of my flying is this?", this
/// answers "am I flying more than I was, and across more ships?".
///
/// `ships_flown` is why this endpoint recomputes the previous period from
/// its own window rather than subtracting the current one from a
/// double-length one: distinct ships are not additive. Fly the Cutlass in
/// both periods and `distinct(2N) - distinct(N)` reports zero ships in the
/// earlier one. The count below is the real distinct count for
/// `[now-2N, now-N)`.
///
/// `None` means "no comparison to draw", never zero — see
/// [`SpendPrevious`] for the full rationale.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FleetPrevious {
    /// Quantum trips summed across every ship in the previous period.
    pub total_trips: i64,
    /// Distinct ships flown in the previous period.
    pub ships_flown: i64,
}

/// Response for GET /v1/me/stats/fleet — "ships you fly", ranked desc.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FleetResponse {
    pub ships: Vec<FleetShipRow>,
    /// Lifetime baseline for the windowed list above. `None` when no
    /// window was requested — see [`FleetLifetime`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<FleetLifetime>,
    /// The same magnitudes for the period before this one. `None` means
    /// there is no honest comparison to draw — NOT zero. See
    /// [`FleetPrevious`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<FleetPrevious>,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/fleet",
    tag = "metrics",
    operation_id = "stats_fleet",
    params(StatsParams),
    responses(
        (status = 200, description = "Ships the caller flies, ranked by trip count", body = FleetResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_fleet<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let range = match parse_stats_range(&params) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let since = range.since;
    let handle = &user.preferred_username;
    let result: Result<FleetResponse, crate::repo::RepoError> = async {
        let buckets = query
            .payload_field_breakdown(
                handle,
                "quantum_target_selected",
                "vehicle_class",
                None,
                since,
                None,
                STATS_BUCKET_LIMIT,
            )
            .await?;
        // Lifetime baseline, only when a window was actually requested.
        // With `since = None` the list above IS lifetime and a twin would
        // merely repeat it.
        //
        // A second pass over the SAME repo method with `since = None`,
        // exactly as `stats_spend` does — no new SQL. Only the two
        // magnitudes are mirrored; the ranking itself is not (see
        // `FleetLifetime`).
        let lifetime = if since.is_some() {
            let all = query
                .payload_field_breakdown(
                    handle,
                    "quantum_target_selected",
                    "vehicle_class",
                    None,
                    None,
                    None,
                    STATS_BUCKET_LIMIT,
                )
                .await?;
            Some(FleetLifetime {
                total_trips: all.iter().map(|b| b.count).sum(),
                ships_flown: all.len() as i64,
            })
        } else {
            None
        };
        // Previous period: the SAME breakdown over the shifted window, so
        // `ships_flown` is a true distinct count for that period rather
        // than a subtraction (see `FleetPrevious`).
        let previous = match previous_window_for(&*query, handle, &range).await? {
            Some(prev) => {
                let before = query
                    .payload_field_breakdown(
                        handle,
                        "quantum_target_selected",
                        "vehicle_class",
                        None,
                        Some(prev.start),
                        Some(prev.end),
                        STATS_BUCKET_LIMIT,
                    )
                    .await?;
                Some(FleetPrevious {
                    total_trips: before.iter().map(|b| b.count).sum(),
                    ships_flown: before.len() as i64,
                })
            }
            None => None,
        };
        Ok(FleetResponse {
            ships: buckets
                .into_iter()
                .map(|b| FleetShipRow {
                    vehicle_class: b.value,
                    trip_count: b.count,
                })
                .collect(),
            lifetime,
            previous,
        })
    }
    .await;
    match result {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_fleet failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockKind {
    Hangar,
    Pad,
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockSize {
    Small,
    Medium,
    Large,
    Xl,
    Unknown,
}

/// Classify a `vehicle_stowed.landing_area` string into (kind, size).
/// Order matters: check `XL` before the other size tokens.
fn classify_landing_area(la: &str) -> (DockKind, DockSize) {
    let kind = if la.contains("Pad") {
        DockKind::Pad
    } else if la.contains("Hangar") {
        DockKind::Hangar
    } else {
        DockKind::Other
    };
    let size = if la.contains("XL") {
        DockSize::Xl
    } else if la.contains("Large") || la.contains("LrgB") || la.contains("_Lrg") {
        DockSize::Large
    } else if la.contains("Medium") || la.contains("MedB") || la.contains("_Med") {
        DockSize::Medium
    } else if la.contains("Small") || la.contains("SmlB") || la.contains("_Sml") {
        DockSize::Small
    } else {
        DockSize::Unknown
    };
    (kind, size)
}

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct DockKindCounts {
    pub hangar: i64,
    pub pad: i64,
    pub other: i64,
}
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct DockSizeCounts {
    pub small: i64,
    pub medium: i64,
    pub large: i64,
    pub xl: i64,
    pub unknown: i64,
}

/// Lifetime twin for a windowed [`DockingResponse`].
///
/// UX Rule 2: a bare number means nothing. "11 stows" tells a player
/// nothing until they know whether that is a busy week or a quiet one.
///
/// Only `total_stows` is mirrored. `by_kind`/`by_size` are the
/// COMPOSITION of that total — which hangars, which ship sizes — and a
/// composition is not a baseline for anything; it is the same judgement
/// that makes `SpendLifetime` omit `top_shop`. A player comparing
/// "hangar vs pad this week" against "hangar vs pad ever" wants two
/// shapes side by side, which is a different feature from a baseline
/// magnitude, and shipping it here would double the response for every
/// caller that never asked.
///
/// `None` when no window was requested: with no `hours` the figure above
/// IS lifetime and a twin would merely repeat it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DockingLifetime {
    pub total_stows: i64,
}

/// Previous-period twin for a windowed [`DockingResponse`]: `total_stows`
/// over the same-length window shifted back once.
///
/// Only the total, for the same reason [`DockingLifetime`] carries only
/// the total: `by_kind`/`by_size` are the composition of that total, and a
/// composition trends in five directions at once, which is a chart, not a
/// baseline.
///
/// `None` means "no comparison to draw", never zero — see
/// [`SpendPrevious`] for the full rationale.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DockingPrevious {
    pub total_stows: i64,
}

/// Response for GET /v1/me/stats/docking — hangar/pad + ship-size dock profile.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct DockingResponse {
    pub total_stows: i64,
    pub by_kind: DockKindCounts,
    pub by_size: DockSizeCounts,
    /// Lifetime baseline for `total_stows` above. `None` when no window
    /// was requested — see [`DockingLifetime`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<DockingLifetime>,
    /// `total_stows` for the period before this one. `None` means there is
    /// no honest comparison to draw — NOT zero. See [`DockingPrevious`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<DockingPrevious>,
}

fn docking_response_from_occurrences(occurrences: DockingOccurrences) -> DockingResponse {
    let mut response = DockingResponse {
        total_stows: occurrences.total(),
        ..DockingResponse::default()
    };
    for bucket in occurrences.landing_areas {
        let (kind, size) = classify_landing_area(&bucket.value);
        match kind {
            DockKind::Hangar => response.by_kind.hangar += bucket.count,
            DockKind::Pad => response.by_kind.pad += bucket.count,
            DockKind::Other => response.by_kind.other += bucket.count,
        }
        match size {
            DockSize::Small => response.by_size.small += bucket.count,
            DockSize::Medium => response.by_size.medium += bucket.count,
            DockSize::Large => response.by_size.large += bucket.count,
            DockSize::Xl => response.by_size.xl += bucket.count,
            DockSize::Unknown => response.by_size.unknown += bucket.count,
        }
    }
    response.by_kind.other += occurrences.unknown;
    response.by_size.unknown += occurrences.unknown;
    response
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/docking",
    tag = "metrics",
    operation_id = "stats_docking",
    params(StatsParams),
    responses(
        (status = 200, description = "Dock profile (hangar/pad + ship size)", body = DockingResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_docking<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let range = match parse_stats_range(&params) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let since = range.since;
    let handle = &user.preferred_username;
    let result: Result<DockingResponse, crate::repo::RepoError> = async {
        let occurrences = query.docking_occurrences(handle, since, None).await?;
        let mut r = docking_response_from_occurrences(occurrences);
        // Lifetime baseline, only when a window was actually requested.
        // With `since = None` the figures above ARE lifetime and a twin
        // would merely repeat them.
        //
        // A second pass over the SAME repo method with `since = None`,
        // exactly as `stats_spend` does — no new SQL. Only the total is
        // mirrored; the kind/size composition is not (see
        // `DockingLifetime`).
        r.lifetime = if since.is_some() {
            let all = query.docking_occurrences(handle, None, None).await?;
            Some(DockingLifetime {
                total_stows: all.total(),
            })
        } else {
            None
        };
        // Previous period: the same breakdown over the shifted window,
        // summed to the one magnitude this endpoint trends.
        r.previous = match previous_window_for(&*query, handle, &range).await? {
            Some(prev) => {
                let before = query
                    .docking_occurrences(handle, Some(prev.start), Some(prev.end))
                    .await?;
                Some(DockingPrevious {
                    total_stows: before.total(),
                })
            }
            None => None,
        };
        Ok(r)
    }
    .await;
    match result {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_docking failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

// ── Correlation surfaces (mission_objective / quantum_route /
//    shop_buy_request / item_equip_change) ─────────────────────────────

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RouteRow {
    pub destination: String,
    pub count: i64,
}

/// Lifetime twin for a windowed [`RoutesResponse`].
///
/// UX Rule 2: a bare number means nothing. "9 jumps to 4 places" tells a
/// player nothing until they know how much of their travelling that is.
///
/// Same shape of judgement as [`FleetLifetime`]: `RoutesResponse` has no
/// scalar of its own, so the baseline is the ranked list's two
/// MAGNITUDES — how much travelling, to how many places. Their windowed
/// counterparts are the sum of `routes[].count` and `routes.len()`, from
/// the very same breakdown. The `routes` ranking itself is deliberately
/// NOT mirrored: a second list names components, not a magnitude.
///
/// Both figures inherit the top-`STATS_BUCKET_LIMIT` truncation of the
/// breakdown they are derived from — the same truncation the windowed
/// list already has.
///
/// `None` when no window was requested: with no `hours` the response
/// already IS lifetime and a twin would merely repeat it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoutesLifetime {
    /// Quantum trips summed across every destination, lifetime.
    pub total_trips: i64,
    /// Distinct destinations travelled to, lifetime.
    pub destinations: i64,
}

/// Previous-period twin for a windowed [`RoutesResponse`]: the same two
/// magnitudes over the same-length window shifted back once.
///
/// `destinations` carries the same non-additivity as
/// [`FleetPrevious::ships_flown`] — a player who flew to Crusader in both
/// periods would vanish from a subtracted count — so this is computed from
/// its own window, not derived.
///
/// `None` means "no comparison to draw", never zero — see
/// [`SpendPrevious`] for the full rationale.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoutesPrevious {
    /// Quantum trips summed across every destination in the previous period.
    pub total_trips: i64,
    /// Distinct destinations travelled to in the previous period.
    pub destinations: i64,
}

/// Response for GET /v1/me/stats/routes — most-travelled quantum
/// destinations, ranked by trip count (from `quantum_route.destination`).
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct RoutesResponse {
    pub routes: Vec<RouteRow>,
    /// Lifetime baseline for the windowed list above. `None` when no
    /// window was requested — see [`RoutesLifetime`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<RoutesLifetime>,
    /// The same magnitudes for the period before this one. `None` means
    /// there is no honest comparison to draw — NOT zero. See
    /// [`RoutesPrevious`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<RoutesPrevious>,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/routes",
    tag = "metrics",
    operation_id = "stats_routes",
    params(StatsParams),
    responses(
        (status = 200, description = "Top quantum destinations by trip count", body = RoutesResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_routes<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let range = match parse_stats_range(&params) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let since = range.since;
    let handle = &user.preferred_username;
    let result: Result<RoutesResponse, crate::repo::RepoError> = async {
        let buckets = query
            .payload_field_breakdown(
                handle,
                "quantum_route",
                "destination",
                None,
                since,
                None,
                STATS_BUCKET_LIMIT,
            )
            .await?;
        // Lifetime baseline, only when a window was actually requested.
        // With `since = None` the list above IS lifetime and a twin would
        // merely repeat it.
        //
        // A second pass over the SAME repo method with `since = None`,
        // exactly as `stats_spend` does — no new SQL. Only the two
        // magnitudes are mirrored; the ranking itself is not (see
        // `RoutesLifetime`).
        let lifetime = if since.is_some() {
            let all = query
                .payload_field_breakdown(
                    handle,
                    "quantum_route",
                    "destination",
                    None,
                    None,
                    None,
                    STATS_BUCKET_LIMIT,
                )
                .await?;
            Some(RoutesLifetime {
                total_trips: all.iter().map(|b| b.count).sum(),
                destinations: all.len() as i64,
            })
        } else {
            None
        };
        // Previous period: the SAME breakdown over the shifted window, so
        // `destinations` is a true distinct count for that period rather
        // than a subtraction (see `RoutesPrevious`).
        let previous = match previous_window_for(&*query, handle, &range).await? {
            Some(prev) => {
                let before = query
                    .payload_field_breakdown(
                        handle,
                        "quantum_route",
                        "destination",
                        None,
                        Some(prev.start),
                        Some(prev.end),
                        STATS_BUCKET_LIMIT,
                    )
                    .await?;
                Some(RoutesPrevious {
                    total_trips: before.iter().map(|b| b.count).sum(),
                    destinations: before.len() as i64,
                })
            }
            None => None,
        };
        Ok(RoutesResponse {
            routes: buckets
                .into_iter()
                .map(|b| RouteRow {
                    destination: b.value,
                    count: b.count,
                })
                .collect(),
            lifetime,
            previous,
        })
    }
    .await;
    match result {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_routes failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

/// Fold raw outcome counts into the two DERIVED figures both the windowed
/// response and its lifetime twin need: `total` (every distinct objective
/// seen) and `completion_pct` (completed over RESOLVED — `no_outcome` is
/// excluded from the denominator, see [`ObjectivesResponse`]).
///
/// Shared by both paths on purpose: a baseline computed by a second copy
/// of this arithmetic could drift from the figure it is a baseline for,
/// and a drifting baseline is worse than no baseline.
fn objective_derived(o: &crate::repo::ObjectiveOutcomes) -> (i64, Option<i64>) {
    let total = o.completed + o.failed + o.unresolved + o.no_outcome;
    let resolved = o.completed + o.failed + o.unresolved;
    let completion_pct = if resolved > 0 {
        Some(o.completed * 100 / resolved)
    } else {
        None
    };
    (total, completion_pct)
}

/// Lifetime twin for a windowed [`ObjectivesResponse`].
///
/// UX Rule 2: a bare number means nothing. "4 completed, 60%" tells a
/// player nothing until they know whether 60% is better or worse than
/// how they usually do.
///
/// Every field is mirrored here, because every field of
/// `ObjectivesResponse` is already a magnitude — there is no list and no
/// "top X" component to leave behind, unlike `SpendLifetime` skipping
/// `top_shop`. `completion_pct` is a rate rather than a count, but it is
/// the headline figure of this endpoint and the one a baseline is most
/// worth having for; it is a scalar, not a component, so it belongs.
///
/// `None` when no window was requested: with no `hours` the figures above
/// ARE lifetime and a twin would merely repeat them.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ObjectivesLifetime {
    pub completed: i64,
    pub failed: i64,
    pub unresolved: i64,
    pub no_outcome: i64,
    pub total: i64,
    /// Lifetime completion rate, on the same completed-over-resolved
    /// basis as [`ObjectivesResponse::completion_pct`]. `None` when
    /// nothing has ever resolved.
    pub completion_pct: Option<i64>,
}

/// Previous-period twin for a windowed [`ObjectivesResponse`]: every
/// figure over the same-length window shifted back once.
///
/// Where [`ObjectivesLifetime`] says "60% is below your usual 75%", this
/// says "75% last week, 60% this week" — a direction rather than a
/// standing. Every field is mirrored, for the same reason the lifetime
/// twin mirrors every field: they are all magnitudes, and the endpoint has
/// no list component to leave behind.
///
/// The counts are a fold over DISTINCT objectives inside the previous
/// window (`objective_outcomes` with both bounds set), not a subtraction:
/// an objective spanning both periods would otherwise be miscounted, and
/// one that reported `in_progress` before the window and completed inside
/// it resolves on the evidence the window actually contains.
///
/// `completion_pct` is `None` here on the same terms as everywhere else —
/// nothing resolved in that window — which is distinct from the whole
/// struct being `None`. The first says "they played, nothing finished";
/// the second says "there is nobody to compare to".
///
/// `None` for the struct means "no comparison to draw", never zero — see
/// [`SpendPrevious`] for the full rationale.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ObjectivesPrevious {
    pub completed: i64,
    pub failed: i64,
    pub unresolved: i64,
    pub no_outcome: i64,
    pub total: i64,
    /// Completion rate for the previous period, on the same
    /// completed-over-resolved basis as
    /// [`ObjectivesResponse::completion_pct`]. `None` when nothing
    /// resolved in that window.
    pub completion_pct: Option<i64>,
}

/// Response for GET /v1/me/stats/objectives — mission-objective outcomes,
/// counted per DISTINCT objective at its terminal state (not per state
/// transition). `completion_pct` = completed / (completed + failed +
/// unresolved); `no_outcome` objectives are excluded from the ratio (they
/// never resolved), `None` when nothing has resolved yet. `unresolved`
/// unions the stored states `withdrawn` (parsed as such from v1.8.149 on)
/// and `unknown` (the parser catch-all — how older collectors stored
/// WITHDRAWN, and where a state CIG ships that the parser has no variant
/// for yet still lands). Such an objective IS resolved and was NOT
/// completed, so it belongs in the denominator.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ObjectivesResponse {
    pub completed: i64,
    pub failed: i64,
    /// Resolved but not completed — the stored states `withdrawn` and
    /// `unknown` (see the type doc). Counts toward the completion
    /// denominator.
    pub unresolved: i64,
    /// Objectives for which no terminal state was ever observed — the
    /// only payload state ever seen for them was `in_progress`. This is
    /// NOT a count of currently-active objectives (a player holds only a
    /// handful at a time); it's missions the parser never saw resolve —
    /// abandoned, the app exited, or the log rotated mid-mission.
    /// Excluded from `completion_pct`'s denominator: an objective with no
    /// observed outcome hasn't resolved one way or the other.
    pub no_outcome: i64,
    pub total: i64,
    pub completion_pct: Option<i64>,
    /// Lifetime baseline for the windowed figures above. `None` when no
    /// window was requested — see [`ObjectivesLifetime`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<ObjectivesLifetime>,
    /// The same figures for the period before this one. `None` means
    /// there is no honest comparison to draw — NOT zero. See
    /// [`ObjectivesPrevious`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<ObjectivesPrevious>,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/objectives",
    tag = "metrics",
    operation_id = "stats_objectives",
    params(StatsParams),
    responses(
        (status = 200, description = "Mission-objective completion breakdown", body = ObjectivesResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_objectives<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let range = match parse_stats_range(&params) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let since = range.since;
    let handle = &user.preferred_username;
    let result: Result<ObjectivesResponse, crate::repo::RepoError> = async {
        let o = query.objective_outcomes(handle, since, None).await?;
        let (total, completion_pct) = objective_derived(&o);
        // Lifetime baseline, only when a window was actually requested.
        // With `since = None` the figures above ARE lifetime and a twin
        // would merely repeat them.
        //
        // A second pass over the SAME repo method with `since = None`,
        // exactly as `stats_spend` does — no new SQL.
        let lifetime = if since.is_some() {
            let all = query.objective_outcomes(handle, None, None).await?;
            let (total, completion_pct) = objective_derived(&all);
            Some(ObjectivesLifetime {
                completed: all.completed,
                failed: all.failed,
                unresolved: all.unresolved,
                no_outcome: all.no_outcome,
                total,
                completion_pct,
            })
        } else {
            None
        };
        // Previous period: the same distinct-objective fold, bounded on
        // BOTH sides so the objectives counted are the ones that objective
        // window actually saw (see `ObjectivesPrevious`). `objective_derived`
        // is reused rather than re-typed so the previous `completion_pct`
        // cannot drift from the one it is compared against.
        let previous = match previous_window_for(&*query, handle, &range).await? {
            Some(prev) => {
                let before = query
                    .objective_outcomes(handle, Some(prev.start), Some(prev.end))
                    .await?;
                let (total, completion_pct) = objective_derived(&before);
                Some(ObjectivesPrevious {
                    completed: before.completed,
                    failed: before.failed,
                    unresolved: before.unresolved,
                    no_outcome: before.no_outcome,
                    total,
                    completion_pct,
                })
            }
            None => None,
        };
        Ok(ObjectivesResponse {
            completed: o.completed,
            failed: o.failed,
            unresolved: o.unresolved,
            no_outcome: o.no_outcome,
            total,
            completion_pct,
            lifetime,
            previous,
        })
    }
    .await;
    match result {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_objectives failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

/// Lower a `StepState` to its DB/DTO TEXT form — same rationale as
/// `crate::repo::contract_state_str`/`closed_by_str`: written out
/// explicitly rather than round-tripping through `serde_json`, but kept
/// spelled identically to the enum's own
/// `#[serde(rename_all = "snake_case")]` — don't let the two drift.
fn step_state_str(s: StepState) -> &'static str {
    match s {
        StepState::InProgress => "in_progress",
        StepState::Complete => "complete",
        StepState::Withdrawn => "withdrawn",
        StepState::Failed => "failed",
    }
}

/// One step within a materialised contract run, DTO-shaped for the wire.
/// Mirrors `starstats_core::contract_life::ContractStep`, with `state`
/// lowered to its lowercase snake_case TEXT form (see [`step_state_str`]),
/// matching how `ContractRunRow.state`/`closed_by` are already plain
/// `String`s rather than enums. `text` is passed through byte-for-byte —
/// it's the readable HUD banner text, verbatim; see `ContractStep::text`'s
/// own doc for why it must never be trimmed, cased, or rewritten here.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContractStepRow {
    pub objective_id: Option<String>,
    pub order: i32,
    pub text: Option<String>,
    pub state: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl From<ContractStep> for ContractStepRow {
    fn from(s: ContractStep) -> Self {
        Self {
            objective_id: s.objective_id,
            order: s.order as i32,
            text: s.text,
            state: step_state_str(s.state).to_string(),
            started_at: s.started_at,
            completed_at: s.completed_at,
        }
    }
}

/// One materialised contract run, DTO-shaped for the wire. Mirrors
/// `crate::repo::ContractRunRow`, with `steps` lowered through
/// [`ContractStepRow`] (see its own doc). The run-level counts below
/// (`step_count`/`steps_complete`/`steps_remaining`) stay as-is: a cheap
/// summary for views that don't need the per-step breakdown, alongside
/// the full one in `steps` for those that do.
///
/// `steps` is always present — never null or absent — but is only
/// populated when the request opts in via `?include_steps=true` (see
/// [`ContractsParams`]'s doc for why it defaults off). An empty `steps`
/// therefore means "not requested", NOT "this run has no steps" — a
/// consumer must not read it as the latter.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
    pub steps: Vec<ContractStepRow>,
}

impl From<crate::repo::ContractRunRow> for ContractRunRow {
    fn from(r: crate::repo::ContractRunRow) -> Self {
        Self {
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
            steps: r.steps.into_iter().map(ContractStepRow::from).collect(),
        }
    }
}

/// Response for `GET /v1/me/stats/contracts` — materialised contract runs
/// (`starstats_core::contract_life::derive_contract_runs`, rolled up by
/// migration 0060's `contract_runs` table) plus an outcome summary, so a
/// later widget needs one call for both.
///
/// `total` and the six named buckets below all EXCLUDE `Superseded` runs:
/// a re-accept of the same mission closes the earlier run as bookkeeping,
/// not an outcome (measured on 280 real logs: 69 of 609 runs are
/// `Superseded` — 11%, not a rounding detail). A superseded run still
/// appears in `runs` (it's real history for the mission) but never
/// contributes to a headline count, so `total` can be less than
/// `runs.len()`.
///
/// `withdrawn` and `unknown` aren't named in the original spec for this
/// endpoint; they're added deliberately rather than folded into an
/// existing bucket. `ObjectivesResponse` collapses its own equivalent
/// ambiguity into a single `unresolved` field, but that's because its
/// source parser genuinely can't distinguish the cases. Here the fold
/// (`ContractState`) already distinguishes `Withdrawn` (player explicitly
/// backed out via the HUD) from `Unknown` (stream ended long ago with no
/// closing evidence, deliberately kept distinct from `InProgress` — see
/// that enum's doc) from `Abandoned` (closed by inference: crash, session
/// gap, shard change). Collapsing real fidelity the fold already computed
/// would be the wrong kind of guess.
///
/// `completion_pct` = completed / (completed + failed + abandoned).
/// `withdrawn`/`in_progress`/`unknown` runs are excluded from that ratio's
/// denominator — none of them is the failure `abandoned`/`failed` track,
/// and `in_progress` is still open. `None` when the denominator is 0.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ContractsResponse {
    pub total: i64,
    pub completed: i64,
    pub failed: i64,
    /// `ContractState::Withdrawn` — player explicitly backed out via the
    /// HUD. Resolved, but not a failure; excluded from `completion_pct`'s
    /// denominator (see struct doc).
    pub withdrawn: i64,
    /// Closed by inference (session end, crash, session gap, shard
    /// change) rather than an observed HUD banner.
    pub abandoned: i64,
    pub in_progress: i64,
    /// `ContractState::Unknown` — left open by a stream that ended long
    /// ago with no closing evidence; excluded from `completion_pct`'s
    /// denominator (see struct doc).
    pub unknown: i64,
    pub completion_pct: Option<i64>,
    /// Every materialised run in the window, newest-accepted first
    /// (including `Superseded` ones) — see the struct doc.
    pub runs: Vec<ContractRunRow>,
}

/// Pure: fold materialised runs into the summary response. `Superseded`
/// runs (and any state string the fold doesn't emit) contribute to no
/// bucket — bookkeeping from a re-accept, not an outcome — but are kept
/// in `runs` untouched. Extracted for a store-free unit test, mirroring
/// [`biggest_confirmed_trade`].
///
/// `include_steps` gates the wire-level `steps` field per run (see
/// [`ContractsParams`]'s doc) — cleared here, before the DTO conversion,
/// rather than after, so the `?include_steps` default doesn't pay to
/// build wire step rows just to throw them away.
fn summarize_contracts(
    mut runs: Vec<crate::repo::ContractRunRow>,
    include_steps: bool,
) -> ContractsResponse {
    if !include_steps {
        for run in &mut runs {
            run.steps.clear();
        }
    }
    let mut r = ContractsResponse::default();
    for run in &runs {
        match run.state.as_str() {
            "completed" => r.completed += 1,
            "failed" => r.failed += 1,
            "withdrawn" => r.withdrawn += 1,
            "abandoned" => r.abandoned += 1,
            "in_progress" => r.in_progress += 1,
            "unknown" => r.unknown += 1,
            _ => {}
        }
    }
    r.total = r.completed + r.failed + r.withdrawn + r.abandoned + r.in_progress + r.unknown;
    let resolved = r.completed + r.failed + r.abandoned;
    r.completion_pct = if resolved > 0 {
        Some(r.completed * 100 / resolved)
    } else {
        None
    };
    r.runs = runs.into_iter().map(ContractRunRow::from).collect();
    r
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/contracts",
    tag = "metrics",
    operation_id = "stats_contracts",
    params(ContractsParams),
    responses(
        (status = 200, description = "Materialised contract runs plus an outcome summary. `steps` is populated only when `include_steps=true` is passed", body = ContractsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_contracts<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<ContractsParams>,
) -> impl IntoResponse {
    // Shares the range-scoped validation, but takes only `since` — this
    // endpoint carries no lifetime twin and so has no previous one either.
    let since = match parse_stats_range(&StatsParams {
        hours: params.hours,
    }) {
        Ok(r) => r.since,
        Err(r) => return r,
    };
    let include_steps = params.include_steps.unwrap_or(false);
    match query.contract_runs(&user.preferred_username, since).await {
        Ok(runs) => (
            StatusCode::OK,
            Json(summarize_contracts(runs, include_steps)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_contracts failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

/// One run-observed contract name with no matching row in the
/// published catalog. Wire-shaped mirror of `crate::repo::ContractGapRow`
/// — see that type's doc for the matching rule.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractGapDto {
    pub name: String,
    pub run_count: i64,
    pub distinct_handles: i64,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

impl From<crate::repo::ContractGapRow> for ContractGapDto {
    fn from(r: crate::repo::ContractGapRow) -> Self {
        Self {
            name: r.name,
            run_count: r.run_count,
            distinct_handles: r.distinct_handles,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
        }
    }
}

/// Response for `GET /v1/admin/contracts/gaps` — run-observed contract
/// names missing from the published catalog, ranked by OCCURRENCE, not
/// distinct name count (see `crate::repo::PostgresStore::contract_catalog_gaps`'s
/// doc for why: Combat Gauntlet is ~5% of distinct unmatched names in a
/// 280-log corpus but 37% of all runs — a name-ranked list buries it).
///
/// `total_unmatched_runs` is the grand total across EVERY gap name, not
/// just the `gaps` page below — the admin's "how big is the whole gap"
/// headline number, independent of `?limit`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractCatalogGapsResponse {
    pub gaps: Vec<ContractGapDto>,
    pub total_unmatched_runs: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ContractGapsParams {
    /// Max ranked rows to return, clamped to [1, 200]. Defaults to 20.
    #[serde(default)]
    pub limit: Option<i64>,
}

const CONTRACT_GAPS_LIMIT_DEFAULT: i64 = 20;
const CONTRACT_GAPS_LIMIT_MAX: i64 = 200;

/// GET /v1/admin/contracts/gaps — surfaces run-observed contract names
/// the catalog is missing, so a maintainer can see (and prioritise
/// publishing) the biggest gaps at a glance. Read-only diagnostic, so
/// gated on moderator (admins inherit) — same posture as
/// `admin_routes::list_audit`.
#[utoipa::path(
    get,
    path = "/v1/admin/contracts/gaps",
    tag = "admin",
    params(ContractGapsParams),
    responses(
        (status = 200, description = "Run-observed contract names missing from the catalog, ranked by occurrence", body = ContractCatalogGapsResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn contract_catalog_gaps(
    State(store): State<Arc<PostgresStore>>,
    _: RequireModerator,
    Query(params): Query<ContractGapsParams>,
) -> impl IntoResponse {
    let limit = params
        .limit
        .unwrap_or(CONTRACT_GAPS_LIMIT_DEFAULT)
        .clamp(1, CONTRACT_GAPS_LIMIT_MAX);

    let gaps = match store.contract_catalog_gaps(limit).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "contract_catalog_gaps failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };
    let total_unmatched_runs = match store.contract_catalog_gaps_total().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "contract_catalog_gaps_total failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };

    (
        StatusCode::OK,
        Json(ContractCatalogGapsResponse {
            gaps: gaps.into_iter().map(ContractGapDto::from).collect(),
            total_unmatched_runs,
        }),
    )
        .into_response()
}

/// Lifetime twin for a windowed [`SpendResponse`].
///
/// UX Rule 2: a bare number means nothing. "12,000 aUEC" tells a player
/// nothing until they know whether that is a big or small slice of what
/// they have ever spent.
///
/// Deliberately the INVERSE of `LivesWindow`. There, the top-level
/// fields are lifetime and `window` is the slice. Here the top-level
/// fields are already the requested window — widgets pass `hours` and
/// depend on the top level BEING the scoped figure — so the twin adds
/// the lifetime baseline instead. Flipping the top level would silently
/// change what every existing caller receives.
///
/// `None` when no window was requested: with no `hours` the top-level
/// figures ARE lifetime and a twin would just repeat them.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SpendLifetime {
    pub total_auec: i64,
    pub purchases: i64,
}

/// Previous-period twin for a windowed [`SpendResponse`]: the SAME figures
/// over the same-length window shifted back once (`hours=168` → the week
/// before last week).
///
/// Answers a different question from [`SpendLifetime`]. The lifetime twin
/// gives share-of-career ("is 12,000 aUEC much of what I have ever
/// spent?"); this one gives DIRECTION ("am I spending more than I was?").
/// A player can be well below their lifetime average and still climbing
/// hard, so neither twin implies the other.
///
/// Mirrors the windowed scalars only. `top_shop` is deliberately absent
/// for exactly the reason `SpendLifetime` omits it: it names a component,
/// not a magnitude, and a trend needs magnitudes.
///
/// `None` — and this is the point of the field being an `Option` — means
/// "no comparison to draw", never "zero". See [`parse_stats_range`] for
/// the two structural cases and [`previous_window_for`] for the per-handle
/// one. A player who was active but spent nothing gets `Some` with zeros,
/// because that IS a real comparison and "down from 12,000" is a true
/// thing to say about them.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SpendPrevious {
    pub total_auec: i64,
    pub purchases: i64,
}

/// Response for GET /v1/me/stats/spend — kiosk spending. `total_auec`
/// sums `shop_buy_request.price` (modern kiosk lines only carry a price);
/// `purchases` counts ALL shop-buy events (priced + legacy); `top_shop`
/// is the most-frequent raw `shop_name` (web prettifies).
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct SpendResponse {
    pub total_auec: i64,
    pub purchases: i64,
    pub top_shop: Option<String>,
    /// Lifetime baseline for the windowed figures above. `None` when no
    /// window was requested — see `SpendLifetime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<SpendLifetime>,
    /// The same figures for the period before this one. `None` means
    /// there is no honest comparison to draw — NOT zero. See
    /// [`SpendPrevious`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<SpendPrevious>,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/spend",
    tag = "metrics",
    operation_id = "stats_spend",
    params(StatsParams),
    responses(
        (status = 200, description = "Kiosk spending totals", body = SpendResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_spend<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let range = match parse_stats_range(&params) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let since = range.since;
    let handle = &user.preferred_username;
    let result: Result<SpendResponse, crate::repo::RepoError> = async {
        let total_auec = query
            .payload_numeric_sum(handle, "shop_buy_request", "price", since, None)
            .await?;
        let purchases = query
            .count_event_type(handle, "shop_buy_request", None, since, None)
            .await? as i64;
        let top_shop = query
            .payload_field_breakdown(
                handle,
                "shop_buy_request",
                "shop_name",
                None,
                since,
                None,
                1,
            )
            .await?
            .into_iter()
            .next()
            .map(|b| b.value);
        // Lifetime baseline, only when a window was actually requested.
        // With `since = None` the figures above ARE lifetime and a twin
        // would merely repeat them.
        //
        // A second pass over the same repo methods with `since = None`,
        // exactly as `stats_records` and `stats_lives` do — no new SQL.
        // `top_shop` is deliberately NOT mirrored: it names a component,
        // not a magnitude, so it is not a baseline for anything.
        let lifetime = if since.is_some() {
            Some(SpendLifetime {
                total_auec: query
                    .payload_numeric_sum(handle, "shop_buy_request", "price", None, None)
                    .await?,
                purchases: query
                    .count_event_type(handle, "shop_buy_request", None, None, None)
                    .await? as i64,
            })
        } else {
            None
        };
        // Previous period: the same two repo methods over the shifted
        // window. Computed directly rather than as `window(2N) - window(N)`
        // — subtraction happens to be exact for a sum and a count, but the
        // sibling endpoints have distinct-counts where it is simply wrong,
        // and a family of trend figures where some are exact and some are
        // subtly inflated is worse than one that is uniformly honest.
        let previous = match previous_window_for(&*query, handle, &range).await? {
            Some(prev) => Some(SpendPrevious {
                total_auec: query
                    .payload_numeric_sum(
                        handle,
                        "shop_buy_request",
                        "price",
                        Some(prev.start),
                        Some(prev.end),
                    )
                    .await?,
                purchases: query
                    .count_event_type(
                        handle,
                        "shop_buy_request",
                        None,
                        Some(prev.start),
                        Some(prev.end),
                    )
                    .await? as i64,
            }),
            None => None,
        };
        Ok(SpendResponse {
            total_auec,
            purchases,
            lifetime,
            previous,
            top_shop,
        })
    }
    .await;
    match result {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_spend failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoadoutItemRow {
    pub item_class: String,
    pub count: i64,
}

/// Response for GET /v1/me/stats/loadout-activity — gear equip/store
/// churn over time (distinct from the /me/loadout snapshot). `equips` /
/// `stores` come from `item_equip_change.action`; `top_items` ranks the
/// most-changed `item_class` values.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct LoadoutActivityResponse {
    pub equips: i64,
    pub stores: i64,
    pub top_items: Vec<LoadoutItemRow>,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/loadout-activity",
    tag = "metrics",
    operation_id = "stats_loadout_activity",
    responses(
        (status = 200, description = "Gear equip/store activity", body = LoadoutActivityResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_loadout_activity<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    let handle = &user.preferred_username;
    let result: Result<LoadoutActivityResponse, crate::repo::RepoError> = async {
        let actions = query
            .payload_field_breakdown(
                handle,
                "item_equip_change",
                "action",
                None,
                None,
                None,
                STATS_BUCKET_LIMIT,
            )
            .await?;
        let items = query
            .payload_field_breakdown(
                handle,
                "item_equip_change",
                "item_class",
                None,
                None,
                None,
                STATS_BUCKET_LIMIT,
            )
            .await?;
        let mut r = LoadoutActivityResponse::default();
        for b in actions {
            match b.value.as_str() {
                "equip" => r.equips += b.count,
                "store" => r.stores += b.count,
                _ => {}
            }
        }
        r.top_items = items
            .into_iter()
            .map(|b| LoadoutItemRow {
                item_class: b.value,
                count: b.count,
            })
            .collect();
        Ok(r)
    }
    .await;
    match result {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "stats_loadout_activity failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed")
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoadoutStatsResponse {
    pub hours: i64,
    pub attachments: u64,
    pub top_items: Vec<StatsBucket>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StabilityStatsResponse {
    pub hours: i64,
    pub crashes: u64,
    pub by_channel: Vec<StatsBucket>,
}

fn parse_stats_window(params: &StatsParams) -> Result<(i64, Option<DateTime<Utc>>), Response> {
    let hours = params.hours.unwrap_or(STATS_DEFAULT_HOURS);
    if hours <= 0 || hours > STATS_MAX_HOURS {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_hours"));
    }
    let since = Some(Utc::now() - chrono::Duration::hours(hours));
    Ok((hours, since))
}

/// The period IMMEDIATELY BEFORE a requested window: same length, shifted
/// back once. For `hours = N` that is `[now-2N, now-N)`.
///
/// `end` is EXCLUSIVE and is exactly the current window's INCLUSIVE
/// `since`, so the two windows tile without overlapping: an event landing
/// on the shared instant belongs to the current window and is counted
/// once across the pair. See [`crate::repo::EventQuery`]'s window
/// convention, which the repo methods implement on both backends.
#[derive(Debug, Clone, Copy)]
struct PreviousWindow {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

/// Resolved time bounds for one range-scoped stats request.
///
/// Both fields come from a SINGLE `Utc::now()`. Reading the clock twice
/// would leave a sliver of time between the current window's lower bound
/// and the previous window's upper bound — small, but an event in it would
/// belong to neither period and silently vanish from the comparison.
struct StatsRange {
    /// Inclusive lower bound of the requested window; `None` = lifetime.
    since: Option<DateTime<Utc>>,
    /// The same-length window shifted back one period, or `None` when
    /// there cannot be a meaningful one — see [`parse_stats_range`].
    previous: Option<PreviousWindow>,
}

/// Range-scoped variant of [`parse_stats_window`] for the endpoints that
/// were historically lifetime-only (`fleet` / `docking` / `routes` /
/// `objectives` / `spend`). Unlike `parse_stats_window`, an ABSENT `hours`
/// stays lifetime (`since = None`, no filter) rather than defaulting to a
/// 30-day window — so these endpoints remain backward-compatible when the
/// dashboard range selector is unset. A present `hours` is validated
/// identically (`<= 0` or `> STATS_MAX_HOURS` → 400 `invalid_hours`) and
/// resolved to the trailing-window lower bound.
///
/// `previous` is `None` in two cases, both of which would otherwise
/// produce a comparison against nothing:
///
/// 1. **No window requested.** The response already IS lifetime; there is
///    no period to sit "before" it.
/// 2. **The window already spans retention.** `STATS_MAX_HOURS` is a full
///    year, which is also the retention limit — the dashboard's `all`
///    range. Its previous period, `[now-730d, now-365d)`, lies entirely
///    outside what the database still holds, so it would read as an empty
///    window for EVERY player and render as "-100%, trending down" against
///    data that was merely swept, not un-played.
///
/// Case 2 is a floor, not a full fix: a 300-day window's previous period
/// also reaches past retention and will read low. Retention truncation is
/// gradual and this guard is deliberately the one bright line we can draw
/// without a per-handle retention probe. Dashboard ranges below `all` are
/// weeks and months, comfortably inside it.
///
/// Being non-`None` here only means the comparison is well-FORMED. Whether
/// it is MEANINGFUL — whether the player existed yet — is a separate
/// question answered per handle by
/// [`crate::repo::EventQuery::has_events_in_window`].
fn parse_stats_range(params: &StatsParams) -> Result<StatsRange, Response> {
    let Some(hours) = params.hours else {
        return Ok(StatsRange {
            since: None,
            previous: None,
        });
    };
    if hours <= 0 || hours > STATS_MAX_HOURS {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_hours"));
    }
    let now = Utc::now();
    let since = now - chrono::Duration::hours(hours);
    let previous = if hours >= STATS_MAX_HOURS {
        None
    } else {
        Some(PreviousWindow {
            start: now - chrono::Duration::hours(hours * 2),
            end: since,
        })
    };
    Ok(StatsRange {
        since: Some(since),
        previous,
    })
}

/// Resolve the previous-period window a handle should actually be compared
/// against, or `None` if there is no honest comparison to make.
///
/// This is the second of the two gates described on [`parse_stats_range`]:
/// it asks whether the handle was PRESENT in that window at all.
///
/// The distinction it protects is between a player who logged in and
/// happened to buy nothing (a real zero, worth comparing against) and one
/// who had not signed up yet (no data, nothing to compare against).
/// Both look like zero in every aggregate, and only the first is a fact
/// about how they played. Reporting "+47 more than the previous period" to
/// someone in their first week invents a baseline out of their own absence.
async fn previous_window_for<Q: EventQuery>(
    query: &Q,
    handle: &str,
    range: &StatsRange,
) -> Result<Option<PreviousWindow>, crate::repo::RepoError> {
    let Some(prev) = range.previous else {
        return Ok(None);
    };
    if query
        .has_events_in_window(handle, prev.start, prev.end)
        .await?
    {
        Ok(Some(prev))
    } else {
        Ok(None)
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/combat",
    tag = "metrics",
    operation_id = "stats_combat",
    params(StatsParams),
    responses(
        (status = 200, description = "Combat stats", body = CombatStatsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_combat<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let (hours, since) = match parse_stats_window(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let handle = user.preferred_username.as_str();
    let killer_filter = PayloadFilter {
        field: "killer",
        equals: handle,
    };
    let victim_filter = PayloadFilter {
        field: "victim",
        equals: handle,
    };

    // Kills + deaths in ONE FILTER scan instead of three separate index scans:
    //   kills        = actor_death where the caller is the killer
    //   deaths_actor = actor_death where the caller is the victim (legacy CIG)
    //   deaths_player= all player_death rows (modern CIG format — what live
    //                  builds emit; without this union a current-build user
    //                  sees deaths=0 even after dying repeatedly)
    let (kills, deaths_actor, deaths_player) = query
        .combat_counts(handle, handle, since)
        .await
        .unwrap_or((0, 0, 0));
    let deaths = deaths_actor.saturating_add(deaths_player);
    // Top weapons used by the caller — scoped to kills, otherwise
    // we'd be showing weapons that killed the caller (a different,
    // less-flattering stat that lives under deaths_by_zone next door).
    // player_death has no weapon field in modern logs, so kill-side
    // weapons stay actor_death-only.
    let top_weapons = query
        .payload_field_breakdown(
            handle,
            "actor_death",
            "weapon",
            Some(killer_filter),
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    // Deaths by zone: merge actor_death.zone (victim=caller) and
    // player_death.zone (no filter needed — player_death rows are
    // intrinsically the caller's). Rows where zone is null are
    // already dropped by the repo (filter_map on Option<String>).
    let zone_actor = query
        .payload_field_breakdown(
            handle,
            "actor_death",
            "zone",
            Some(victim_filter),
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    let zone_player = query
        .payload_field_breakdown(
            handle,
            "player_death",
            "zone",
            None,
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    let deaths_by_zone = merge_buckets(zone_actor, zone_player, STATS_BUCKET_LIMIT as usize);
    // Provenance for the death total. Reuses the existing breakdown
    // query rather than adding a repo method: `player_death` rows carry
    // `body_class`, and the reconstructed ones are marked "inferred" —
    // the same marker `character_life` keys `death_inferred` on.
    //
    // Best-effort: a breakdown failure means we cannot say how much was
    // inferred, and claiming zero would assert "all observed" without
    // evidence. `deaths_inferred` then stays 0 and the surface simply
    // shows no provenance marker, which is the honest fallback.
    let deaths_inferred = query
        .payload_field_breakdown(
            handle,
            "player_death",
            "body_class",
            None,
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.value.eq_ignore_ascii_case("inferred"))
        .map(|b| b.count.max(0) as u64)
        .sum::<u64>()
        .min(deaths);
    (
        StatusCode::OK,
        Json(CombatStatsResponse {
            hours,
            kills,
            deaths,
            deaths_inferred,
            top_weapons: top_weapons.into_iter().map(StatsBucket::from).collect(),
            deaths_by_zone: deaths_by_zone.into_iter().map(StatsBucket::from).collect(),
        }),
    )
        .into_response()
}

/// Sum bucket counts across two lists keyed by `value`, then re-sort
/// by descending count (tie-broken by value asc) and cap at `limit`.
/// Used to merge the same logical dimension from two event sources
/// (e.g. zone from `actor_death` and `player_death`).
fn merge_buckets(
    a: Vec<PayloadFieldBucket>,
    b: Vec<PayloadFieldBucket>,
    limit: usize,
) -> Vec<PayloadFieldBucket> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, i64> = HashMap::new();
    for bucket in a.into_iter().chain(b.into_iter()) {
        *counts.entry(bucket.value).or_insert(0) += bucket.count;
    }
    let mut merged: Vec<PayloadFieldBucket> = counts
        .into_iter()
        .map(|(value, count)| PayloadFieldBucket { value, count })
        .collect();
    merged.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    merged.truncate(limit);
    merged
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/travel",
    tag = "metrics",
    operation_id = "stats_travel",
    params(StatsParams),
    responses(
        (status = 200, description = "Travel stats", body = TravelStatsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_travel<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let (hours, since) = match parse_stats_window(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let quantum_jumps = query
        .count_event_type(
            &user.preferred_username,
            "quantum_target_selected",
            None,
            since,
            None,
        )
        .await
        .unwrap_or(0);
    let top_destinations = query
        .payload_field_breakdown(
            &user.preferred_username,
            "quantum_target_selected",
            "destination",
            None,
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    let planets_visited = query
        .payload_field_breakdown(
            &user.preferred_username,
            "planet_terrain_load",
            "planet",
            None,
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(TravelStatsResponse {
            hours,
            quantum_jumps,
            top_destinations: top_destinations
                .into_iter()
                .map(StatsBucket::from)
                .collect(),
            planets_visited: planets_visited.into_iter().map(StatsBucket::from).collect(),
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/loadout",
    tag = "metrics",
    operation_id = "stats_loadout",
    params(StatsParams),
    responses(
        (status = 200, description = "Loadout stats", body = LoadoutStatsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_loadout<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let (hours, since) = match parse_stats_window(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let attachments = query
        .count_event_type(
            &user.preferred_username,
            "attachment_received",
            None,
            since,
            None,
        )
        .await
        .unwrap_or(0);
    let top_items = query
        .payload_field_breakdown(
            &user.preferred_username,
            "attachment_received",
            "item_class",
            None,
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(LoadoutStatsResponse {
            hours,
            attachments,
            top_items: top_items.into_iter().map(StatsBucket::from).collect(),
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/stability",
    tag = "metrics",
    operation_id = "stats_stability",
    params(StatsParams),
    responses(
        (status = 200, description = "Stability stats", body = StabilityStatsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_stability<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let (hours, since) = match parse_stats_window(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let crashes = query
        .count_event_type(&user.preferred_username, "game_crash", None, since, None)
        .await
        .unwrap_or(0);
    let by_channel = query
        .payload_field_breakdown(
            &user.preferred_username,
            "game_crash",
            "channel",
            None,
            since,
            None,
            STATS_BUCKET_LIMIT,
        )
        .await
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(StabilityStatsResponse {
            hours,
            crashes,
            by_channel: by_channel.into_iter().map(StatsBucket::from).collect(),
        }),
    )
        .into_response()
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PlaytimeStatsResponse {
    pub hours: i64,
    pub total_playtime_secs: i64,
    pub session_count: i64,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/playtime",
    tag = "metrics",
    operation_id = "stats_playtime",
    params(PlaytimeParams),
    responses(
        (status = 200, description = "Total playtime over the window (or all-time when all_time=true)", body = PlaytimeStatsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_playtime<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<PlaytimeParams>,
) -> impl IntoResponse {
    // all_time short-circuits the bounded window: since=None aggregates
    // over every recorded session, and hours=0 is the sentinel echoed
    // back to the caller.
    let (hours, since) = if params.all_time.unwrap_or(false) {
        (0, None)
    } else {
        match parse_stats_window(&StatsParams {
            hours: params.hours,
        }) {
            Ok(v) => v,
            Err(r) => return r,
        }
    };
    let handle = user.preferred_username.as_str();
    let total_playtime_secs = match query.total_playtime_secs(handle, since).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "stats_playtime total_playtime_secs failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response();
        }
    };
    let session_count = match query.count_sessions_since(handle, since).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "stats_playtime count_sessions_since failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response();
        }
    };
    (
        StatusCode::OK,
        Json(PlaytimeStatsResponse {
            hours,
            total_playtime_secs,
            session_count,
        }),
    )
        .into_response()
}

/// Response for `GET /v1/me/stats/records` — all-time "records" computed
/// server-side over the FULL event history. Replaces the web widget's
/// fetch-capped, client-side computation (audit F9).
///
/// Not (yet) in the OpenAPI spec — the web consumes it via a hand-typed
/// wrapper in `lib/api.ts`; add `#[utoipa::path]` + regen the TS client
/// when a typed consumer needs it.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordsResponse {
    pub longest_session_secs: i64,
    pub busiest_session_events: i64,
    pub longest_survival_streak_secs: i64,
    pub deadliest_session_deaths: i64,
    /// Present only when the caller passes `?hours=N`: the same records
    /// computed over just the trailing N-hour window. `None` (omitted)
    /// for the default all-time request, keeping the shape backward
    /// compatible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<RecordsWindow>,
}

/// Range-windowed twin of [`RecordsResponse`]'s lifetime fields, computed
/// over just the trailing `hours`-hour window. Not in the OpenAPI spec —
/// mirrors the hand-typed `RecordsResponse` in `lib/api.ts`.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordsWindow {
    /// The (clamped) window size the figures below were computed over.
    pub hours: i64,
    pub longest_session_secs: i64,
    pub busiest_session_events: i64,
    pub longest_survival_streak_secs: i64,
    pub deadliest_session_deaths: i64,
}

pub async fn stats_records<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let handle = user.preferred_username.as_str();

    // Lifetime figures are always computed (since = None).
    let lifetime = match query.records_for_handle(handle, None).await {
        Ok(rec) => rec,
        Err(e) => {
            tracing::error!(error = %e, "stats_records failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };

    // A positive `hours` adds the windowed twin, clamped to the same
    // one-year ceiling the other stats endpoints use.
    let window = match params.hours {
        Some(h) if h > 0 => {
            let hours = h.min(STATS_MAX_HOURS);
            let since = Some(Utc::now() - chrono::Duration::hours(hours));
            match query.records_for_handle(handle, since).await {
                Ok(rec) => Some(RecordsWindow {
                    hours,
                    longest_session_secs: rec.longest_session_secs,
                    busiest_session_events: rec.busiest_session_events,
                    longest_survival_streak_secs: rec.longest_survival_streak_secs,
                    deadliest_session_deaths: rec.deadliest_session_deaths,
                }),
                Err(e) => {
                    tracing::error!(error = %e, "stats_records window failed");
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
                }
            }
        }
        _ => None,
    };

    (
        StatusCode::OK,
        Json(RecordsResponse {
            longest_session_secs: lifetime.longest_session_secs,
            busiest_session_events: lifetime.busiest_session_events,
            longest_survival_streak_secs: lifetime.longest_survival_streak_secs,
            deadliest_session_deaths: lifetime.deadliest_session_deaths,
            window,
        }),
    )
        .into_response()
}

/// One `starstats_core::character_life::Life` span, DTO-shaped for the
/// wire. `ended_by` is the snake_case name of the
/// `starstats_core::character_life::LifeEnd` variant.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LifeRow {
    pub start_ts: Option<String>,
    pub end_ts: Option<String>,
    pub duration_secs: Option<i64>,
    pub ended_by: String,
    pub incap_count: u32,
    pub death_zone: Option<String>,
    pub death_inferred: bool,
}

/// `ended_by` on the wire is the snake_case name of the `LifeEnd`
/// variant (`Death` -> `"death"`, etc.) rather than the derived `Debug`
/// form, so the shape stays stable if the enum's `Debug` output ever
/// changes.
fn life_end_str(e: LifeEnd) -> &'static str {
    match e {
        LifeEnd::Death => "death",
        LifeEnd::Crash => "crash",
        LifeEnd::SessionGap => "session_gap",
        LifeEnd::StillAlive => "still_alive",
    }
}

/// Response for `GET /v1/me/stats/lives` — character-life FSM summary
/// (character-life-fsm Phase 1): the caller's FULL event history
/// segmented into spawn -> death/crash/gap spans.
///
/// `sessions` / `deaths_per_session` are deliberately NOT the FSM's own
/// time-gap based figures (`LifeSummary::sessions`) — they're
/// recomputed from the app's CANONICAL (marker-based) session count
/// (`EventQuery::lives_for_handle`'s `LivesData::sessions`) so this
/// endpoint's session count always agrees with
/// `/v1/users/{handle}/sessions`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LivesResponse {
    pub total_lives: u32,
    pub deaths: u32,
    /// How many of `deaths` were inferred rather than observed.
    ///
    /// Travels WITH the total so a surface showing "12 deaths" can say
    /// how much of it is reconstructed. Aggregates otherwise hide
    /// provenance precisely by aggregating: the per-life
    /// `death_inferred` flag exists, but summing it away loses it.
    pub deaths_inferred: u32,
    pub mean_life_secs: Option<i64>,
    pub longest_life_secs: Option<i64>,
    /// Canonical (marker-based) session count — see the struct doc.
    pub sessions: u32,
    /// `deaths / sessions`; `None` when `sessions == 0`.
    pub deaths_per_session: Option<f32>,
    pub lives_ended_by_crash: u32,
    /// The 50 most-recent lives, newest first.
    pub recent_lives: Vec<LifeRow>,
    /// Present only when the caller passes `?hours=N`: the same numeric
    /// figures computed over just the trailing N-hour window (no
    /// `recent_lives` — that stays lifetime-only). `None` (omitted) for
    /// the default all-time request, keeping the shape backward
    /// compatible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<LivesWindow>,
}

/// Range-windowed twin of [`LivesResponse`]'s lifetime numeric fields,
/// computed over just the trailing `hours`-hour window. `recent_lives`
/// is deliberately NOT mirrored here — the recent-lives list stays
/// lifetime-only.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LivesWindow {
    /// The (clamped) window size the figures below were computed over.
    pub hours: i64,
    pub total_lives: u32,
    pub deaths: u32,
    pub mean_life_secs: Option<i64>,
    pub longest_life_secs: Option<i64>,
    /// Canonical (marker-based) session count, scoped to the window.
    pub sessions: u32,
    /// `deaths / sessions`; `None` when `sessions == 0`.
    pub deaths_per_session: Option<f32>,
    pub lives_ended_by_crash: u32,
}

const RECENT_LIVES_LIMIT: usize = 50;

#[utoipa::path(
    get,
    path = "/v1/me/stats/lives",
    tag = "metrics",
    operation_id = "stats_lives",
    params(StatsParams),
    responses(
        (status = 200, description = "Character-life FSM summary (spawn -> death/crash/gap spans) for the caller", body = LivesResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_lives<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let handle = user.preferred_username.as_str();

    // Lifetime figures are always computed (since = None).
    let LivesData { summary, sessions } = match query.lives_for_handle(handle, None).await {
        Ok(data) => data,
        Err(e) => {
            tracing::error!(error = %e, "stats_lives failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
        }
    };
    let deaths_per_session = (sessions > 0).then(|| summary.deaths as f32 / sessions as f32);
    let recent_lives = summary
        .lives
        .iter()
        .rev()
        .take(RECENT_LIVES_LIMIT)
        .map(|l| LifeRow {
            start_ts: l.start_ts.clone(),
            end_ts: l.end_ts.clone(),
            duration_secs: l.duration_secs,
            ended_by: life_end_str(l.ended_by).to_string(),
            incap_count: l.incap_count,
            death_zone: l.death_zone.clone(),
            death_inferred: l.death_inferred,
        })
        .collect();

    // A positive `hours` adds the windowed twin (numeric fields only,
    // no recent_lives), clamped to the same one-year ceiling the other
    // stats endpoints use.
    let window = match params.hours {
        Some(h) if h > 0 => {
            let hours = h.min(STATS_MAX_HOURS);
            let since = Some(Utc::now() - chrono::Duration::hours(hours));
            match query.lives_for_handle(handle, since).await {
                Ok(LivesData {
                    summary: w,
                    sessions: w_sessions,
                }) => {
                    let w_deaths_per_session =
                        (w_sessions > 0).then(|| w.deaths as f32 / w_sessions as f32);
                    Some(LivesWindow {
                        hours,
                        total_lives: w.total_lives,
                        deaths: w.deaths,
                        mean_life_secs: w.mean_life_secs,
                        longest_life_secs: w.longest_life_secs,
                        sessions: w_sessions,
                        deaths_per_session: w_deaths_per_session,
                        lives_ended_by_crash: w.lives_ended_by_crash,
                    })
                }
                Err(e) => {
                    tracing::error!(error = %e, "stats_lives window failed");
                    return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
                }
            }
        }
        _ => None,
    };

    (
        StatusCode::OK,
        Json(LivesResponse {
            total_lives: summary.total_lives,
            deaths: summary.deaths,
            deaths_inferred: summary.deaths_inferred,
            mean_life_secs: summary.mean_life_secs,
            longest_life_secs: summary.longest_life_secs,
            sessions,
            deaths_per_session,
            lives_ended_by_crash: summary.lives_ended_by_crash,
            recent_lives,
            window,
        }),
    )
        .into_response()
}

/// Response for `GET /v1/me/stats/biggest-trade` — the largest CONFIRMED
/// commerce purchase by quantity over the caller's FULL history. Replaces
/// the web widget's 500-capped client scan of the recent-commerce list
/// (audit F9). Me-scoped (owner-only, C2). Hand-typed like
/// [`RecordsResponse`] — not in the OpenAPI spec.
#[derive(Debug, Serialize, Deserialize)]
pub struct BiggestTradeResponse {
    /// Quantity of the biggest confirmed trade; `None` when the caller has
    /// no confirmed trades.
    pub quantity: Option<f64>,
    /// Item of that trade, when the event carried one.
    pub item: Option<String>,
}

/// Pure: pick the biggest CONFIRMED trade by quantity. Mirrors the web
/// widget's old scan (confirmed + has-quantity, max by quantity) so the
/// server value matches what the client used to compute — just over the
/// full history instead of the capped recent list. Extracted for a
/// store-free unit test.
fn biggest_confirmed_trade(txs: &[starstats_core::Transaction]) -> BiggestTradeResponse {
    let biggest = txs
        .iter()
        .filter(|t| t.status == starstats_core::TransactionStatus::Confirmed)
        .filter_map(|t| t.quantity.map(|q| (q, t.item.clone())))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    match biggest {
        Some((quantity, item)) => BiggestTradeResponse {
            quantity: Some(quantity),
            item,
        },
        None => BiggestTradeResponse {
            quantity: None,
            item: None,
        },
    }
}

pub async fn stats_biggest_trade<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    // Commerce events are rare per user, so fetching ALL of them (by the
    // handful of commerce event_types) and pairing is bounded — and yields
    // a true all-time max instead of the recent-list-capped client scan.
    // Generous per-type cap: far beyond any real player's lifetime commerce
    // volume, but finite so a pathological account can't unbounded-fetch.
    const PER_TYPE_LIMIT: i64 = 50_000;
    let mut game_events: Vec<starstats_core::GameEvent> = Vec::new();
    for ty in starstats_core::transactions::COMMERCE_EVENT_TYPES {
        let filters = EventFilters {
            cursor: None,
            event_type: Some((*ty).to_string()),
            since: None,
            until: None,
            limit: PER_TYPE_LIMIT,
        };
        match query.list_filtered(&user.preferred_username, filters).await {
            Ok(rows) => game_events.extend(
                rows.into_iter()
                    .filter_map(|e| serde_json::from_value(e.payload).ok()),
            ),
            Err(e) => {
                tracing::error!(error = %e, event_type = ty, "stats_biggest_trade list_filtered failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
            }
        }
    }

    // `window_secs` is irrelevant to Confirmed status (response-driven, not
    // time-driven), so pass i64::MAX — no unmatched request ages into a
    // different status across a multi-year history.
    let now = Utc::now().to_rfc3339();
    let txs = starstats_core::pair_transactions(&game_events, &now, i64::MAX);

    (StatusCode::OK, Json(biggest_confirmed_trade(&txs))).into_response()
}

/// Response for `GET /v1/me/stats/locations`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LocationsStatsResponse {
    pub hours: i64,
    /// Number of distinct locations visited in the window, keyed by
    /// `system|planet|city` — same identity as [`crate::locations::location_key`].
    pub unique_locations: i64,
    /// Top locations by visit count, descending (ties broken alphabetically
    /// by key). Capped at 10.
    pub top_locations: Vec<StatsBucket>,
}

#[utoipa::path(
    get,
    path = "/v1/me/stats/locations",
    tag = "metrics",
    operation_id = "stats_locations",
    params(StatsParams),
    responses(
        (status = 200, description = "Distinct locations visited over the window", body = LocationsStatsResponse),
        (status = 400, description = "Invalid hours window"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn stats_locations<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<StatsParams>,
) -> impl IntoResponse {
    let (hours, since) = match parse_stats_window(&params) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let stream = match query
        .location_event_stream(
            &user.preferred_username,
            locations::LOCATION_EVENT_TYPES,
            since.unwrap_or_else(|| Utc::now() - chrono::Duration::hours(hours)),
            STATS_LOCATIONS_RAW_LIMIT,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "stats_locations location_event_stream failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query failed").into_response();
        }
    };
    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for ev in stream {
        if let Some(loc) = locations::resolve(&ev.event_type, &ev.payload, ev.event_timestamp, None)
        {
            let key = locations::location_key(&loc);
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    let unique_locations = counts.len() as i64;
    let mut top: Vec<StatsBucket> = counts
        .into_iter()
        .map(|(value, count)| StatsBucket { value, count })
        .collect();
    top.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
    top.truncate(10);
    (
        StatusCode::OK,
        Json(LocationsStatsResponse {
            hours,
            unique_locations,
            top_locations: top,
        }),
    )
        .into_response()
}

/// Query params for `GET /v1/me/commerce/recent`.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CommerceRecentParams {
    /// How many transactions to return. Capped at 500.
    #[serde(default = "default_commerce_limit")]
    pub limit: u32,
    /// Window for the "if no response in N seconds, mark timed out"
    /// classification. Mirrors the tray client's default of 30s.
    /// This is a *pairing* timeout, NOT a time-range filter — see
    /// `hours` below for the range-filter knob.
    #[serde(default = "default_commerce_window_secs")]
    pub window_secs: i64,
    /// Optional time-range filter in hours. When set, only events
    /// newer than `now - hours` are considered when pairing. Bounds
    /// match the stats endpoints (1..=STATS_MAX_HOURS). Absent =
    /// no filter (legacy behavior — pull recent ~1000 events).
    #[serde(default)]
    pub hours: Option<i64>,
}

fn default_commerce_limit() -> u32 {
    100
}
fn default_commerce_window_secs() -> i64 {
    30
}

/// Recent shop / commodity transactions for the caller, paired
/// `Send*Request` ↔ `*FlowResponse` via
/// [`starstats_core::pair_transactions`].
///
/// Strategy: pull the last ~1000 events (regardless of type) for the
/// user, deserialise each `payload`, filter to commerce variants,
/// then run the pure pairer. Commerce events are rare per-user so a
/// 1000-row cap covers a wide window without needing per-type
/// queries.
#[utoipa::path(
    get,
    path = "/v1/me/commerce/recent",
    tag = "query",
    params(CommerceRecentParams),
    responses(
        (status = 200, description = "Paired commerce transactions", body = CommerceRecentResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Query failed"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn commerce_recent<Q: EventQuery>(
    State(query): State<Arc<Q>>,
    user: AuthenticatedUser,
    Query(params): Query<CommerceRecentParams>,
) -> impl IntoResponse {
    // Cap aggressively — this is a "recent" view, not a forensic dump.
    let limit = params.limit.clamp(1, 500);

    // Optional time-window filter. Bounds match `parse_stats_window`
    // so the same range chips drive every page.
    let since = match params.hours {
        Some(h) if h > 0 && h <= STATS_MAX_HOURS => Some(Utc::now() - chrono::Duration::hours(h)),
        Some(_) => return err(StatusCode::BAD_REQUEST, "invalid_hours"),
        None => None,
    };

    // Fetch commerce events BY TYPE — the same per-type query
    // `stats_biggest_trade` uses. This previously pulled the newest N
    // events of ANY type and filtered commerce out in-process, which
    // handed a player an empty tile at EVERY range once they had more
    // than N non-commerce events since their last purchase: `hours` only
    // moves the `since` lower bound, while the cap truncates from the
    // newest end, so widening the range could never recover them. Asking
    // the database for the types we actually want makes the result
    // independent of how much unrelated activity sits in front of them.
    //
    // Per-type cap: emitting `limit` transactions needs at most `limit`
    // requests of a given kind plus their `limit` responses, so 2x the
    // requested limit covers pairing across the fetch boundary. Floored
    // so a small `limit` still has pairing headroom, ceilinged so a
    // pathological account stays bounded.
    let per_type_limit: i64 = (limit as i64).saturating_mul(2).clamp(200, 2_000);

    // Deserialise each payload as GameEvent. Drop any that fail (the
    // store may hold legacy or malformed rows from an earlier client).
    let mut game_events: Vec<starstats_core::GameEvent> = Vec::new();
    for ty in starstats_core::transactions::COMMERCE_EVENT_TYPES {
        let filters = EventFilters {
            cursor: None,
            event_type: Some((*ty).to_string()),
            since,
            until: None,
            limit: per_type_limit,
        };
        match query.list_filtered(&user.preferred_username, filters).await {
            Ok(rows) => game_events.extend(
                rows.into_iter()
                    .filter_map(|e| serde_json::from_value(e.payload).ok()),
            ),
            Err(e) => {
                tracing::error!(error = %e, event_type = ty, "commerce_recent list_filtered failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "query_failed");
            }
        }
    }

    let now = Utc::now().to_rfc3339();
    let txs = starstats_core::pair_transactions(&game_events, &now, params.window_secs);

    // Trim to the requested limit (newest first by started_at after
    // pair_transactions sorts ascending — reverse + take). Convert
    // each row to the utoipa-friendly DTO so the OpenAPI spec stays
    // canonical without forcing utoipa onto starstats-core.
    let trimmed: Vec<CommerceTransactionDto> = txs
        .into_iter()
        .rev()
        .take(limit as usize)
        .map(CommerceTransactionDto::from)
        .collect();

    (
        StatusCode::OK,
        Json(CommerceRecentResponse {
            transactions: trimmed,
        }),
    )
        .into_response()
}

/// Wire-format wrapper for the commerce endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommerceRecentResponse {
    /// Paired transactions, newest first by started_at.
    pub transactions: Vec<CommerceTransactionDto>,
}

/// Mirrors `starstats_core::Transaction` but in a utoipa-friendly
/// shape. Field-for-field identical at the JSON layer.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommerceTransactionDto {
    pub kind: String,
    pub status: String,
    pub started_at: String,
    pub confirmed_at: Option<String>,
    pub shop_id: Option<String>,
    pub item: Option<String>,
    pub quantity: Option<f64>,
    pub raw_request: String,
    pub raw_response: Option<String>,
}

impl From<starstats_core::Transaction> for CommerceTransactionDto {
    fn from(t: starstats_core::Transaction) -> Self {
        Self {
            kind: serde_json::to_value(t.kind)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default(),
            status: serde_json::to_value(t.status)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default(),
            started_at: t.started_at,
            confirmed_at: t.confirmed_at,
            shop_id: t.shop_id,
            item: t.item,
            quantity: t.quantity,
            raw_request: t.raw_request,
            raw_response: t.raw_response,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::repo::{test_support::MemoryQuery, StoredQueryEvent};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::routing::get;
    use axum::{Extension, Router};
    use serde_json::json;
    use tower::ServiceExt;

    fn sign_token(issuer: &TokenIssuer, username: &str) -> String {
        issuer
            .sign_user(&format!("user-{username}"), username)
            .expect("sign user token")
    }

    fn router(query: Arc<MemoryQuery>, verifier: Arc<AuthVerifier>) -> Router {
        // Empty catalog: classification falls back to title-cased raw,
        // which is exactly what the resolution tests assert against.
        router_with_catalog(query, verifier, LocationCatalogCache::empty())
    }

    fn router_with_catalog(
        query: Arc<MemoryQuery>,
        verifier: Arc<AuthVerifier>,
        catalog: LocationCatalogCache,
    ) -> Router {
        // SpiceDB extension is required by `summary`; tests run
        // without a configured client, so we inject a None-valued
        // Arc — the handler treats that as "skipped".
        let spicedb: Arc<Option<crate::spicedb::SpicedbClient>> = Arc::new(None);
        Router::new()
            .route("/v1/me/events", get(list_events::<MemoryQuery>))
            .route("/v1/me/summary", get(summary::<MemoryQuery>))
            .route("/v1/me/timeline", get(timeline::<MemoryQuery>))
            .route(
                "/v1/me/metrics/event-types",
                get(metrics_event_types::<MemoryQuery>),
            )
            .route(
                "/v1/me/metrics/sessions",
                get(metrics_sessions::<MemoryQuery>),
            )
            .route("/v1/me/ingest-history", get(ingest_history::<MemoryQuery>))
            .route(
                "/v1/me/location/current",
                get(location_current::<MemoryQuery>),
            )
            .route("/v1/me/stats/combat", get(stats_combat::<MemoryQuery>))
            .route("/v1/me/stats/playtime", get(stats_playtime::<MemoryQuery>))
            .route(
                "/v1/me/stats/biggest-trade",
                get(stats_biggest_trade::<MemoryQuery>),
            )
            .route(
                "/v1/me/stats/locations",
                get(stats_locations::<MemoryQuery>),
            )
            .route("/v1/me/stats/lives", get(stats_lives::<MemoryQuery>))
            .route("/v1/me/stats/fleet", get(stats_fleet::<MemoryQuery>))
            .route("/v1/me/stats/docking", get(stats_docking::<MemoryQuery>))
            .route("/v1/me/stats/routes", get(stats_routes::<MemoryQuery>))
            .route(
                "/v1/me/stats/objectives",
                get(stats_objectives::<MemoryQuery>),
            )
            .route(
                "/v1/me/stats/contracts",
                get(stats_contracts::<MemoryQuery>),
            )
            .route("/v1/me/stats/spend", get(stats_spend::<MemoryQuery>))
            .route(
                "/v1/me/stats/loadout-activity",
                get(stats_loadout_activity::<MemoryQuery>),
            )
            .route(
                "/v1/me/commerce/recent",
                get(commerce_recent::<MemoryQuery>),
            )
            .route("/v1/me/location/trace", get(location_trace::<MemoryQuery>))
            .route(
                "/v1/me/location/breakdown",
                get(location_breakdown::<MemoryQuery>),
            )
            .layer(Extension(verifier))
            .layer(Extension(spicedb))
            .layer(Extension(catalog))
            .with_state(query)
    }

    #[test]
    fn biggest_confirmed_trade_picks_max_confirmed_quantity() {
        use starstats_core::{Transaction, TransactionKind, TransactionStatus};
        let tx = |status, qty: Option<f64>, item: &str| Transaction {
            kind: TransactionKind::Shop,
            status,
            started_at: "2026-01-01T00:00:00Z".into(),
            confirmed_at: None,
            shop_id: None,
            item: Some(item.into()),
            quantity: qty,
            raw_request: String::new(),
            raw_response: None,
        };
        let txs = vec![
            tx(TransactionStatus::Confirmed, Some(3.0), "a"),
            tx(TransactionStatus::Confirmed, Some(9.0), "b"), // biggest confirmed
            tx(TransactionStatus::Rejected, Some(99.0), "c"), // bigger but not confirmed
            tx(TransactionStatus::Submitted, Some(50.0), "d"), // not confirmed
            tx(TransactionStatus::Confirmed, None, "e"),      // confirmed but no qty
        ];
        let r = biggest_confirmed_trade(&txs);
        assert_eq!(r.quantity, Some(9.0));
        assert_eq!(r.item.as_deref(), Some("b"));

        // No confirmed trades → None.
        let empty = biggest_confirmed_trade(&[]);
        assert_eq!(empty.quantity, None);
        assert_eq!(empty.item, None);
    }

    fn evt(seq: i64, handle: &str, ty: &str, ts: Option<DateTime<Utc>>) -> StoredQueryEvent {
        StoredQueryEvent {
            seq,
            claimed_handle: handle.into(),
            event_type: ty.into(),
            event_timestamp: ts,
            log_source: "live".into(),
            source_offset: 0,
            payload: json!({"type": ty}),
            resolved_location: None,
            hidden_at: None,
        }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        app: Router,
        uri: &str,
        token: &str,
    ) -> (StatusCode, T) {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: T = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "decode {}: {} (body={})",
                std::any::type_name::<T>(),
                e,
                String::from_utf8_lossy(&bytes)
            )
        });
        (status, parsed)
    }

    #[tokio::test]
    async fn stats_lives_requires_auth() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (_issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/stats/lives")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stats_fleet_ranks_ships_by_trip_count() {
        // 3x Tiburon, 1x Perseus for alice; a decoy (other handle + other
        // event type) must not pollute the ranking.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_target_selected",
                now,
                json!({"vehicle_class": "AEGS_Tiburon"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_target_selected",
                now,
                json!({"vehicle_class": "AEGS_Tiburon"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "quantum_target_selected",
                now,
                json!({"vehicle_class": "AEGS_Tiburon"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "quantum_target_selected",
                now,
                json!({"vehicle_class": "RSI_Perseus"}),
            ),
            evt_with_payload(
                5,
                "bob",
                "quantum_target_selected",
                now,
                json!({"vehicle_class": "ORIG_m80"}),
            ), // other handle -> excluded
            evt_with_payload(
                6,
                "alice",
                "vehicle_stowed",
                now,
                json!({"vehicle_class": "IGNORED"}),
            ), // other event type -> excluded
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<FleetResponse>(app, "/v1/me/stats/fleet", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.ships.len(), 2);
        assert_eq!(body.ships[0].vehicle_class, "AEGS_Tiburon");
        assert_eq!(body.ships[0].trip_count, 3);
        assert_eq!(body.ships[1].vehicle_class, "RSI_Perseus");
        assert_eq!(body.ships[1].trip_count, 1);
    }

    #[tokio::test]
    async fn stats_fleet_requires_auth() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (_issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/stats/fleet")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stats_routes_ranks_destinations() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_route",
                now,
                json!({"start_system":"Stanton","destination":"Crusader"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_route",
                now,
                json!({"start_system":"Stanton","destination":"Crusader"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "quantum_route",
                now,
                json!({"start_system":"Stanton","destination":"microTech"}),
            ),
            // other handle -> excluded
            evt_with_payload(
                4,
                "bob",
                "quantum_route",
                now,
                json!({"destination":"ARC-L1"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<RoutesResponse>(app, "/v1/me/stats/routes", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.routes.len(), 2);
        assert_eq!(body.routes[0].destination, "Crusader");
        assert_eq!(body.routes[0].count, 2);
    }

    #[tokio::test]
    async fn stats_objectives_computes_completion_pct() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now,
                json!({"state":"completed"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                now,
                json!({"state":"completed"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "mission_objective",
                now,
                json!({"state":"completed"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "mission_objective",
                now,
                json!({"state":"failed"}),
            ),
            evt_with_payload(
                5,
                "alice",
                "mission_objective",
                now,
                json!({"state":"in_progress"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) =
            get_json::<ObjectivesResponse>(app, "/v1/me/stats/objectives", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.completed, 3);
        assert_eq!(body.failed, 1);
        assert_eq!(body.no_outcome, 1);
        assert_eq!(body.unresolved, 0);
        assert_eq!(body.total, 5);
        // 3 completed / (3 completed + 1 failed + 0 unresolved) = 75%.
        assert_eq!(body.completion_pct, Some(75));
    }

    #[tokio::test]
    async fn stats_objectives_counts_each_objective_once() {
        // obj-a: in_progress -> completed  (ONE completed objective)
        // obj-b: in_progress -> unknown  (ONE withdrawn objective; WITHDRAWN
        //         is stored as "unknown" — the parser has no Withdrawn variant)
        // obj-c: in_progress only          (ONE still in progress)
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-a","state":"in_progress"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-a","state":"completed"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-b","state":"in_progress"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-b","state":"unknown"}),
            ),
            evt_with_payload(
                5,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-c","state":"in_progress"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) =
            get_json::<ObjectivesResponse>(app, "/v1/me/stats/objectives", &token).await;
        assert_eq!(status, StatusCode::OK);
        // 3 distinct objectives from 5 transition rows.
        assert_eq!(body.total, 3);
        assert_eq!(body.completed, 1);
        assert_eq!(body.unresolved, 1);
        assert_eq!(body.no_outcome, 1);
        assert_eq!(body.failed, 0);
        // Resolved = completed + failed + unresolved = 2. A withdrawn
        // objective IS resolved and was NOT completed, so 1/2 = 50%.
        assert_eq!(body.completion_pct, Some(50));
    }

    // -- /v1/me/stats/contracts ------------------------------------

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

    /// Same shape as [`hud_payload`], with `objective_id` set — mirrors
    /// `contract_life.rs`'s own `hud()` test helper, whose "New
    /// Objective"/"Objective Complete" banners are what the fold turns
    /// into `ContractStep` rows (see `steps_are_counted_by_distinct_objective_id`).
    fn hud_objective_payload(
        ts: &str,
        text: &str,
        notification_id: u64,
        mission_id: &str,
        objective_id: &str,
    ) -> serde_json::Value {
        json!({
            "type": "hud_notification",
            "timestamp": ts,
            "text": text,
            "notification_id": notification_id,
            "mission_id": mission_id,
            "objective_id": objective_id,
        })
    }

    #[tokio::test]
    async fn stats_contracts_reports_completed_run() {
        // RED test for `GET /v1/me/stats/contracts` (Task 3 step 1) —
        // fails to compile until the handler/response types/route exist.
        // Seeds an accept -> complete pair through MemoryQuery (real
        // `derive_contract_runs`, not a mock) and checks the one run it
        // yields end to end.
        let mid = "7de35808-d909-4a6d-affe-edadf3e6fe77";
        let mq = Arc::new(MemoryQuery::new(vec![
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
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) =
            get_json::<ContractsResponse>(app, "/v1/me/stats/contracts", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.runs.len(), 1);
        assert_eq!(body.runs[0].mission_id, mid);
        assert_eq!(body.runs[0].name, "Combat Gauntlet - Scenario #5");
        assert_eq!(body.runs[0].state, "completed");
        assert_eq!(body.completed, 1);
        assert_eq!(body.total, 1);
        assert_eq!(body.completion_pct, Some(100));
    }

    /// Pairs both sides of the opt-in: the default (no query param) call
    /// must come back with `steps` present-but-empty on every run (the
    /// widget-safe shape `ContractsParams`'s doc promises), and only
    /// `?include_steps=true` populates it. A test that only checked the
    /// populated path would let a future edit silently flip the default
    /// back to "always populated" without failing anything.
    #[tokio::test]
    async fn stats_contracts_include_steps_flag_gates_step_population() {
        let mid = "3f6a6f0d-1111-4d33-9b7d-6c1f6b6b6b6b";
        let oid = "obj-euterpe";
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "hud_notification",
                ts("2026-07-26T13:00:00Z"),
                hud_payload("2026-07-26T13:00:00Z", "Contract Accepted:  Test: ", 1, mid),
            ),
            evt_with_payload(
                2,
                "alice",
                "hud_notification",
                ts("2026-07-26T13:01:00Z"),
                hud_objective_payload(
                    "2026-07-26T13:01:00Z",
                    "New Objective: Go to Euterpe: ",
                    2,
                    mid,
                    oid,
                ),
            ),
            evt_with_payload(
                3,
                "alice",
                "hud_notification",
                ts("2026-07-26T13:03:00Z"),
                hud_objective_payload(
                    "2026-07-26T13:03:00Z",
                    "Objective Complete: Go to Euterpe",
                    3,
                    mid,
                    oid,
                ),
            ),
            evt_with_payload(
                4,
                "alice",
                "hud_notification",
                ts("2026-07-26T13:04:00Z"),
                hud_payload("2026-07-26T13:04:00Z", "Contract Complete: Test: ", 4, mid),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");

        let (status, default_body) =
            get_json::<ContractsResponse>(app.clone(), "/v1/me/stats/contracts", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(default_body.runs.len(), 1);
        assert_eq!(
            default_body.runs[0].step_count, 1,
            "run-level count is unaffected by the flag"
        );
        assert!(
            default_body.runs[0].steps.is_empty(),
            "steps must be empty (not absent) by default"
        );

        let (status, opt_in_body) =
            get_json::<ContractsResponse>(app, "/v1/me/stats/contracts?include_steps=true", &token)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(opt_in_body.runs[0].steps.len(), 1);
        assert_eq!(
            opt_in_body.runs[0].steps[0].text.as_deref(),
            Some("Go to Euterpe")
        );
        assert_eq!(opt_in_body.runs[0].steps[0].state, "complete");
    }

    #[tokio::test]
    async fn stats_contracts_requires_auth() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (_issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/stats/contracts")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn contract_catalog_gaps_requires_auth() {
        // Lazy pool — RequireModerator's auth check runs before the
        // handler ever touches `State`, so this never opens a real
        // connection (same pattern as `event_timeline::router_for_test`).
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/starstats_test_unused")
            .expect("connect_lazy is infallible for a syntactically valid URL");
        let store = Arc::new(crate::repo::PostgresStore::new(pool));
        let app = Router::new()
            .route("/v1/admin/contracts/gaps", get(contract_catalog_gaps))
            .with_state(store);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/admin/contracts/gaps")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Fixture for [`summarize_contracts`]'s store-free unit tests —
    /// every field but `state` is irrelevant to bucketing, so it's fixed
    /// to a cheap default.
    fn contract_row(state: &str) -> crate::repo::ContractRunRow {
        crate::repo::ContractRunRow {
            mission_id: format!("mission-{state}"),
            name: "Test Contract".to_string(),
            state: state.to_string(),
            closed_by: "none".to_string(),
            step_count: 0,
            steps_complete: 0,
            steps_remaining: 0,
            partial_history: false,
            connected_server: None,
            accepted_at: None,
            closed_at: None,
            last_event_at: None,
            steps: Vec::new(),
        }
    }

    #[test]
    fn summarize_contracts_excludes_superseded_and_buckets_withdrawn_unknown_separately() {
        // One run of each of the fold's 7 states. Mirrors
        // `biggest_confirmed_trade`'s pure-function test pattern so every
        // state is exercised without having to drive the real
        // `derive_contract_runs` fold through a session-gap/re-accept
        // event sequence for each one.
        let runs = vec![
            contract_row("completed"),
            contract_row("failed"),
            contract_row("withdrawn"),
            contract_row("abandoned"),
            contract_row("in_progress"),
            contract_row("unknown"),
            contract_row("superseded"),
        ];
        let r = summarize_contracts(runs, false);
        assert_eq!(r.completed, 1);
        assert_eq!(r.failed, 1);
        assert_eq!(r.withdrawn, 1);
        assert_eq!(r.abandoned, 1);
        assert_eq!(r.in_progress, 1);
        assert_eq!(r.unknown, 1);
        // Superseded contributes to no bucket and is excluded from
        // `total`, even though all 7 rows still come back in `runs`.
        assert_eq!(r.total, 6);
        assert_eq!(r.runs.len(), 7);
        // completed / (completed + failed + abandoned) = 1 / 3 = 33%.
        // withdrawn/in_progress/unknown are NOT in this denominator.
        assert_eq!(r.completion_pct, Some(33));
    }

    #[test]
    fn summarize_contracts_pct_none_when_nothing_resolved() {
        let runs = vec![contract_row("in_progress"), contract_row("unknown")];
        let r = summarize_contracts(runs, false);
        assert_eq!(r.total, 2);
        assert_eq!(r.completion_pct, None);
    }

    /// Proves `steps` survives the repo → wire conversion and that `text`
    /// arrives byte-identical, including the trailing `": "` and internal
    /// space before the final colon that the game's multi-line stitcher
    /// writes for real records -- that's stored verbatim by design, and
    /// this test would fail if a future edit "cleaned up" the text or if
    /// the `From` impl dropped `steps` again.
    #[test]
    fn contract_run_row_carries_steps_through_with_text_verbatim() {
        let mut row = contract_row("completed");
        row.steps = vec![ContractStep {
            objective_id: Some("obj-1".to_string()),
            order: 1,
            text: Some("Go to a debris field above Euterpe : ".to_string()),
            state: StepState::Complete,
            started_at: Some("2026-07-01T00:00:00Z".to_string()),
            completed_at: Some("2026-07-01T00:05:00Z".to_string()),
        }];

        let wire = ContractRunRow::from(row);
        assert_eq!(wire.steps.len(), 1);
        let step = &wire.steps[0];
        assert_eq!(step.objective_id.as_deref(), Some("obj-1"));
        assert_eq!(step.order, 1);
        assert_eq!(
            step.text.as_deref(),
            Some("Go to a debris field above Euterpe : ")
        );
        assert_eq!(step.state, "complete");
        assert_eq!(step.started_at.as_deref(), Some("2026-07-01T00:00:00Z"));
        assert_eq!(step.completed_at.as_deref(), Some("2026-07-01T00:05:00Z"));
    }

    #[tokio::test]
    async fn stats_spend_sums_price_and_counts_all() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now,
                json!({"price":15000,"shop_name":"SCShop_Aparelli"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                now,
                json!({"price":2500,"shop_name":"SCShop_Aparelli"}),
            ),
            // legacy line: no price -> counted in purchases, contributes 0 to total.
            evt_with_payload(
                3,
                "alice",
                "shop_buy_request",
                now,
                json!({"item_class":"legacy_no_price"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<SpendResponse>(app, "/v1/me/stats/spend", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.total_auec, 17500);
        assert_eq!(body.purchases, 3);
        assert_eq!(body.top_shop.as_deref(), Some("SCShop_Aparelli"));
    }

    #[tokio::test]
    async fn stats_loadout_activity_splits_equip_store() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "item_equip_change",
                now,
                json!({"action":"equip","item_class":"GRIN_Helmet"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "item_equip_change",
                now,
                json!({"action":"equip","item_class":"GRIN_Helmet"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "item_equip_change",
                now,
                json!({"action":"store","item_class":"RSI_Undersuit"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) =
            get_json::<LoadoutActivityResponse>(app, "/v1/me/stats/loadout-activity", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.equips, 2);
        assert_eq!(body.stores, 1);
        assert_eq!(body.top_items[0].item_class, "GRIN_Helmet");
        assert_eq!(body.top_items[0].count, 2);
    }

    #[tokio::test]
    async fn stats_routes_hours_window_excludes_old_destinations() {
        // Two recent Crusader routes + a 60-day-old microTech route. The
        // lifetime view sees all three; a 24-hour window drops the old one.
        // The two views differ on BOTH magnitudes — 2 trips to 1 place in
        // the window, 3 trips to 2 places lifetime — so a baseline that
        // was secretly computed over the window could not pass.
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_route",
                now,
                json!({"destination":"Crusader"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_route",
                old,
                json!({"destination":"microTech"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "quantum_route",
                now,
                json!({"destination":"Crusader"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let verifier = Arc::new(verifier);
        let token = sign_token(&issuer, "alice");

        let app = router(mq.clone(), verifier.clone());
        let (status, all) = get_json::<RoutesResponse>(app, "/v1/me/stats/routes", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all.routes.len(), 2);

        let app = router(mq, verifier);
        let (status, windowed) =
            get_json::<RoutesResponse>(app, "/v1/me/stats/routes?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);

        // NON-BREAKING GUARANTEE: the top-level list still means "the
        // requested window". Flipping it to lifetime would silently change
        // what every existing caller receives.
        assert_eq!(windowed.routes.len(), 1, "top level must stay windowed");
        assert_eq!(windowed.routes[0].destination, "Crusader");
        assert_eq!(windowed.routes[0].count, 2);

        // UX Rule 2: the windowed list must arrive WITH a baseline, or
        // "2 jumps" tells a player nothing about how much they travel.
        let lt = windowed
            .lifetime
            .as_ref()
            .expect("a windowed response must carry its lifetime baseline");
        assert_eq!(lt.total_trips, 3, "baseline is lifetime, not the window");
        assert_eq!(lt.destinations, 2, "baseline is lifetime, not the window");

        // And with no window there is nothing to compare against, so no
        // twin — repeating the same numbers would be noise.
        assert!(
            all.lifetime.is_none(),
            "an unwindowed response must not carry a redundant twin"
        );
    }

    #[tokio::test]
    async fn stats_fleet_hours_window_carries_lifetime_baseline() {
        // Two recent Cutlass trips; a 60-day-old Cutlass trip and a
        // 60-day-old Gladius trip. Window: 2 trips, 1 ship. Lifetime:
        // 4 trips, 2 ships — different on both magnitudes.
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_target_selected",
                now,
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_target_selected",
                now,
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "quantum_target_selected",
                old,
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "quantum_target_selected",
                old,
                json!({"vehicle_class":"AEGS_Gladius"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let verifier = Arc::new(verifier);
        let token = sign_token(&issuer, "alice");

        let app = router(mq.clone(), verifier.clone());
        let (status, all) = get_json::<FleetResponse>(app, "/v1/me/stats/fleet", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all.ships.len(), 2);

        let app = router(mq, verifier);
        let (status, windowed) =
            get_json::<FleetResponse>(app, "/v1/me/stats/fleet?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);

        // NON-BREAKING GUARANTEE: the top-level list still means "the
        // requested window".
        assert_eq!(windowed.ships.len(), 1, "top level must stay windowed");
        assert_eq!(windowed.ships[0].vehicle_class, "DRAK_Cutlass_Black");
        assert_eq!(windowed.ships[0].trip_count, 2);

        // UX Rule 2: "2 trips in one ship" means nothing without knowing
        // how much flying, across how many ships, that is measured against.
        let lt = windowed
            .lifetime
            .as_ref()
            .expect("a windowed response must carry its lifetime baseline");
        assert_eq!(lt.total_trips, 4, "baseline is lifetime, not the window");
        assert_eq!(lt.ships_flown, 2, "baseline is lifetime, not the window");

        // No window, no twin — it would only repeat the top level.
        assert!(
            all.lifetime.is_none(),
            "an unwindowed response must not carry a redundant twin"
        );
    }

    #[tokio::test]
    async fn stats_objectives_hours_window_carries_lifetime_baseline() {
        // Window (24h): one completed objective -> 1/1 = 100%.
        // Lifetime: 2 completed, 1 failed, 1 never resolved -> total 4,
        // 2/3 = 66%. Every mirrored field differs between the two views,
        // including the completion rate.
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-recent","state":"in_progress"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                now,
                json!({"objective_id":"obj-recent","state":"completed"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "mission_objective",
                old,
                json!({"objective_id":"obj-old-done","state":"completed"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "mission_objective",
                old,
                json!({"objective_id":"obj-old-failed","state":"failed"}),
            ),
            evt_with_payload(
                5,
                "alice",
                "mission_objective",
                old,
                json!({"objective_id":"obj-old-open","state":"in_progress"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let verifier = Arc::new(verifier);
        let token = sign_token(&issuer, "alice");

        let app = router(mq.clone(), verifier.clone());
        let (status, all) =
            get_json::<ObjectivesResponse>(app, "/v1/me/stats/objectives", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all.total, 4);
        assert_eq!(all.completion_pct, Some(66));

        let app = router(mq, verifier);
        let (status, windowed) =
            get_json::<ObjectivesResponse>(app, "/v1/me/stats/objectives?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);

        // NON-BREAKING GUARANTEE: the top-level figures still mean "the
        // requested window".
        assert_eq!(windowed.completed, 1, "top level must stay windowed");
        assert_eq!(windowed.failed, 0, "top level must stay windowed");
        assert_eq!(windowed.no_outcome, 0, "top level must stay windowed");
        assert_eq!(windowed.total, 1, "top level must stay windowed");
        assert_eq!(
            windowed.completion_pct,
            Some(100),
            "top level must stay windowed"
        );

        // UX Rule 2: "100% this week" means nothing until the player knows
        // they usually run at 66%.
        let lt = windowed
            .lifetime
            .as_ref()
            .expect("a windowed response must carry its lifetime baseline");
        assert_eq!(lt.completed, 2, "baseline is lifetime, not the window");
        assert_eq!(lt.failed, 1, "baseline is lifetime, not the window");
        assert_eq!(lt.unresolved, 0);
        assert_eq!(lt.no_outcome, 1, "baseline is lifetime, not the window");
        assert_eq!(lt.total, 4, "baseline is lifetime, not the window");
        assert_eq!(
            lt.completion_pct,
            Some(66),
            "baseline is lifetime, not the window"
        );

        // No window, no twin — it would only repeat the top level.
        assert!(
            all.lifetime.is_none(),
            "an unwindowed response must not carry a redundant twin"
        );
    }

    #[tokio::test]
    async fn stats_spend_hours_window_excludes_old_purchases() {
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now,
                json!({"price":15000,"shop_name":"SCShop_Aparelli"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                old,
                json!({"price":2500,"shop_name":"SCShop_Old"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let verifier = Arc::new(verifier);
        let token = sign_token(&issuer, "alice");

        let app = router(mq.clone(), verifier.clone());
        let (status, all) = get_json::<SpendResponse>(app, "/v1/me/stats/spend", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all.total_auec, 17500);
        assert_eq!(all.purchases, 2);

        let app = router(mq, verifier);
        let (status, windowed) =
            get_json::<SpendResponse>(app, "/v1/me/stats/spend?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(windowed.total_auec, 15000);
        assert_eq!(windowed.purchases, 1);
        assert_eq!(windowed.top_shop.as_deref(), Some("SCShop_Aparelli"));

        // UX Rule 2: the windowed figure must arrive WITH a baseline, or
        // "15,000 aUEC" tells a player nothing about whether that is a
        // lot. The lifetime twin is the reference point.
        let lt = windowed
            .lifetime
            .as_ref()
            .expect("a windowed response must carry its lifetime baseline");
        assert_eq!(lt.total_auec, 17500, "baseline is lifetime, not the window");
        assert_eq!(lt.purchases, 2);

        // NON-BREAKING GUARANTEE: the top-level fields still mean "the
        // requested window". Flipping them to lifetime would silently
        // change what every existing caller receives — spend.tsx already
        // passes hours and depends on the top level being scoped.
        assert_eq!(windowed.total_auec, 15000, "top level must stay windowed");

        // And with no window there is nothing to compare against, so no
        // twin — repeating the same numbers would be noise.
        assert!(
            all.lifetime.is_none(),
            "an unwindowed response must not carry a redundant twin"
        );
    }

    #[tokio::test]
    async fn stats_docking_hours_window_excludes_old_stows() {
        let now = Utc::now();
        let old = now - chrono::Duration::days(60);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now,
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "vehicle_stowed",
                old,
                json!({"landing_area":"Pad_Small_01"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let verifier = Arc::new(verifier);
        let token = sign_token(&issuer, "alice");

        let app = router(mq.clone(), verifier.clone());
        let (status, all) = get_json::<DockingResponse>(app, "/v1/me/stats/docking", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all.total_stows, 2);

        let app = router(mq, verifier);
        let (status, windowed) =
            get_json::<DockingResponse>(app, "/v1/me/stats/docking?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);

        // NON-BREAKING GUARANTEE: the top-level figures still mean "the
        // requested window". Flipping them to lifetime would silently
        // change what every existing caller receives.
        assert_eq!(windowed.total_stows, 1, "top level must stay windowed");
        assert_eq!(windowed.by_kind.hangar, 1, "top level must stay windowed");
        assert_eq!(windowed.by_kind.pad, 0, "top level must stay windowed");

        // UX Rule 2: "1 stow" tells a player nothing until they know
        // whether that is a busy week or a quiet one.
        let lt = windowed
            .lifetime
            .as_ref()
            .expect("a windowed response must carry its lifetime baseline");
        assert_eq!(lt.total_stows, 2, "baseline is lifetime, not the window");

        // And with no window there is nothing to compare against, so no
        // twin — repeating the same numbers would be noise.
        assert!(
            all.lifetime.is_none(),
            "an unwindowed response must not carry a redundant twin"
        );
    }

    // ---- previous-period twins -------------------------------------
    //
    // Every test below issues `hours=24`, so the requested window is
    // `[now-24h, now]` and the previous period is `[now-48h, now-24h)`.
    //
    // Three timestamps recur:
    //   * `now - 1h`  — inside the requested window
    //   * `now - 30h` — inside the previous period
    //   * `now - 60h` — older than BOTH
    //
    // The 60h event is not decoration: it is what proves the previous
    // period has a LOWER bound as well as an upper one. A "previous" that
    // only moved its upper edge back would swallow it and report a
    // baseline covering the player's whole history.

    const TREND_URI_SUFFIX: &str = "?hours=24";

    /// A `join_pu` inside the previous period. Carries no field any stats
    /// endpoint reads, so it moves no figure — its only job is to make
    /// `has_events_in_window` true, i.e. to say "this player existed and
    /// was logged in back then".
    fn presence_only_event(seq: i64, now: DateTime<Utc>) -> StoredQueryEvent {
        evt_with_payload(
            seq,
            "alice",
            "join_pu",
            now - chrono::Duration::hours(30),
            json!({"shard":"1a"}),
        )
    }

    #[tokio::test]
    async fn stats_spend_previous_period_counts_only_the_earlier_window() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(1),
                json!({"price":15000,"shop_name":"SCShop_Now"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(30),
                json!({"price":2500,"shop_name":"SCShop_Before"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(60),
                json!({"price":900,"shop_name":"SCShop_Ancient"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<SpendResponse>(
            app,
            &format!("/v1/me/stats/spend{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // The top level is still the requested window — unchanged by this
        // feature.
        assert_eq!(r.total_auec, 15000, "top level must stay windowed");
        assert_eq!(r.purchases, 1);

        let prev = r
            .previous
            .as_ref()
            .expect("an active player's windowed response must carry a previous period");
        // 2500 alone. Not 17500 (would mean the previous query has no
        // upper bound and is re-reading the current window) and not 3400
        // (would mean it has no lower bound and is reading all history
        // before now-24h).
        assert_eq!(
            prev.total_auec, 2500,
            "previous period is [now-48h, now-24h) — not everything before now-24h"
        );
        assert_eq!(prev.purchases, 1);
    }

    #[tokio::test]
    async fn stats_spend_previous_and_current_split_a_boundary_event_exactly_once() {
        // An event sitting on the shared edge of the two windows. The
        // handler reads its own `Utc::now()` microseconds after this test
        // reads one, so which SIDE the edge lands on is not knowable from
        // out here — the exact-instant direction (`until` exclusive) is
        // pinned in `repo.rs` by
        // `until_is_exclusive_so_adjacent_windows_count_the_edge_once`,
        // where both bounds are passed literally.
        //
        // What IS knowable, and is the invariant that matters to a trend
        // figure, is that the two windows TILE: two purchases exist, and
        // "this period" plus "the period before" must account for exactly
        // two. Three would mean the windows overlap and the player is
        // being shown their own current spending as their baseline.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(1),
                json!({"price":15000,"shop_name":"SCShop_Now"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(24),
                json!({"price":700,"shop_name":"SCShop_Edge"}),
            ),
            // Anchors the presence probe so `previous` is `Some` no matter
            // which side the edge event falls on.
            presence_only_event(3, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<SpendResponse>(
            app,
            &format!("/v1/me/stats/spend{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let prev = r
            .previous
            .as_ref()
            .expect("previous period must be present");
        assert_eq!(
            r.purchases + prev.purchases,
            2,
            "each purchase must fall in exactly one of the two windows — \
             3 means they overlap, 1 means the edge event was lost"
        );
        assert_eq!(
            r.total_auec + prev.total_auec,
            15700,
            "the edge purchase's aUEC must be counted once, on one side or the other"
        );
    }

    #[tokio::test]
    async fn stats_spend_previous_is_none_for_a_handle_with_no_prior_activity() {
        // A brand-new player: every event they have is inside the
        // requested window. Their previous period is empty because they
        // did not exist yet, not because they spent nothing — so there is
        // no comparison to draw and the field must be absent entirely.
        // Returning zeros here would render as "+15,000, trending
        // upwards" against a baseline the player never lived through.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "shop_buy_request",
            now - chrono::Duration::hours(1),
            json!({"price":15000,"shop_name":"SCShop_Now"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<SpendResponse>(
            app,
            &format!("/v1/me/stats/spend{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(r.total_auec, 15000);
        assert!(
            r.previous.is_none(),
            "a handle with no events before the window has no previous period to compare against"
        );
    }

    #[tokio::test]
    async fn stats_spend_previous_is_some_zeros_when_the_player_was_active_but_did_not_spend() {
        // The other side of the same coin. This player WAS logged in
        // during the previous period — they just bought nothing. "0 last
        // week, 15,000 this week" is a true and useful statement, so the
        // twin must be present and read zero.
        //
        // The previous-period presence is proven by a `join_pu`, an event
        // type `stats_spend` never reads. A probe scoped to
        // `shop_buy_request` would see nothing and wrongly suppress the
        // comparison — this test is what separates the two designs.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(1),
                json!({"price":15000,"shop_name":"SCShop_Now"}),
            ),
            presence_only_event(2, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<SpendResponse>(
            app,
            &format!("/v1/me/stats/spend{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r.previous.as_ref().expect(
            "a player who was active but spent nothing gets a real zero, not a suppression",
        );
        assert_eq!(prev.total_auec, 0);
        assert_eq!(prev.purchases, 0);
    }

    #[tokio::test]
    async fn stats_fleet_previous_period_counts_only_the_earlier_window() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(1),
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(30),
                json!({"vehicle_class":"AEGS_Gladius"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(30),
                json!({"vehicle_class":"AEGS_Gladius"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(60),
                json!({"vehicle_class":"ANVL_Carrack"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<FleetResponse>(
            app,
            &format!("/v1/me/stats/fleet{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(r.ships.len(), 1, "top level must stay windowed");

        let prev = r
            .previous
            .as_ref()
            .expect("an active player's windowed response must carry a previous period");
        assert_eq!(prev.total_trips, 2, "two Gladius trips, 30h ago");
        assert_eq!(
            prev.ships_flown, 1,
            "one distinct ship in the previous period — the Carrack is older than it"
        );
    }

    #[tokio::test]
    async fn stats_fleet_previous_and_current_split_a_boundary_event_exactly_once() {
        // Same tiling invariant as the spend boundary test.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(1),
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(24),
                json!({"vehicle_class":"AEGS_Gladius"}),
            ),
            presence_only_event(3, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<FleetResponse>(
            app,
            &format!("/v1/me/stats/fleet{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("previous period must be present");
        let windowed_trips: i64 = r.ships.iter().map(|s| s.trip_count).sum();
        assert_eq!(
            windowed_trips + prev.total_trips,
            2,
            "each trip must fall in exactly one of the two windows"
        );
    }

    #[tokio::test]
    async fn stats_fleet_previous_is_none_for_a_handle_with_no_prior_activity() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "quantum_target_selected",
            now - chrono::Duration::hours(1),
            json!({"vehicle_class":"DRAK_Cutlass_Black"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<FleetResponse>(
            app,
            &format!("/v1/me/stats/fleet{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            r.previous.is_none(),
            "a handle with no events before the window has no previous period to compare against"
        );
    }

    #[tokio::test]
    async fn stats_fleet_previous_is_some_zeros_when_the_player_was_active_but_did_not_fly() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(1),
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            presence_only_event(2, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<FleetResponse>(
            app,
            &format!("/v1/me/stats/fleet{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("a player who was active but flew nowhere gets a real zero");
        assert_eq!(prev.total_trips, 0);
        assert_eq!(prev.ships_flown, 0);
    }

    #[tokio::test]
    async fn stats_fleet_previous_distinct_ships_are_not_a_subtraction() {
        // The same ship flown in BOTH periods. A `previous` computed as
        // `distinct(48h) - distinct(24h)` would report 0 ships flown
        // before, because the Cutlass is a member of both sets. Computing
        // the previous window on its own terms reports 1, which is what
        // actually happened.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(1),
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_target_selected",
                now - chrono::Duration::hours(30),
                json!({"vehicle_class":"DRAK_Cutlass_Black"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<FleetResponse>(
            app,
            &format!("/v1/me/stats/fleet{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("previous period must be present");
        assert_eq!(
            prev.ships_flown, 1,
            "a ship flown in both periods still counts as one ship in the earlier one"
        );
        assert_eq!(prev.total_trips, 1);
    }

    #[tokio::test]
    async fn stats_routes_previous_period_counts_only_the_earlier_window() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(1),
                json!({"destination":"Crusader"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(30),
                json!({"destination":"microTech"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(30),
                json!({"destination":"ArcCorp"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(60),
                json!({"destination":"Hurston"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<RoutesResponse>(
            app,
            &format!("/v1/me/stats/routes{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(r.routes.len(), 1, "top level must stay windowed");

        let prev = r
            .previous
            .as_ref()
            .expect("an active player's windowed response must carry a previous period");
        assert_eq!(prev.total_trips, 2);
        assert_eq!(
            prev.destinations, 2,
            "microTech and ArcCorp — Hurston is older than the previous period"
        );
    }

    #[tokio::test]
    async fn stats_routes_previous_and_current_split_a_boundary_event_exactly_once() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(1),
                json!({"destination":"Crusader"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(24),
                json!({"destination":"microTech"}),
            ),
            presence_only_event(3, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<RoutesResponse>(
            app,
            &format!("/v1/me/stats/routes{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("previous period must be present");
        let windowed_trips: i64 = r.routes.iter().map(|d| d.count).sum();
        assert_eq!(
            windowed_trips + prev.total_trips,
            2,
            "each trip must fall in exactly one of the two windows"
        );
        assert_eq!(
            r.routes.len() as i64 + prev.destinations,
            2,
            "and so must each distinct destination"
        );
    }

    #[tokio::test]
    async fn stats_routes_previous_is_none_for_a_handle_with_no_prior_activity() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "quantum_route",
            now - chrono::Duration::hours(1),
            json!({"destination":"Crusader"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<RoutesResponse>(
            app,
            &format!("/v1/me/stats/routes{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            r.previous.is_none(),
            "a handle with no events before the window has no previous period to compare against"
        );
    }

    #[tokio::test]
    async fn stats_routes_previous_is_some_zeros_when_the_player_was_active_but_did_not_travel() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "quantum_route",
                now - chrono::Duration::hours(1),
                json!({"destination":"Crusader"}),
            ),
            presence_only_event(2, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<RoutesResponse>(
            app,
            &format!("/v1/me/stats/routes{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("a player who was active but travelled nowhere gets a real zero");
        assert_eq!(prev.total_trips, 0);
        assert_eq!(prev.destinations, 0);
    }

    #[tokio::test]
    async fn stats_docking_previous_period_folds_one_multi_ship_stow_run() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(1),
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(30),
                json!({"landing_area":"Pad_Small_01"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(30),
                json!({"landing_area":"Pad_Medium_02"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(60),
                json!({"landing_area":"Hangar_XL_09"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<DockingResponse>(
            app,
            &format!("/v1/me/stats/docking{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(r.total_stows, 1, "top level must stay windowed");

        let prev = r
            .previous
            .as_ref()
            .expect("an active player's windowed response must carry a previous period");
        assert_eq!(
            prev.total_stows, 1,
            "the two same-time ship stows are one occurrence; the 60h and current rows are excluded"
        );
    }

    #[tokio::test]
    async fn stats_docking_previous_and_current_split_a_boundary_event_exactly_once() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(1),
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(24),
                json!({"landing_area":"Pad_Small_01"}),
            ),
            presence_only_event(3, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<DockingResponse>(
            app,
            &format!("/v1/me/stats/docking{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("previous period must be present");
        assert_eq!(
            r.total_stows + prev.total_stows,
            2,
            "each stow must fall in exactly one of the two windows"
        );
    }

    #[tokio::test]
    async fn stats_docking_raw_run_crossing_boundary_belongs_to_its_first_row() {
        let now = Utc::now();
        let edge = now - chrono::Duration::hours(24);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                edge - chrono::Duration::seconds(1),
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "vehicle_stowed",
                edge + chrono::Duration::seconds(1),
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            presence_only_event(3, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, response) = get_json::<DockingResponse>(
            app,
            &format!("/v1/me/stats/docking{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response.total_stows, 0,
            "the current window only has a tail member"
        );
        assert_eq!(
            response.previous.expect("previous period").total_stows,
            1,
            "the occurrence is anchored by the first row before the boundary"
        );
    }

    #[tokio::test]
    async fn stats_docking_previous_is_none_for_a_handle_with_no_prior_activity() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "vehicle_stowed",
            now - chrono::Duration::hours(1),
            json!({"landing_area":"Hangar_Large_01"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<DockingResponse>(
            app,
            &format!("/v1/me/stats/docking{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            r.previous.is_none(),
            "a handle with no events before the window has no previous period to compare against"
        );
    }

    #[tokio::test]
    async fn stats_docking_previous_is_some_zeros_when_the_player_was_active_but_did_not_stow() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now - chrono::Duration::hours(1),
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            presence_only_event(2, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<DockingResponse>(
            app,
            &format!("/v1/me/stats/docking{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("a player who was active but stowed nothing gets a real zero");
        assert_eq!(prev.total_stows, 0);
    }

    #[tokio::test]
    async fn stats_objectives_previous_period_counts_only_the_earlier_window() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(1),
                json!({"objective_id":"obj-now","state":"completed"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(30),
                json!({"objective_id":"obj-before-a","state":"completed"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(30),
                json!({"objective_id":"obj-before-b","state":"failed"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(60),
                json!({"objective_id":"obj-ancient","state":"completed"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<ObjectivesResponse>(
            app,
            &format!("/v1/me/stats/objectives{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(r.completed, 1, "top level must stay windowed");
        assert_eq!(r.completion_pct, Some(100));

        let prev = r
            .previous
            .as_ref()
            .expect("an active player's windowed response must carry a previous period");
        assert_eq!(prev.completed, 1);
        assert_eq!(prev.failed, 1);
        assert_eq!(
            prev.total, 2,
            "the two 30h-old objectives — the 60h one is before the previous period"
        );
        // The figure the whole endpoint is about: 100% this period against
        // 50% last period is the comparison a player actually reads.
        assert_eq!(prev.completion_pct, Some(50));
    }

    #[tokio::test]
    async fn stats_objectives_previous_and_current_split_a_boundary_event_exactly_once() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(1),
                json!({"objective_id":"obj-now","state":"completed"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(24),
                json!({"objective_id":"obj-edge","state":"completed"}),
            ),
            presence_only_event(3, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<ObjectivesResponse>(
            app,
            &format!("/v1/me/stats/objectives{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("previous period must be present");
        assert_eq!(
            r.total + prev.total,
            2,
            "each distinct objective must fall in exactly one of the two windows"
        );
        assert_eq!(r.completed + prev.completed, 2);
    }

    #[tokio::test]
    async fn stats_objectives_previous_is_none_for_a_handle_with_no_prior_activity() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "mission_objective",
            now - chrono::Duration::hours(1),
            json!({"objective_id":"obj-now","state":"completed"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<ObjectivesResponse>(
            app,
            &format!("/v1/me/stats/objectives{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            r.previous.is_none(),
            "a handle with no events before the window has no previous period to compare against"
        );
    }

    #[tokio::test]
    async fn stats_objectives_previous_is_some_zeros_when_the_player_ran_no_missions() {
        // Note the shape of the zero here: `total` is 0 AND
        // `completion_pct` is `None` — nothing resolved, so there is no
        // rate. That inner `None` means "no rate", which is a different
        // statement from the outer `previous: None` meaning "no
        // comparison". Both appear in this endpoint and they must not be
        // conflated.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "mission_objective",
                now - chrono::Duration::hours(1),
                json!({"objective_id":"obj-now","state":"completed"}),
            ),
            presence_only_event(2, now),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<ObjectivesResponse>(
            app,
            &format!("/v1/me/stats/objectives{TREND_URI_SUFFIX}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let prev = r
            .previous
            .as_ref()
            .expect("a player who was active but ran no missions gets a real zero");
        assert_eq!(prev.completed, 0);
        assert_eq!(prev.failed, 0);
        assert_eq!(prev.unresolved, 0);
        assert_eq!(prev.no_outcome, 0);
        assert_eq!(prev.total, 0);
        assert_eq!(
            prev.completion_pct, None,
            "nothing resolved in that window, so there is no rate — \
             which is not the same as there being no comparison"
        );
    }

    #[tokio::test]
    async fn previous_period_is_absent_for_the_all_range() {
        // `hours = STATS_MAX_HOURS` is the dashboard's `all`: a full year,
        // which is also the retention limit. Its previous period reaches
        // from two years ago to one year ago — entirely outside what the
        // database still holds. Every player would read as zero there, and
        // "-100%, trending down" against swept data is a lie about their
        // play, not a fact about it.
        //
        // Seeded with plenty of activity a year and two years back, so the
        // suppression cannot be mistaken for an empty fixture.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::days(30),
                json!({"price":15000,"shop_name":"SCShop_Now"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::days(500),
                json!({"price":2500,"shop_name":"SCShop_Ago"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<SpendResponse>(
            app,
            &format!("/v1/me/stats/spend?hours={}", STATS_MAX_HOURS),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(r.total_auec, 15000, "the `all` window itself still works");
        assert!(
            r.previous.is_none(),
            "the period before a retention-length window is entirely outside retention"
        );
    }

    #[tokio::test]
    async fn previous_period_is_absent_when_no_window_was_requested() {
        // No `hours` means the response already IS lifetime. There is no
        // period that sits "before" all of history.
        //
        // Purchases are seeded at four depths — hours, days, weeks and
        // months back. The point is that NO window a buggy handler could
        // invent here would come back empty, so the assertion below can
        // only pass because the twin was never computed, not because the
        // handle happened to look inactive in whatever period was picked.
        // (An earlier single-event version of this fixture passed under a
        // mutation that fabricated a 30-day previous window, purely
        // because the one event fell outside it.)
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(1),
                json!({"price":100,"shop_name":"SCShop_Hour"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::hours(30),
                json!({"price":2500,"shop_name":"SCShop_Before"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::days(45),
                json!({"price":400,"shop_name":"SCShop_Weeks"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "shop_buy_request",
                now - chrono::Duration::days(200),
                json!({"price":700,"shop_name":"SCShop_Months"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, r) = get_json::<SpendResponse>(app, "/v1/me/stats/spend", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert!(r.lifetime.is_none());
        assert!(
            r.previous.is_none(),
            "an unwindowed response has no previous period"
        );
    }

    #[tokio::test]
    async fn stats_routes_rejects_invalid_hours() {
        // Present-but-invalid `hours` 400s; absent stays lifetime (covered
        // by the window test above).
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));
        let (status, _body) =
            get_json::<serde_json::Value>(app, "/v1/me/stats/routes?hours=0", &token).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stats_spend_requires_auth() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (_issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/stats/spend")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn lists_events_for_authenticated_handle() {
        let mq = Arc::new(MemoryQuery::new(vec![
            StoredQueryEvent {
                seq: 1,
                claimed_handle: "Alice".into(),
                event_type: "join_pu".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({"type":"join_pu"}),
                resolved_location: None,
                hidden_at: None,
            },
            StoredQueryEvent {
                seq: 2,
                claimed_handle: "Bob".into(),
                event_type: "join_pu".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({"type":"join_pu"}),
                resolved_location: None,
                hidden_at: None,
            },
            StoredQueryEvent {
                seq: 3,
                claimed_handle: "Alice".into(),
                event_type: "actor_death".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({"type":"actor_death"}),
                resolved_location: None,
                hidden_at: None,
            },
        ]));

        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/events")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: EventsListResponse = serde_json::from_slice(&bytes).unwrap();

        // Only Alice's two events.
        assert_eq!(parsed.events.len(), 2);
        assert!(parsed
            .events
            .iter()
            .all(|e| e.event_type != "join_pu" || e.payload["type"] == "join_pu"));
        assert_eq!(parsed.next_after, Some(3));
    }

    #[tokio::test]
    async fn list_events_does_not_echo_client_supplied_resolved_location() {
        // Trust boundary (F4): the collector-supplied `resolved_location`
        // is untrusted — a malicious client can stamp any KB slug on it,
        // which the web renders as a `/kb/location/{slug}` link. The
        // events feed must re-derive the location server-side from the
        // event's own payload and never echo the stored slug. This event
        // carries no classifiable location, so the server-derived value
        // is None and the spoofed slug is dropped.
        let mq = Arc::new(MemoryQuery::new(vec![StoredQueryEvent {
            seq: 1,
            claimed_handle: "Alice".into(),
            event_type: "process_init".into(),
            event_timestamp: None,
            log_source: "live".into(),
            source_offset: 0,
            payload: json!({"type": "process_init"}),
            resolved_location: Some(json!({
                "slug": "phishing-target",
                "display_name": "Totally Legit Place"
            })),
            hidden_at: None,
        }]));

        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, resp) = get_json::<EventsListResponse>(app, "/v1/me/events", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(
            resp.events[0].resolved_location, None,
            "client-supplied resolved_location must not be echoed to the web"
        );
    }

    #[tokio::test]
    async fn summary_aggregates_by_type() {
        let mq = Arc::new(MemoryQuery::new(vec![
            StoredQueryEvent {
                seq: 1,
                claimed_handle: "Alice".into(),
                event_type: "join_pu".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({}),
                resolved_location: None,
                hidden_at: None,
            },
            StoredQueryEvent {
                seq: 2,
                claimed_handle: "Alice".into(),
                event_type: "join_pu".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({}),
                resolved_location: None,
                hidden_at: None,
            },
            StoredQueryEvent {
                seq: 3,
                claimed_handle: "Alice".into(),
                event_type: "actor_death".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({}),
                resolved_location: None,
                hidden_at: None,
            },
            StoredQueryEvent {
                seq: 4,
                claimed_handle: "Bob".into(),
                event_type: "join_pu".into(),
                event_timestamp: None,
                log_source: "live".into(),
                source_offset: 0,
                payload: json!({}),
                resolved_location: None,
                hidden_at: None,
            },
        ]));

        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/summary")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: SummaryResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.claimed_handle, "Alice");
        assert_eq!(parsed.total, 3);
        let join = parsed
            .by_type
            .iter()
            .find(|t| t.event_type == "join_pu")
            .unwrap();
        assert_eq!(join.count, 2);
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[tokio::test]
    async fn list_events_filter_by_event_type_returns_only_matching() {
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "join_pu", None),
            evt(2, "Alice", "actor_death", None),
            evt(3, "Alice", "join_pu", None),
            evt(4, "Alice", "vehicle_destruction", None),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<EventsListResponse>(app, "/v1/me/events?event_type=join_pu", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.events.len(), 2);
        assert!(parsed.events.iter().all(|e| e.event_type == "join_pu"));
    }

    #[tokio::test]
    async fn list_events_filter_by_since_returns_events_after_timestamp() {
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", Some(ts("2026-04-01T00:00:00Z"))),
            evt(2, "Alice", "x", Some(ts("2026-04-15T00:00:00Z"))),
            evt(3, "Alice", "x", Some(ts("2026-05-01T00:00:00Z"))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<EventsListResponse>(app, "/v1/me/events?since=2026-04-15T00:00:00Z", &token)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.events.len(), 2);
        let seqs: Vec<i64> = parsed.events.iter().map(|e| e.seq).collect();
        assert!(seqs.contains(&2));
        assert!(seqs.contains(&3));
    }

    #[tokio::test]
    async fn list_events_filter_by_until_returns_events_before_timestamp() {
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", Some(ts("2026-04-01T00:00:00Z"))),
            evt(2, "Alice", "x", Some(ts("2026-04-15T00:00:00Z"))),
            evt(3, "Alice", "x", Some(ts("2026-05-01T00:00:00Z"))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<EventsListResponse>(app, "/v1/me/events?until=2026-04-15T00:00:00Z", &token)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.events.len(), 2);
        let seqs: Vec<i64> = parsed.events.iter().map(|e| e.seq).collect();
        assert!(seqs.contains(&1));
        assert!(seqs.contains(&2));
    }

    #[tokio::test]
    async fn list_events_before_seq_paginates_descending() {
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", None),
            evt(2, "Alice", "x", None),
            evt(3, "Alice", "x", None),
            evt(4, "Alice", "x", None),
            evt(5, "Alice", "x", None),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<EventsListResponse>(app, "/v1/me/events?before_seq=4&limit=2", &token).await;
        assert_eq!(status, StatusCode::OK);
        let seqs: Vec<i64> = parsed.events.iter().map(|e| e.seq).collect();
        // Strictly less than 4, DESC, limit 2 -> [3, 2].
        assert_eq!(seqs, vec![3, 2]);
    }

    #[tokio::test]
    async fn list_events_after_seq_paginates_ascending() {
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", None),
            evt(2, "Alice", "x", None),
            evt(3, "Alice", "x", None),
            evt(4, "Alice", "x", None),
            evt(5, "Alice", "x", None),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<EventsListResponse>(app, "/v1/me/events?after_seq=2&limit=2", &token).await;
        assert_eq!(status, StatusCode::OK);
        let seqs: Vec<i64> = parsed.events.iter().map(|e| e.seq).collect();
        // Strictly greater than 2, ASC, limit 2 -> [3, 4].
        assert_eq!(seqs, vec![3, 4]);
    }

    #[tokio::test]
    async fn list_events_rejects_both_cursors() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/events?before_seq=10&after_seq=2")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.error, "conflicting_cursors");
    }

    #[tokio::test]
    async fn list_events_rejects_invalid_event_type() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        // Uppercase + dash -> invalid by [a-z0-9_]{1,64} rule.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/events?event_type=Join-PU")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.error, "invalid_event_type");
    }

    #[tokio::test]
    async fn timeline_returns_per_day_counts_with_zero_padding() {
        let now = Utc::now();
        let one_day_ago = now - Duration::days(1);
        let three_days_ago = now - Duration::days(3);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", Some(now)),
            evt(2, "Alice", "x", Some(now)),
            evt(3, "Alice", "x", Some(one_day_ago)),
            evt(4, "Alice", "x", Some(three_days_ago)),
            // Bob's events should be excluded.
            evt(5, "Bob", "x", Some(now)),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<TimelineResponse>(app, "/v1/me/timeline?days=7", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.days, 7);
        // Zero-padded to exactly `days` buckets.
        assert_eq!(parsed.buckets.len(), 7);

        let total: u64 = parsed.buckets.iter().map(|b| b.count).sum();
        // Alice has 4 events inside the window (now, now, -1d, -3d).
        assert_eq!(total, 4);

        let today_key = now.date_naive().format("%Y-%m-%d").to_string();
        let today_bucket = parsed
            .buckets
            .iter()
            .find(|b| b.date == today_key)
            .expect("today bucket present");
        assert_eq!(today_bucket.count, 2);

        // At least one bucket is zero — that's the zero-padding.
        assert!(parsed.buckets.iter().any(|b| b.count == 0));

        // Buckets must be ordered ascending by date.
        let dates: Vec<&String> = parsed.buckets.iter().map(|b| &b.date).collect();
        let mut sorted = dates.clone();
        sorted.sort();
        assert_eq!(dates, sorted);
    }

    #[tokio::test]
    async fn timeline_rejects_days_above_max() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/timeline?days=400")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.error, "invalid_days");
    }

    #[tokio::test]
    async fn metrics_event_types_returns_count_and_last_seen() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "join_pu", Some(now - Duration::days(40))),
            evt(2, "Alice", "join_pu", Some(now - Duration::days(2))),
            evt(3, "Alice", "actor_death", Some(now - Duration::hours(1))),
            evt(4, "Bob", "join_pu", Some(now)),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) = get_json::<EventTypeBreakdownResponse>(
            app,
            "/v1/me/metrics/event-types?range=30d",
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.range, "30d");
        // Within the 30-day window: 1 join_pu + 1 actor_death.
        // Bob's row excluded.
        let join = parsed
            .types
            .iter()
            .find(|t| t.event_type == "join_pu")
            .expect("join_pu present");
        assert_eq!(join.count, 1);
        let death = parsed
            .types
            .iter()
            .find(|t| t.event_type == "actor_death")
            .expect("actor_death present");
        assert_eq!(death.count, 1);
        assert!(death.last_seen.is_some());
    }

    #[tokio::test]
    async fn metrics_event_types_range_all_is_bounded_to_retention() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "join_pu", Some(now - Duration::days(400))),
            evt(2, "Alice", "join_pu", Some(now - Duration::days(2))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) = get_json::<EventTypeBreakdownResponse>(
            app,
            "/v1/me/metrics/event-types?range=all",
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.range, "all");
        let join = parsed
            .types
            .iter()
            .find(|t| t.event_type == "join_pu")
            .unwrap();
        // `all` is 365 DAYS, not unbounded — 365 is the hard retention
        // limit, so "everything we have" and "the last year" are the
        // same set. The 400-day-old row is outside retention and
        // therefore outside `all`; only the recent one counts.
        //
        // This test previously asserted 2, encoding `all` as an absent
        // filter. Bounding it explicitly stops the API promising a depth
        // the data does not have.
        assert_eq!(join.count, 1);
    }

    #[tokio::test]
    async fn metrics_event_types_supports_a_real_24h_bucket() {
        // The UI offers 24h. Without a server bucket the client widened
        // it to 7d and rendered a WEEK under a "24h" label — a
        // confidently wrong number, worse than a missing one.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "join_pu", Some(now - Duration::hours(2))),
            evt(2, "Alice", "join_pu", Some(now - Duration::days(3))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) = get_json::<EventTypeBreakdownResponse>(
            app,
            "/v1/me/metrics/event-types?range=24h",
            &token,
        )
        .await;

        assert_eq!(status, StatusCode::OK, "24h must be an accepted range");
        assert_eq!(parsed.range, "24h");
        let join = parsed
            .types
            .iter()
            .find(|t| t.event_type == "join_pu")
            .expect("join_pu present");
        assert_eq!(join.count, 1, "the 3-day-old row is outside 24h");
    }

    #[tokio::test]
    async fn metrics_event_types_rejects_unknown_range() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/metrics/event-types?range=year")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.error, "invalid_range");
    }

    #[tokio::test]
    async fn metrics_sessions_groups_events_within_idle_threshold() {
        // Three events 5 minutes apart -> 1 session of 3 events.
        // Then a 60-minute gap -> new session with 2 more events.
        let base = ts("2026-04-15T10:00:00Z");
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", Some(base)),
            evt(2, "Alice", "x", Some(base + Duration::minutes(5))),
            evt(3, "Alice", "x", Some(base + Duration::minutes(10))),
            evt(4, "Alice", "x", Some(base + Duration::minutes(70))),
            evt(5, "Alice", "x", Some(base + Duration::minutes(80))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<SessionsResponse>(app, "/v1/me/metrics/sessions", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed.sessions.len(), 2);

        // Newest first.
        assert_eq!(parsed.sessions[0].event_count, 2);
        assert_eq!(parsed.sessions[0].start_at, base + Duration::minutes(70));
        assert_eq!(parsed.sessions[0].end_at, base + Duration::minutes(80));

        assert_eq!(parsed.sessions[1].event_count, 3);
        assert_eq!(parsed.sessions[1].start_at, base);
        assert_eq!(parsed.sessions[1].end_at, base + Duration::minutes(10));
    }

    #[tokio::test]
    async fn metrics_sessions_excludes_other_handles() {
        let base = ts("2026-04-15T10:00:00Z");
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", Some(base)),
            evt(2, "Bob", "x", Some(base + Duration::minutes(5))),
            evt(3, "Alice", "x", Some(base + Duration::minutes(8))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<SessionsResponse>(app, "/v1/me/metrics/sessions", &token).await;
        assert_eq!(status, StatusCode::OK);
        // Only Alice's two events; one session of 2.
        assert_eq!(parsed.sessions.len(), 1);
        assert_eq!(parsed.sessions[0].event_count, 2);
    }

    fn batch(seq: i64, occurred_at: DateTime<Utc>, total: i64, accepted: i64) -> IngestBatchRow {
        IngestBatchRow {
            seq,
            occurred_at,
            batch_id: format!("b{seq}"),
            game_build: Some("4.0-LIVE.test".into()),
            device_id: None,
            total,
            accepted,
            duplicate: 0,
            rejected: total - accepted,
        }
    }

    fn batch_with_device(
        seq: i64,
        occurred_at: DateTime<Utc>,
        device_id: uuid::Uuid,
    ) -> IngestBatchRow {
        IngestBatchRow {
            seq,
            occurred_at,
            batch_id: format!("b{seq}"),
            game_build: Some("4.0-LIVE.test".into()),
            device_id: Some(device_id),
            total: 10,
            accepted: 10,
            duplicate: 0,
            rejected: 0,
        }
    }

    #[tokio::test]
    async fn ingest_history_returns_calling_handles_batches_newest_first() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![]).with_ingest_history(vec![
            (
                "Alice".into(),
                batch(10, now - Duration::hours(1), 200, 198),
            ),
            (
                "Alice".into(),
                batch(11, now - Duration::minutes(20), 50, 50),
            ),
            ("Bob".into(), batch(12, now - Duration::minutes(5), 30, 30)),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) =
            get_json::<IngestHistoryResponse>(app, "/v1/me/ingest-history", &token).await;
        assert_eq!(status, StatusCode::OK);
        // Bob's row excluded, Alice's two rows newest first.
        assert_eq!(parsed.batches.len(), 2);
        assert_eq!(parsed.batches[0].seq, 11);
        assert_eq!(parsed.batches[1].seq, 10);
        assert_eq!(parsed.batches[0].total, 50);
        assert_eq!(parsed.batches[1].accepted, 198);
    }

    #[tokio::test]
    async fn ingest_history_filters_by_device_id_when_passed() {
        // Three batches under "Alice": two from device-A, one from
        // device-B, one legacy (no device). Device-scoped query
        // returns only the matching device's rows; absent query
        // returns the whole account stream.
        let now = Utc::now();
        let dev_a = uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let dev_b = uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let mq = Arc::new(MemoryQuery::new(vec![]).with_ingest_history(vec![
            (
                "Alice".into(),
                batch_with_device(1, now - Duration::hours(3), dev_a),
            ),
            ("Alice".into(), batch(2, now - Duration::hours(2), 5, 5)),
            (
                "Alice".into(),
                batch_with_device(3, now - Duration::hours(1), dev_b),
            ),
            (
                "Alice".into(),
                batch_with_device(4, now - Duration::minutes(10), dev_a),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, all) =
            get_json::<IngestHistoryResponse>(app.clone(), "/v1/me/ingest-history", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all.batches.len(), 4);

        let (status, only_a) = get_json::<IngestHistoryResponse>(
            app.clone(),
            &format!("/v1/me/ingest-history?device_id={dev_a}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(only_a.batches.len(), 2);
        assert!(only_a.batches.iter().all(|b| b.device_id == Some(dev_a)));
        // Newest first.
        assert_eq!(only_a.batches[0].seq, 4);
        assert_eq!(only_a.batches[1].seq, 1);

        let (status, only_b) = get_json::<IngestHistoryResponse>(
            app,
            &format!("/v1/me/ingest-history?device_id={dev_b}"),
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(only_b.batches.len(), 1);
        assert_eq!(only_b.batches[0].seq, 3);
        assert_eq!(only_b.batches[0].device_id, Some(dev_b));
    }

    #[tokio::test]
    async fn ingest_history_paginates_via_offset() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![]).with_ingest_history(vec![
            ("Alice".into(), batch(1, now - Duration::hours(3), 10, 10)),
            ("Alice".into(), batch(2, now - Duration::hours(2), 10, 10)),
            ("Alice".into(), batch(3, now - Duration::hours(1), 10, 10)),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, parsed) = get_json::<IngestHistoryResponse>(
            app,
            "/v1/me/ingest-history?limit=2&offset=1",
            &token,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Newest-first ordering means seq 3,2,1 -> offset 1 limit 2 -> [2,1].
        assert_eq!(parsed.batches.len(), 2);
        assert_eq!(parsed.batches[0].seq, 2);
        assert_eq!(parsed.batches[1].seq, 1);
    }

    #[tokio::test]
    async fn metrics_sessions_paginates_via_offset() {
        let base = ts("2026-04-15T10:00:00Z");
        // Three sessions, each separated by > 30 min.
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "x", Some(base)),
            evt(2, "Alice", "x", Some(base + Duration::hours(2))),
            evt(3, "Alice", "x", Some(base + Duration::hours(4))),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq.clone(), Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");

        let (status, page1) =
            get_json::<SessionsResponse>(app, "/v1/me/metrics/sessions?limit=2&offset=0", &token)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page1.sessions.len(), 2);

        let (issuer2, verifier2) = fresh_pair();
        let app2 = router(mq, Arc::new(verifier2));
        let token2 = sign_token(&issuer2, "Alice");
        let (_, page2) =
            get_json::<SessionsResponse>(app2, "/v1/me/metrics/sessions?limit=2&offset=2", &token2)
                .await;
        assert_eq!(page2.sessions.len(), 1);
    }

    // -- /v1/me/location/current ---------------------------------

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

    async fn get_status_and_bytes(app: Router, uri: &str, token: &str) -> (StatusCode, Vec<u8>) {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec();
        (status, bytes)
    }

    #[tokio::test]
    async fn location_current_returns_204_when_user_has_no_events() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, _) = get_status_and_bytes(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn location_current_returns_real_place_even_when_stale() {
        // A stale but real place (2h old, formerly outside the 90-min gate)
        // is now returned with 200 so the UI can show it with an honest age
        // ("2h ago") instead of hiding the pill entirely. 204 is reserved for
        // the "no real place at all" case, not "real place but old".
        let stale = Utc::now() - Duration::hours(2);
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "planet_terrain_load",
            stale,
            json!({"planet": "OOC_Stanton_2b_Daymar"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "stale-but-real place must surface with 200, not 204 — age is shown via last_seen_at"
        );
        assert_eq!(body.location.planet.as_deref(), Some("Daymar"));
    }

    #[tokio::test]
    async fn location_current_resolves_planet_terrain_within_window() {
        let recent = Utc::now() - Duration::minutes(5);
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "planet_terrain_load",
            recent,
            json!({"planet": "OOC_Stanton_2b_Daymar"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.location.planet.as_deref(), Some("Daymar"));
        assert_eq!(body.location.system.as_deref(), Some("Stanton"));
        assert_eq!(body.location.source_event_type, "planet_terrain_load");
    }

    #[tokio::test]
    async fn location_current_includes_classified_resolved_location() {
        // The raw engine key must be classified at query time so the web
        // "you are here" surface gets a friendly name instead of the raw
        // identifier. Empty catalog → heuristic/fallback, but a non-empty
        // display_name is still guaranteed.
        let recent = Utc::now() - Duration::minutes(5);
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "planet_terrain_load",
            recent,
            json!({"planet": "OOC_Stanton_2b_Daymar"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        let resolved = body
            .location
            .resolved_location
            .expect("resolved_location must be populated from the raw planet key");
        assert!(
            !resolved.display_name.is_empty(),
            "classifier always yields a display_name"
        );
    }

    #[tokio::test]
    async fn location_trace_classifies_each_entry() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "planet_terrain_load",
            now - Duration::minutes(30),
            json!({"planet": "OOC_Stanton_2b_Daymar"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<TraceResponse>(app, "/v1/me/location/trace?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.entries.len(), 1);
        let entry = &body.entries[0];
        let resolved = entry
            .resolved_location
            .as_ref()
            .expect("each dwell entry carries a classified resolved_location");
        assert!(!resolved.display_name.is_empty());
    }

    #[tokio::test]
    async fn location_trace_returns_oldest_first() {
        // The web client's `toDistinctStops` dwell collapse assumes
        // oldest-first input (enteredAt = first seen in a run); the
        // timeline reverses for newest-first DISPLAY. So the API must
        // hand back oldest-first, with the most-recent N kept on
        // truncation. Three distinct stops at t-60 / t-40 / t-20.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(60),
                json!({"planet": "OOC_Stanton_2a_Cellin"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(40),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
            evt_with_payload(
                3,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(20),
                json!({"planet": "OOC_Stanton_3a_Yela"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<TraceResponse>(app, "/v1/me/location/trace?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.entries.len(), 3, "three distinct stops");
        assert!(
            body.entries[0].started_at < body.entries[1].started_at
                && body.entries[1].started_at < body.entries[2].started_at,
            "entries must be oldest-first (ascending started_at), got {:?}",
            body.entries
                .iter()
                .map(|e| e.started_at)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn location_trace_large_window_is_bounded_and_ordered() {
        // A window far wider than the old one-week cap (a full year)
        // must NOT 400, and the response must stay BOUNDED regardless of
        // how many raw events fall in the window. 300 events alternating
        // between two distinct planets over ~250 days: adjacent readings
        // always differ, so none collapse — the walker yields 300 dwell
        // entries, truncated to the most-recent `TRACE_LIMIT_DEFAULT`
        // (200). Oldest-first ordering must survive the truncation.
        let now = Utc::now();
        let mut rows = Vec::new();
        for i in 0..300i64 {
            // Ascending timestamps: i=0 is the oldest (~250 days back).
            let ts = now - Duration::hours((300 - i) * 20);
            let planet = if i % 2 == 0 {
                "OOC_Stanton_2b_Daymar"
            } else {
                "OOC_Stanton_3a_Yela"
            };
            rows.push(evt_with_payload(
                i + 1,
                "Alice",
                "planet_terrain_load",
                ts,
                json!({ "planet": planet }),
            ));
        }
        let mq = Arc::new(MemoryQuery::new(rows));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        // hours = 24 * 365 — an "All"-scale range that would have 400'd
        // under the old one-week cap.
        let (status, body) =
            get_json::<TraceResponse>(app, "/v1/me/location/trace?hours=8760", &token).await;
        assert_eq!(status, StatusCode::OK, "a year-wide window must not 400");
        assert_eq!(
            body.entries.len(),
            TRACE_LIMIT_DEFAULT as usize,
            "result is bounded to the most-recent {TRACE_LIMIT_DEFAULT} dwell entries",
        );
        assert!(
            body.entries
                .windows(2)
                .all(|w| w[0].started_at <= w[1].started_at),
            "entries must remain oldest-first after truncation",
        );
    }

    #[tokio::test]
    async fn location_breakdown_large_window_does_not_400() {
        // The breakdown cap also lifted to a year (bounded by
        // `BREAKDOWN_RAW_LIMIT`). A window past the old 30-day cap must
        // now succeed rather than 400.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "planet_terrain_load",
                now - Duration::days(200),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                now - Duration::days(100),
                json!({"planet": "OOC_Stanton_3a_Yela"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        // hours = 24 * 300 (well past the old 30-day / 720-hour cap).
        let (status, body) =
            get_json::<BreakdownResponse>(app, "/v1/me/location/breakdown?hours=7200", &token)
                .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a window past the old 30-day cap must not 400",
        );
        assert!(
            !body.entries.is_empty(),
            "both in-window locations should aggregate",
        );
    }

    #[tokio::test]
    async fn location_current_prefers_most_recent_event_over_older_more_precise_one() {
        // Older inventory request (precise: city) followed by newer
        // planet_terrain (less precise: planet only). The handler
        // surfaces the most-recent reading, NOT the most-precise one
        // — staleness is the dominant axis. The precise reading might
        // mis-represent where the user is RIGHT NOW.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(60),
                json!({"location": "Stanton1_Lorville"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(2),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.location.planet.as_deref(), Some("Daymar"));
        assert!(
            body.location.city.is_none(),
            "city should be None — the recent event was planet-only"
        );
    }

    #[tokio::test]
    async fn location_current_attaches_shard_hint_from_separate_join_pu() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "join_pu",
                now - Duration::minutes(30),
                json!({"shard": "pub_euw1b_test", "address": "1.2.3.4", "port": 64300, "location_id": "1"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(2),
                json!({"planet": "OOC_Stanton_1_Hurston"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        // Headline is the planet from the recent event…
        assert_eq!(body.location.planet.as_deref(), Some("Hurston"));
        // …with the shard from the older join_pu carried as context.
        assert_eq!(body.location.shard.as_deref(), Some("pub_euw1b_test"));
    }

    #[tokio::test]
    async fn location_current_sets_entered_at_to_oldest_event_in_contiguous_run() {
        // Three planet_terrain_load events at Daymar over the last 20
        // minutes. The user has clearly been at Daymar the whole time
        // — entered_at should anchor on the OLDEST of the three, not
        // the newest.
        let now = Utc::now();
        let oldest = now - Duration::minutes(20);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "planet_terrain_load",
                oldest,
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(10),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
            evt_with_payload(
                3,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(2),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.location.entered_at, Some(oldest));
        assert!(!body.location.entered_at_is_lower_bound);
    }

    #[tokio::test]
    async fn location_current_entered_at_stops_at_first_location_change() {
        // Older Lorville event (different key) followed by two Daymar
        // events. entered_at should be the FIRST Daymar event — the
        // moment the user transitioned in — not the Lorville one.
        let now = Utc::now();
        let first_at_daymar = now - Duration::minutes(15);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(60),
                json!({"location": "Stanton1_Lorville"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                first_at_daymar,
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
            evt_with_payload(
                3,
                "Alice",
                "planet_terrain_load",
                now - Duration::minutes(3),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.location.planet.as_deref(), Some("Daymar"));
        assert_eq!(body.location.entered_at, Some(first_at_daymar));
        // Run ended naturally inside the batch — not a lower bound.
        assert!(!body.location.entered_at_is_lower_bound);
    }

    #[tokio::test]
    async fn location_current_omits_entered_at_when_only_one_event() {
        // Single event → no run to walk → entered_at should be None
        // (echoing last_seen_at would mislead the UI into rendering
        // "here 0s" the instant the user logs in).
        let recent = Utc::now() - Duration::minutes(5);
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "planet_terrain_load",
            recent,
            json!({"planet": "OOC_Stanton_2b_Daymar"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.location.entered_at.is_none());
        assert!(!body.location.entered_at_is_lower_bound);
    }

    #[tokio::test]
    async fn location_current_marks_lower_bound_when_run_saturates_batch() {
        // Seed exactly ENTERED_AT_RUN_LIMIT events all at the same
        // place. The walk-back consumes the whole batch without
        // finding a transition — entered_at is the oldest event we
        // saw, and `entered_at_is_lower_bound` must be true so the
        // UI renders the trailing "+".
        let now = Utc::now();
        let limit = crate::locations::ENTERED_AT_RUN_LIMIT;
        let mut rows = Vec::with_capacity(limit as usize);
        for i in 0..limit {
            let ts = now - Duration::minutes(i + 1);
            rows.push(evt_with_payload(
                i + 1,
                "Alice",
                "planet_terrain_load",
                ts,
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ));
        }
        let mq = Arc::new(MemoryQuery::new(rows));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.location.entered_at.is_some());
        assert!(
            body.location.entered_at_is_lower_bound,
            "expected lower-bound flag when the batch saturates without a key change"
        );
    }

    #[tokio::test]
    async fn stats_combat_splits_kills_and_deaths_via_payload_filter() {
        // Three actor_death events in the user's stream:
        //   1. Caller killed npc_pirate (caller is killer)
        //   2. npc_pirate killed caller (caller is victim)
        //   3. Two npcs fighting each other (caller is neither)
        // Expected: kills=1, deaths=1. The third row inflates the
        // pre-fix "total deaths" count but is correctly excluded by
        // both filters.
        let now = Utc::now() - Duration::hours(1);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "actor_death",
                now,
                json!({
                    "killer": "Alice",
                    "victim": "npc_pirate",
                    "weapon": "P4AR",
                    "zone": "ArcCorp",
                    "damage_type": "Bullet"
                }),
            ),
            evt_with_payload(
                2,
                "Alice",
                "actor_death",
                now + Duration::minutes(5),
                json!({
                    "killer": "npc_pirate",
                    "victim": "Alice",
                    "weapon": "S71",
                    "zone": "Daymar",
                    "damage_type": "Bullet"
                }),
            ),
            evt_with_payload(
                3,
                "Alice",
                "actor_death",
                now + Duration::minutes(10),
                json!({
                    "killer": "npc_a",
                    "victim": "npc_b",
                    "weapon": "Knife",
                    "zone": "Lorville",
                    "damage_type": "Melee"
                }),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CombatStatsResponse>(app, "/v1/me/stats/combat?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.kills, 1, "kills counts only killer==Alice rows");
        assert_eq!(body.deaths, 1, "deaths counts only victim==Alice rows");
        // Top weapons scoped to kills — only the P4AR (used by Alice)
        // should appear, NOT the S71 (used to kill Alice) or Knife
        // (NPC vs NPC).
        assert_eq!(body.top_weapons.len(), 1);
        assert_eq!(body.top_weapons[0].value, "P4AR");
        // Hot zones scoped to deaths — only Daymar (where Alice died)
        // should appear, NOT ArcCorp (where Alice killed) or Lorville
        // (NPC vs NPC).
        assert_eq!(body.deaths_by_zone.len(), 1);
        assert_eq!(body.deaths_by_zone[0].value, "Daymar");
    }

    /// A death total must say how much of itself was reconstructed.
    /// CIG removed the Actor Death lines, so `player_death` rows with
    /// `body_class = "inferred"` are the Corpse-derived ones — and
    /// summing them into a single count is exactly what hides that.
    #[tokio::test]
    async fn combat_deaths_report_how_many_were_inferred() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "player_death",
                now,
                json!({ "body_class": "inferred", "zone": "Daymar" }),
            ),
            evt_with_payload(
                2,
                "Alice",
                "player_death",
                now + Duration::minutes(2),
                json!({ "body_class": "inferred", "zone": "Daymar" }),
            ),
            evt_with_payload(
                3,
                "Alice",
                "player_death",
                now + Duration::minutes(4),
                json!({ "body_class": "body_01", "zone": "Daymar" }),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CombatStatsResponse>(app, "/v1/me/stats/combat?hours=24", &token).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.deaths, 3);
        assert_eq!(body.deaths_inferred, 2, "the observed death must not count");
        assert!(
            body.deaths_inferred <= body.deaths,
            "a split can never exceed the total it describes"
        );
    }

    #[tokio::test]
    async fn location_current_scopes_by_authenticated_handle() {
        // Bob is paired and online; Alice has no events. The endpoint
        // must NOT leak Bob's location to Alice.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Bob",
            "planet_terrain_load",
            now - Duration::minutes(2),
            json!({"planet": "OOC_Stanton_2b_Daymar"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, _) = get_status_and_bytes(app, "/v1/me/location/current", &token).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "Alice must not see Bob's location"
        );
    }

    // -- Fix 1: current location ignores INVALID / shard-only readings --

    #[tokio::test]
    async fn location_current_skips_invalid_location_for_last_real_place() {
        // Newest event is a STALE INVALID_LOCATION_ID (older than the
        // 10-min in-transit window), older one is a real city (Orison).
        // A stale loading-screen reading must NOT stick as "in transit"
        // — it reverts to the last KNOWN place.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(20),
                json!({"location": "Stanton2_Orison"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(12),
                json!({"location": "INVALID_LOCATION_ID"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.location.city.as_deref(),
            Some("Orison"),
            "should surface the last real place, not the newer INVALID reading"
        );
        assert_eq!(body.location.planet.as_deref(), Some("Crusader"));
    }

    #[tokio::test]
    async fn location_current_skips_shard_only_join_pu_for_real_place() {
        // Newest event is a shard-only join_pu; older is a real city.
        // The headline place wins over the bare shard reading, and the
        // shard still rides along as context.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(8),
                json!({"location": "Stanton2_Orison"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "join_pu",
                now - Duration::minutes(1),
                json!({"shard": "pub_euw1b_test", "address": "1.2.3.4", "port": 64300, "location_id": "1"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.location.city.as_deref(),
            Some("Orison"),
            "the city must win over a newer shard-only join_pu"
        );
        assert_eq!(
            body.location.shard.as_deref(),
            Some("pub_euw1b_test"),
            "shard still carried as context"
        );
    }

    #[tokio::test]
    async fn location_current_returns_204_when_only_shard_and_no_real_place() {
        // Only a shard-only join_pu in the window: no real place (city or
        // planet) has ever been recorded. We return 204 so the UI hides the
        // pill rather than headline a raw shard id. The shard string is NOT
        // a suitable "current location" headline.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "join_pu",
            now - Duration::minutes(2),
            json!({"shard": "pub_euw1b_test", "address": "1.2.3.4", "port": 64300, "location_id": "1"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, _) = get_status_and_bytes(app, "/v1/me/location/current", &token).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "shard-only session must return 204 — no real place to headline"
        );
    }

    #[tokio::test]
    async fn location_current_shows_in_transit_for_fresh_invalid() {
        // A FRESH INVALID_LOCATION_ID reading (synced within the 10-min
        // window) means the user is actively in transit right now — show
        // "In transit" (a placeless 200 the UI renders as such), not 204.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "location_inventory_requested",
            now - Duration::minutes(2),
            json!({"location": "INVALID_LOCATION_ID"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.location.city.is_none(), "in transit = no city");
        assert!(body.location.planet.is_none(), "in transit = no planet");
    }

    #[tokio::test]
    async fn location_current_shows_in_transit_over_last_place_for_fresh_invalid() {
        // A fresh INVALID overrides the last confirmed place while the
        // user is moving — they're between stops right now.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(20),
                json!({"location": "Stanton2_Orison"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "location_inventory_requested",
                now - Duration::minutes(3),
                json!({"location": "INVALID_LOCATION_ID"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.location.city.is_none(),
            "fresh transit wins over last place"
        );
    }

    #[tokio::test]
    async fn location_current_returns_204_for_stale_invalid_only() {
        // A STALE INVALID (older than the window) with no real place ever
        // → 204. The transient transit reading is too old to surface, and
        // there's no confirmed place to fall back to.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "location_inventory_requested",
            now - Duration::minutes(15),
            json!({"location": "INVALID_LOCATION_ID"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, _) = get_status_and_bytes(app, "/v1/me/location/current", &token).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "stale INVALID + no place = 204"
        );
    }

    #[tokio::test]
    async fn location_current_shows_last_real_place_when_newest_is_join_pu() {
        // Core product behaviour: user is online NOW (join_pu @2m ago) but
        // their last real place was Orison @15h ago. The headline must be
        // Orison with its honest 15h-ago timestamp — NOT a 204, and NOT the
        // raw shard string. The shard still rides along as context.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "location_inventory_requested",
                now - Duration::hours(15),
                json!({"location": "Stanton2_Orison"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "join_pu",
                now - Duration::minutes(2),
                json!({"shard": "pub_euw1b_test", "address": "1.2.3.4", "port": 64300, "location_id": "1"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a 15h-old real place is never hidden — age is shown via last_seen_at"
        );
        assert_eq!(
            body.location.city.as_deref(),
            Some("Orison"),
            "city must be the last real place, not the shard"
        );
        assert_eq!(body.location.planet.as_deref(), Some("Crusader"));
        assert_eq!(
            body.location.shard.as_deref(),
            Some("pub_euw1b_test"),
            "shard from the recent join_pu still carried as context"
        );
        // last_seen_at must reflect the PLACE event age, not the join_pu age.
        let age_h = (now - body.location.last_seen_at).num_hours();
        assert!(
            age_h >= 14,
            "last_seen_at should be ~15h ago (the place event), got {age_h}h"
        );
    }

    #[tokio::test]
    async fn location_current_returns_204_when_only_shard_never_any_real_place() {
        // Brand-new user: only ever connected, never got a real place reading.
        // No place-type events at all — 204 is correct.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "join_pu",
            now - Duration::minutes(5),
            json!({"shard": "pub_euw1b_test", "address": "1.2.3.4", "port": 64300, "location_id": "1"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, _) = get_status_and_bytes(app, "/v1/me/location/current", &token).await;
        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "user with only shard events and no real place gets 204"
        );
    }

    // -- Fix 2: catalog hierarchy backfill --------------------------

    /// Aberdeen catalog entry: a moon of Hurston in Stanton. Token
    /// `aberdeen` (from the engine key) resolves via display-name, so
    /// the classifier yields `parent_body = Hurston`, `system = Stanton`.
    fn aberdeen_catalog() -> LocationCatalogCache {
        use starstats_core::location_catalog::{LocationCatalog, LocationCatalogEntry};
        use starstats_core::location_taxonomy::LocationTaxonomy;
        let entry = LocationCatalogEntry {
            slug: "aberdeen".into(),
            display_name: "Aberdeen".into(),
            class_name: "Aberdeen".into(),
            engine_tag: Some("Stanton1b".into()),
            system: Some("Stanton".into()),
            parent_body: Some("Hurston".into()),
            classification: Some("Moon".into()),
            taxonomy: LocationTaxonomy::default(),
        };
        LocationCatalogCache::from_catalog_for_test(LocationCatalog::from_entries(vec![entry]))
    }

    #[tokio::test]
    async fn location_current_backfills_planet_from_catalog_parent_body() {
        // `Stanton1_Aberdeen` is an unknown city for the naive parser:
        // planet=None, city=Aberdeen, system=Stanton. The catalog knows
        // Aberdeen sits on Hurston in Stanton, so the handler backfills
        // planet=Hurston rather than collapsing the breadcrumb.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "location_inventory_requested",
            now - Duration::minutes(2),
            json!({"location": "Stanton1_Aberdeen"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router_with_catalog(mq, Arc::new(verifier), aberdeen_catalog());
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<CurrentLocationResponse>(app, "/v1/me/location/current", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.location.planet.as_deref(),
            Some("Hurston"),
            "planet backfilled from the catalog parent_body"
        );
        assert_eq!(body.location.system.as_deref(), Some("Stanton"));
        assert_eq!(body.location.city.as_deref(), Some("Aberdeen"));
    }

    #[tokio::test]
    async fn location_trace_backfills_planet_from_catalog_parent_body() {
        // Same backfill must apply to the journey trace so the chain
        // breadcrumb carries the planet too — `collapse_to_trace` path.
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "Alice",
            "location_inventory_requested",
            now - Duration::minutes(30),
            json!({"location": "Stanton1_Aberdeen"}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router_with_catalog(mq, Arc::new(verifier), aberdeen_catalog());
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<TraceResponse>(app, "/v1/me/location/trace?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.entries.len(), 1);
        let entry = &body.entries[0];
        assert_eq!(
            entry.planet.as_deref(),
            Some("Hurston"),
            "trace entry planet backfilled from catalog parent_body"
        );
        assert_eq!(entry.system.as_deref(), Some("Stanton"));
        assert_eq!(entry.city.as_deref(), Some("Aberdeen"));
    }

    // ---- apply_catalog_hierarchy: friendly headline override -------

    /// Build a bare server-side `ResolvedLocation` with the given
    /// naive city/planet for the headline-override tests.
    fn resolved_with(city: Option<&str>, planet: Option<&str>) -> ResolvedLocation {
        ResolvedLocation {
            planet: planet.map(str::to_string),
            city: city.map(str::to_string),
            system: None,
            shard: None,
            last_seen_at: Utc::now(),
            source_event_type: "location_inventory_requested".into(),
            raw_planet_key: None,
            raw_city_key: None,
            entered_at: None,
            entered_at_is_lower_bound: false,
            resolved_location: None,
        }
    }

    /// Build a `LocationClassification` with the given display_name and
    /// source for the headline-override tests.
    fn classification_with(
        display_name: &str,
        source: ClassificationSource,
    ) -> starstats_core::location_classifier::LocationClassification {
        use starstats_core::location_taxonomy::LocationTier;
        starstats_core::location_classifier::LocationClassification {
            display_name: display_name.into(),
            slug: None,
            tier: LocationTier::SpaceStation,
            subtype: Some("gateway".into()),
            system: None,
            parent_body: None,
            placement: None,
            engine_tag: None,
            raw: "JP_Stanton_Pyro".into(),
            operator: None,
            faction: None,
            source,
        }
    }

    #[test]
    fn apply_catalog_hierarchy_overrides_city_with_friendly_display_name() {
        // Naive parse produced the humanized raw "Jp Stanton Pyro" for
        // an uncatalogued gateway key; the classifier confidently
        // identified it as "Pyro Gateway" (Synthetic). The headline
        // city must become the friendly name.
        let mut resolved = resolved_with(Some("Jp Stanton Pyro"), None);
        let classification = classification_with("Pyro Gateway", ClassificationSource::Synthetic);
        apply_catalog_hierarchy(&mut resolved, &classification);
        assert_eq!(resolved.city.as_deref(), Some("Pyro Gateway"));
    }

    #[test]
    fn apply_catalog_hierarchy_overrides_planet_when_no_city() {
        // No city present: the primary headline is the planet, so the
        // override lands there instead.
        let mut resolved = resolved_with(None, Some("Jp Stanton Pyro"));
        let classification = classification_with("Pyro Gateway", ClassificationSource::Synthetic);
        apply_catalog_hierarchy(&mut resolved, &classification);
        assert_eq!(resolved.planet.as_deref(), Some("Pyro Gateway"));
    }

    #[test]
    fn apply_catalog_hierarchy_does_not_override_on_fallback_source() {
        // Fallback display is just the humanized raw — no better than
        // the naive parse, so the city must be left untouched.
        let mut resolved = resolved_with(Some("Jp Stanton Pyro"), None);
        let classification = classification_with("Jp Stanton Pyro", ClassificationSource::Fallback);
        apply_catalog_hierarchy(&mut resolved, &classification);
        assert_eq!(resolved.city.as_deref(), Some("Jp Stanton Pyro"));
    }

    #[tokio::test]
    async fn stats_playtime_returns_total_secs_and_session_count() {
        // Two sessions anchored 2h before now so they fall inside the
        // 24-hour window that parse_stats_window computes:
        //   Session A: base+0min → base+20min  = 1200s
        //   Session B: base+51min → base+61min = 600s  (31-min gap → new session)
        // Total = 1800s, session_count = 2.
        let base = Utc::now() - Duration::hours(2);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "join_pu", Some(base)),
            evt(
                2,
                "Alice",
                "actor_death",
                Some(base + Duration::minutes(20)),
            ),
            evt(3, "Alice", "join_pu", Some(base + Duration::minutes(51))),
            evt(
                4,
                "Alice",
                "actor_death",
                Some(base + Duration::minutes(61)),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<PlaytimeStatsResponse>(app, "/v1/me/stats/playtime?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.total_playtime_secs, 1800,
            "session A (1200s) + session B (600s)"
        );
        assert_eq!(body.session_count, 2, "two distinct sessions");
        assert_eq!(body.hours, 24);
    }

    #[tokio::test]
    async fn stats_playtime_all_time_includes_sessions_outside_window() {
        // A single 30-min session 60 days ago — outside the default
        // 30-day window and any hours-bounded window. `all_time=true`
        // must still count it (since=None), and report hours=0 as the
        // all-time sentinel.
        let base = Utc::now() - Duration::days(60);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt(1, "Alice", "join_pu", Some(base)),
            evt(
                2,
                "Alice",
                "actor_death",
                Some(base + Duration::minutes(30)),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<PlaytimeStatsResponse>(app, "/v1/me/stats/playtime?all_time=true", &token)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.session_count, 1,
            "all_time must include the 60-day-old session"
        );
        assert_eq!(body.total_playtime_secs, 1800, "single 30-min session");
        assert_eq!(body.hours, 0, "hours=0 signals all-time");
    }

    // -- /v1/me/stats/locations ----------------------------------

    #[tokio::test]
    async fn stats_locations_returns_distinct_count_and_top_list() {
        // Three location events resolving to two distinct (planet, city) places:
        //   - "OOC_Stanton_2b_Daymar"  → planet="Daymar", system="Stanton"     (appears twice)
        //   - "OOC_Stanton_3a_Yela"    → planet="Yela",   system="Stanton"     (appears once)
        // Expected: unique_locations == 2, top_locations[0].value == Daymar key (count=2).
        let base = Utc::now() - Duration::hours(1);
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "Alice",
                "planet_terrain_load",
                base,
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
            evt_with_payload(
                2,
                "Alice",
                "planet_terrain_load",
                base + Duration::minutes(10),
                json!({"planet": "OOC_Stanton_3a_Yela"}),
            ),
            evt_with_payload(
                3,
                "Alice",
                "planet_terrain_load",
                base + Duration::minutes(20),
                json!({"planet": "OOC_Stanton_2b_Daymar"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "Alice");
        let (status, body) =
            get_json::<LocationsStatsResponse>(app, "/v1/me/stats/locations?hours=24", &token)
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.unique_locations, 2, "two distinct planets");
        assert_eq!(body.hours, 24);
        assert_eq!(body.top_locations.len(), 2);
        // Daymar appeared twice so it must be first
        assert_eq!(body.top_locations[0].count, 2);
        assert_eq!(body.top_locations[1].count, 1);
    }

    // -- /v1/me/stats/docking --------------------------------------

    #[test]
    fn classify_landing_area_buckets_real_strings() {
        let cases = [
            (
                "LandingArea_ShipElevator_HangarMediumTop",
                DockKind::Hangar,
                DockSize::Medium,
            ),
            (
                "LandingArea_ShipElevator_HangarSmallFront",
                DockKind::Hangar,
                DockSize::Small,
            ),
            (
                "LandingArea_ShipElevator_HangarLargeTop",
                DockKind::Hangar,
                DockSize::Large,
            ),
            (
                "LandingArea_ShipElevator_HangarXLTop",
                DockKind::Hangar,
                DockSize::Xl,
            ),
            (
                "[PROC]LandingArea_Pad_MedB-abc_1",
                DockKind::Pad,
                DockSize::Medium,
            ),
            (
                "[PROC]LandingArea_Pad_SmlB-xyz_2",
                DockKind::Pad,
                DockSize::Small,
            ),
            ("NewBab_Garage_3", DockKind::Other, DockSize::Unknown),
            ("", DockKind::Other, DockSize::Unknown),
        ];
        for (s, k, sz) in cases {
            assert_eq!(classify_landing_area(s), (k, sz), "for {s}");
        }
    }

    #[tokio::test]
    async fn stats_docking_folds_same_time_ship_stows_into_one_occurrence() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now,
                json!({"landing_area": "LandingArea_ShipElevator_HangarMediumTop"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "vehicle_stowed",
                now,
                json!({"landing_area": "LandingArea_ShipElevator_HangarMediumTop"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "vehicle_stowed",
                now,
                json!({"landing_area": "LandingArea_ShipElevator_HangarSmallFront"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "vehicle_stowed",
                now + Duration::seconds(121),
                json!({"landing_area": "[PROC]LandingArea_Pad_MedB-g_1"}),
            ),
            evt_with_payload(
                5,
                "bob",
                "vehicle_stowed",
                now,
                json!({"landing_area": "LandingArea_ShipElevator_HangarLargeTop"}),
            ), // other handle -> excluded
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<DockingResponse>(app, "/v1/me/stats/docking", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.total_stows, 2);
        assert_eq!(body.by_kind.hangar, 1);
        assert_eq!(body.by_kind.pad, 1);
        assert_eq!(body.by_size.medium, 2);
        assert_eq!(body.by_size.small, 0);
    }

    #[tokio::test]
    async fn stats_docking_interleaved_telemetry_does_not_split_one_stow_episode() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now,
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            evt_with_payload(2, "alice", "join_pu", now, json!({"shard":"1a"})),
            evt_with_payload(
                3,
                "alice",
                "vehicle_stowed",
                now + Duration::seconds(1),
                json!({"landing_area":"Pad_Small_01"}),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<DockingResponse>(app, "/v1/me/stats/docking", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.total_stows, 1);
        assert_eq!(body.by_kind.hangar, 1);
        assert_eq!(body.by_kind.pad, 0);
    }

    #[tokio::test]
    async fn stats_docking_raw_episode_and_matching_burst_count_once() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![
            evt_with_payload(
                1,
                "alice",
                "vehicle_stowed",
                now,
                json!({"landing_area":"Hangar_Large_01"}),
            ),
            evt_with_payload(
                2,
                "alice",
                "vehicle_stowed",
                now + Duration::milliseconds(1),
                json!({"landing_area":"Hangar_Medium_02"}),
            ),
            evt_with_payload(
                3,
                "alice",
                "vehicle_stowed",
                now + Duration::milliseconds(2),
                json!({"landing_area":"Hangar_Small_03"}),
            ),
            evt_with_payload(
                4,
                "alice",
                "burst_summary",
                now,
                json!({
                    "rule_id":"vehicle_stowed_burst",
                    "size":3,
                    "end_timestamp":(now + Duration::milliseconds(2)).to_rfc3339(),
                }),
            ),
        ]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<DockingResponse>(app, "/v1/me/stats/docking", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.total_stows, 1);
        assert_eq!(body.by_kind.hangar, 1);
        assert_eq!(body.by_kind.other, 0);
        assert_eq!(body.by_size.large, 1);
        assert_eq!(body.by_size.unknown, 0);
    }

    #[tokio::test]
    async fn stats_docking_counts_collapsed_vehicle_stow_burst_once() {
        let now = Utc::now();
        let mq = Arc::new(MemoryQuery::new(vec![evt_with_payload(
            1,
            "alice",
            "burst_summary",
            now,
            json!({"rule_id":"vehicle_stowed_burst","size":15}),
        )]));
        let (issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));
        let token = sign_token(&issuer, "alice");
        let (status, body) = get_json::<DockingResponse>(app, "/v1/me/stats/docking", &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.total_stows, 1,
            "the burst is one docking occurrence, not 0 or 15"
        );
        assert_eq!(body.by_kind.other, 1);
        assert_eq!(body.by_size.unknown, 1);
    }

    #[tokio::test]
    async fn stats_docking_requires_auth() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (_issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/stats/docking")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ---- commerce_recent -------------------------------------------------
    //
    // The bug these guard: `commerce_recent` used to pull the newest N
    // events of ANY type and filter commerce out in-process. Once a player
    // accumulated more than that cap in non-commerce events after their
    // last purchase, every transaction fell outside the pull and the
    // Economy tile went blank — at EVERY range, because `hours` only moves
    // the `since` lower bound while the cap truncates from the newest end.
    // Widening the range made it strictly worse, which is why "no data at
    // any range" was the symptom that identified this rather than an
    // empty window.
    //
    // `stats_biggest_trade` never had the bug: it queries by event_type.
    // These tests hold `commerce_recent` to the same contract.

    /// One shop purchase as the two rows the parser actually stores:
    /// the optimistic request and the server's confirmation.
    fn shop_pair(seq: i64, handle: &str, at: DateTime<Utc>, shop: &str) -> Vec<StoredQueryEvent> {
        let ts = at.to_rfc3339();
        vec![
            evt_with_payload(
                seq,
                handle,
                "shop_buy_request",
                at,
                json!({
                    "type": "shop_buy_request",
                    "timestamp": ts,
                    "shop_id": shop,
                    "item_class": "helmet",
                    "quantity": 1,
                    "raw": "SendShopBuyRequest",
                    "price": 1200,
                }),
            ),
            evt_with_payload(
                seq + 1,
                handle,
                "shop_flow_response",
                at + chrono::Duration::seconds(1),
                json!({
                    "type": "shop_flow_response",
                    "timestamp": (at + chrono::Duration::seconds(1)).to_rfc3339(),
                    "shop_id": shop,
                    "success": true,
                    "raw": "ShopFlowResponse",
                }),
            ),
        ]
    }

    /// Non-commerce filler — the ordinary background traffic of playing
    /// the game, which is what buried the commerce rows.
    fn filler(from_seq: i64, handle: &str, at: DateTime<Utc>, n: i64) -> Vec<StoredQueryEvent> {
        (0..n)
            .map(|i| {
                evt_with_payload(
                    from_seq + i,
                    handle,
                    "join_pu",
                    at + chrono::Duration::seconds(i),
                    json!({"type": "join_pu", "timestamp": (at + chrono::Duration::seconds(i)).to_rfc3339()}),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn commerce_recent_survives_a_wall_of_newer_noise() {
        // A purchase, then 1,500 ordinary events on top of it. The old
        // implementation pulled the newest 1,000 rows of any type, so the
        // purchase sat 500 rows below the cut and vanished entirely.
        let bought_at = Utc::now() - chrono::Duration::hours(6);
        let mut rows = shop_pair(1, "alice", bought_at, "shop-cru-l1");
        rows.extend(filler(
            100,
            "alice",
            bought_at + chrono::Duration::minutes(1),
            1_500,
        ));

        let mq = Arc::new(MemoryQuery::new(rows));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));

        let (status, body): (StatusCode, serde_json::Value) =
            get_json(app, "/v1/me/commerce/recent?limit=100", &token).await;
        assert_eq!(status, StatusCode::OK);

        let txs = body["transactions"].as_array().expect("transactions array");
        assert_eq!(
            txs.len(),
            1,
            "the purchase must survive newer unrelated traffic; got {body}"
        );
        assert_eq!(txs[0]["kind"], "shop");
        assert_eq!(txs[0]["status"], "confirmed");
        assert_eq!(txs[0]["shop_id"], "shop-cru-l1");
    }

    #[tokio::test]
    async fn commerce_recent_still_honours_the_hours_window() {
        // Type-filtering must not turn the range chips into decoration:
        // a purchase older than the window is still excluded.
        let recent = Utc::now() - chrono::Duration::hours(2);
        let ancient = Utc::now() - chrono::Duration::days(40);
        let mut rows = shop_pair(1, "alice", recent, "shop-recent");
        rows.extend(shop_pair(50, "alice", ancient, "shop-ancient"));

        let mq = Arc::new(MemoryQuery::new(rows));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));

        let (status, body): (StatusCode, serde_json::Value) =
            get_json(app, "/v1/me/commerce/recent?hours=24", &token).await;
        assert_eq!(status, StatusCode::OK);

        let txs = body["transactions"].as_array().expect("transactions array");
        assert_eq!(txs.len(), 1, "only the in-window purchase; got {body}");
        assert_eq!(txs[0]["shop_id"], "shop-recent");
    }

    #[tokio::test]
    async fn commerce_recent_excludes_other_handles() {
        // Scoping is the repo's job, but it is worth pinning: the per-type
        // fetch must not widen who a transaction belongs to.
        let at = Utc::now() - chrono::Duration::hours(1);
        let mut rows = shop_pair(1, "alice", at, "shop-alice");
        rows.extend(shop_pair(50, "bob", at, "shop-bob"));

        let mq = Arc::new(MemoryQuery::new(rows));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));

        let (status, body): (StatusCode, serde_json::Value) =
            get_json(app, "/v1/me/commerce/recent", &token).await;
        assert_eq!(status, StatusCode::OK);

        let txs = body["transactions"].as_array().expect("transactions array");
        assert_eq!(txs.len(), 1, "alice's purchase only; got {body}");
        assert_eq!(txs[0]["shop_id"], "shop-alice");
    }

    #[tokio::test]
    async fn commerce_recent_trims_to_limit_newest_first() {
        // Three purchases an hour apart; asking for two must return the
        // two newest, newest first.
        let base = Utc::now() - chrono::Duration::hours(5);
        let mut rows = shop_pair(1, "alice", base, "oldest");
        rows.extend(shop_pair(
            10,
            "alice",
            base + chrono::Duration::hours(1),
            "middle",
        ));
        rows.extend(shop_pair(
            20,
            "alice",
            base + chrono::Duration::hours(2),
            "newest",
        ));

        let mq = Arc::new(MemoryQuery::new(rows));
        let (issuer, verifier) = fresh_pair();
        let token = sign_token(&issuer, "alice");
        let app = router(mq, Arc::new(verifier));

        let (status, body): (StatusCode, serde_json::Value) =
            get_json(app, "/v1/me/commerce/recent?limit=2", &token).await;
        assert_eq!(status, StatusCode::OK);

        let txs = body["transactions"].as_array().expect("transactions array");
        assert_eq!(txs.len(), 2, "trimmed to the requested limit; got {body}");
        assert_eq!(txs[0]["shop_id"], "newest");
        assert_eq!(txs[1]["shop_id"], "middle");
    }

    #[tokio::test]
    async fn commerce_recent_requires_auth() {
        let mq = Arc::new(MemoryQuery::new(vec![]));
        let (_issuer, verifier) = fresh_pair();
        let app = router(mq, Arc::new(verifier));

        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/commerce/recent")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
