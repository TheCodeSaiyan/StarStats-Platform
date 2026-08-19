//! Admin users sub-router.
//!
//! Gated on the moderator role for read operations and the admin
//! role for role grants/revokes (mirrors how staff escalation works
//! in most systems — moderators can investigate, admins can promote).
//!
//! Endpoints:
//!   GET    /v1/admin/users
//!   GET    /v1/admin/users/:id
//!   POST   /v1/admin/users/:id/roles
//!   DELETE /v1/admin/users/:id/roles/:role
//!
//! Audit trail: every grant/revoke writes one `staff.grant` or
//! `staff.revoke` row via the existing audit log. The grant/revoke
//! routes are intentionally idempotent — replaying a grant for an
//! already-active role returns 200 with `changed: false` instead of
//! erroring, which keeps retry-on-network-blip from surfacing as
//! "Forbidden / already a moderator" to the admin UI.

use crate::admin_routes::{RequireAdmin, RequireModerator};
use crate::admin_user_insights::{ActivitySummary, AdminUserInsightsStore, SyncState};
use crate::audit::{AuditEntry, AuditLog};
use crate::staff_roles::{StaffRole, StaffRoleStore};
use crate::users::{ListUsersFilters, User, UserStore};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// -- DTOs -----------------------------------------------------------------

/// Lightweight admin-side view of a user. Skips secrets (password
/// hash, TOTP secret) and verification tokens. Keeps only the bits
/// an admin needs to triage an account.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserDto {
    pub id: Uuid,
    pub email: String,
    pub claimed_handle: String,
    pub created_at: DateTime<Utc>,
    pub email_verified: bool,
    pub rsi_verified: bool,
    pub totp_enabled: bool,
    /// Active staff roles (e.g. `["moderator"]`, `["admin"]`).
    /// Always present; empty array for ordinary users.
    pub staff_roles: Vec<String>,
    /// One of `never` | `off` | `stale` | `live`. Derived from the
    /// user's device fleet, not stored — see `admin_user_insights`.
    pub sync_state: String,
    /// Lifetime event count from the `stat_event_counts` rollup.
    pub entry_count: i64,
    /// `None` means the user has never sent an event. Distinct from a
    /// zero `entry_count`, and must not be rendered as a date.
    pub last_activity_at: Option<DateTime<Utc>>,
}

/// Per-device row on the user-detail page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserDeviceDto {
    pub label: String,
    pub sync_enabled: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Per-event-type breakdown on the user-detail page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserEventTypeCountDto {
    pub event_type: String,
    pub event_count: i64,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

/// Retention context. Deliberately does NOT carry a "swept by
/// retention" count: sweep totals are aggregate and transient
/// (logged, never persisted per user), and retention only writes an
/// audit row when it deleted something — so any per-user figure here
/// would be invented.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserRetentionDto {
    /// `free` or `supporter` — the RETENTION tier. A lapsed supporter
    /// keeps their pill but reverts to free-tier retention.
    pub tier: String,
    /// `None` means unlimited. Never render as `0`.
    pub retention_days: Option<i32>,
    pub oldest_entry_at: Option<DateTime<Utc>>,
    /// Events older than this are eligible for purging. `None` when
    /// retention is unlimited.
    pub cutoff: Option<DateTime<Utc>>,
}

