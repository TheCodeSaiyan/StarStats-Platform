//! Admin moderation surface for tray-promoted parser submissions.
//!
//! Endpoints (all gated by [`RequireModerator`] — admins inherit):
//!
//!   - `GET    /v1/admin/parser-submissions?status=&limit=&after=` -- list
//!   - `GET    /v1/admin/parser-submissions/{id}`                  -- detail
//!   - `PATCH  /v1/admin/parser-submissions/{id}`                  -- update
//!
//! The list endpoint sorts by *popularity* (`submitter_count DESC,
//! total_occurrence_count DESC, last_submitted_at DESC`) so a rule
//! author sees the highest-impact shapes first; pagination uses an
//! opaque `i64` id cursor that the client just echoes back. Status
//! is closed-vocabulary text — see [`SubmissionStatus`] — and an
//! invalid value 400s before the DB is touched.
//!
//! The PATCH handler is the moderator's "done" workflow: status change
//! (optional), reviewer notes (optional — `Some("")` clears), and
//! `rule_id` (optional). Each successful change writes one
//! `admin.parser_submission.update` row to the audit log mirroring
//! the posture used by `admin_submission_routes.rs`. Audit emission
//! is best-effort: a failing append never poisons the response.
//!
//! Store posture mirrors `entity_rollup.rs` and `share_reports.rs`:
//! a trait, a Postgres impl, and an in-memory impl under
//! `mod test_support` so the route-layer tests don't need a live DB.
//! The handlers consume `Arc<dyn AdminParserSubmissionsStore>` via an
//! Extension layer installed by `main.rs`.

use crate::admin_routes::RequireModerator;
use crate::audit::{AuditEntry, AuditLog};
use crate::submissions::{PromotedSubmission, SubmissionStore, COMMUNITY_USER_ID};
use async_trait::async_trait;
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use starstats_core::wire::{LogSource, ParserSubmission};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Default page size for the list endpoint. Mirrors the share-report
/// queue default — a moderator's eye-scan budget is ~50 rows.
pub const LIST_LIMIT_DEFAULT: i64 = 50;
/// Hard cap on the list endpoint regardless of caller. Mirrors the
/// share-report queue cap.
pub const LIST_LIMIT_MAX: i64 = 200;

/// Free-text cap on `reviewer_notes`. Generous because a rule
/// author's reasoning can run long, but bounded so a stray paste
/// can't inflate the JSONB row past TOAST-friendly territory.
pub const REVIEWER_NOTES_MAX_LEN: usize = 4_000;

/// Free-text cap on `rule_id`. Manifest rule ids are short — a few
/// dozen chars — but we allow some slack for future namespacing.
pub const RULE_ID_MAX_LEN: usize = 256;

/// Hard cap on the preview slice of the first raw example surfaced
/// in the list view. The detail view returns the full payload.
const RAW_EXAMPLE_PREVIEW_BYTES: usize = 200;

// -- Status enum (closed vocabulary, text-on-the-wire) ---------------

/// Moderation status for a parser submission row. Stored as TEXT in
/// `parser_submissions.status`. Mirrors the convention from
/// `share_reports::ShareReportStatus` — round-trips through
/// `as_str()`/`parse()` so adding a variant doesn't need a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionStatus {
    Pending,
    Drafting,
    RuleWritten,
    Dismissed,
}

impl SubmissionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SubmissionStatus::Pending => "pending",
            SubmissionStatus::Drafting => "drafting",
            SubmissionStatus::RuleWritten => "rule_written",
            SubmissionStatus::Dismissed => "dismissed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(SubmissionStatus::Pending),
            "drafting" => Some(SubmissionStatus::Drafting),
            "rule_written" => Some(SubmissionStatus::RuleWritten),
            "dismissed" => Some(SubmissionStatus::Dismissed),
            _ => None,
        }
    }
}

// -- Wire DTOs -------------------------------------------------------

/// One row in the moderator list. The `raw_example_preview` field is
/// the first entry of `payload.raw_examples` truncated to
/// [`RAW_EXAMPLE_PREVIEW_BYTES`] chars so a moderator can eyeball
/// each shape without paying for the full payload in the list view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminSubmissionSummary {
    pub id: i64,
    pub shape_hash: String,
    /// RFC3339 UTC.
    pub first_submitted_at: String,
    /// RFC3339 UTC.
    pub last_submitted_at: String,
    pub submitter_count: u32,
    pub total_occurrence_count: u32,
    /// One of `pending | drafting | rule_written | dismissed`.
    pub status: String,
    pub shell_tag: Option<String>,
    pub raw_example_preview: Option<String>,
    /// Query-time coarse grouping key (`core::coarse_shape_of` of a raw
    /// example, or the `shape_hash` when no raw example is present).
    /// Near-duplicate shapes share this; the triage UI groups on it.
    pub coarse_shape: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminSubmissionsListResponse {
    pub submissions: Vec<AdminSubmissionSummary>,
    /// Cursor for the next page; echo back as `after`. `None` when
    /// the response exhausted the matching set.
    pub next_after: Option<i64>,
}

