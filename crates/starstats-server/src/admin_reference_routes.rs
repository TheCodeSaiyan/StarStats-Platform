//! Admin reference-data sub-router.
//!
//! Read-only inspection of the wiki-sync output (the daily cron that
//! fills `reference_registry`). Lets a moderator see which categories
//! are populated, when each was last refreshed, and browse a paged
//! list of entries within a category to spot-check the sync.
//!
//! Endpoints:
//!   GET /v1/admin/reference/categories
//!   GET /v1/admin/reference/:category
//!
//! Both gated on the moderator role. No write surface — refreshes
//! still happen via the in-tree cron; this is purely diagnostic.

use crate::admin_routes::RequireModerator;
use crate::reference_data::{ReferenceCategory, ReferenceEntry};
use crate::reference_store::{CategorySummary, ReferenceStore};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminReferenceCategoryDto {
    /// Lowercase category slug (`vehicle` / `weapon` / `item` / `location`)
    /// — matches the value used in the `category` URL segment.
    pub category: String,
    pub entry_count: i64,
    /// `MAX(updated_at)` across the rows for this category. Null when
    /// the cron hasn't populated this category yet.
    pub latest_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminReferenceCategoriesResponse {
    pub categories: Vec<AdminReferenceCategoryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminReferenceEntryDto {
    pub class_name: String,
    pub display_name: String,
    /// Free-form JSON object holding per-category extras (manufacturer,
    /// role, size, parent system…). Schema-on-read — the cron writes
    /// whatever the wiki returns and this passes it through verbatim.
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminReferenceEntriesResponse {
    pub category: String,
    pub entries: Vec<AdminReferenceEntryDto>,
    /// Total rows in the category — same number the summary endpoint
    /// returns. Surfaced here so the UI can paginate without a second
    /// call.
    pub total: usize,
    /// Substring filter that was applied (after lowercase normalize),
    /// or null when none. Echoed back to make the URL self-describing
    /// in the admin UI.
    pub q: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AdminReferenceEntriesParams {
    /// Optional case-insensitive substring filter over class_name +
    /// display_name. Applied in-memory after the store returns the
    /// full category — fine because category sizes top out around
    /// ~20k (items) and the admin tool isn't a hot path.
    #[serde(default)]
    pub q: Option<String>,
    /// Page size; defaults to 100, capped at 500. Larger than the
    /// 50/200 used elsewhere because entries are pure metadata —
    /// no SpiceDB fan-out, no JOINs — so wider pages are cheap.
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

const ENTRIES_PAGE_DEFAULT: usize = 100;
const ENTRIES_PAGE_MAX: usize = 500;

fn err_response(status: StatusCode, error: &str) -> Response {
    (status, Json(serde_json::json!({ "error": error }))).into_response()
}

fn entry_to_dto(e: ReferenceEntry) -> AdminReferenceEntryDto {
    AdminReferenceEntryDto {
        class_name: e.class_name,
        display_name: e.display_name,
        metadata: e.metadata,
    }
}

fn summary_to_dto(s: CategorySummary) -> AdminReferenceCategoryDto {
    AdminReferenceCategoryDto {
        category: s.category.as_str().to_string(),
        entry_count: s.entry_count,
        latest_updated_at: s.latest_updated_at,
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/reference/categories",
    tag = "admin",
    responses(
        (status = 200, description = "Per-category summary of reference_registry", body = AdminReferenceCategoriesResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_reference_categories<R: ReferenceStore>(
    _: RequireModerator,
    State(refs): State<Arc<R>>,
) -> Response {
    match refs.category_summaries().await {
        Ok(rows) => (
            StatusCode::OK,
            Json(AdminReferenceCategoriesResponse {
                categories: rows.into_iter().map(summary_to_dto).collect(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "category_summaries failed");
            err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/reference/{category}",
    tag = "admin",
    params(
        ("category" = String, Path, description = "vehicle | weapon | item | location"),
        AdminReferenceEntriesParams,
    ),
    responses(
        (status = 200, description = "Paged entry list within a category", body = AdminReferenceEntriesResponse),
        (status = 400, description = "Unknown category"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_reference_entries<R: ReferenceStore>(
    _: RequireModerator,
    State(refs): State<Arc<R>>,
    Path(category): Path<String>,
    Query(params): Query<AdminReferenceEntriesParams>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return err_response(StatusCode::BAD_REQUEST, "unknown_category");
    };

    let entries = match refs.list_category(cat).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, category = %category, "list_category failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    // Lowercase substring filter — applied in-memory because the
    // store API is full-list-per-category. `q` is trimmed so empty
    // queries don't filter everything out.
    let needle = params
        .q
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let filtered: Vec<ReferenceEntry> = match &needle {
        Some(n) => entries
            .into_iter()
            .filter(|e| {
                e.class_name.to_lowercase().contains(n) || e.display_name.to_lowercase().contains(n)
            })
            .collect(),
        None => entries,
    };

    let total = filtered.len();
    let limit = params
        .limit
        .unwrap_or(ENTRIES_PAGE_DEFAULT)
        .clamp(1, ENTRIES_PAGE_MAX);
    let offset = params.offset.unwrap_or(0);

    let page: Vec<AdminReferenceEntryDto> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(entry_to_dto)
        .collect();

    (
        StatusCode::OK,
        Json(AdminReferenceEntriesResponse {
            category: cat.as_str().to_string(),
            entries: page,
            total,
            q: needle,
        }),
    )
        .into_response()
}

/// Outcome of a manual reference sync request.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReferenceSyncResponse {
    /// `true` when a sync was started, `false` when one was already
    /// running and this request was a no-op.
    pub started: bool,
    pub detail: &'static str,
}

/// Trigger a reference-data sync from the upstream community API.
///
/// Replaces a 24-hour polling loop. The upstream data is reshaped and
/// RETAINED in our own database, so a daily pull was re-fetching data we
/// already own; the registry is authoritative between syncs and only
/// moves when someone asks it to.
///
/// This is also the ONLY path that applies a sync-time reshape — the
/// commodity-variant collapse, for instance, only takes effect when a
/// reconcile runs. Without a trigger, such changes are dead code.
///
/// Returns immediately: the worker owns the fetch/reconcile/cache-prime
/// sequence and a full four-category pull is far too slow to hold a
/// request open. Progress is in the server logs.
#[utoipa::path(
    post,
    path = "/v1/admin/reference/sync",
    tag = "admin",
    responses(
        (status = 202, description = "Sync started", body = ReferenceSyncResponse),
        (status = 409, description = "A sync is already running", body = ReferenceSyncResponse),
        (status = 403, description = "Not a moderator"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn trigger_reference_sync(
    _moderator: RequireModerator,
    State(tx): State<mpsc::Sender<()>>,
) -> Response {
    // `try_send`, not `send`: a queued sync makes a second one
    // redundant, and awaiting would hold the request open behind a
    // multi-minute four-category pull.
    match tx.try_send(()) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ReferenceSyncResponse {
                started: true,
                detail: "reference sync started; progress is in the server logs",
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::CONFLICT,
            Json(ReferenceSyncResponse {
                started: false,
                detail: "a reference sync is already queued or running",
            }),
        )
            .into_response(),
    }
}

/// Sub-router for the manual sync trigger. Separate from [`router`]
/// because it carries a different state type (the trigger channel
/// rather than the store).
pub fn sync_router(tx: mpsc::Sender<()>) -> Router {
    Router::new()
        .route("/v1/admin/reference/sync", post(trigger_reference_sync))
        .with_state(tx)
}

pub fn router<R: ReferenceStore>(refs: Arc<R>) -> Router {
    Router::new()
        .route(
            "/v1/admin/reference/categories",
            get(list_reference_categories::<R>),
        )
        .route(
            "/v1/admin/reference/:category",
            get(list_reference_entries::<R>),
        )
        .with_state(refs)
}

#[cfg(test)]
mod sync_trigger_tests {
    use super::*;

    /// A queued sync makes a second request redundant. Awaiting instead
    /// of `try_send` would hold the request open behind a multi-minute
    /// four-category pull; queueing duplicates would re-fetch the whole
    /// upstream for no benefit.
    #[tokio::test]
    async fn a_second_trigger_is_refused_while_one_is_queued() {
        let (tx, _rx) = mpsc::channel::<()>(1);

        // First fills the single slot.
        assert!(tx.try_send(()).is_ok());
        // Second must be refused, not queued.
        assert!(
            tx.try_send(()).is_err(),
            "capacity 1 must refuse a duplicate rather than queue it"
        );
    }

    #[tokio::test]
    async fn the_slot_frees_once_the_worker_takes_the_trigger() {
        // Otherwise one sync would permanently wedge the endpoint at 409.
        let (tx, mut rx) = mpsc::channel::<()>(1);
        assert!(tx.try_send(()).is_ok());
        assert!(tx.try_send(()).is_err());

        rx.recv().await.expect("worker takes the trigger");
        assert!(
            tx.try_send(()).is_ok(),
            "a completed sync must leave the endpoint triggerable again"
        );
    }

    #[tokio::test]
    async fn the_worker_loop_exits_when_every_sender_is_dropped() {
        // `recv()` returning None is shutdown, not a reason to spin.
        let (tx, mut rx) = mpsc::channel::<()>(1);
        drop(tx);
        assert!(rx.recv().await.is_none());
    }
}
