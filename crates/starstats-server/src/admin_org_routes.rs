//! Admin orgs sub-router.
//!
//! Surfaces the system-wide org catalogue. Read-only listing +
//! member count + admin force-delete. The owner-facing edit flow
//! still lives in `org_routes.rs`; this module is strictly for the
//! admin console.
//!
//! Endpoints:
//!   GET    /v1/admin/orgs
//!   GET    /v1/admin/orgs/:slug
//!   DELETE /v1/admin/orgs/:slug
//!
//! All gated on the moderator role for read, admin for delete.

use crate::admin_routes::{RequireAdmin, RequireModerator};
use crate::audit::{AuditEntry, AuditLog};
use crate::orgs::{Org, OrgStore};
use crate::spicedb::SpicedbClient;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{delete, get},
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminOrgDto {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub owner_user_id: String,
    pub created_at: DateTime<Utc>,
    /// Active SpiceDB member relationships (owner + admin + member
    /// roles, deduplicated). Surfaced so admins can spot
    /// abandoned-but-not-empty orgs at a glance.
    pub member_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminOrgListResponse {
    pub orgs: Vec<AdminOrgDto>,
    pub has_more: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AdminOrgListParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AdminOrgDeleteResponse {
    pub deleted: bool,
}

const ORGS_PAGE_DEFAULT: i64 = 50;
const ORGS_PAGE_MAX: i64 = 200;

fn err_response(status: StatusCode, error: &str) -> Response {
    (status, Json(serde_json::json!({ "error": error }))).into_response()
}

/// Resolve member-count for one org via SpiceDB. Returns 0 when
/// SpiceDB is unavailable so the listing still renders.
async fn member_count_for(slug: &str, spicedb: &Option<SpicedbClient>) -> u32 {
    let Some(client) = spicedb else {
        return 0;
    };
    match client.list_org_members(slug).await {
        Ok(rows) => {
            let mut handles: Vec<&str> = rows.iter().map(|(h, _)| h.as_str()).collect();
            handles.sort();
            handles.dedup();
            handles.len() as u32
        }
        Err(e) => {
            tracing::warn!(error = %e, slug, "list_org_members failed; defaulting to 0");
            0
        }
    }
}

async fn to_dto(org: Org, spicedb: &Arc<Option<SpicedbClient>>) -> AdminOrgDto {
    let member_count = member_count_for(&org.slug, spicedb.as_ref()).await;
    AdminOrgDto {
        id: org.id.to_string(),
        name: org.name,
        slug: org.slug,
        owner_user_id: org.owner_user_id.to_string(),
        created_at: org.created_at,
        member_count,
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/orgs",
    tag = "admin",
    params(AdminOrgListParams),
    responses(
        (status = 200, description = "Orgs page (most recent first)", body = AdminOrgListResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_orgs_admin<O: OrgStore>(
    _: RequireModerator,
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Query(params): Query<AdminOrgListParams>,
) -> Response {
    let limit = params
        .limit
        .unwrap_or(ORGS_PAGE_DEFAULT)
        .clamp(1, ORGS_PAGE_MAX);
    let offset = params.offset.unwrap_or(0).max(0);

    let rows = match orgs.list_all(params.q.as_deref(), limit, offset).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "list_all orgs failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let has_more = rows.len() as i64 >= limit;
    let mut dtos = Vec::with_capacity(rows.len());
    for org in rows {
        dtos.push(to_dto(org, &spicedb).await);
    }
    (
        StatusCode::OK,
        Json(AdminOrgListResponse {
            orgs: dtos,
            has_more,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/orgs/{slug}",
    tag = "admin",
    params(("slug" = String, Path, description = "Org slug")),
    responses(
        (status = 200, description = "Org detail with member count", body = AdminOrgDto),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "Org not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_org_admin<O: OrgStore>(
    _: RequireModerator,
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Path(slug): Path<String>,
) -> Response {
    let org = match orgs.find_by_slug(&slug).await {
        Ok(Some(o)) => o,
        Ok(None) => return err_response(StatusCode::NOT_FOUND, "org_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_slug failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    (StatusCode::OK, Json(to_dto(org, &spicedb).await)).into_response()
}

#[utoipa::path(
    delete,
    path = "/v1/admin/orgs/{slug}",
    tag = "admin",
    params(("slug" = String, Path, description = "Org slug")),
    responses(
        (status = 200, description = "Org deleted (idempotent)", body = AdminOrgDeleteResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 404, description = "Org not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn delete_org_admin<O: OrgStore>(
    actor: RequireAdmin,
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(slug): Path<String>,
) -> Response {
    let org = match orgs.find_by_slug(&slug).await {
        Ok(Some(o)) => o,
        Ok(None) => return err_response(StatusCode::NOT_FOUND, "org_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_slug failed");
            return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    // Best-effort SpiceDB cleanup. The owner-facing delete handler
    // does the same — if SpiceDB is down we still drop the
    // Postgres row, and a reconciliation job (out of scope) can
    // sweep orphan relationships later.
    if let Some(client) = spicedb.as_ref() {
        if let Err(e) = client.delete_all_org_relationships(&org.slug).await {
            tracing::warn!(
                error = %e,
                slug,
                "delete_all_org_relationships best-effort cleanup failed"
            );
        }
    }
    if let Err(e) = orgs.delete_by_id(org.id).await {
        tracing::error!(error = %e, slug, "delete_by_id failed");
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(actor.0.sub.clone()),
            actor_handle: Some(actor.0.preferred_username.clone()),
            action: "admin.org.deleted".to_string(),
            payload: serde_json::json!({
                "org_id": org.id,
                "slug": org.slug,
                "name": org.name,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit append (admin.org.deleted) failed");
    }

    (
        StatusCode::OK,
        Json(AdminOrgDeleteResponse { deleted: true }),
    )
        .into_response()
}

pub fn router<O: OrgStore>(orgs: Arc<O>) -> Router {
    Router::new()
        .route("/v1/admin/orgs", get(list_orgs_admin::<O>))
        .route("/v1/admin/orgs/:slug", get(get_org_admin::<O>))
        .route("/v1/admin/orgs/:slug", delete(delete_org_admin::<O>))
        .with_state(orgs)
}