/// OpenAPI-only mirror of `starstats_core::wire::ContextExample`.
/// Lets the admin detail response carry the structured payload
/// without dragging `utoipa` into the core wire crate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContextExampleSchema {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// OpenAPI-only mirror of `starstats_core::wire::ParserSubmission`.
/// Same posture as `parser_submissions::ParserSubmissionSchema` —
/// we re-state the shape so the spec carries the response body
/// while the underlying (de)serialization still flows through the
/// core type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParserSubmissionPayloadSchema {
    pub shape_hash: String,
    pub raw_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub partial_structured: std::collections::BTreeMap<String, String>,
    pub shell_tag: Option<String>,
    pub suggested_event_name: Option<String>,
    pub suggested_field_names: Option<std::collections::BTreeMap<String, String>>,
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_examples: Vec<ContextExampleSchema>,
    pub game_build: Option<String>,
    /// One of `live | ptu | eptu | hotfix | tech | other`.
    pub channel: String,
    pub occurrence_count: u32,
    pub client_anon_id: String,
    /// Whether the tray user opted into attribution for this submission.
    pub attributed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminSubmissionDetail {
    pub id: i64,
    pub shape_hash: String,
    pub first_submitted_at: String,
    pub last_submitted_at: String,
    pub submitter_count: u32,
    pub total_occurrence_count: u32,
    pub status: String,
    pub reviewer_notes: Option<String>,
    pub rule_id: Option<String>,
    /// Set once this shape has been promoted to the public community
    /// queue. `None` until published. Drives the "already published"
    /// badge on the admin detail page and mirrors the publish
    /// endpoint's idempotency link.
    pub community_submission_id: Option<Uuid>,
    /// Attribution identity captured from the tray at submit time.
    /// `Some` only when the tray user chose to attribute the submission;
    /// anonymous submissions store `None` here.
    pub submitter_handle: Option<String>,
    /// Full structured submission payload. Carries the moderator's
    /// raw examples + context lines + everything the tray promoted.
    #[schema(value_type = ParserSubmissionPayloadSchema)]
    pub payload: ParserSubmission,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AdminSubmissionPatch {
    /// `pending | drafting | rule_written | dismissed`. Unknown
    /// values 400. `None` leaves the column unchanged.
    pub status: Option<String>,
    /// `None` leaves notes unchanged; `Some("")` clears them.
    pub reviewer_notes: Option<String>,
    /// `None` leaves the rule_id unchanged; `Some("")` clears.
    pub rule_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// `pending` (default), `drafting`, `rule_written`, `dismissed`,
    /// or `all`. Unknown values 400.
    #[serde(default)]
    pub status: Option<String>,
    /// Page size. Clamped to [1, [`LIST_LIMIT_MAX`]].
    #[serde(default)]
    pub limit: Option<i64>,
    /// Opaque cursor — pass the previous response's `next_after`.
    #[serde(default)]
    pub after: Option<i64>,
}

// -- Store (trait + Postgres impl) -----------------------------------

/// Internal row shape — what the store hands back to the handler.
/// `payload_json` is `serde_json::Value` because the store layer
/// doesn't depend on the core wire types; the handler decodes into
/// `ParserSubmission` (with `#[serde(default)]` slack) on its way
/// out to the caller.
#[derive(Debug, Clone)]
pub struct StoredSubmissionRow {
    pub id: i64,
    pub shape_hash: String,
    /// Per-install identity. Read from the table for completeness but
    /// the admin surface never re-emits it; tray-side identity
    /// belongs in the `payload_json.client_anon_id` and is surfaced
    /// through the detail endpoint via the typed payload.
    #[allow(dead_code)]
    pub client_anon_id: String,
    pub first_submitted_at: DateTime<Utc>,
    pub last_submitted_at: DateTime<Utc>,
    pub submitter_count: i32,
    pub total_occurrence_count: i32,
    pub status: String,
    pub reviewer_notes: Option<String>,
    pub rule_id: Option<String>,
    pub payload_json: serde_json::Value,
    /// Attribution + promotion link (migration 0053). All nullable:
    /// `submitter_user_id`/`submitter_handle` are set only when the tray
    /// user opted into attribution; `community_submission_id` is set once
    /// the shape has been promoted to the public queue (drives the
    /// publish endpoint's idempotency check).
    pub submitter_user_id: Option<Uuid>,
    pub submitter_handle: Option<String>,
    pub community_submission_id: Option<Uuid>,
}

/// Subset patch applied by [`AdminParserSubmissionsStore::update`].
/// Each field uses `Option<Option<T>>` so the caller can distinguish
/// "leave unchanged" (`None`) from "clear" (`Some(None)`) — but at
/// the public store boundary we collapse this with a simpler
/// `Option<T>` + a separate `clear_*` flag for the two clearable
/// strings.
#[derive(Debug, Clone, Default)]
pub struct UpdatePatch {
    /// `None` => leave column alone. `Some(s)` => set to `s`.
    pub status: Option<SubmissionStatus>,
    /// `None` => leave alone. `Some(None)` => clear (NULL).
    /// `Some(Some(s))` => set to s.
    pub reviewer_notes: Option<Option<String>>,
    /// Same semantics as `reviewer_notes`.
    pub rule_id: Option<Option<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AdminStoreError {
    #[error("not found")]
    NotFound,
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("payload decode: {0}")]
    PayloadDecode(#[from] serde_json::Error),
}

#[async_trait]
pub trait AdminParserSubmissionsStore: Send + Sync + 'static {
    /// List rows filtered by `status`. `None` returns every row
    /// regardless of status (the `all` filter). Sorted by
    /// `(submitter_count DESC, total_occurrence_count DESC,
    /// last_submitted_at DESC, id ASC)` so the most-impactful shapes
    /// land first. `after` is an opaque `id` cursor — pass the
    /// previous response's `next_after` to fetch the next slice;
    /// behaviour is "skip past the row with that id in the materialised
    /// sort order". The store walks the full sorted set and drops
    /// every row at-or-before the cursor row; this is O(n_total) per
    /// page rather than O(page_size), which at the homelab volume the
    /// admin queue targets is fine, and keeps the cursor token a
    /// simple `i64` instead of a multi-column composite key.
    async fn list(
        &self,
        status: Option<SubmissionStatus>,
        limit: i64,
        after: Option<i64>,
    ) -> Result<Vec<StoredSubmissionRow>, AdminStoreError>;

    async fn find_by_id(&self, id: i64) -> Result<Option<StoredSubmissionRow>, AdminStoreError>;

    /// Apply a patch. Returns the post-update row. `NotFound` when
    /// the id doesn't exist. Bumps `updated_at` on every call where
    /// at least one column actually changes; a no-op patch
    /// (everything `None`) still returns the row but leaves
    /// `updated_at` alone.
    async fn update(
        &self,
        id: i64,
        patch: &UpdatePatch,
    ) -> Result<StoredSubmissionRow, AdminStoreError>;

    /// Link a parser-submission to the community `submissions` row it was
    /// promoted into (writes `community_submission_id`). The publish
    /// handler calls this exactly once per shape, on a row that had no
    /// link yet; a missing id is a `NotFound`.
    async fn link_community(&self, id: i64, community_id: Uuid) -> Result<(), AdminStoreError>;
}

pub struct PostgresAdminParserSubmissionsStore {
    pool: PgPool,
}

impl PostgresAdminParserSubmissionsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminParserSubmissionsStore for PostgresAdminParserSubmissionsStore {
    async fn list(
        &self,
        status: Option<SubmissionStatus>,
        limit: i64,
        after: Option<i64>,
    ) -> Result<Vec<StoredSubmissionRow>, AdminStoreError> {
        // Skip-style cursor: pull the full sorted bucket and trim
        // past the cursor row in app code. Single-column cursors on
        // a multi-column sort can't slice the result deterministically
        // (id alone doesn't know the popularity tuple), and at the
        // homelab volume this admin surface targets the O(n) read is
        // cheaper than maintaining a composite cursor token. The
        // bucket is bounded by status (pending typically a few hundred
        // rows max) plus the route-layer LIMIT-1 trick that asks for
        // limit+1 to compute `next_after`.
        let rows: Vec<(
            i64,
            String,
            String,
            DateTime<Utc>,
            DateTime<Utc>,
            i32,
            i32,
            String,
            Option<String>,
            Option<String>,
            serde_json::Value,
            Option<Uuid>,
            Option<String>,
            Option<Uuid>,
        )> = sqlx::query_as(
            r#"
            SELECT id,
                   shape_hash,
                   client_anon_id,
                   first_submitted_at,
                   last_submitted_at,
                   submitter_count,
                   total_occurrence_count,
                   status,
                   reviewer_notes,
                   rule_id,
                   payload_json,
                   submitter_user_id,
                   submitter_handle,
                   community_submission_id
              FROM parser_submissions
             WHERE ($1::text IS NULL OR status = $1)
          ORDER BY submitter_count DESC,
                   total_occurrence_count DESC,
                   last_submitted_at DESC,
                   id ASC
            "#,
        )
        .bind(status.map(|s| s.as_str()))
        .fetch_all(&self.pool)
        .await?;

        let mut mapped: Vec<StoredSubmissionRow> = rows
            .into_iter()
            .map(
                |(
                    id,
                    shape_hash,
                    client_anon_id,
                    first_submitted_at,
                    last_submitted_at,
                    submitter_count,
                    total_occurrence_count,
                    status,
                    reviewer_notes,
                    rule_id,
                    payload_json,
                    submitter_user_id,
                    submitter_handle,
                    community_submission_id,
                )| StoredSubmissionRow {
                    id,
                    shape_hash,
                    client_anon_id,
                    first_submitted_at,
                    last_submitted_at,
                    submitter_count,
                    total_occurrence_count,
                    status,
                    reviewer_notes,
                    rule_id,
                    payload_json,
                    submitter_user_id,
                    submitter_handle,
                    community_submission_id,
                },
            )
            .collect();

        if let Some(cursor_id) = after {
            // Drop every row up to and including the cursor row.
            // If the cursor is unknown (e.g. row was deleted), we
            // return the full bucket so the caller doesn't get stuck.
            if let Some(pos) = mapped.iter().position(|r| r.id == cursor_id) {
                mapped.drain(..=pos);
            }
        }
        mapped.truncate(limit as usize);
        Ok(mapped)
    }

    async fn find_by_id(&self, id: i64) -> Result<Option<StoredSubmissionRow>, AdminStoreError> {
        let row: Option<(
            i64,
            String,
            String,
            DateTime<Utc>,
            DateTime<Utc>,
            i32,
            i32,
            String,
            Option<String>,
            Option<String>,
            serde_json::Value,
            Option<Uuid>,
            Option<String>,
            Option<Uuid>,
        )> = sqlx::query_as(
            r#"
            SELECT id,
                   shape_hash,
                   client_anon_id,
                   first_submitted_at,
                   last_submitted_at,
                   submitter_count,
                   total_occurrence_count,
                   status,
                   reviewer_notes,
                   rule_id,
                   payload_json,
                   submitter_user_id,
                   submitter_handle,
                   community_submission_id
              FROM parser_submissions
             WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(
                id,
                shape_hash,
                client_anon_id,
                first_submitted_at,
                last_submitted_at,
                submitter_count,
                total_occurrence_count,
                status,
                reviewer_notes,
                rule_id,
                payload_json,
                submitter_user_id,
                submitter_handle,
                community_submission_id,
            )| StoredSubmissionRow {
                id,
                shape_hash,
                client_anon_id,
                first_submitted_at,
                last_submitted_at,
                submitter_count,
                total_occurrence_count,
                status,
                reviewer_notes,
                rule_id,
                payload_json,
                submitter_user_id,
                submitter_handle,
                community_submission_id,
            },
        ))
    }

    async fn update(
        &self,
        id: i64,
        patch: &UpdatePatch,
    ) -> Result<StoredSubmissionRow, AdminStoreError> {
        // One round-trip UPDATE with a per-column COALESCE. For the
        // two clearable strings we use a flag column: when the caller
        // wants to clear, we pass NULL + clear_flag=true and let
        // SQL pick NULL; otherwise NULL+clear_flag=false leaves the
        // existing value via COALESCE. `updated_at = NOW()` always
        // fires on a found row — a true no-op patch is filtered out
        // at the handler layer before getting here.
        let (notes_value, notes_clear) = match &patch.reviewer_notes {
            None => (None, false),
            Some(None) => (None, true),
            Some(Some(s)) => (Some(s.clone()), false),
        };
        let (rule_value, rule_clear) = match &patch.rule_id {
            None => (None, false),
            Some(None) => (None, true),
            Some(Some(s)) => (Some(s.clone()), false),
        };
        let status_str = patch.status.map(|s| s.as_str().to_string());

        let row: Option<(
            i64,
            String,
            String,
            DateTime<Utc>,
            DateTime<Utc>,
            i32,
            i32,
            String,
            Option<String>,
            Option<String>,
            serde_json::Value,
            Option<Uuid>,
            Option<String>,
            Option<Uuid>,
        )> = sqlx::query_as(
            r#"
            UPDATE parser_submissions
               SET status         = COALESCE($2, status),
                   reviewer_notes = CASE WHEN $4 THEN NULL ELSE COALESCE($3, reviewer_notes) END,
                   rule_id        = CASE WHEN $6 THEN NULL ELSE COALESCE($5, rule_id) END,
                   updated_at     = NOW()
             WHERE id = $1
         RETURNING id,
                   shape_hash,
                   client_anon_id,
                   first_submitted_at,
                   last_submitted_at,
                   submitter_count,
                   total_occurrence_count,
                   status,
                   reviewer_notes,
                   rule_id,
                   payload_json,
                   submitter_user_id,
                   submitter_handle,
                   community_submission_id
            "#,
        )
        .bind(id)
        .bind(status_str)
        .bind(notes_value)
        .bind(notes_clear)
        .bind(rule_value)
        .bind(rule_clear)
        .fetch_optional(&self.pool)
        .await?;

        let r = row.ok_or(AdminStoreError::NotFound)?;
        Ok(StoredSubmissionRow {
            id: r.0,
            shape_hash: r.1,
            client_anon_id: r.2,
            first_submitted_at: r.3,
            last_submitted_at: r.4,
            submitter_count: r.5,
            total_occurrence_count: r.6,
            status: r.7,
            reviewer_notes: r.8,
            rule_id: r.9,
            payload_json: r.10,
            submitter_user_id: r.11,
            submitter_handle: r.12,
            community_submission_id: r.13,
        })
    }

    async fn link_community(&self, id: i64, community_id: Uuid) -> Result<(), AdminStoreError> {
        let result =
            sqlx::query("UPDATE parser_submissions SET community_submission_id = $2 WHERE id = $1")
                .bind(id)
                .bind(community_id)
                .execute(&self.pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(AdminStoreError::NotFound);
        }
        Ok(())
    }
}

