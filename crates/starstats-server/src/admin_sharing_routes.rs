//! Admin sharing-overview HTTP handlers.
//!
//! Two read-only endpoints surfaced on the `/admin/sharing` page,
//! both gated on the moderator role (admins inherit via
//! [`RequireModerator`]):
//!
//!   - `GET /v1/admin/sharing/overview`        — headline counters +
//!     time-windowed totals + top-20 granters.
//!   - `GET /v1/admin/sharing/scope-histogram` — distribution of
//!     `share_metadata.scope->>'kind'` across active shares plus a
//!     per-tab usage breakdown for `kind = 'tabs'` rows.
//!
//! The overview replaces a frontend proxy that used the recent
//! audit-log window — that proxy could see at most N events, so the
//! "active shares" card lied as soon as activity exceeded N. These
//! endpoints aggregate the real `share_metadata` table so the cards
//! match ground truth, with `audit_log` only consulted for the
//! genuinely time-windowed counters (30-day grants / revokes / views).

use crate::admin_routes::RequireModerator;
use crate::audit::{AuditEntry, AuditFilters, AuditLog, AuditQuery};
use crate::share_metadata::ShareMetadataStore;
use crate::share_reports::{
    ShareReport, ShareReportError, ShareReportStatus, ShareReportStore, RESOLUTION_NOTE_MAX_LEN,
};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// -- DTOs ------------------------------------------------------------

/// One row of the "top granters" leaderboard. Mirrors
/// [`crate::share_metadata::GranterCount`] but typed for the wire
/// (counts narrowed to u64 since they're always non-negative).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopGranter {
    /// Owner handle as stored on the underlying row (case-preserved
    /// display copy). Suitable for direct rendering in the UI.
    pub handle: String,
    /// Number of `share_metadata` rows owned by this handle that are
    /// still active (no expiry, or expiry in the future).
    pub active_share_count: u64,
}

/// Response body for `GET /v1/admin/sharing/overview`. The
/// `active_shares_*` counters come from a snapshot of `share_metadata`;
/// the `total_*_30d` counters come from a `WHERE occurred_at >
/// NOW() - INTERVAL '30 days'` filter on `audit_log`. Both shapes are
/// flattened into one DTO because the admin UI renders them as a
/// single card row — splitting the response into two would force the
/// page to make two round-trips for one render.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminSharingOverview {
    /// COUNT(*) from `share_metadata` WHERE `expires_at IS NULL OR
    /// expires_at > NOW()`. The headline "live shares" number.
    pub active_shares_total: u64,
    /// User-to-user (outbound) active shares. `share_metadata` only
    /// stores user-to-user shares today; org-direction grants live
    /// in SpiceDB without a side-table row, so this matches `_total`
    /// until the org table lands.
    pub active_shares_outbound: u64,
    /// Subset of `_total` where the share has an explicit expiry in
    /// the future (i.e. excludes legacy "no expiry" rows).
    pub active_shares_with_expiry: u64,
    /// Top owner handles by active-share count, sorted DESC, capped
    /// at 20. Empty when the table is empty.
    pub top_granters: Vec<TopGranter>,
    /// `share.granted` audit rows in the last 30 days. Time-windowed
    /// rather than absolute — the audit log doesn't keep "current
    /// state".
    pub total_grants_30d: u64,
    /// `share.revoked` audit rows in the last 30 days.
    pub total_revocations_30d: u64,
    /// `share.viewed` audit rows in the last 30 days.
    pub total_views_30d: u64,
}

/// Response body for `GET /v1/admin/sharing/scope-histogram`. NULL
/// scope rows fold into `full` (the legacy default preserved by
/// migration 0025). `tab_usage` is per-tab counts for `kind = 'tabs'`
/// rows; map keys are stable (BTreeMap) so the wire payload is
/// reproducible across requests with the same data.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ScopeHistogram {
    pub full: u64,
    pub timeline: u64,
    pub aggregates: u64,
    pub tabs: u64,
    /// Per-tab usage on `kind = 'tabs'` rows. Always emitted (empty
    /// `{}` when no tabs-scope rows exist) so the UI can render the
    /// card without a presence check.
    pub tab_usage: BTreeMap<String, u64>,
}

// -- Handlers --------------------------------------------------------

/// Cap on `top_granters`. Twenty is large enough to cover an entire
/// power-user cohort while keeping the JSON payload trivially small.
const TOP_GRANTERS_LIMIT: i64 = 20;

