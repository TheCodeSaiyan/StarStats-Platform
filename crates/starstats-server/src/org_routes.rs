//! Organization endpoints (Wave 2B).
//!
//! These handlers manage org metadata in Postgres + membership in
//! SpiceDB:
//!
//!  - `POST   /v1/orgs` — create an org. Slug auto-generated from
//!    the display name; on collision the handler tries `-2` ... `-9`
//!    then 409s.
//!  - `GET    /v1/orgs` — list orgs the caller owns. Member-only
//!    listing is a known gap; see module docs in `orgs.rs`.
//!  - `GET    /v1/orgs/:slug` — fetch one org. Gated by SpiceDB's
//!    `view` permission so non-members get 404.
//!  - `DELETE /v1/orgs/:slug` — best-effort cleanup of every SpiceDB
//!    relationship pointing at the org, then DELETE the Postgres
//!    row. `manage_org` gated.
//!  - `POST   /v1/orgs/:slug/members` — add a member at a given role.
//!  - `DELETE /v1/orgs/:slug/members/:handle` — remove every role
//!    for a user. Idempotent.
//!
//! Failure posture mirrors `sharing_routes`: SpiceDB unreachable →
//! 503, unknown org slug or non-member viewer → 404, validation
//! failure → 400.

use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::AuthenticatedUser;
use crate::orgs::{slug_with_suffix, slugify, OrgError, OrgStore, PostgresOrgStore};
use crate::spicedb::{ObjectRef, SpicedbClient};
use crate::users::{PostgresUserStore, UserStore};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Build the `/v1/orgs/*` sub-router.
///
/// Two internal sub-routers because `add_member` needs both
/// `OrgStore` and `UserStore` as State, while the rest only need
/// `OrgStore`.
pub fn routes(orgs: Arc<PostgresOrgStore>, users: Arc<PostgresUserStore>) -> Router {
    let org_router = Router::new()
        .route(
            "/v1/orgs",
            post(create_org::<PostgresOrgStore>).get(list_orgs::<PostgresOrgStore>),
        )
        .route(
            "/v1/orgs/:slug",
            get(get_org::<PostgresOrgStore>).delete(delete_org::<PostgresOrgStore>),
        )
        .route(
            "/v1/orgs/:slug/members/:handle",
            delete(remove_member::<PostgresOrgStore>),
        )
        .with_state(orgs.clone());

    let member_router = Router::new()
        .route(
            "/v1/orgs/:slug/members",
            post(add_member::<PostgresOrgStore, PostgresUserStore>),
        )
        .with_state((orgs, users));

    org_router.merge(member_router)
}