// -- Handler helpers -------------------------------------------------

/// Map a row + decoded payload to the wire summary. Pulls the
/// `shell_tag` and the first raw example off the JSON payload — we
/// don't denormalize either column at write-side because tray-side
/// payloads can refresh independently and we want a single source
/// of truth for both fields.
fn row_to_summary(row: &StoredSubmissionRow) -> AdminSubmissionSummary {
    let shell_tag = row
        .payload_json
        .get("shell_tag")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let first_raw: Option<&str> = row
        .payload_json
        .get("raw_examples")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str());
    let raw_example_preview = first_raw.map(|s| {
        if s.chars().count() > RAW_EXAMPLE_PREVIEW_BYTES {
            let cut: String = s.chars().take(RAW_EXAMPLE_PREVIEW_BYTES).collect();
            format!("{cut}…")
        } else {
            s.to_string()
        }
    });
    let coarse_shape = first_raw
        .map(starstats_core::coarse_shape_of)
        .unwrap_or_else(|| row.shape_hash.clone());
    AdminSubmissionSummary {
        id: row.id,
        shape_hash: row.shape_hash.clone(),
        first_submitted_at: row.first_submitted_at.to_rfc3339(),
        last_submitted_at: row.last_submitted_at.to_rfc3339(),
        submitter_count: row.submitter_count.max(0) as u32,
        total_occurrence_count: row.total_occurrence_count.max(0) as u32,
        status: row.status.clone(),
        shell_tag,
        raw_example_preview,
        coarse_shape,
    }
}

fn row_to_detail(row: StoredSubmissionRow) -> Result<AdminSubmissionDetail, AdminStoreError> {
    let payload: ParserSubmission = serde_json::from_value(row.payload_json.clone())?;
    Ok(AdminSubmissionDetail {
        id: row.id,
        shape_hash: row.shape_hash,
        first_submitted_at: row.first_submitted_at.to_rfc3339(),
        last_submitted_at: row.last_submitted_at.to_rfc3339(),
        submitter_count: row.submitter_count.max(0) as u32,
        total_occurrence_count: row.total_occurrence_count.max(0) as u32,
        status: row.status,
        reviewer_notes: row.reviewer_notes,
        rule_id: row.rule_id,
        community_submission_id: row.community_submission_id,
        submitter_handle: row.submitter_handle,
        payload,
    })
}

fn err(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({ "error": code }))).into_response()
}