/// GET /v1/admin/sharing/overview — replaces the audit-window proxy
/// that the admin UI used before this endpoint existed. Gated on
/// moderator; admins inherit.
#[utoipa::path(
    get,
    path = "/v1/admin/sharing/overview",
    tag = "admin",
    responses(
        (status = 200, description = "Sharing overview snapshot", body = AdminSharingOverview),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_overview(
    _: RequireModerator,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
    Extension(audit_query): Extension<Arc<dyn AuditQuery>>,
) -> Response {
    // Active-share counters off the `share_metadata` table.
    let counts = match meta.active_share_counts().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "share_metadata active_share_counts failed");
            return overview_500();
        }
    };
    let top = match meta.top_active_granters(TOP_GRANTERS_LIMIT).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "share_metadata top_active_granters failed");
            return overview_500();
        }
    };

    // 30-day audit-log counters. We reuse `AuditQuery::list` with the
    // `since` filter plus a tight `limit`; the page size is just an
    // upper bound on what we'll read, since we only consume the
    // returned length. 500 is the store's max page size — exceeding
    // it silently clamps, which would understate the counter. For the
    // homelab data volume this is fine; a follow-up wave can add a
    // dedicated COUNT(*) when audit grows past the cap.
    let since = Utc::now() - Duration::days(30);
    let grants_30d = match count_audit_action(&*audit_query, "share.granted", since).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "audit count (share.granted) failed");
            return overview_500();
        }
    };
    let revokes_30d = match count_audit_action(&*audit_query, "share.revoked", since).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "audit count (share.revoked) failed");
            return overview_500();
        }
    };
    let views_30d = match count_audit_action(&*audit_query, "share.viewed", since).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "audit count (share.viewed) failed");
            return overview_500();
        }
    };

    let body = AdminSharingOverview {
        active_shares_total: counts.total.max(0) as u64,
        // share_metadata only stores user→user (outbound) rows today;
        // see DTO doc for the org caveat.
        active_shares_outbound: counts.total.max(0) as u64,
        active_shares_with_expiry: counts.with_expiry.max(0) as u64,
        top_granters: top
            .into_iter()
            .map(|g| TopGranter {
                handle: g.handle,
                active_share_count: g.active_share_count.max(0) as u64,
            })
            .collect(),
        total_grants_30d: grants_30d,
        total_revocations_30d: revokes_30d,
        total_views_30d: views_30d,
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// GET /v1/admin/sharing/scope-histogram — distribution of scope
/// kinds across active shares. Gated on moderator.
#[utoipa::path(
    get,
    path = "/v1/admin/sharing/scope-histogram",
    tag = "admin",
    responses(
        (status = 200, description = "Scope-kind distribution across active shares", body = ScopeHistogram),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_scope_histogram(
    _: RequireModerator,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
) -> Response {
    let h = match meta.scope_histogram_active().await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "share_metadata scope_histogram_active failed");
            return overview_500();
        }
    };
    let body = ScopeHistogram {
        full: h.full.max(0) as u64,
        timeline: h.timeline.max(0) as u64,
        aggregates: h.aggregates.max(0) as u64,
        tabs: h.tabs.max(0) as u64,
        tab_usage: h
            .tab_usage
            .into_iter()
            .map(|(k, v)| (k, v.max(0) as u64))
            .collect(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

// -- /v1/admin/sharing/reports ---------------------------------------

/// Wire shape for one report row. Mirrors `share_reports::ShareReport`
/// but flattens the enums to their wire strings so the OpenAPI surface
/// doesn't drag the Rust-side `#[serde(rename_all)]` machinery into
/// the spec.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ShareReportRowDto {
    pub id: Uuid,
    pub reporter_handle: String,
    pub owner_handle: String,
    pub recipient_handle: String,
    /// One of `abuse | spam | data_misuse | other`.
    pub reason: String,
    pub details: Option<String>,
    /// One of `open | dismissed | share_revoked | user_suspended`.
    pub status: String,
    pub created_at: chrono::DateTime<Utc>,
    pub resolved_at: Option<chrono::DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolution_note: Option<String>,
}

impl From<ShareReport> for ShareReportRowDto {
    fn from(r: ShareReport) -> Self {
        Self {
            id: r.id,
            reporter_handle: r.reporter_handle,
            owner_handle: r.owner_handle,
            recipient_handle: r.recipient_handle,
            reason: r.reason.as_str().to_string(),
            details: r.details,
            status: r.status.as_str().to_string(),
            created_at: r.created_at,
            resolved_at: r.resolved_at,
            resolved_by: r.resolved_by,
            resolution_note: r.resolution_note,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ShareReportListResponse {
    pub items: Vec<ShareReportRowDto>,
}

/// Query parameters for `GET /v1/admin/sharing/reports`. Defaults
/// mirror the moderator's "what landed today" mental model: status
/// `open`, most recent first, 50 rows.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ReportsListQuery {
    /// One of `open | dismissed | share_revoked | user_suspended | all`.
    /// Defaults to `open`. Unknown values 400.
    #[serde(default)]
    pub status: Option<String>,
    /// Page size. 1..=200; defaults to 50.
    #[serde(default)]
    pub limit: Option<i64>,
    /// Page offset. Defaults to 0.
    #[serde(default)]
    pub offset: Option<i64>,
}

/// GET `/v1/admin/sharing/reports` — moderator queue page driver.
/// Gated on moderator (admins inherit).
#[utoipa::path(
    get,
    path = "/v1/admin/sharing/reports",
    tag = "admin",
    params(ReportsListQuery),
    responses(
        (status = 200, description = "Reports page", body = ShareReportListResponse),
        (status = 400, description = "Unknown status value"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_reports(
    _: RequireModerator,
    Extension(reports): Extension<Arc<dyn ShareReportStore>>,
    Query(q): Query<ReportsListQuery>,
) -> Response {
    let status_filter: Option<ShareReportStatus> = match q.status.as_deref() {
        None | Some("") | Some("open") => Some(ShareReportStatus::Open),
        Some("dismissed") => Some(ShareReportStatus::Dismissed),
        Some("share_revoked") => Some(ShareReportStatus::ShareRevoked),
        Some("user_suspended") => Some(ShareReportStatus::UserSuspended),
        Some("all") => None,
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid_status"})),
            )
                .into_response();
        }
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    match reports.list(status_filter, limit, offset).await {
        Ok(rows) => {
            let body = ShareReportListResponse {
                items: rows.into_iter().map(Into::into).collect(),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "share_reports.list failed");
            overview_500()
        }
    }
}

/// Body for `POST /v1/admin/sharing/reports/:id/resolve`. `outcome`
/// must be a resolution variant (not `open`) — open is the starting
/// state, not a destination.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ResolveReportRequest {
    /// One of `dismissed | share_revoked | user_suspended`.
    pub outcome: String,
    /// Optional moderator note. Capped at `RESOLUTION_NOTE_MAX_LEN`.
    pub note: Option<String>,
}

/// Revoke every outbound grant the owner holds.
///
/// Best-effort per grant: one stuck relationship must not abort the
/// loop and leave the remaining shares live.
async fn revoke_owner_shares(client: &crate::spicedb::SpicedbClient, owner: &str) {
    match client.list_share_with_user(owner).await {
        Ok(recipients) => {
            for r in recipients {
                if let Err(e) = client.delete_share_with_user(owner, &r).await {
                    tracing::error!(error = %e, %owner, recipient = %r, "suspend: share revoke failed");
                }
            }
        }
        Err(e) => tracing::error!(error = %e, %owner, "suspend: listing shares failed"),
    }
    if let Err(e) = client.delete_public_view(owner).await {
        tracing::error!(error = %e, %owner, "suspend: delete_public_view failed");
    }
}

/// POST `/v1/admin/sharing/reports/{id}/resolve` — moderator triage
/// action. Idempotency: a second resolve on the same row returns 409
/// (`already_resolved`) so callers can distinguish "your action
/// landed" from "someone got there first".
#[utoipa::path(
    post,
    path = "/v1/admin/sharing/reports/{id}/resolve",
    tag = "admin",
    request_body = ResolveReportRequest,
    params(("id" = Uuid, Path, description = "share_reports.id of the report being resolved")),
    responses(
        (status = 200, description = "Report resolved", body = ShareReportRowDto),
        (status = 400, description = "Invalid outcome / note length"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "No such report"),
        (status = 409, description = "Report already resolved"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn resolve_report(
    RequireModerator(user): RequireModerator,
    Extension(reports): Extension<Arc<dyn ShareReportStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(restrictions): Extension<
        Arc<dyn crate::account_restrictions::AccountRestrictionStore>,
    >,
    Extension(spicedb): Extension<Arc<Option<crate::spicedb::SpicedbClient>>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ResolveReportRequest>,
) -> Response {
    let outcome = match ShareReportStatus::parse(&body.outcome) {
        Some(s) if s.is_resolution() => s,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid_outcome"})),
            )
                .into_response();
        }
    };
    let note = match body.note.as_deref().map(str::trim) {
        Some("") => None,
        Some(s) if s.chars().count() > RESOLUTION_NOTE_MAX_LEN => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "note_too_long"})),
            )
                .into_response();
        }
        Some(s) => Some(s.to_string()),
        None => None,
    };

    let moderator_handle = &user.preferred_username;
    match reports
        .resolve(id, moderator_handle, outcome, note.as_deref())
        .await
    {
        Ok(row) => {
            // Perform the enforcement the outcome NAMES.
            //
            // Until this slice, resolving as `user_suspended` wrote the
            // status and stopped there, while the confirmation dialog
            // told the moderator the owner's shares had been revoked
            // and that they could not create new ones. The status had
            // zero read-sites; nothing enforced it. Recording a
            // decision is not taking one.
            if row.status == ShareReportStatus::UserSuspended {
                let restriction = crate::account_restrictions::Restriction {
                    ingest_blocked: true,
                    sharing_blocked: true,
                    public_profile_blocked: true,
                    submissions_blocked: true,
                    reason: note
                        .clone()
                        .unwrap_or_else(|| format!("Suspended after share report {}", row.id)),
                    restricted_by: moderator_handle.clone(),
                    restricted_at: Utc::now(),
                    // Suspensions from the report queue do not
                    // auto-expire; a moderator lifts them explicitly.
                    expires_at: None,
                };
                match restrictions
                    .upsert_by_handle(&row.owner_handle, &restriction)
                    .await
                {
                    Ok(Some(target_id)) => {
                        if let Some(client) = spicedb.as_ref() {
                            revoke_owner_shares(client, &row.owner_handle).await;
                        } else {
                            tracing::error!(
                                owner = %row.owner_handle,
                                "report suspension applied but SpiceDB unavailable;                                  existing shares NOT revoked"
                            );
                        }
                        tracing::info!(
                            owner = %row.owner_handle, %target_id,
                            "share report resolved as user_suspended; account restricted"
                        );
                    }
                    Ok(None) => {
                        // The report names a handle with no account.
                        // Say so rather than reporting a suspension
                        // that could not have happened.
                        tracing::error!(
                            owner = %row.owner_handle,
                            "cannot suspend: no user matches the reported owner handle"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, owner = %row.owner_handle, "report suspension: restriction upsert failed");
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({"error": "suspension_failed"})),
                        )
                            .into_response();
                    }
                }
            } else if row.status == ShareReportStatus::ShareRevoked {
                // Revoke the ONE grant this report is about, not the
                // account. Conflating the two would let a moderator
                // nuke an account while believing they removed a share.
                match spicedb.as_ref() {
                    Some(client) => {
                        if let Err(e) = client
                            .delete_share_with_user(&row.owner_handle, &row.recipient_handle)
                            .await
                        {
                            tracing::error!(error = %e, owner = %row.owner_handle, recipient = %row.recipient_handle, "report share revoke failed");
                        }
                    }
                    None => tracing::error!(
                        owner = %row.owner_handle,
                        "report resolved as share_revoked but SpiceDB unavailable; share NOT revoked"
                    ),
                }
            }

            // Best-effort audit emission. Same posture as the rest of
            // the sharing surface — never poison the response.
            if let Err(e) = audit
                .append(AuditEntry {
                    actor_sub: Some(user.sub.clone()),
                    actor_handle: Some(moderator_handle.clone()),
                    action: "share.report_resolved".to_string(),
                    payload: serde_json::json!({
                        "report_id": row.id,
                        "outcome": row.status.as_str(),
                        "owner_handle": row.owner_handle,
                        "recipient_handle": row.recipient_handle,
                    }),
                })
                .await
            {
                tracing::warn!(error = %e, "audit log append failed (share.report_resolved)");
            }
            (StatusCode::OK, Json(ShareReportRowDto::from(row))).into_response()
        }
        Err(ShareReportError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        )
            .into_response(),
        Err(ShareReportError::AlreadyResolved) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "already_resolved"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "share_reports.resolve failed");
            overview_500()
        }
    }
}