// -- DTOs ------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateOrgRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OrgDto {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub owner_user_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateOrgResponse {
    pub org: OrgDto,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListOrgsResponse {
    pub orgs: Vec<OrgDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OrgMemberDto {
    pub handle: String,
    /// One of `"owner"`, `"admin"`, `"member"`.
    pub role: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GetOrgResponse {
    pub org: OrgDto,
    pub members: Vec<OrgMemberDto>,
    /// `"owner"`, `"admin"`, `"member"`, or `null` if the caller has
    /// view-without-explicit-role (shouldn't happen in v1 but the
    /// nullable wire shape leaves space).
    pub your_role: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeleteOrgResponse {
    pub deleted: bool,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddMemberRequest {
    pub handle: String,
    /// `"admin"` or `"member"`. The `owner` role is implicit at
    /// create time and not assignable through this endpoint.
    pub role: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddMemberResponse {
    pub added: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RemoveMemberResponse {
    pub removed: bool,
}

// -- Helpers ---------------------------------------------------------

const SLUG_COLLISION_MAX: u32 = 9;

fn err(status: StatusCode, code: &str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

fn no_store() -> [(header::HeaderName, &'static str); 1] {
    [(header::CACHE_CONTROL, "no-store")]
}

use crate::users::validate_handle;

fn validate_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= crate::orgs::SLUG_MAX_LEN
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn org_to_dto(o: &crate::orgs::Org) -> OrgDto {
    OrgDto {
        id: o.id.to_string(),
        name: o.name.clone(),
        slug: o.slug.clone(),
        owner_user_id: o.owner_user_id.to_string(),
        created_at: o.created_at,
    }
}

// -- POST /v1/orgs ---------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/orgs",
    tag = "orgs",
    request_body = CreateOrgRequest,
    responses(
        (status = 200, description = "Organization created", body = CreateOrgResponse),
        (status = 400, description = "Empty or unusable name", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 409, description = "Slug collision exhausted retries", body = ApiErrorBody),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn create_org<O: OrgStore>(
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
    Json(req): Json<CreateOrgRequest>,
) -> Response {
    let name = req.name.trim();
    if name.is_empty() {
        return err(StatusCode::BAD_REQUEST, "invalid_name");
    }
    let base_slug = slugify(name);
    if base_slug.is_empty() {
        // E.g. an all-Unicode name like "漢字" slugifies to "" — refuse
        // rather than insert an empty slug.
        return err(StatusCode::BAD_REQUEST, "invalid_name");
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return err(StatusCode::UNAUTHORIZED, "invalid_subject"),
    };

    // Try the base slug first, then `-2`...`-9`. Capping at 9 keeps
    // the loop bounded and the slugs readable. Anything beyond that
    // is a pathological collision and the user can tweak the name.
    let mut last_err: Option<OrgError> = None;
    let mut created: Option<crate::orgs::Org> = None;
    let mut attempts = 0u32;
    while attempts <= SLUG_COLLISION_MAX {
        let candidate = if attempts == 0 {
            base_slug.clone()
        } else {
            slug_with_suffix(&base_slug, attempts + 1)
        };
        match orgs.create(name, &candidate, user_id).await {
            Ok(o) => {
                created = Some(o);
                break;
            }
            Err(OrgError::SlugTaken) => {
                attempts += 1;
                last_err = Some(OrgError::SlugTaken);
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, "create org failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
            }
        }
    }

    let org = match created {
        Some(o) => o,
        None => {
            tracing::warn!(
                base_slug = %base_slug,
                last_err = ?last_err,
                "org slug collision retries exhausted"
            );
            return err(StatusCode::CONFLICT, "slug_collision");
        }
    };

    // Now write the owner relationship in SpiceDB. If SpiceDB is
    // unreachable we roll back the Postgres row so we don't leak
    // an org with no admin.
    let Some(client) = spicedb.as_ref() else {
        // Best-effort rollback. If this fails too, there's a stale
        // row but no SpiceDB pointer at it — the next call will try
        // to recreate it (slug collision) and surface the issue.
        if let Err(e) = orgs.delete_by_id(org.id).await {
            tracing::warn!(error = %e, "rollback after spicedb_unavailable failed");
        }
        return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable");
    };
    if let Err(e) = client
        .write_org_role(&org.slug, &auth.preferred_username, "owner")
        .await
    {
        tracing::error!(error = %e, "write_org_role(owner) failed");
        if let Err(e2) = orgs.delete_by_id(org.id).await {
            tracing::warn!(error = %e2, "rollback after spicedb write failure failed");
        }
        return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: "org.created".to_string(),
            payload: serde_json::json!({
                "org_id": org.id.to_string(),
                "slug": org.slug,
                "name": org.name,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed (org.created)");
    }

    (
        StatusCode::OK,
        Json(CreateOrgResponse {
            org: org_to_dto(&org),
        }),
    )
        .into_response()
}

// -- GET /v1/orgs ----------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/orgs",
    tag = "orgs",
    responses(
        (status = 200, description = "Orgs you own", body = ListOrgsResponse),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_orgs<O: OrgStore>(
    State(orgs): State<Arc<O>>,
    auth: AuthenticatedUser,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return err(StatusCode::UNAUTHORIZED, "invalid_subject"),
    };

    match orgs.list_for_owner(user_id).await {
        Ok(items) => {
            let dtos: Vec<OrgDto> = items.iter().map(org_to_dto).collect();
            (
                StatusCode::OK,
                no_store(),
                Json(ListOrgsResponse { orgs: dtos }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list_for_owner failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

// -- GET /v1/orgs/:slug ----------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/orgs/{slug}",
    tag = "orgs",
    params(("slug" = String, Path, description = "Organization slug")),
    responses(
        (status = 200, description = "Org details + members", body = GetOrgResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Not found or no view permission"),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_org<O: OrgStore>(
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
) -> Response {
    if !validate_slug(&slug) {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    let Some(client) = spicedb.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable");
    };

    let org = match orgs.find_by_slug(&slug).await {
        Ok(Some(o)) => o,
        Ok(None) => return (StatusCode::NOT_FOUND, ()).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "find_by_slug failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    // Permission gate: caller must have `view` on the org. Unknown
    // membership returns 404 to avoid leaking org existence.
    let resource = ObjectRef::new("organization", &org.slug);
    let subject = ObjectRef::new("user", &auth.preferred_username);
    let allowed = match client.check_permission(resource, "view", subject).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "check view org failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
        }
    };
    if !allowed {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    let members_raw = match client.list_org_members(&org.slug).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "list_org_members failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
        }
    };

    // Resolve caller's own role from the membership list. Highest
    // privilege wins (owner > admin > member) so the UI can render
    // controls correctly even if a user is in two relations.
    let mut your_role: Option<String> = None;
    for (handle, role) in &members_raw {
        if handle.eq_ignore_ascii_case(&auth.preferred_username) {
            your_role = Some(match (your_role.as_deref(), role.as_str()) {
                (Some("owner"), _) => "owner".into(),
                (_, "owner") => "owner".into(),
                (Some("admin"), _) => "admin".into(),
                (_, "admin") => "admin".into(),
                (_, r) => r.to_string(),
            });
        }
    }
    let members: Vec<OrgMemberDto> = members_raw
        .into_iter()
        .map(|(handle, role)| OrgMemberDto { handle, role })
        .collect();

    (
        StatusCode::OK,
        no_store(),
        Json(GetOrgResponse {
            org: org_to_dto(&org),
            members,
            your_role,
        }),
    )
        .into_response()
}

// -- DELETE /v1/orgs/:slug -------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/orgs/{slug}",
    tag = "orgs",
    params(("slug" = String, Path, description = "Organization slug")),
    responses(
        (status = 200, description = "Org deleted", body = DeleteOrgResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Not the org owner", body = ApiErrorBody),
        (status = 404, description = "Org not found"),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn delete_org<O: OrgStore>(
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
) -> Response {
    if !validate_slug(&slug) {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    let Some(client) = spicedb.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable");
    };

    let org = match orgs.find_by_slug(&slug).await {
        Ok(Some(o)) => o,
        Ok(None) => return (StatusCode::NOT_FOUND, ()).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "find_by_slug failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let resource = ObjectRef::new("organization", &org.slug);
    let subject = ObjectRef::new("user", &auth.preferred_username);
    let allowed = match client
        .check_permission(resource, "manage_org", subject)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "check manage_org failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
        }
    };
    if !allowed {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }

    // Best-effort SpiceDB cleanup. We log + continue on failure
    // because Postgres is the system of record for org existence;
    // stale relationships are visible via /v1/orgs/:slug returning
    // 404 (Postgres row gone) which is the user-facing truth.
    if let Err(e) = client.delete_all_org_relationships(&org.slug).await {
        tracing::warn!(
            error = %e,
            slug = %org.slug,
            "delete_all_org_relationships best-effort cleanup failed; \
             proceeding with Postgres delete (gap documented in module docs)"
        );
    }

    if let Err(e) = orgs.delete_by_id(org.id).await {
        tracing::error!(error = %e, "delete org row failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: "org.deleted".to_string(),
            payload: serde_json::json!({
                "org_id": org.id.to_string(),
                "slug": org.slug,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed (org.deleted)");
    }

    (StatusCode::OK, Json(DeleteOrgResponse { deleted: true })).into_response()
}

// -- POST /v1/orgs/:slug/members -------------------------------------

#[utoipa::path(
    post,
    path = "/v1/orgs/{slug}/members",
    tag = "orgs",
    params(("slug" = String, Path, description = "Organization slug")),
    request_body = AddMemberRequest,
    responses(
        (status = 200, description = "Member added", body = AddMemberResponse),
        (status = 400, description = "Invalid handle or role", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "No manage_members permission", body = ApiErrorBody),
        (status = 404, description = "Org or recipient not found", body = ApiErrorBody),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn add_member<O: OrgStore, U: UserStore>(
    State(state): State<(Arc<O>, Arc<U>)>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
    Json(req): Json<AddMemberRequest>,
) -> Response {
    let (orgs, users) = state;
    if !validate_slug(&slug) {
        return err(StatusCode::NOT_FOUND, "not_found");
    }
    let handle = req.handle.trim();
    if !validate_handle(handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    let role = req.role.trim().to_ascii_lowercase();
    if role != "admin" && role != "member" {
        return err(StatusCode::BAD_REQUEST, "invalid_role");
    }

    // Look up the recipient first (this is the one Postgres-side
    // validation that doesn't need SpiceDB to be live).
    let recipient = match users.find_by_handle(handle).await {
        Ok(Some(u)) => u,
        Ok(None) => return err(StatusCode::NOT_FOUND, "recipient_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_handle failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let Some(client) = spicedb.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable");
    };

    let org = match orgs.find_by_slug(&slug).await {
        Ok(Some(o)) => o,
        Ok(None) => return err(StatusCode::NOT_FOUND, "org_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_slug failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    // Permission gate.
    let resource = ObjectRef::new("organization", &org.slug);
    let subject = ObjectRef::new("user", &auth.preferred_username);
    let allowed = match client
        .check_permission(resource, "manage_members", subject)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "check manage_members failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
        }
    };
    if !allowed {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }

    if let Err(e) = client
        .write_org_role(&org.slug, &recipient.claimed_handle, &role)
        .await
    {
        tracing::error!(error = %e, "write_org_role failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: "org.member_added".to_string(),
            payload: serde_json::json!({
                "org_slug": org.slug,
                "recipient_handle": recipient.claimed_handle,
                "role": role,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed (org.member_added)");
    }

    (StatusCode::OK, Json(AddMemberResponse { added: true })).into_response()
}

// -- DELETE /v1/orgs/:slug/members/:handle ---------------------------

#[utoipa::path(
    delete,
    path = "/v1/orgs/{slug}/members/{handle}",
    tag = "orgs",
    params(
        ("slug" = String, Path, description = "Organization slug"),
        ("handle" = String, Path, description = "Member handle to remove"),
    ),
    responses(
        (status = 200, description = "Member removed (idempotent)", body = RemoveMemberResponse),
        (status = 400, description = "Invalid handle in path", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "No manage_members permission", body = ApiErrorBody),
        (status = 404, description = "Org not found", body = ApiErrorBody),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn remove_member<O: OrgStore>(
    State(orgs): State<Arc<O>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
    Path((slug, handle)): Path<(String, String)>,
) -> Response {
    if !validate_slug(&slug) {
        return err(StatusCode::NOT_FOUND, "not_found");
    }
    if !validate_handle(&handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }

    let Some(client) = spicedb.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable");
    };

    let org = match orgs.find_by_slug(&slug).await {
        Ok(Some(o)) => o,
        Ok(None) => return err(StatusCode::NOT_FOUND, "org_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_slug failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let resource = ObjectRef::new("organization", &org.slug);
    let subject = ObjectRef::new("user", &auth.preferred_username);
    let allowed = match client
        .check_permission(resource, "manage_members", subject)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "check manage_members failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
        }
    };
    if !allowed {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }

    if let Err(e) = client.delete_org_member_all_roles(&org.slug, &handle).await {
        tracing::error!(error = %e, "delete_org_member_all_roles failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "spicedb_error");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: "org.member_removed".to_string(),
            payload: serde_json::json!({
                "org_slug": org.slug,
                "recipient_handle": handle,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed (org.member_removed)");
    }

    (StatusCode::OK, Json(RemoveMemberResponse { removed: true })).into_response()
}

// -- Tests -----------------------------------------------------------
//
// SpiceDB-touching paths need a live sidecar; we exercise the
// validation + slug-collision logic only. The `spicedb` extension is
// always `None` here, so handlers should short-circuit with 503
// after the validation gates run.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::AuthVerifier;
    use crate::orgs::test_support::MemoryOrgStore;
    use crate::users::test_support::MemoryUserStore;
    use crate::users::{hash_password, UserStore};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::routing::{delete, get, post};
    use axum::Router;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn router(
        orgs: Arc<MemoryOrgStore>,
        users: Arc<MemoryUserStore>,
        verifier: Arc<AuthVerifier>,
        spicedb: Arc<Option<SpicedbClient>>,
        audit: Arc<dyn AuditLog>,
    ) -> Router {
        let create = Router::new()
            .route(
                "/v1/orgs",
                post(create_org::<MemoryOrgStore>).get(list_orgs::<MemoryOrgStore>),
            )
            .route(
                "/v1/orgs/:slug",
                get(get_org::<MemoryOrgStore>).delete(delete_org::<MemoryOrgStore>),
            )
            .route(
                "/v1/orgs/:slug/members/:handle",
                delete(remove_member::<MemoryOrgStore>),
            )
            .with_state(orgs.clone());
        let members = Router::new()
            .route(
                "/v1/orgs/:slug/members",
                post(add_member::<MemoryOrgStore, MemoryUserStore>),
            )
            .with_state((orgs, users));
        Router::new()
            .merge(create)
            .merge(members)
            .layer(Extension(verifier))
            .layer(Extension(spicedb))
            .layer(Extension(audit))
    }

    async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    fn token_for(handle: &str, issuer: &crate::auth::TokenIssuer, sub: &str) -> String {
        issuer.sign_user(sub, handle).expect("sign user token")
    }

    async fn seed_user(store: &MemoryUserStore, email: &str, handle: &str) -> Uuid {
        let phc = hash_password("password-123-abcdef").unwrap();
        let u = store.create(email, &phc, handle).await.unwrap();
        u.id
    }

    #[tokio::test]
    async fn create_org_with_empty_name_returns_400() {
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name":"   "}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_name");
    }

    #[tokio::test]
    async fn create_org_with_unicode_only_name_returns_400() {
        // Slugifying to "" is rejected up front so we never hit the
        // SpiceDB layer with an empty resource id.
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name":"漢字"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_name");
    }

    #[tokio::test]
    async fn create_org_returns_503_when_spicedb_skipped() {
        // SpiceDB unavailability rolls back the Postgres row.
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        let (issuer, verifier) = fresh_pair();
        let audit_mem = Arc::new(MemoryAuditLog::default());
        let audit: Arc<dyn AuditLog> = audit_mem.clone();
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs.clone(), users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name":"Cool Org"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "spicedb_unavailable");
        // Rolled back — no rows left.
        assert!(orgs.find_by_slug("cool-org").await.unwrap().is_none());
        // No audit row written either.
        assert!(audit_mem.snapshot().is_empty());
    }

    #[tokio::test]
    async fn create_org_with_taken_slug_appends_suffix() {
        // Drive the in-memory store directly to seed a colliding slug,
        // then call create_org and confirm the second attempt lands on
        // `cool-org-2`. SpiceDB stays None so the handler still 503s on
        // the *write* — but only after picking the new slug. We assert
        // the row exists with the suffix even after the rollback path
        // runs (the rollback deletes by id, so we re-seed before the
        // call to verify the suffix path was chosen).
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        // Pre-seed so the base "cool-org" is taken.
        orgs.create("Existing", "cool-org", alice_id).await.unwrap();

        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs.clone(), users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"name":"Cool Org"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // SpiceDB unavailable -> 503 + rollback. The rollback removes
        // the row we just created; that's expected. What we *can*
        // verify is that the original "cool-org" row still exists
        // (the rollback only deleted the suffixed one) AND that the
        // suffix path didn't 409 us.
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Original row untouched.
        assert!(orgs.find_by_slug("cool-org").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn add_member_validates_handle_exists() {
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        orgs.create("Org", "test-org", alice_id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs/test-org/members")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"handle":"NobodyHere","role":"member"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "recipient_not_found");
    }

    #[tokio::test]
    async fn add_member_rejects_unknown_role() {
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        seed_user(&users, "bob@example.com", "Bob").await;
        orgs.create("Org", "test-org", alice_id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs/test-org/members")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"handle":"Bob","role":"superuser"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_role");
    }

    #[tokio::test]
    async fn add_member_rejects_owner_role() {
        // Owner is implicit at create time, not assignable.
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        seed_user(&users, "bob@example.com", "Bob").await;
        orgs.create("Org", "test-org", alice_id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/orgs/test-org/members")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"handle":"Bob","role":"owner"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_role");
    }

    #[tokio::test]
    async fn delete_org_returns_503_when_spicedb_skipped() {
        // The handler hits SpiceDB before doing the Postgres delete,
        // so a missing client short-circuits with 503 and the row
        // stays in place. Stand-in for "ownership required" since
        // we can't run a real check_permission here.
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        orgs.create("Org", "test-org", alice_id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs.clone(), users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/orgs/test-org")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Row still there.
        assert!(orgs.find_by_slug("test-org").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn list_orgs_returns_owner_only() {
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        let bob_id = seed_user(&users, "b@example.com", "Bob").await;
        orgs.create("A1", "a1", alice_id).await.unwrap();
        orgs.create("A2", "a2", alice_id).await.unwrap();
        orgs.create("B1", "b1", bob_id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("GET")
            .uri("/v1/orgs")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::OK);
        let arr = body["orgs"].as_array().expect("orgs array");
        assert_eq!(arr.len(), 2);
        let slugs: Vec<&str> = arr.iter().map(|o| o["slug"].as_str().unwrap()).collect();
        assert!(slugs.contains(&"a1"));
        assert!(slugs.contains(&"a2"));
        assert!(!slugs.contains(&"b1"));
    }

    #[tokio::test]
    async fn remove_member_invalid_handle_returns_400() {
        let orgs = Arc::new(MemoryOrgStore::new());
        let users = Arc::new(MemoryUserStore::new());
        let alice_id = seed_user(&users, "a@example.com", "Alice").await;
        orgs.create("Org", "test-org", alice_id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let app = router(orgs, users, Arc::new(verifier), spicedb, audit);
        let token = token_for("Alice", &issuer, &alice_id.to_string());

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/orgs/test-org/members/bad$handle")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let (status, body) = read_body(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_handle");
    }
}