/// Superset of [`AdminUserDto`] returned by the detail endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserDetailDto {
    #[serde(flatten)]
    pub user: AdminUserDto,
    pub devices: Vec<AdminUserDeviceDto>,
    pub event_type_counts: Vec<AdminUserEventTypeCountDto>,
    pub retention: AdminUserRetentionDto,
    /// Live restriction, or `None` when unrestricted. Expired rows read
    /// as `None` -- the store applies expiry, so the console cannot show
    /// a restriction that is no longer being enforced.
    pub restriction: Option<crate::admin_restriction_routes::AdminRestrictionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminUserListResponse {
    pub users: Vec<AdminUserDto>,
    pub has_more: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AdminUserListParams {
    /// Substring search against `claimed_handle` OR `email`.
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GrantRoleRequest {
    pub role: String,
    /// Optional free-text note for the audit trail (e.g.
    /// "promoted at quarterly review"). Capped at 280 chars by the
    /// handler; longer values rejected with 400.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoleTransitionResponse {
    /// Whether the call actually changed state. `false` for a grant
    /// against an already-active role or a revoke against an
    /// already-inactive role.
    pub changed: bool,
    /// The user's active role set after the operation. Lets the UI
    /// re-render without a follow-up GET.
    pub staff_roles: Vec<String>,
}

const USERS_PAGE_DEFAULT: i64 = 50;
const USERS_PAGE_MAX: i64 = 200;
const REASON_MAX_LEN: usize = 280;

fn err_body(error: &str) -> serde_json::Value {
    serde_json::json!({ "error": error })
}

fn err_response(status: StatusCode, error: &str) -> Response {
    (status, Json(err_body(error))).into_response()
}

/// Materialise an `AdminUserDto` from a `User` plus its active
/// staff-role set and its already-fetched insight rows.
///
/// `sync` and `activity` come from BATCHED lookups keyed by the whole
/// page — do not re-query per user here. A missing sync entry means the
/// user has no devices at all, which is `never`, not an error.
fn build_dto(
    user: User,
    roles: Vec<String>,
    sync: Option<SyncState>,
    activity: Option<&ActivitySummary>,
) -> AdminUserDto {
    AdminUserDto {
        id: user.id,
        email: user.email,
        claimed_handle: user.claimed_handle,
        created_at: user.created_at,
        email_verified: user.email_verified_at.is_some(),
        rsi_verified: user.rsi_verified_at.is_some(),
        totp_enabled: user.totp_enabled_at.is_some(),
        staff_roles: roles,
        sync_state: sync.unwrap_or(SyncState::Never).as_str().to_string(),
        entry_count: activity.map(|a| a.entry_count).unwrap_or(0),
        last_activity_at: activity.and_then(|a| a.last_activity_at),
    }
}

/// Fetch one user's staff roles.
///
/// STILL PER-USER, and therefore still an N+1 on the list route: a
/// 50-row page issues 50 of these. Sync state and activity ARE batched
/// (one query each for the whole page), so this slice takes the page
/// from ~51 queries to ~52 rather than to ~200. Batching this one too
/// needs a `list_active_for_many` on `StaffRoleStore`, which touches
/// the trait and every implementor; deliberately left for its own
/// change rather than smuggled in here.
async fn roles_for(
    user_id: Uuid,
    staff: &Arc<dyn StaffRoleStore>,
) -> Result<Vec<String>, Response> {
    match staff.list_active_for_user(user_id).await {
        Ok(r) => Ok(r.as_strings()),
        Err(e) => {
            tracing::error!(error = %e, user_id = %user_id, "staff_roles lookup failed");
            Err(err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "staff_roles_lookup_failed",
            ))
        }
    }
}

// -- Handlers -------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/users",
    tag = "admin",
    params(AdminUserListParams),
    responses(
        (status = 200, description = "Users page (most recent first)", body = AdminUserListResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_users_admin<U: UserStore>(
    _: RequireModerator,
    State(users): State<Arc<U>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
    Extension(insights): Extension<Arc<dyn AdminUserInsightsStore>>,
    Query(params): Query<AdminUserListParams>,
) -> Response {
    let limit = params
        .limit
        .unwrap_or(USERS_PAGE_DEFAULT)
        .clamp(1, USERS_PAGE_MAX);
    let offset = params.offset.unwrap_or(0).max(0);

    let filters = ListUsersFilters {
        q: params
            .q
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        limit,
        offset,
    };

    let user_rows = match users.list_users(filters).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "list_users failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let has_more = user_rows.len() as i64 >= limit;

    // Batched enrichment: TWO queries for the whole page, not two per
    // user. Doing these per-row would have taken a 50-row page from ~51
    // queries to ~150. (The staff-role lookup below is still per-user —
    // see `roles_for`.)
    let ids: Vec<Uuid> = user_rows.iter().map(|u| u.id).collect();
    // Lowercased: `stat_event_counts` enforces lowercase handles, so
    // querying with the users-table casing matches nothing and every
    // user silently reads zero entries.
    let handles: Vec<String> = user_rows
        .iter()
        .map(|u| u.claimed_handle.to_lowercase())
        .collect();

    let sync_map = match insights.sync_states(&ids).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "sync_states lookup failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let activity_map = match insights.activity(&handles).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "activity lookup failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let mut dtos = Vec::with_capacity(user_rows.len());
    for user in user_rows {
        let roles = match roles_for(user.id, &staff).await {
            Ok(r) => r,
            Err(resp) => return resp,
        };
        let sync = sync_map.get(&user.id).copied();
        let activity = activity_map.get(&user.claimed_handle.to_lowercase());
        dtos.push(build_dto(user, roles, sync, activity));
    }
    (
        StatusCode::OK,
        Json(AdminUserListResponse {
            users: dtos,
            has_more,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/users/{id}",
    tag = "admin",
    params(("id" = String, Path, description = "User UUID")),
    responses(
        (status = 200, description = "User detail", body = AdminUserDetailDto),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "User not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_user_admin<U: UserStore>(
    _: RequireModerator,
    State(users): State<Arc<U>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
    Extension(insights): Extension<Arc<dyn AdminUserInsightsStore>>,
    Extension(restrictions): Extension<
        Arc<dyn crate::account_restrictions::AccountRestrictionStore>,
    >,
    Path(id_str): Path<String>,
) -> Response {
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return err_response(StatusCode::NOT_FOUND, "user_not_found");
    };
    let user = match users.find_by_id(id).await {
        Ok(Some(u)) => u,
        Ok(None) => return err_response(StatusCode::NOT_FOUND, "user_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let roles = match roles_for(user.id, &staff).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let handle = user.claimed_handle.clone();
    let user_id = user.id;

    let devices = match insights.devices(user_id).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(error = %e, "devices lookup failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let type_counts = match insights.event_type_counts(&handle).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "event_type_counts lookup failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let retention = match insights.retention(user_id, &handle).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "retention lookup failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    // Derive the summary fields from the SAME device/type rows the
    // detail sections render, rather than issuing the batched lookups
    // again — the page would otherwise be able to show a "live" chip
    // above a device table that disagrees with it.
    let sync = crate::admin_user_insights::classify_sync(&devices, Utc::now());
    let activity = ActivitySummary {
        entry_count: type_counts.iter().map(|c| c.event_count).sum(),
        last_activity_at: type_counts.iter().filter_map(|c| c.last_seen_at).max(),
    };

    // Read the live restriction for the console. Expiry is applied by
    // the store, so an expired row reads as None and the page cannot
    // display a restriction that is no longer enforced.
    let restriction_now = match restrictions.effective(user_id).await {
        Ok(r) => {
            r.map(|r| crate::admin_restriction_routes::AdminRestrictionDto::from_restriction(&r, 0))
        }
        Err(e) => {
            tracing::error!(error = %e, "restriction lookup failed (admin user detail)");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let dto = build_dto(user, roles, Some(sync), Some(&activity));

    (
        StatusCode::OK,
        Json(AdminUserDetailDto {
            user: dto,
            devices: devices
                .into_iter()
                .map(|d| AdminUserDeviceDto {
                    label: d.label,
                    sync_enabled: d.sync_enabled,
                    last_seen_at: d.last_seen_at,
                    revoked_at: d.revoked_at,
                })
                .collect(),
            event_type_counts: type_counts
                .into_iter()
                .map(|c| AdminUserEventTypeCountDto {
                    event_type: c.event_type,
                    event_count: c.event_count,
                    first_seen_at: c.first_seen_at,
                    last_seen_at: c.last_seen_at,
                })
                .collect(),
            restriction: restriction_now,
            retention: AdminUserRetentionDto {
                tier: retention.tier,
                retention_days: retention.retention_days,
                oldest_entry_at: retention.oldest_entry_at,
                cutoff: retention.cutoff,
            },
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/users/{id}/roles",
    tag = "admin",
    request_body = GrantRoleRequest,
    params(("id" = String, Path, description = "Target user UUID")),
    responses(
        (status = 200, description = "Role granted (idempotent)", body = RoleTransitionResponse),
        (status = 400, description = "Invalid role or reason"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 404, description = "Target user not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn grant_role<U: UserStore>(
    actor: RequireAdmin,
    State(users): State<Arc<U>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id_str): Path<String>,
    Json(req): Json<GrantRoleRequest>,
) -> Response {
    let Ok(target_id) = Uuid::parse_str(&id_str) else {
        return err_response(StatusCode::NOT_FOUND, "user_not_found");
    };
    let Ok(role) = req.role.parse::<StaffRole>() else {
        return err_response(StatusCode::BAD_REQUEST, "invalid_role");
    };
    if let Some(r) = req.reason.as_ref() {
        if r.chars().count() > REASON_MAX_LEN {
            return err_response(StatusCode::BAD_REQUEST, "reason_too_long");
        }
    }

    match users.find_by_id(target_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return err_response(StatusCode::NOT_FOUND, "user_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in grant_role");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }

    let Ok(actor_id) = Uuid::parse_str(&actor.0.sub) else {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject");
    };
    let reason_str = req
        .reason
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let changed = match staff
        .grant(target_id, role, Some(actor_id), reason_str)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "staff grant failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "staff_store_error");
        }
    };

    if changed {
        if let Err(e) = audit
            .append(AuditEntry {
                actor_sub: Some(actor.0.sub.clone()),
                actor_handle: Some(actor.0.preferred_username.clone()),
                action: "staff.grant".to_string(),
                payload: serde_json::json!({
                    "target_user_id": target_id,
                    "role": role.as_str(),
                    "reason": reason_str,
                }),
            })
            .await
        {
            tracing::warn!(error = %e, "audit append (staff.grant) failed");
        }
    }

    let roles = match staff.list_active_for_user(target_id).await {
        Ok(r) => r.as_strings(),
        Err(e) => {
            tracing::error!(error = %e, "staff list after grant failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "staff_store_error");
        }
    };

    (
        StatusCode::OK,
        Json(RoleTransitionResponse {
            changed,
            staff_roles: roles,
        }),
    )
        .into_response()
}

#[utoipa::path(
    delete,
    path = "/v1/admin/users/{id}/roles/{role}",
    tag = "admin",
    params(
        ("id" = String, Path, description = "Target user UUID"),
        ("role" = String, Path, description = "Role to revoke (moderator|admin)"),
    ),
    responses(
        (status = 200, description = "Role revoked (idempotent)", body = RoleTransitionResponse),
        (status = 400, description = "Invalid role"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 404, description = "Target user not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn revoke_role<U: UserStore>(
    actor: RequireAdmin,
    State(users): State<Arc<U>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path((id_str, role_str)): Path<(String, String)>,
) -> Response {
    let Ok(target_id) = Uuid::parse_str(&id_str) else {
        return err_response(StatusCode::NOT_FOUND, "user_not_found");
    };
    let Ok(role) = role_str.parse::<StaffRole>() else {
        return err_response(StatusCode::BAD_REQUEST, "invalid_role");
    };

    // Don't let an admin revoke their own admin role — they'd lock
    // themselves out, and the UI's "are you sure" guard is best
    // mirrored server-side too.
    if let Ok(actor_id) = Uuid::parse_str(&actor.0.sub) {
        if actor_id == target_id && role == StaffRole::Admin {
            return err_response(StatusCode::BAD_REQUEST, "cannot_revoke_own_admin");
        }
    }

    match users.find_by_id(target_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return err_response(StatusCode::NOT_FOUND, "user_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in revoke_role");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }

    let actor_id = Uuid::parse_str(&actor.0.sub).ok();
    let changed = match staff.revoke(target_id, role, actor_id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "staff revoke failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "staff_store_error");
        }
    };

    if changed {
        if let Err(e) = audit
            .append(AuditEntry {
                actor_sub: Some(actor.0.sub.clone()),
                actor_handle: Some(actor.0.preferred_username.clone()),
                action: "staff.revoke".to_string(),
                payload: serde_json::json!({
                    "target_user_id": target_id,
                    "role": role.as_str(),
                }),
            })
            .await
        {
            tracing::warn!(error = %e, "audit append (staff.revoke) failed");
        }
    }

    let roles = match staff.list_active_for_user(target_id).await {
        Ok(r) => r.as_strings(),
        Err(e) => {
            tracing::error!(error = %e, "staff list after revoke failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "staff_store_error");
        }
    };

    (
        StatusCode::OK,
        Json(RoleTransitionResponse {
            changed,
            staff_roles: roles,
        }),
    )
        .into_response()
}

/// How thoroughly to delete an account.
///
/// `Pseudonymise` is the same operation a user gets from
/// `DELETE /v1/auth/me`: account, devices and shares go, event rows
/// stay with the handle replaced by a non-resolvable tombstone so
/// anyone they shared with keeps a coherent timeline.
///
/// `Purge` additionally deletes the events and every derived
/// per-handle table. Irreversible, and it breaks recipients'
/// timelines — which is why it must be asked for explicitly and is
/// never the default in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    Pseudonymise,
    Purge,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct AdminDeleteUserRequest {
    /// Must match the target's handle, case-insensitively. Mirrors the
    /// org force-delete and the self-serve account delete.
    pub confirm_handle: String,
    pub mode: DeleteMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminDeleteUserResponse {
    pub deleted: bool,
    pub mode: DeleteMode,
}

#[utoipa::path(
    delete,
    path = "/v1/admin/users/{id}",
    tag = "admin",
    params(("id" = String, Path, description = "User UUID")),
    request_body = AdminDeleteUserRequest,
    responses(
        (status = 200, description = "Account deleted", body = AdminDeleteUserResponse),
        (status = 400, description = "Handle mismatch, or targeting yourself"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role, or target is an admin"),
        (status = 404, description = "User not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn delete_user_admin<U: UserStore>(
    RequireAdmin(actor): RequireAdmin,
    State(users): State<Arc<U>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id_str): Path<String>,
    Json(body): Json<AdminDeleteUserRequest>,
) -> Response {
    let Ok(target_id) = Uuid::parse_str(&id_str) else {
        return err_response(StatusCode::NOT_FOUND, "user_not_found");
    };

    // Self-protection, mirroring cannot_revoke_own_admin.
    if actor.sub == id_str {
        return err_response(StatusCode::BAD_REQUEST, "cannot_delete_yourself");
    }

    let target = match users.find_by_id(target_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return err_response(StatusCode::NOT_FOUND, "user_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    if !body
        .confirm_handle
        .trim()
        .eq_ignore_ascii_case(&target.claimed_handle)
    {
        return err_response(StatusCode::BAD_REQUEST, "confirm_mismatch");
    }

    match staff.list_active_for_user(target_id).await {
        Ok(roles) if roles.has(StaffRole::Admin) => {
            return err_response(StatusCode::FORBIDDEN, "cannot_delete_an_admin");
        }
        Ok(_) => {}
        Err(e) => {
            // Fail closed. "I could not determine whether this account
            // belongs to an admin" is not grounds to delete it.
            tracing::error!(error = %e, "staff role lookup failed; refusing to delete");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }

    // Audit BEFORE the delete, and a failed append ABORTS it. Same
    // posture as the self-serve delete_account: if we cannot record the
    // action we must not perform it. An admin deletion nobody can
    // account for is worse than a deletion that failed.
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(actor.sub.clone()),
            actor_handle: Some(actor.preferred_username.clone()),
            action: "account.deleted".to_string(),
            payload: serde_json::json!({
                "target_user_id": target_id,
                "target_handle": target.claimed_handle,
                "mode": body.mode,
            }),
        })
        .await
    {
        tracing::error!(error = %e, %target_id, "audit append failed; aborting admin delete");
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }

    let outcome = match body.mode {
        DeleteMode::Pseudonymise => users.delete_user(target_id).await,
        DeleteMode::Purge => users.purge_user(target_id).await,
    };
    if let Err(e) = outcome {
        tracing::error!(error = %e, %target_id, mode = ?body.mode, "admin delete failed");
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }

    (
        StatusCode::OK,
        Json(AdminDeleteUserResponse {
            deleted: true,
            mode: body.mode,
        }),
    )
        .into_response()
}

pub fn router<U: UserStore>(users: Arc<U>) -> Router {
    Router::new()
        .route("/v1/admin/users", get(list_users_admin::<U>))
        .route(
            "/v1/admin/users/:id",
            get(get_user_admin::<U>).delete(delete_user_admin::<U>),
        )
        .route("/v1/admin/users/:id/roles", post(grant_role::<U>))
        .route("/v1/admin/users/:id/roles/:role", delete(revoke_role::<U>))
        .with_state(users)
}

#[cfg(test)]
mod admin_delete_tests {
    //! Admin-initiated deletion.
    //!
    //! Every rejection test asserts the user STILL EXISTS afterwards.
    //! Checking only the status code passes against a handler that
    //! deletes and then rejects — and for an irreversible operation
    //! that is the difference between a bug and an incident.

    use super::*;
    use crate::account_restrictions::test_support::MemoryAccountRestrictionStore;
    use crate::account_restrictions::AccountRestrictionStore;
    use crate::admin_user_insights::test_support::MemoryAdminUserInsights;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::AuthVerifier;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    struct Env {
        app: Router,
        users: Arc<MemoryUserStore>,
        staff: Arc<MemoryStaffRoleStore>,
        issuer: crate::auth::TokenIssuer,
    }

    fn build(audit: Arc<dyn AuditLog>) -> Env {
        let users = Arc::new(MemoryUserStore::default());
        let staff_mem = Arc::new(MemoryStaffRoleStore::new());
        let staff: Arc<dyn StaffRoleStore> = staff_mem.clone();
        let restrictions: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::new());
        let insights: Arc<dyn AdminUserInsightsStore> = Arc::new(MemoryAdminUserInsights::new());
        let (issuer, verifier) = fresh_pair();

        let app = router(users.clone())
            .layer(Extension(staff))
            .layer(Extension(audit))
            .layer(Extension(restrictions))
            .layer(Extension(insights))
            .layer(Extension(Arc::new(verifier) as Arc<AuthVerifier>));

        Env {
            app,
            users,
            staff: staff_mem,
            issuer,
        }
    }

    fn env() -> Env {
        build(Arc::new(MemoryAuditLog::default()))
    }

    async fn seed(users: &MemoryUserStore, email: &str, handle: &str) -> Uuid {
        let phc = hash_password("password-123-abcdef").unwrap();
        users.create(email, &phc, handle).await.unwrap().id
    }

    async fn admin_token(env: &Env, handle: &str) -> (Uuid, String) {
        let id = Uuid::now_v7();
        env.staff
            .grant(id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        (id, env.issuer.sign_user(&id.to_string(), handle).unwrap())
    }

    async fn moderator_token(env: &Env, handle: &str) -> String {
        let id = Uuid::now_v7();
        env.staff
            .grant(id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        env.issuer.sign_user(&id.to_string(), handle).unwrap()
    }

    fn del_req(token: &str, target: Uuid, confirm: &str, mode: &str) -> Request<Body> {
        Request::builder()
            .method("DELETE")
            .uri(format!("/v1/admin/users/{target}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({ "confirm_handle": confirm, "mode": mode })).unwrap(),
            ))
            .unwrap()
    }

    async fn still_exists(users: &MemoryUserStore, id: Uuid) -> bool {
        users.find_by_id(id).await.unwrap().is_some()
    }

    #[tokio::test]
    async fn admin_can_pseudonymise_delete() {
        let env = env();
        let target = seed(&env.users, "t@example.test", "Target").await;
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "Target", "pseudonymise"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!still_exists(&env.users, target).await);
    }

    #[tokio::test]
    async fn admin_can_purge_delete() {
        let env = env();
        let target = seed(&env.users, "t@example.test", "Target").await;
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "Target", "purge"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!still_exists(&env.users, target).await);
    }

    #[tokio::test]
    async fn confirm_handle_is_case_insensitive() {
        let env = env();
        let target = seed(&env.users, "t@example.test", "TheCodeSaiyan").await;
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "thecodesaiyan", "pseudonymise"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_moderator_cannot_delete() {
        let env = env();
        let target = seed(&env.users, "t@example.test", "Target").await;
        let token = moderator_token(&env, "modhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "Target", "pseudonymise"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(still_exists(&env.users, target).await);
    }

    #[tokio::test]
    async fn cannot_delete_yourself() {
        let env = env();
        let (admin_id, token) = admin_token(&env, "adminhandle").await;
        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, admin_id, "adminhandle", "purge"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cannot_delete_another_admin() {
        let env = env();
        let target = seed(&env.users, "a@example.test", "OtherAdmin").await;
        env.staff
            .grant(target, StaffRole::Admin, None, None)
            .await
            .unwrap();
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "OtherAdmin", "purge"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(still_exists(&env.users, target).await);
    }

    #[tokio::test]
    async fn a_mismatched_confirm_handle_deletes_nothing() {
        let env = env();
        let target = seed(&env.users, "t@example.test", "Target").await;
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "NotTheHandle", "purge"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(still_exists(&env.users, target).await);
    }

    #[tokio::test]
    async fn an_unknown_mode_is_rejected_and_deletes_nothing() {
        let env = env();
        let target = seed(&env.users, "t@example.test", "Target").await;
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "Target", "obliterate"))
            .await
            .unwrap();
        // serde rejects the unknown variant before the handler runs.
        assert!(resp.status().is_client_error());
        assert!(still_exists(&env.users, target).await);
    }

    #[tokio::test]
    async fn a_failed_audit_aborts_the_delete() {
        // An admin deletion nobody can account for is worse than one
        // that failed. delete_account already takes this posture; this
        // asserts the admin path matches it.
        let env = build(Arc::new(MemoryAuditLog::failing()));
        let target = seed(&env.users, "t@example.test", "Target").await;
        let (_, token) = admin_token(&env, "adminhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(del_req(&token, target, "Target", "purge"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            still_exists(&env.users, target).await,
            "the account must survive an unrecordable deletion"
        );
    }
}