// -- /v1/admin/sharing/by-user/:handle -------------------------------
//
// Audit v2.1 §C admin sub-tab — per-user sharing context. One
// round-trip returns everything a moderator needs to triage a
// specific user's sharing footprint: outbound + inbound shares
// from `share_metadata`, plus the open + recent reports involving
// them (as reporter OR as the share's owner).

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserShareEdge {
    /// The OTHER party on the share. For an outbound row, this is
    /// the recipient; for an inbound row, this is the owner. Saves
    /// the client from having to track which end of the relation
    /// they're looking at.
    pub counterparty_handle: String,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub note: Option<String>,
    /// Stringified scope kind (`full | timeline | aggregates | tabs`).
    /// `None` when the row has a NULL scope (legacy "full" default).
    pub scope_kind: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserSharingContext {
    /// The handle that was looked up (case-preserved from the path
    /// after Postgres lower() matching).
    pub handle: String,
    /// Shares THIS user owns — outbound, by recipient handle.
    pub outbound_shares: Vec<UserShareEdge>,
    /// Shares OTHERS own where THIS user is the recipient — inbound.
    pub inbound_shares: Vec<UserShareEdge>,
    /// Reports this user has filed (any status). Most recent first,
    /// capped at 50.
    pub reports_filed: Vec<ShareReportRowDto>,
    /// Reports filed AGAINST shares this user owns. Most recent
    /// first, capped at 50.
    pub reports_against: Vec<ShareReportRowDto>,
}

const PER_USER_REPORT_CAP: usize = 50;

#[utoipa::path(
    get,
    path = "/v1/admin/sharing/by-user/{handle}",
    tag = "admin",
    params(("handle" = String, Path, description = "RSI handle (case-insensitive)")),
    responses(
        (status = 200, description = "Per-user sharing context", body = UserSharingContext),
        (status = 400, description = "Empty handle"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_user_sharing_context(
    _: RequireModerator,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
    Extension(reports): Extension<Arc<dyn ShareReportStore>>,
    Path(handle): Path<String>,
) -> Response {
    let needle = handle.trim();
    if needle.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_handle"})),
        )
            .into_response();
    }

    let outbound = match meta.list_by_owner(needle).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "list_by_owner failed");
            return overview_500();
        }
    };
    let inbound = match meta.list_by_recipient(needle).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "list_by_recipient failed");
            return overview_500();
        }
    };

    // For reports we walk both populations and filter in memory —
    // the store's `list` doesn't take a handle filter, but at
    // homelab volume the 500-row page is a comfortable upper bound.
    // A future wave can add dedicated by-subject methods when volume
    // warrants the extra index surface.
    let all_reports = match reports.list(None, 500, 0).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "share_reports.list failed");
            return overview_500();
        }
    };
    let needle_lc = needle.to_ascii_lowercase();
    let reports_filed: Vec<ShareReportRowDto> = all_reports
        .iter()
        .filter(|r| r.reporter_handle.to_ascii_lowercase() == needle_lc)
        .take(PER_USER_REPORT_CAP)
        .cloned()
        .map(Into::into)
        .collect();
    let reports_against: Vec<ShareReportRowDto> = all_reports
        .into_iter()
        .filter(|r| r.owner_handle.to_ascii_lowercase() == needle_lc)
        .take(PER_USER_REPORT_CAP)
        .map(Into::into)
        .collect();

    let body = UserSharingContext {
        handle: handle.clone(),
        outbound_shares: outbound
            .into_iter()
            .map(|m| UserShareEdge {
                counterparty_handle: m.recipient_handle,
                expires_at: m.expires_at,
                note: m.note,
                scope_kind: m
                    .scope
                    .as_ref()
                    .and_then(|v| v.get("kind"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                created_at: m.created_at,
            })
            .collect(),
        inbound_shares: inbound
            .into_iter()
            .map(|m| UserShareEdge {
                counterparty_handle: m.owner_handle,
                expires_at: m.expires_at,
                note: m.note,
                scope_kind: m
                    .scope
                    .as_ref()
                    .and_then(|v| v.get("kind"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                created_at: m.created_at,
            })
            .collect(),
        reports_filed,
        reports_against,
    };
    (StatusCode::OK, Json(body)).into_response()
}

// -- /v1/admin/sharing/by-org/:slug ----------------------------------
//
// Audit v2.1 §C admin sub-tab — per-org sharing context. The shape
// mirrors the per-user version but the "outbound/inbound shares"
// section can't surface row-level edges directly: shares are
// owner→recipient at the user level, not the org level. Instead we
// aggregate PER MEMBER (count of outbound + inbound), giving the
// moderator a footprint scan + a drilldown path to the per-user view
// for full detail. Reports section walks the same all-reports
// population as the per-user handler and filters by the member set.
//
// SpiceDB is the authority for org membership — Postgres has no
// member table. A missing SpiceDB sidecar surfaces as 503
// `spicedb_unavailable`, same as elsewhere.

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgMemberSharingSlice {
    /// The member's claimed RSI handle (SpiceDB user object_id).
    pub member_handle: String,
    /// `owner`, `admin`, or `member` — the role tuple from
    /// `list_org_members`.
    pub member_role: String,
    /// Count of shares this member OWNS (outbound).
    pub outbound_count: u64,
    /// Count of shares this member is a recipient of (inbound).
    pub inbound_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgSharingContext {
    /// The slug that was looked up (case-preserved from the path).
    pub slug: String,
    /// One row per org member, with aggregate share counts so the
    /// moderator can see who's actively sharing without 1+N drill-ins.
    /// Empty list when the org has no SpiceDB members.
    pub members: Vec<OrgMemberSharingSlice>,
    /// Reports filed by ANY member of this org (any status). Most
    /// recent first, capped at `PER_ORG_REPORT_CAP`.
    pub reports_filed_by_members: Vec<ShareReportRowDto>,
    /// Reports filed AGAINST shares owned by any member of this org.
    /// Most recent first, capped at `PER_ORG_REPORT_CAP`.
    pub reports_against_members: Vec<ShareReportRowDto>,
}

const PER_ORG_REPORT_CAP: usize = 50;

#[utoipa::path(
    get,
    path = "/v1/admin/sharing/by-org/{slug}",
    tag = "admin",
    params(("slug" = String, Path, description = "Org slug (case-insensitive)")),
    responses(
        (status = 200, description = "Per-org sharing context", body = OrgSharingContext),
        (status = 400, description = "Empty slug"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
        (status = 503, description = "SpiceDB not configured", body = serde_json::Value),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_org_sharing_context(
    _: RequireModerator,
    Extension(spicedb): Extension<Arc<Option<crate::spicedb::SpicedbClient>>>,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
    Extension(reports): Extension<Arc<dyn ShareReportStore>>,
    Path(slug): Path<String>,
) -> Response {
    let needle = slug.trim();
    if needle.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_slug"})),
        )
            .into_response();
    }

    // Org membership lives in SpiceDB. Without it we can't enumerate
    // members, so the only honest response is 503 — same posture the
    // sharing routes use when SpiceDB is unreachable.
    let Some(client) = spicedb.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "spicedb_unavailable"})),
        )
            .into_response();
    };

    let members = match client.list_org_members(needle).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "list_org_members failed");
            return overview_500();
        }
    };

    // Per-member aggregation: O(members × 2) ShareMetadataStore round
    // trips. At homelab volume the trade-off favours simplicity over
    // a custom batch query — orgs are small (10s of members), the
    // store is a local Postgres, and this endpoint is admin-only.
    let mut member_slices: Vec<OrgMemberSharingSlice> = Vec::with_capacity(members.len());
    for (handle, role) in &members {
        let outbound_count = match meta.list_by_owner(handle).await {
            Ok(rows) => rows.len() as u64,
            Err(e) => {
                tracing::error!(error = %e, handle = %handle, "per-org list_by_owner failed");
                return overview_500();
            }
        };
        let inbound_count = match meta.list_by_recipient(handle).await {
            Ok(rows) => rows.len() as u64,
            Err(e) => {
                tracing::error!(error = %e, handle = %handle, "per-org list_by_recipient failed");
                return overview_500();
            }
        };
        member_slices.push(OrgMemberSharingSlice {
            member_handle: handle.clone(),
            member_role: role.clone(),
            outbound_count,
            inbound_count,
        });
    }

    // Reports: same posture as the per-user handler — walk the
    // recent-500 population and filter in memory.
    let all_reports = match reports.list(None, 500, 0).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "per-org share_reports.list failed");
            return overview_500();
        }
    };
    use std::collections::HashSet;
    let member_set: HashSet<String> = members
        .iter()
        .map(|(h, _)| h.to_ascii_lowercase())
        .collect();
    let reports_filed_by_members: Vec<ShareReportRowDto> = all_reports
        .iter()
        .filter(|r| member_set.contains(&r.reporter_handle.to_ascii_lowercase()))
        .take(PER_ORG_REPORT_CAP)
        .cloned()
        .map(Into::into)
        .collect();
    let reports_against_members: Vec<ShareReportRowDto> = all_reports
        .into_iter()
        .filter(|r| member_set.contains(&r.owner_handle.to_ascii_lowercase()))
        .take(PER_ORG_REPORT_CAP)
        .map(Into::into)
        .collect();

    let body = OrgSharingContext {
        slug: slug.clone(),
        members: member_slices,
        reports_filed_by_members,
        reports_against_members,
    };
    (StatusCode::OK, Json(body)).into_response()
}