fn err_500() -> Response {
    err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "admin_parser_submissions_failed",
    )
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/parser-submissions",
    tag = "admin",
    params(ListQuery),
    responses(
        (status = 200, description = "Page of submissions for the moderator queue", body = AdminSubmissionsListResponse),
        (status = 400, description = "Invalid status value"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_submissions(
    _: RequireModerator,
    Extension(store): Extension<Arc<dyn AdminParserSubmissionsStore>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let status_filter = match q.status.as_deref() {
        None | Some("") | Some("pending") => Some(SubmissionStatus::Pending),
        Some("drafting") => Some(SubmissionStatus::Drafting),
        Some("rule_written") => Some(SubmissionStatus::RuleWritten),
        Some("dismissed") => Some(SubmissionStatus::Dismissed),
        Some("all") => None,
        Some(_) => return err(StatusCode::BAD_REQUEST, "invalid_status"),
    };
    let limit = q
        .limit
        .unwrap_or(LIST_LIMIT_DEFAULT)
        .clamp(1, LIST_LIMIT_MAX);

    // Ask the store for `limit + 1` so we know whether a next page
    // exists without a separate COUNT. The N+1 trick is the same
    // posture the admin queue uses; documented at the call site.
    let mut rows = match store.list(status_filter, limit + 1, q.after).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions list failed");
            return err_500();
        }
    };
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_after = if has_more {
        rows.last().map(|r| r.id)
    } else {
        None
    };
    let submissions: Vec<AdminSubmissionSummary> = rows.iter().map(row_to_summary).collect();
    (
        StatusCode::OK,
        Json(AdminSubmissionsListResponse {
            submissions,
            next_after,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/parser-submissions/{id}",
    tag = "admin",
    params(("id" = i64, Path, description = "parser_submissions.id")),
    responses(
        (status = 200, description = "Full submission detail", body = AdminSubmissionDetail),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "Submission not found"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_submission(
    _: RequireModerator,
    Extension(store): Extension<Arc<dyn AdminParserSubmissionsStore>>,
    Path(id): Path<i64>,
) -> Response {
    let row = match store.find_by_id(id).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions find_by_id failed");
            return err_500();
        }
    };
    match row_to_detail(row) {
        Ok(detail) => (StatusCode::OK, Json(detail)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions payload decode failed");
            err_500()
        }
    }
}

#[utoipa::path(
    patch,
    path = "/v1/admin/parser-submissions/{id}",
    tag = "admin",
    params(("id" = i64, Path, description = "parser_submissions.id")),
    request_body = AdminSubmissionPatch,
    responses(
        (status = 200, description = "Submission updated", body = AdminSubmissionDetail),
        (status = 400, description = "Invalid status / oversized field"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "Submission not found"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn patch_submission(
    moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn AdminParserSubmissionsStore>>,
    Extension(submissions): Extension<Arc<dyn SubmissionStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<i64>,
    Json(body): Json<AdminSubmissionPatch>,
) -> Response {
    // Validate status first — if the caller passed something we
    // don't recognise we 400 before touching the DB.
    let status = match body.status.as_deref() {
        None => None,
        Some(s) => match SubmissionStatus::parse(s) {
            Some(v) => Some(v),
            None => return err(StatusCode::BAD_REQUEST, "invalid_status"),
        },
    };

    // Reviewer notes: `Some("")` clears, `Some(non-empty)` sets,
    // anything past the cap 400s. We trim whitespace-only-but-non-empty
    // strings as well — a moderator sending `"   "` clearly meant
    // "clear it" rather than "store five spaces".
    let reviewer_notes = match body.reviewer_notes {
        None => None,
        Some(ref s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Some(None)
            } else if trimmed.chars().count() > REVIEWER_NOTES_MAX_LEN {
                return err(StatusCode::BAD_REQUEST, "reviewer_notes_too_long");
            } else {
                Some(Some(trimmed.to_string()))
            }
        }
    };

    let rule_id = match body.rule_id {
        None => None,
        Some(ref s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Some(None)
            } else if trimmed.chars().count() > RULE_ID_MAX_LEN {
                return err(StatusCode::BAD_REQUEST, "rule_id_too_long");
            } else if !trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            {
                // Mirrors the parser-defs identifier charset used by
                // event_timeline + entity_rollup handle validation
                // (alphanumeric + `_-`), extended with `.` so manifest
                // rule ids like `combat.kill.v1` round-trip.
                return err(StatusCode::BAD_REQUEST, "invalid_rule_id");
            } else {
                Some(Some(trimmed.to_string()))
            }
        }
    };

    // True no-op: nothing to do. Return the existing row without
    // bumping `updated_at`. Mirrors the idempotent posture in
    // admin_submission_routes' transition handlers.
    if status.is_none() && reviewer_notes.is_none() && rule_id.is_none() {
        let row = match store.find_by_id(id).await {
            Ok(Some(r)) => r,
            Ok(None) => return err(StatusCode::NOT_FOUND, "not_found"),
            Err(e) => {
                tracing::error!(error = %e, "admin parser-submissions noop find failed");
                return err_500();
            }
        };
        return match row_to_detail(row) {
            Ok(detail) => (StatusCode::OK, Json(detail)).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "admin parser-submissions payload decode failed");
                err_500()
            }
        };
    }

    // Cross-field guard: transitioning to `rule_written` requires a
    // non-empty rule_id once the patch is applied. We resolve the
    // effective post-patch state against the current row so a PATCH
    // that only flips status (and relies on a previously-stamped
    // rule_id) still passes. Done before audit emission so a bad
    // request never bumps `updated_at` or writes an audit row.
    let needs_effective_check =
        matches!(status, Some(SubmissionStatus::RuleWritten)) || rule_id.is_some();
    if needs_effective_check {
        let current = match store.find_by_id(id).await {
            Ok(Some(r)) => r,
            Ok(None) => return err(StatusCode::NOT_FOUND, "not_found"),
            Err(e) => {
                tracing::error!(error = %e, "admin parser-submissions precheck find failed");
                return err_500();
            }
        };
        let effective_status: String = match status {
            Some(s) => s.as_str().to_string(),
            None => current.status.clone(),
        };
        let effective_rule_id: Option<String> = match &rule_id {
            Some(Some(v)) => Some(v.clone()),
            Some(None) => None,
            None => current.rule_id.clone(),
        };
        if effective_status == "rule_written" && effective_rule_id.is_none() {
            return err(StatusCode::BAD_REQUEST, "rule_written_requires_rule_id");
        }
    }

    let patch = UpdatePatch {
        status,
        reviewer_notes,
        rule_id,
    };

    let row = match store.update(id, &patch).await {
        Ok(r) => r,
        Err(AdminStoreError::NotFound) => return err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions update failed");
            return err_500();
        }
    };

    // Auto-ship the linked community submission: once a rule author flips
    // this shape to `rule_written`, the community-facing row (if this
    // shape was ever published via `publish_to_community`) should reflect
    // that the pattern has shipped. Best-effort like the audit emission
    // below -- a hiccup here must never fail the moderator's PATCH.
    if patch.status == Some(SubmissionStatus::RuleWritten) && row.community_submission_id.is_some()
    {
        if let Err(e) = submissions.mark_shipped_by_source(&row.shape_hash).await {
            tracing::warn!(
                error = %e,
                shape_hash = %row.shape_hash,
                "failed to auto-ship linked community submission"
            );
        }
    }

    // Best-effort audit emission. Mirrors the admin_submission_routes
    // posture — never poison the response on a chain hiccup. We
    // record the *applied* patch (resolved against the row) rather
    // than the raw request so a moderator reviewing the audit
    // sees exactly what changed.
    let payload = serde_json::json!({
        "submission_id": row.id,
        "shape_hash": row.shape_hash,
        "new_status": row.status,
        "new_reviewer_notes_set": patch.reviewer_notes.is_some(),
        "new_rule_id_set": patch.rule_id.is_some(),
    });
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(moderator.0.sub.clone()),
            actor_handle: Some(moderator.0.preferred_username.clone()),
            action: "admin.parser_submission.update".to_string(),
            payload,
        })
        .await
    {
        tracing::warn!(
            error = %e,
            "audit log append failed (admin.parser_submission.update)"
        );
    }

    match row_to_detail(row) {
        Ok(detail) => (StatusCode::OK, Json(detail)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions payload decode failed");
            err_500()
        }
    }
}

/// Request body for [`publish_to_community`]. The moderator supplies the
/// three author-facing fields the community submission needs; the sample
/// line, log source, and submitter attribution are all derived
/// server-side from the stored parser-submission row.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishCommunityRequest {
    pub proposed_label: String,
    pub description: String,
    pub pattern: String,
    /// When `true`, publish anonymously under the `community` system
    /// account even if the parser-submission carries an attributed tray
    /// user -- a moderator override for e.g. an abusive handle. Defaults
    /// to `false` (preserve attribution / existing behaviour).
    /// One-directional: this only ever routes TO the community account,
    /// so it can never de-anonymize an already-anonymous row.
    #[serde(default)]
    pub force_anonymous: bool,
}

/// Response for [`publish_to_community`]. `already_published` is `true`
/// when the parser-submission was already linked to a community row —
/// the call was a no-op and the status is 200 rather than 201.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PublishCommunityResponse {
    pub community_submission_id: Uuid,
    pub already_published: bool,
}