// -- Helpers ---------------------------------------------------------

/// 500 envelope used by both handlers — kept local because the
/// existing `api_error::ApiErrorBody` is overkill for these
/// admin-only routes where the UI never parses the failure body.
fn overview_500() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "sharing_overview_failed"})),
    )
        .into_response()
}

/// Run a single `AuditQuery::list` for one action over the given
/// window. Returns the row count.
///
/// We don't have a dedicated COUNT method on `AuditQuery` yet — adding
/// one would touch every implementer, and the homelab volume comfortably
/// fits in the 500-row page cap. The cap is documented at the call
/// site.
async fn count_audit_action(
    audit: &dyn AuditQuery,
    action: &str,
    since: chrono::DateTime<Utc>,
) -> Result<u64, crate::audit::AuditError> {
    let rows = audit
        .list(AuditFilters {
            actor_handle: None,
            action: Some(action.to_string()),
            since: Some(since),
            until: None,
            limit: 500,
            offset: 0,
        })
        .await?;
    Ok(rows.len() as u64)
}

/// Build the sharing-admin sub-router. Parameterless: both handlers
/// pull their stores off `Extension` layers installed on the outer
/// router (`Arc<dyn ShareMetadataStore>` + `Arc<dyn AuditQuery>` are
/// already provided by main.rs for the existing sharing routes and
/// the /v1/admin/audit endpoint respectively).
pub fn router() -> Router {
    Router::new()
        .route("/v1/admin/sharing/overview", get(get_overview))
        .route(
            "/v1/admin/sharing/scope-histogram",
            get(get_scope_histogram),
        )
        .route("/v1/admin/sharing/reports", get(get_reports))
        .route(
            "/v1/admin/sharing/reports/:id/resolve",
            post(resolve_report),
        )
        .route(
            "/v1/admin/sharing/by-user/:handle",
            get(get_user_sharing_context),
        )
        .route(
            "/v1/admin/sharing/by-org/:slug",
            get(get_org_sharing_context),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::audit::{AuditEntry, AuditLog};
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::share_metadata::test_support::MemoryShareMetadataStore;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::to_bytes;
    use axum::http::Request;
    use chrono::Duration as ChronoDuration;
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Wire the sharing-admin router with the full extension stack.
    fn build_app(
        meta: Arc<MemoryShareMetadataStore>,
        audit: Arc<MemoryAuditLog>,
        staff: Arc<MemoryStaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let meta_dyn: Arc<dyn ShareMetadataStore> = meta;
        let audit_dyn: Arc<dyn AuditQuery> = audit;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        router()
            .layer(Extension(verifier))
            .layer(Extension(meta_dyn))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_dyn))
    }

    async fn json_body<T: for<'de> serde::Deserialize<'de>>(resp: Response) -> T {
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

    /// Build a moderator-roled user and return its bearer token.
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

    /// Bearer for a regular (non-staff) user.
    fn plain_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&Uuid::now_v7().to_string(), handle)
            .expect("sign plain token")
    }

    fn get(uri: &str, token: &str) -> Request<axum::body::Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap()
    }

    // -- /overview ---------------------------------------------------

    #[tokio::test]
    async fn overview_empty_returns_zeroes_for_moderator() {
        let meta = Arc::new(MemoryShareMetadataStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(meta, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;

        let resp = app
            .oneshot(get("/v1/admin/sharing/overview", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AdminSharingOverview = json_body(resp).await;
        assert_eq!(body.active_shares_total, 0);
        assert_eq!(body.active_shares_outbound, 0);
        assert_eq!(body.active_shares_with_expiry, 0);
        assert!(body.top_granters.is_empty());
        assert_eq!(body.total_grants_30d, 0);
        assert_eq!(body.total_revocations_30d, 0);
        assert_eq!(body.total_views_30d, 0);
    }

    #[tokio::test]
    async fn overview_rejects_non_staff_with_403() {
        let meta = Arc::new(MemoryShareMetadataStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(meta, audit, staff, Arc::new(verifier));
        let tok = plain_token(&issuer, "rando");

        let resp = app
            .oneshot(get("/v1/admin/sharing/overview", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn overview_counts_active_shares_top_granters_and_audit_window() {
        let meta = Arc::new(MemoryShareMetadataStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();

        // Seed share_metadata: alice owns 3 active (1 with expiry),
        // bob owns 1 expired (should not count), carol owns 2 active
        // (no expiry).
        let future = Utc::now() + ChronoDuration::days(7);
        let past = Utc::now() - ChronoDuration::days(1);
        meta.upsert("alice", "r1", None, None, None).await.unwrap();
        meta.upsert("alice", "r2", None, None, None).await.unwrap();
        meta.upsert("alice", "r3", Some(future), None, None)
            .await
            .unwrap();
        meta.upsert("bob", "r4", Some(past), None, None)
            .await
            .unwrap();
        meta.upsert("carol", "r5", None, None, None).await.unwrap();
        meta.upsert("carol", "r6", None, None, None).await.unwrap();

        // Seed audit entries in the last 30d.
        for _ in 0..4 {
            audit
                .append(AuditEntry {
                    actor_sub: None,
                    actor_handle: Some("alice".into()),
                    action: "share.granted".into(),
                    payload: json!({}),
                })
                .await
                .unwrap();
        }
        for _ in 0..2 {
            audit
                .append(AuditEntry {
                    actor_sub: None,
                    actor_handle: Some("alice".into()),
                    action: "share.revoked".into(),
                    payload: json!({}),
                })
                .await
                .unwrap();
        }
        for _ in 0..7 {
            audit
                .append(AuditEntry {
                    actor_sub: None,
                    actor_handle: Some("recipient".into()),
                    action: "share.viewed".into(),
                    payload: json!({}),
                })
                .await
                .unwrap();
        }
        // Noise: an unrelated action that must NOT be counted.
        audit
            .append(AuditEntry {
                actor_sub: None,
                actor_handle: Some("alice".into()),
                action: "share.visibility_changed".into(),
                payload: json!({}),
            })
            .await
            .unwrap();

        let app = build_app(meta, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(get("/v1/admin/sharing/overview", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AdminSharingOverview = json_body(resp).await;
        assert_eq!(body.active_shares_total, 5);
        assert_eq!(body.active_shares_outbound, 5);
        assert_eq!(body.active_shares_with_expiry, 1);
        assert_eq!(body.top_granters.len(), 2);
        assert_eq!(body.top_granters[0].handle, "alice");
        assert_eq!(body.top_granters[0].active_share_count, 3);
        assert_eq!(body.top_granters[1].handle, "carol");
        assert_eq!(body.top_granters[1].active_share_count, 2);
        assert_eq!(body.total_grants_30d, 4);
        assert_eq!(body.total_revocations_30d, 2);
        assert_eq!(body.total_views_30d, 7);
    }

    // -- /scope-histogram --------------------------------------------

    #[tokio::test]
    async fn scope_histogram_empty_returns_zeroes() {
        let meta = Arc::new(MemoryShareMetadataStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(meta, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;

        let resp = app
            .oneshot(get("/v1/admin/sharing/scope-histogram", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: ScopeHistogram = json_body(resp).await;
        assert_eq!(body.full, 0);
        assert_eq!(body.timeline, 0);
        assert_eq!(body.aggregates, 0);
        assert_eq!(body.tabs, 0);
        assert!(body.tab_usage.is_empty());
    }

    #[tokio::test]
    async fn scope_histogram_rejects_non_staff_with_403() {
        let meta = Arc::new(MemoryShareMetadataStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = build_app(meta, audit, staff, Arc::new(verifier));
        let tok = plain_token(&issuer, "rando");

        let resp = app
            .oneshot(get("/v1/admin/sharing/scope-histogram", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn scope_histogram_tallies_kinds_and_tab_usage() {
        let meta = Arc::new(MemoryShareMetadataStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();

        // Two NULL-scope rows (count as full), one explicit-full,
        // one timeline, one aggregates, one expired (must NOT count),
        // plus two tabs-scope rows sharing some tab names.
        meta.upsert("o1", "r1", None, None, None).await.unwrap();
        meta.upsert("o2", "r2", None, None, None).await.unwrap();
        meta.upsert("o3", "r3", None, None, Some(&json!({"kind": "full"})))
            .await
            .unwrap();
        meta.upsert("o4", "r4", None, None, Some(&json!({"kind": "timeline"})))
            .await
            .unwrap();
        meta.upsert("o5", "r5", None, None, Some(&json!({"kind": "aggregates"})))
            .await
            .unwrap();
        let past = Utc::now() - ChronoDuration::days(1);
        meta.upsert("o6", "r6", Some(past), None, Some(&json!({"kind": "full"})))
            .await
            .unwrap();
        meta.upsert(
            "o7",
            "r7",
            None,
            None,
            Some(&json!({"kind": "tabs", "tabs": ["combat", "travel"]})),
        )
        .await
        .unwrap();
        meta.upsert(
            "o8",
            "r8",
            None,
            None,
            Some(&json!({"kind": "tabs", "tabs": ["combat", "loadout"]})),
        )
        .await
        .unwrap();

        let app = build_app(meta, audit, staff.clone(), Arc::new(verifier));
        let tok = moderator_token(&staff, &issuer, "mod").await;
        let resp = app
            .oneshot(get("/v1/admin/sharing/scope-histogram", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: ScopeHistogram = json_body(resp).await;
        assert_eq!(body.full, 3, "two NULL-scope + one explicit kind=full");
        assert_eq!(body.timeline, 1);
        assert_eq!(body.aggregates, 1);
        assert_eq!(body.tabs, 2);
        assert_eq!(body.tab_usage.get("combat"), Some(&2));
        assert_eq!(body.tab_usage.get("travel"), Some(&1));
        assert_eq!(body.tab_usage.get("loadout"), Some(&1));
    }
}

#[cfg(test)]
mod report_enforcement_tests {
    //! Resolving a report as `user_suspended` must actually suspend.
    //!
    //! Before this slice, `resolve_report` performed exactly one
    //! UPDATE on `share_reports` and appended an audit row. It did not
    //! revoke shares, did not touch the account, and `UserSuspended`
    //! had zero read-sites anywhere in the codebase — while the
    //! confirmation dialog told the moderator that "all of their shares
    //! are revoked and they can't create new ones until reinstated".
    //!
    //! These tests assert the ACCOUNT STATE after resolution, not the
    //! 200 and not the report row. The broken version wrote its report
    //! row faithfully; that is precisely why nobody noticed.

    use super::*;
    use crate::account_restrictions::{
        test_support::MemoryAccountRestrictionStore, AccountRestrictionStore,
    };
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::AuthVerifier;
    use crate::share_reports::test_support::MemoryShareReportStore;
    use crate::share_reports::{ShareReportReason, ShareReportStore};
    use crate::spicedb::SpicedbClient;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    struct Env {
        app: Router,
        restrictions: Arc<dyn AccountRestrictionStore>,
        report_id: Uuid,
        token: String,
    }

    async fn env(owner_handle: &str, owner_id: Uuid) -> Env {
        let reports = Arc::new(MemoryShareReportStore::default());
        let report = reports
            .create(
                "reporter",
                owner_handle,
                "recipient",
                ShareReportReason::Abuse,
                Some("they are posting slurs"),
            )
            .await
            .expect("seed report");

        let restrictions: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::new().with_handle(owner_id, owner_handle));
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let staff_mem = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();

        let mod_id = Uuid::now_v7();
        staff_mem
            .grant(mod_id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        let token = issuer.sign_user(&mod_id.to_string(), "modhandle").unwrap();

        let reports_dyn: Arc<dyn ShareReportStore> = reports;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff_mem;
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);

        let app = router()
            .layer(Extension(Arc::new(verifier) as Arc<AuthVerifier>))
            .layer(Extension(reports_dyn))
            .layer(Extension(audit))
            .layer(Extension(staff_dyn))
            .layer(Extension(restrictions.clone()))
            .layer(Extension(spicedb));

        Env {
            app,
            restrictions,
            report_id: report.id,
            token,
        }
    }

    fn resolve_req(token: &str, id: Uuid, outcome: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/admin/sharing/reports/{id}/resolve"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({ "outcome": outcome })).unwrap(),
            ))
            .unwrap()
    }

    #[tokio::test]
    async fn resolving_as_user_suspended_actually_suspends_the_account() {
        let owner_id = Uuid::now_v7();
        let env = env("BadActor", owner_id).await;

        let resp = env
            .app
            .clone()
            .oneshot(resolve_req(&env.token, env.report_id, "user_suspended"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // The assertion that would have caught the original bug.
        let stored = env
            .restrictions
            .effective(owner_id)
            .await
            .unwrap()
            .expect("resolving as user_suspended must create a restriction");
        assert!(
            stored.is_suspension(),
            "user_suspended must block every capability, not just record a status"
        );
        assert_eq!(stored.restricted_by, "modhandle");
    }

    #[tokio::test]
    async fn dismissing_a_report_does_not_restrict_anyone() {
        let owner_id = Uuid::now_v7();
        let env = env("Innocent", owner_id).await;

        let resp = env
            .app
            .clone()
            .oneshot(resolve_req(&env.token, env.report_id, "dismissed"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(env
            .restrictions
            .effective(owner_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn share_revoked_does_not_suspend_the_whole_account() {
        // Revoking one share is not a suspension. If this started
        // suspending, moderators would be nuking accounts while
        // believing they had removed a single grant.
        let owner_id = Uuid::now_v7();
        let env = env("Sharer", owner_id).await;

        let resp = env
            .app
            .clone()
            .oneshot(resolve_req(&env.token, env.report_id, "share_revoked"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(env
            .restrictions
            .effective(owner_id)
            .await
            .unwrap()
            .is_none());
    }
}