/// Canonical lowercase token for a [`LogSource`], matching its serde
/// representation (which is exactly what the stored payload carries).
/// `submissions.log_source` is a free-text column, so the promoted row
/// persists the same token the tray sent. Exhaustive on purpose: a new
/// `LogSource` variant fails to compile here until it's mapped.
fn log_source_str(source: LogSource) -> &'static str {
    match source {
        LogSource::Live => "live",
        LogSource::Ptu => "ptu",
        LogSource::Eptu => "eptu",
        LogSource::Hotfix => "hotfix",
        LogSource::Tech => "tech",
        LogSource::Other => "other",
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/parser-submissions/{id}/publish",
    tag = "admin",
    params(("id" = i64, Path, description = "parser_submissions.id")),
    request_body = PublishCommunityRequest,
    responses(
        (status = 201, description = "Promoted to a new community submission", body = PublishCommunityResponse),
        (status = 200, description = "Already published; existing row returned", body = PublishCommunityResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "Submission not found"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn publish_to_community(
    moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn AdminParserSubmissionsStore>>,
    Extension(submissions): Extension<Arc<dyn SubmissionStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<i64>,
    Json(req): Json<PublishCommunityRequest>,
) -> Response {
    // 1. Load the parser-submission (404 when unknown).
    let row = match store.find_by_id(id).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions publish find failed");
            return err_500();
        }
    };

    // 2. Idempotency (parser side): already promoted → 200 + existing id.
    if let Some(existing) = row.community_submission_id {
        return (
            StatusCode::OK,
            Json(PublishCommunityResponse {
                community_submission_id: existing,
                already_published: true,
            }),
        )
            .into_response();
    }

    // 3. Resolve the submitter. A moderator may force anonymity (e.g. an
    //    abusive handle) via `force_anonymous`; an un-attributed row is
    //    anonymous by nature. Either way the row is authored by the seeded
    //    `community` system account so `submissions.submitter_id` stays
    //    NOT NULL. Otherwise we credit the attributed tray user.
    //    `force_anonymous` only ever routes TO the community account, so
    //    de-anonymizing an attributed row is structurally impossible.
    let (submitter_id, submitter_handle) = match row.submitter_user_id {
        Some(uid) if !req.force_anonymous => (
            uid,
            row.submitter_handle
                .clone()
                .unwrap_or_else(|| "community".to_string()),
        ),
        _ => (COMMUNITY_USER_ID, "community".to_string()),
    };

    // 4. Derive the sample line + log source from the stored payload.
    let payload: ParserSubmission = match serde_json::from_value(row.payload_json.clone()) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions publish payload decode failed");
            return err_500();
        }
    };
    let sample_line = payload.raw_examples.first().cloned().unwrap_or_default();
    let log_source = log_source_str(payload.channel);

    // 5. Promote into the community queue. `create_promoted` is idempotent
    //    on `source_shape_hash`, so a race that double-publishes the same
    //    shape returns the existing row rather than duplicating it.
    let promoted = PromotedSubmission {
        submitter_id,
        submitter_handle: &submitter_handle,
        pattern: &req.pattern,
        proposed_label: &req.proposed_label,
        description: &req.description,
        sample_line: &sample_line,
        log_source,
        source_shape_hash: &row.shape_hash,
    };
    tracing::info!(
        parser_submission_id = id,
        submitter_handle = promoted.submitter_handle,
        source_shape_hash = promoted.source_shape_hash,
        "promoting parser submission to community queue"
    );
    let community = match submissions.create_promoted(promoted).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "admin parser-submissions create_promoted failed");
            return err_500();
        }
    };

    // 6. Link the parser-submission back to the community row.
    if let Err(e) = store.link_community(id, community.id).await {
        tracing::error!(error = %e, "admin parser-submissions link_community failed");
        return err_500();
    }

    // 7. Best-effort audit — a chain hiccup must never poison the publish.
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(moderator.0.sub.clone()),
            actor_handle: Some(moderator.0.preferred_username.clone()),
            action: "admin.parser_submission.published".to_string(),
            payload: serde_json::json!({
                "parser_submission_id": id,
                "community_submission_id": community.id,
                "shape_hash": row.shape_hash,
                "submitter_id": submitter_id,
            }),
        })
        .await
    {
        tracing::warn!(
            error = %e,
            "audit log append failed (admin.parser_submission.published)"
        );
    }

    // 8. Fresh publish → 201.
    (
        StatusCode::CREATED,
        Json(PublishCommunityResponse {
            community_submission_id: community.id,
            already_published: false,
        }),
    )
        .into_response()
}

/// Build the admin parser-submissions sub-router. Parameterless: the
/// store, the community submission store, the audit log, the auth
/// verifier, and the staff role store are all installed as Extension
/// layers on the outer router by `main.rs`.
pub fn router() -> Router {
    Router::new()
        .route("/v1/admin/parser-submissions", get(list_submissions))
        .route(
            "/v1/admin/parser-submissions/:id",
            get(get_submission).patch(patch_submission),
        )
        .route(
            "/v1/admin/parser-submissions/:id/publish",
            post(publish_to_community),
        )
}

// -- Test support (in-memory store) ----------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// In-memory `AdminParserSubmissionsStore` for route-layer tests.
    /// Behaviour mirrors the Postgres impl: same sort key, same
    /// `NotFound` semantics, same per-field patch model. We use an
    /// auto-incrementing counter for `id` so cursor semantics line
    /// up with what Postgres returns.
    pub struct MemoryAdminParserSubmissionsStore {
        inner: Mutex<Inner>,
    }

    struct Inner {
        rows: Vec<StoredSubmissionRow>,
        next_id: i64,
    }

    impl Default for MemoryAdminParserSubmissionsStore {
        fn default() -> Self {
            Self {
                inner: Mutex::new(Inner {
                    rows: Vec::new(),
                    next_id: 1,
                }),
            }
        }
    }

    impl MemoryAdminParserSubmissionsStore {
        /// Test helper: seed a row directly with a custom timestamp
        /// + counter set. Mirrors the `seed_submission` pattern used
        /// in `admin_submission_routes`'s tests. Returns the
        /// generated id.
        pub fn seed(
            &self,
            shape_hash: &str,
            submitter_count: i32,
            total_occurrence_count: i32,
            last_submitted_at: DateTime<Utc>,
            status: SubmissionStatus,
            payload_json: serde_json::Value,
        ) -> i64 {
            let mut g = self.inner.lock().expect("memstore poisoned");
            let id = g.next_id;
            g.next_id += 1;
            g.rows.push(StoredSubmissionRow {
                id,
                shape_hash: shape_hash.to_string(),
                client_anon_id: format!("anon_{id}"),
                first_submitted_at: last_submitted_at,
                last_submitted_at,
                submitter_count,
                total_occurrence_count,
                status: status.as_str().to_string(),
                reviewer_notes: None,
                rule_id: None,
                payload_json,
                submitter_user_id: None,
                submitter_handle: None,
                community_submission_id: None,
            });
            id
        }

        /// Seed a promotable pending row: carries a decodable payload and
        /// optional submitter attribution (`Some((user_id, handle))` for
        /// an attributed shape, `None` for an anonymous one). Returns the
        /// generated id. Used by the publish-endpoint tests.
        pub fn seed_promotable(
            &self,
            shape_hash: &str,
            payload_json: serde_json::Value,
            submitter: Option<(Uuid, String)>,
        ) -> i64 {
            let mut g = self.inner.lock().expect("memstore poisoned");
            let id = g.next_id;
            g.next_id += 1;
            let (submitter_user_id, submitter_handle) = match submitter {
                Some((uid, handle)) => (Some(uid), Some(handle)),
                None => (None, None),
            };
            let now = Utc::now();
            g.rows.push(StoredSubmissionRow {
                id,
                shape_hash: shape_hash.to_string(),
                client_anon_id: format!("anon_{id}"),
                first_submitted_at: now,
                last_submitted_at: now,
                submitter_count: 1,
                total_occurrence_count: 1,
                status: SubmissionStatus::Pending.as_str().to_string(),
                reviewer_notes: None,
                rule_id: None,
                payload_json,
                submitter_user_id,
                submitter_handle,
                community_submission_id: None,
            });
            id
        }
    }

    #[async_trait]
    impl AdminParserSubmissionsStore for MemoryAdminParserSubmissionsStore {
        async fn list(
            &self,
            status: Option<SubmissionStatus>,
            limit: i64,
            after: Option<i64>,
        ) -> Result<Vec<StoredSubmissionRow>, AdminStoreError> {
            let g = self.inner.lock().expect("memstore poisoned");
            let mut rows: Vec<StoredSubmissionRow> = g
                .rows
                .iter()
                .filter(|r| match status {
                    None => true,
                    Some(s) => r.status == s.as_str(),
                })
                .cloned()
                .collect();
            // Sort mirrors the Postgres ORDER BY: popularity DESC,
            // id ASC as the deterministic tiebreaker so the
            // skip-style cursor walk is stable.
            rows.sort_by(|a, b| {
                b.submitter_count
                    .cmp(&a.submitter_count)
                    .then_with(|| b.total_occurrence_count.cmp(&a.total_occurrence_count))
                    .then_with(|| b.last_submitted_at.cmp(&a.last_submitted_at))
                    .then_with(|| a.id.cmp(&b.id))
            });
            if let Some(cursor_id) = after {
                if let Some(pos) = rows.iter().position(|r| r.id == cursor_id) {
                    rows.drain(..=pos);
                }
            }
            rows.truncate(limit as usize);
            Ok(rows)
        }

        async fn find_by_id(
            &self,
            id: i64,
        ) -> Result<Option<StoredSubmissionRow>, AdminStoreError> {
            let g = self.inner.lock().expect("memstore poisoned");
            Ok(g.rows.iter().find(|r| r.id == id).cloned())
        }

        async fn update(
            &self,
            id: i64,
            patch: &UpdatePatch,
        ) -> Result<StoredSubmissionRow, AdminStoreError> {
            let mut g = self.inner.lock().expect("memstore poisoned");
            let row = g
                .rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or(AdminStoreError::NotFound)?;
            if let Some(s) = patch.status {
                row.status = s.as_str().to_string();
            }
            match &patch.reviewer_notes {
                None => {}
                Some(None) => row.reviewer_notes = None,
                Some(Some(s)) => row.reviewer_notes = Some(s.clone()),
            }
            match &patch.rule_id {
                None => {}
                Some(None) => row.rule_id = None,
                Some(Some(s)) => row.rule_id = Some(s.clone()),
            }
            Ok(row.clone())
        }

        async fn link_community(&self, id: i64, community_id: Uuid) -> Result<(), AdminStoreError> {
            let mut g = self.inner.lock().expect("memstore poisoned");
            let row = g
                .rows
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or(AdminStoreError::NotFound)?;
            row.community_submission_id = Some(community_id);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::audit::AuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::to_bytes;
    use axum::http::Request;
    use chrono::Duration as ChronoDuration;
    use serde_json::json;
    use test_support::MemoryAdminParserSubmissionsStore;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn build_app(
        store: Arc<MemoryAdminParserSubmissionsStore>,
        audit: Arc<MemoryAuditLog>,
        staff: Arc<MemoryStaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let store_dyn: Arc<dyn AdminParserSubmissionsStore> = store;
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        // The PATCH handler auto-ships a linked community submission on a
        // `rule_written` transition, so it needs a `SubmissionStore` in
        // scope like the publish handler does. Tests that don't care about
        // that side effect get a fresh, empty memory store here; tests
        // that DO care use `build_publish_app` below instead.
        let submissions_dyn: Arc<dyn SubmissionStore> =
            Arc::new(crate::submissions::test_support::MemorySubmissionStore::default());
        router()
            .layer(Extension(verifier))
            .layer(Extension(store_dyn))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_dyn))
            .layer(Extension(submissions_dyn))
    }

    async fn moderator_token(
        staff: &MemoryStaffRoleStore,
        issuer: &TokenIssuer,
        handle: &str,
    ) -> String {
        let user_id = Uuid::now_v7();
        staff
            .grant(user_id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign moderator token")
    }

    fn plain_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&Uuid::now_v7().to_string(), handle)
            .expect("sign plain token")
    }

    fn sample_payload(shape: &str, raw: &str, shell: Option<&str>) -> serde_json::Value {
        let mut p = json!({
            "shape_hash": shape,
            "raw_examples": [raw],
            "channel": "live",
            "occurrence_count": 1,
            "client_anon_id": "anon_x",
        });
        if let Some(s) = shell {
            p["shell_tag"] = json!(s);
        }
        p
    }

    /// Build a `StoredSubmissionRow` whose payload carries a single raw
    /// example, for exercising `row_to_summary` directly (no HTTP/store
    /// round-trip needed since it's a pure fn).
    fn stored_with_raw(shape_hash: &str, raw: &str) -> StoredSubmissionRow {
        let now = Utc::now();
        StoredSubmissionRow {
            id: 1,
            shape_hash: shape_hash.to_string(),
            client_anon_id: "anon_x".to_string(),
            first_submitted_at: now,
            last_submitted_at: now,
            submitter_count: 1,
            total_occurrence_count: 1,
            status: SubmissionStatus::Pending.as_str().to_string(),
            reviewer_notes: None,
            rule_id: None,
            payload_json: sample_payload(shape_hash, raw, None),
            submitter_user_id: None,
            submitter_handle: None,
            community_submission_id: None,
        }
    }

    /// Same as [`stored_with_raw`] but with an empty `raw_examples` list,
    /// exercising the `coarse_shape` fallback to `shape_hash`.
    fn stored_without_raw(shape_hash: &str) -> StoredSubmissionRow {
        let now = Utc::now();
        StoredSubmissionRow {
            id: 1,
            shape_hash: shape_hash.to_string(),
            client_anon_id: "anon_x".to_string(),
            first_submitted_at: now,
            last_submitted_at: now,
            submitter_count: 1,
            total_occurrence_count: 1,
            status: SubmissionStatus::Pending.as_str().to_string(),
            reviewer_notes: None,
            rule_id: None,
            payload_json: json!({
                "shape_hash": shape_hash,
                "raw_examples": [],
                "channel": "live",
                "occurrence_count": 1,
                "client_anon_id": "anon_x",
            }),
            submitter_user_id: None,
            submitter_handle: None,
            community_submission_id: None,
        }
    }

    #[test]
    fn summary_carries_coarse_shape_grouping_near_duplicates() {
        // Two rows whose raw_examples differ only in a class name must
        // yield the SAME coarse_shape; a row with no raw example falls
        // back to its shape_hash.
        let a = row_to_summary(&stored_with_raw(
            "sh_a",
            "Notice CEntity::Kill AEGS_Gladius [12345]",
        ));
        let b = row_to_summary(&stored_with_raw(
            "sh_b",
            "Notice CEntity::Kill RSI_Scorpius [67890]",
        ));
        assert_eq!(a.coarse_shape, b.coarse_shape);
        let none = row_to_summary(&stored_without_raw("sh_c"));
        assert_eq!(none.coarse_shape, "sh_c"); // fallback to shape_hash
    }

    fn auth_get(uri: &str, token: &str) -> Request<axum::body::Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn auth_patch(uri: &str, token: &str, body: serde_json::Value) -> Request<axum::body::Body> {
        Request::builder()
            .method("PATCH")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    async fn json_body<T: for<'de> Deserialize<'de>>(resp: Response) -> T {
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "decode {}: {} (body={})",
                std::any::type_name::<T>(),
                e,
                String::from_utf8_lossy(&bytes)
            )
        })
    }

    // -- Auth gating ------------------------------------------------

    #[tokio::test]
    async fn list_rejects_missing_bearer_token() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (_issuer, verifier) = fresh_pair();
        let app = build_app(store, audit, staff, Arc::new(verifier));

        let req = Request::builder()
            .method("GET")
            .uri("/v1/admin/parser-submissions")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_rejects_non_moderator_with_403() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(store, audit, staff, Arc::new(verifier));
        let tok = plain_token(&issuer, "rando");
        let resp = app
            .oneshot(auth_get("/v1/admin/parser-submissions", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // -- List behaviour --------------------------------------------

    #[tokio::test]
    async fn list_sorts_by_popularity_with_tiebreakers() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let now = Utc::now();
        // Three pending rows: A has the highest submitter_count, B
        // ties on submitters but wins on occurrence, C trails both.
        let _a = store.seed(
            "sh_a",
            5,
            10,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_a", "raw a", None),
        );
        let _b = store.seed(
            "sh_b",
            5,
            7,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_b", "raw b", None),
        );
        let _c = store.seed(
            "sh_c",
            2,
            99,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_c", "raw c", None),
        );
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get("/v1/admin/parser-submissions", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AdminSubmissionsListResponse = json_body(resp).await;
        let hashes: Vec<&str> = body
            .submissions
            .iter()
            .map(|s| s.shape_hash.as_str())
            .collect();
        assert_eq!(hashes, vec!["sh_a", "sh_b", "sh_c"]);
    }

    #[tokio::test]
    async fn list_status_filter_excludes_other_buckets() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let now = Utc::now();
        store.seed(
            "sh_p1",
            1,
            1,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_p1", "x", None),
        );
        store.seed(
            "sh_p2",
            1,
            1,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_p2", "x", None),
        );
        store.seed(
            "sh_d",
            1,
            1,
            now,
            SubmissionStatus::Dismissed,
            sample_payload("sh_d", "x", None),
        );
        store.seed(
            "sh_w",
            1,
            1,
            now,
            SubmissionStatus::RuleWritten,
            sample_payload("sh_w", "x", None),
        );

        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get(
                "/v1/admin/parser-submissions?status=pending",
                &tok,
            ))
            .await
            .unwrap();
        let body: AdminSubmissionsListResponse = json_body(resp).await;
        assert_eq!(body.submissions.len(), 2);
        for s in &body.submissions {
            assert_eq!(s.status, "pending");
        }
    }

    #[tokio::test]
    async fn list_status_all_returns_every_row() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let now = Utc::now();
        store.seed(
            "sh_p",
            1,
            1,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_p", "x", None),
        );
        store.seed(
            "sh_d",
            1,
            1,
            now,
            SubmissionStatus::Dismissed,
            sample_payload("sh_d", "x", None),
        );

        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get("/v1/admin/parser-submissions?status=all", &tok))
            .await
            .unwrap();
        let body: AdminSubmissionsListResponse = json_body(resp).await;
        assert_eq!(body.submissions.len(), 2);
    }

    #[tokio::test]
    async fn list_rejects_unknown_status_with_400() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get("/v1/admin/parser-submissions?status=banana", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_cursor_pagination_walks_full_set() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let now = Utc::now();
        // Four pending rows with descending popularity so the sort
        // is stable and the cursor walk is deterministic.
        for i in 0..4 {
            store.seed(
                &format!("sh_{i}"),
                (10 - i) as i32,
                1,
                now - ChronoDuration::minutes(i),
                SubmissionStatus::Pending,
                sample_payload(&format!("sh_{i}"), "x", None),
            );
        }
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;

        // Page 1: limit=2.
        let resp = app
            .clone()
            .oneshot(auth_get(
                "/v1/admin/parser-submissions?status=pending&limit=2",
                &tok,
            ))
            .await
            .unwrap();
        let page1: AdminSubmissionsListResponse = json_body(resp).await;
        assert_eq!(page1.submissions.len(), 2);
        assert!(page1.next_after.is_some(), "first page must paginate");

        // Page 2 using the cursor — we should see the remaining rows.
        let after = page1.next_after.unwrap();
        let resp = app
            .oneshot(auth_get(
                &format!("/v1/admin/parser-submissions?status=pending&limit=2&after={after}"),
                &tok,
            ))
            .await
            .unwrap();
        let page2: AdminSubmissionsListResponse = json_body(resp).await;
        assert_eq!(page2.submissions.len(), 2);
        assert!(
            page2.next_after.is_none(),
            "second page must exhaust the set"
        );
        // No overlap between pages.
        let p1_ids: Vec<i64> = page1.submissions.iter().map(|s| s.id).collect();
        let p2_ids: Vec<i64> = page2.submissions.iter().map(|s| s.id).collect();
        for id in &p2_ids {
            assert!(!p1_ids.contains(id), "page2 must not echo page1");
        }
    }

    #[tokio::test]
    async fn list_surfaces_shell_tag_and_raw_preview() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let now = Utc::now();
        store.seed(
            "sh_long",
            1,
            1,
            now,
            SubmissionStatus::Pending,
            sample_payload("sh_long", "the quick brown fox jumps", Some("ItemPort")),
        );
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get("/v1/admin/parser-submissions", &tok))
            .await
            .unwrap();
        let body: AdminSubmissionsListResponse = json_body(resp).await;
        let first = &body.submissions[0];
        assert_eq!(first.shell_tag.as_deref(), Some("ItemPort"));
        assert_eq!(
            first.raw_example_preview.as_deref(),
            Some("the quick brown fox jumps")
        );
    }

    // -- Detail ----------------------------------------------------

    #[tokio::test]
    async fn detail_returns_full_payload() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_detail",
            3,
            5,
            Utc::now(),
            SubmissionStatus::Pending,
            sample_payload("sh_detail", "raw 1", Some("Cargo")),
        );
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AdminSubmissionDetail = json_body(resp).await;
        assert_eq!(body.id, id);
        assert_eq!(body.shape_hash, "sh_detail");
        assert_eq!(body.payload.shape_hash, "sh_detail");
        assert_eq!(body.payload.shell_tag.as_deref(), Some("Cargo"));
        assert_eq!(body.payload.raw_examples, vec!["raw 1".to_string()]);
    }

    #[tokio::test]
    async fn detail_404_when_id_unknown() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_get("/v1/admin/parser-submissions/9999", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -- PATCH -----------------------------------------------------

    #[tokio::test]
    async fn patch_updates_status_notes_and_rule_id() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_x",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Pending,
            sample_payload("sh_x", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let body = json!({
            "status": "rule_written",
            "reviewer_notes": "shipped as combat.kill",
            "rule_id": "rule_combat_kill_v1",
        });
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                body,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let detail: AdminSubmissionDetail = json_body(resp).await;
        assert_eq!(detail.status, "rule_written");
        assert_eq!(
            detail.reviewer_notes.as_deref(),
            Some("shipped as combat.kill")
        );
        assert_eq!(detail.rule_id.as_deref(), Some("rule_combat_kill_v1"));

        // Audit row landed with the expected action + actor.
        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "admin.parser_submission.update");
        assert_eq!(entries[0].actor_handle.as_deref(), Some("mod"));
        assert_eq!(
            entries[0]
                .payload
                .get("new_status")
                .and_then(|v| v.as_str()),
            Some("rule_written")
        );
    }

    #[tokio::test]
    async fn patch_rejects_invalid_status_value() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_x",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Pending,
            sample_payload("sh_x", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "status": "approved" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // No DB mutation, no audit row.
        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert!(audit.snapshot().is_empty());
    }

    #[tokio::test]
    async fn patch_clears_reviewer_notes_when_set_to_empty_string() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_x",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Pending,
            sample_payload("sh_x", "raw", None),
        );
        // Seed an initial note via a first patch.
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;

        let r1 = app
            .clone()
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "reviewer_notes": "first pass" }),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        assert_eq!(
            store
                .find_by_id(id)
                .await
                .unwrap()
                .unwrap()
                .reviewer_notes
                .as_deref(),
            Some("first pass")
        );

        let r2 = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "reviewer_notes": "" }),
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);
        assert!(store
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap()
            .reviewer_notes
            .is_none());
    }

    #[tokio::test]
    async fn patch_404_when_id_unknown() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(store, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_patch(
                "/v1/admin/parser-submissions/9999",
                &tok,
                json!({ "status": "drafting" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn patch_done_workflow_rule_written_with_rule_id() {
        // The signature "rule-author done" workflow: flip to
        // rule_written and stamp the manifest rule id in one PATCH.
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_done",
            4,
            12,
            Utc::now(),
            SubmissionStatus::Drafting,
            sample_payload("sh_done", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let body = json!({
            "status": "rule_written",
            "rule_id": "scc_kill_v3",
        });
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                body,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "rule_written");
        assert_eq!(row.rule_id.as_deref(), Some("scc_kill_v3"));

        // One audit row.
        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "admin.parser_submission.update");
    }

    #[tokio::test]
    async fn patch_rejects_invalid_rule_id_charset() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_charset",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Pending,
            sample_payload("sh_charset", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "rule_id": "evil; DROP TABLE" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = json_body(resp).await;
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("invalid_rule_id")
        );

        // No DB mutation, no audit row.
        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert!(row.rule_id.is_none());
        assert!(audit.snapshot().is_empty());
    }

    #[tokio::test]
    async fn patch_rejects_rule_written_without_rule_id() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_norid",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Drafting,
            sample_payload("sh_norid", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "status": "rule_written" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = json_body(resp).await;
        assert_eq!(
            body.get("error").and_then(|v| v.as_str()),
            Some("rule_written_requires_rule_id")
        );

        // No DB mutation, no audit row.
        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "drafting");
        assert!(row.rule_id.is_none());
        assert!(audit.snapshot().is_empty());
    }

    #[tokio::test]
    async fn patch_accepts_rule_written_with_rule_id() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_ok",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Drafting,
            sample_payload("sh_ok", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({
                    "status": "rule_written",
                    "rule_id": "valid.rule_id-123",
                }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "rule_written");
        assert_eq!(row.rule_id.as_deref(), Some("valid.rule_id-123"));
    }

    #[tokio::test]
    async fn patch_preserves_existing_rule_id_when_transitioning_to_rule_written() {
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_prev",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Drafting,
            sample_payload("sh_prev", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;

        // First PATCH: stamp a rule_id while still in `drafting`.
        let r1 = app
            .clone()
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "rule_id": "earlier.id" }),
            ))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);

        // Second PATCH: transition to rule_written WITHOUT resending
        // the rule_id. The effective-state check must read the
        // existing rule_id from the row and allow the transition.
        let r2 = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({ "status": "rule_written" }),
            ))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);

        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "rule_written");
        assert_eq!(row.rule_id.as_deref(), Some("earlier.id"));
    }

    // -- Publish to community queue ---------------------------------

    use crate::submissions::test_support::MemorySubmissionStore;
    use crate::submissions::{SubmissionStore, COMMUNITY_USER_ID};

    /// Handles for a publish-endpoint test app: the seeded parser
    /// submission id, both stores (so tests can assert cross-store
    /// effects), and a pre-minted moderator bearer token.
    struct PublishStores {
        shape_id: i64,
        submissions: Arc<MemorySubmissionStore>,
        parser: Arc<MemoryAdminParserSubmissionsStore>,
        token: String,
    }

    /// Like [`build_app`], but also layers the `SubmissionStore` the
    /// publish handler needs to promote a shape into the community queue.
    fn build_publish_app(
        parser: Arc<MemoryAdminParserSubmissionsStore>,
        submissions: Arc<MemorySubmissionStore>,
        audit: Arc<MemoryAuditLog>,
        staff: Arc<MemoryStaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let parser_dyn: Arc<dyn AdminParserSubmissionsStore> = parser;
        let submissions_dyn: Arc<dyn SubmissionStore> = submissions;
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        router()
            .layer(Extension(verifier))
            .layer(Extension(parser_dyn))
            .layer(Extension(submissions_dyn))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_dyn))
    }

    fn auth_post(uri: &str, token: &str, body: serde_json::Value) -> Request<axum::body::Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    async fn test_app_with_pending_shape(
        shape: &str,
        attributed: Option<(Uuid, String)>,
    ) -> (Router, PublishStores) {
        let (issuer, verifier) = fresh_pair();
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let parser = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let submissions = Arc::new(MemorySubmissionStore::default());

        // Register the attributed user's handle so the promoted row's
        // users-join resolves in the memory mirror.
        if let Some((uid, ref handle)) = attributed {
            submissions.add_user(uid, handle);
        }

        let shape_id = parser.seed_promotable(
            shape,
            sample_payload(shape, "Notice CEntity::Kill AEGS_Gladius [12345]", None),
            attributed.clone(),
        );

        let token = moderator_token(&staff, &issuer, "mod").await;
        let app = build_publish_app(
            parser.clone(),
            submissions.clone(),
            audit,
            staff,
            Arc::new(verifier),
        );
        (
            app,
            PublishStores {
                shape_id,
                submissions,
                parser,
                token,
            },
        )
    }

    fn publish_req(stores: &PublishStores) -> Request<axum::body::Body> {
        auth_post(
            &format!("/v1/admin/parser-submissions/{}/publish", stores.shape_id),
            &stores.token,
            json!({ "proposed_label": "combat.kill", "description": "d", "pattern": "p" }),
        )
    }

    #[tokio::test]
    async fn publish_promotes_anonymous_shape_to_community() {
        let (app, stores) = test_app_with_pending_shape("sh_pub", None).await;
        let resp = app.clone().oneshot(publish_req(&stores)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: PublishCommunityResponse = json_body(resp).await;
        assert!(!body.already_published);

        let com = stores
            .submissions
            .find_by_id(body.community_submission_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(com.submission.submitter_id, COMMUNITY_USER_ID);

        // The parser-submission row is now linked back to the community row.
        let ps = stores
            .parser
            .find_by_id(stores.shape_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ps.community_submission_id,
            Some(body.community_submission_id)
        );
    }

    #[tokio::test]
    async fn publish_twice_returns_existing_row() {
        let (app, stores) = test_app_with_pending_shape("sh_dup", None).await;
        let first: PublishCommunityResponse =
            json_body(app.clone().oneshot(publish_req(&stores)).await.unwrap()).await;
        let resp = app.oneshot(publish_req(&stores)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK); // 200, not 201, on repeat
        let second: PublishCommunityResponse = json_body(resp).await;
        assert!(second.already_published);
        assert_eq!(
            first.community_submission_id,
            second.community_submission_id
        );
    }

    #[tokio::test]
    async fn publish_force_anonymous_overrides_attributed_submitter() {
        // Seed an ATTRIBUTED shape, then have the moderator force
        // anonymity: the community row must be owned by the community
        // system account, NOT the attributed tray user.
        let uid = Uuid::new_v4();
        let (app, stores) =
            test_app_with_pending_shape("sh_force", Some((uid, "Abusive".into()))).await;
        let req = auth_post(
            &format!("/v1/admin/parser-submissions/{}/publish", stores.shape_id),
            &stores.token,
            json!({
                "proposed_label": "combat.kill",
                "description": "d",
                "pattern": "p",
                "force_anonymous": true,
            }),
        );
        let body: PublishCommunityResponse = json_body(app.oneshot(req).await.unwrap()).await;
        let com = stores
            .submissions
            .find_by_id(body.community_submission_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(com.submission.submitter_id, COMMUNITY_USER_ID);
        assert_ne!(
            com.submission.submitter_id, uid,
            "force_anonymous must not credit the attributed user"
        );
    }

    #[tokio::test]
    async fn publish_without_force_anonymous_keeps_attribution() {
        // Regression: `force_anonymous` omitted defaults to false, so an
        // attributed shape still credits the tray user (back-compat).
        let uid = Uuid::new_v4();
        let (app, stores) =
            test_app_with_pending_shape("sh_keepattr", Some((uid, "Nova".into()))).await;
        // publish_req sends no `force_anonymous` key at all.
        let body: PublishCommunityResponse =
            json_body(app.oneshot(publish_req(&stores)).await.unwrap()).await;
        let com = stores
            .submissions
            .find_by_id(body.community_submission_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(com.submission.submitter_id, uid);
    }

    #[tokio::test]
    async fn publish_attributed_shape_credits_the_user() {
        let uid = Uuid::new_v4();
        let (app, stores) =
            test_app_with_pending_shape("sh_attr", Some((uid, "Nova".into()))).await;
        let body: PublishCommunityResponse =
            json_body(app.oneshot(publish_req(&stores)).await.unwrap()).await;
        let com = stores
            .submissions
            .find_by_id(body.community_submission_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(com.submission.submitter_id, uid);
    }

    // -- rule_written auto-ships the linked community row -----------

    #[tokio::test]
    async fn rule_written_ships_linked_community_row() {
        let (app, stores) = test_app_with_pending_shape("sh_ship", None).await;
        let pub_body: PublishCommunityResponse =
            json_body(app.clone().oneshot(publish_req(&stores)).await.unwrap()).await;

        // Transition the shape to rule_written via the existing PATCH.
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{}", stores.shape_id),
                &stores.token,
                json!({"status": "rule_written", "rule_id": "combat.kill_v3"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let com = stores
            .submissions
            .find_by_id(pub_body.community_submission_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            com.submission.status,
            crate::submissions::SubmissionStatus::Shipped
        );
    }

    #[tokio::test]
    async fn rule_written_without_publish_link_does_not_error() {
        // A shape that was never published to the community queue still
        // transitions to rule_written cleanly -- the best-effort ship
        // call is simply skipped since there's no community link.
        let store = Arc::new(MemoryAdminParserSubmissionsStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let id = store.seed(
            "sh_noline",
            1,
            1,
            Utc::now(),
            SubmissionStatus::Drafting,
            sample_payload("sh_noline", "raw", None),
        );
        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(auth_patch(
                &format!("/v1/admin/parser-submissions/{id}"),
                &tok,
                json!({"status": "rule_written", "rule_id": "combat.kill_v4"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let row = store.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.status, "rule_written");
    }
}
