//! Role-gated extractors and the admin sub-router.
//!
//! Wraps [`AuthenticatedUser`] in newtypes that consult the
//! [`StaffRoleStore`] before letting a handler run. Two gates today:
//! [`RequireModerator`] (admins inherit) and [`RequireAdmin`].
//!
//! The router builder is intentionally minimal — Wave B (main.rs glue)
//! is responsible for `.merge`-ing the per-feature admin sub-routers
//! (e.g. `admin_submission_routes::router()`) into this one.
//!
//! Misconfiguration is loud: if the `StaffRoleStore` extension is
//! absent the extractor returns 500 with a server-misconfigured body.
//! Silently accepting an admin request when the gate can't run would
//! be far worse than a hard fail.

use crate::audit::{AuditFilters, AuditQuery};
use crate::auth::AuthenticatedUser;
use crate::staff_roles::{StaffRole, StaffRoleStore};
use async_trait::async_trait;
use axum::{
    extract::{FromRequestParts, Query},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, RequestPartsExt, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Bearer-token-authenticated user that has at minimum the
/// `moderator` staff role (admins satisfy this too via inheritance).
pub struct RequireModerator(pub AuthenticatedUser);

/// Bearer-token-authenticated user that holds the `admin` staff role.
/// Field is `#[allow(dead_code)]` because no /v1/admin/* route currently
/// reads it — the moderation queue uses RequireModerator. Slice 3+
/// (user/device admin, audit log viewer, role grants) will read it.
#[allow(dead_code)]
pub struct RequireAdmin(pub AuthenticatedUser);

/// Pre-rendered rejection covering both gates. Defined locally instead
/// of extending `AuthError` because admin-specific failure modes
/// (forbidden, store missing) don't belong in the auth layer's public
/// error surface — leaking them there would force every existing
/// AuthError consumer to handle variants that have nothing to do with
/// JWT validation.
#[derive(Debug)]
enum AdminAuthRejection {
    /// Underlying JWT/auth failure -- defer to `AuthError`'s renderer.
    Auth(crate::auth::AuthError),
    /// JWT `sub` claim isn't a UUID. Defensive: the issuer always
    /// signs UUIDs, so this only fires on malicious or corrupted
    /// tokens.
    InvalidSub,
    /// `StaffRoleStore` extension wasn't installed on the router. A
    /// configuration bug — the gate cannot evaluate the user's roles
    /// so it must fail closed.
    StoreMissing,
    /// The store returned an error (DB outage, etc).
    StoreError,
    /// The user lacks the required role. `role` is the role-name
    /// string surfaced in the response body.
    Forbidden { role: &'static str },
}

impl IntoResponse for AdminAuthRejection {
    fn into_response(self) -> Response {
        match self {
            AdminAuthRejection::Auth(e) => e.into_response(),
            AdminAuthRejection::InvalidSub => (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error": "invalid token sub"})),
            )
                .into_response(),
            AdminAuthRejection::StoreMissing | AdminAuthRejection::StoreError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "server misconfigured"})),
            )
                .into_response(),
            AdminAuthRejection::Forbidden { role } => (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "forbidden",
                    "detail": format!("{role} role required"),
                })),
            )
                .into_response(),
        }
    }
}

/// Shared core. Resolves the JWT, parses the sub, looks up the store,
/// and returns the [`AuthenticatedUser`] iff `predicate` says yes.
async fn extract_with_role(
    parts: &mut Parts,
    role_label: &'static str,
    predicate: impl Fn(&crate::staff_roles::StaffRoleSet) -> bool,
) -> Result<AuthenticatedUser, AdminAuthRejection> {
    // Step 1: delegate to the existing JWT extractor. This handles
    // bearer parsing, signature/claims validation, and device-token
    // revocation in one shot — the admin gate is purely additive.
    let user = parts
        .extract::<AuthenticatedUser>()
        .await
        .map_err(AdminAuthRejection::Auth)?;

    // Step 2: parse `sub` as a UUID. The issuer always mints UUID
    // subjects (see `TokenIssuer::sign_user`), but we never trust the
    // claim implicitly; a malformed sub shouldn't crash the server.
    let user_id = Uuid::parse_str(&user.sub).map_err(|_| AdminAuthRejection::InvalidSub)?;

    // Step 3: pull the role store off the request extensions. Hard
    // fail if it's not installed -- see module docstring.
    let store = parts
        .extensions
        .get::<Arc<dyn StaffRoleStore>>()
        .cloned()
        .ok_or_else(|| {
            tracing::error!(
                "StaffRoleStore extension missing on admin-gated request; rejecting with 500"
            );
            AdminAuthRejection::StoreMissing
        })?;

    // Step 4: load this user's active role set.
    let set = store.list_active_for_user(user_id).await.map_err(|e| {
        tracing::error!(error = ?e, %user_id, "staff role lookup failed");
        AdminAuthRejection::StoreError
    })?;

    // Step 5: enforce the gate.
    if !predicate(&set) {
        return Err(AdminAuthRejection::Forbidden { role: role_label });
    }

    Ok(user)
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireModerator
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user = extract_with_role(parts, "moderator", |set| set.has(StaffRole::Moderator))
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(RequireModerator(user))
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for RequireAdmin
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user = extract_with_role(parts, "admin", |set| set.has(StaffRole::Admin))
            .await
            .map_err(IntoResponse::into_response)?;
        Ok(RequireAdmin(user))
    }
}

/// Audit list DTO surfaced on `GET /v1/admin/audit`. `payload` is
/// passed through as opaque JSON — the admin UI renders it as
/// pretty-printed JSON because the schema varies by `action`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditEntryDto {
    pub seq: i64,
    pub occurred_at: DateTime<Utc>,
    pub actor_sub: Option<String>,
    pub actor_handle: Option<String>,
    pub action: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditListResponse {
    pub entries: Vec<AuditEntryDto>,
    /// True when the server returned a full page — the next page
    /// likely has more rows. Cheaper than COUNT(*) on a growing
    /// table.
    pub has_more: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditListParams {
    /// Case-insensitive substring match against `actor_handle`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Exact match against the `action` column (e.g.
    /// `share.granted`, `submissions.accepted`).
    #[serde(default)]
    pub action: Option<String>,
    /// ISO-8601 lower bound on `occurred_at`.
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    /// ISO-8601 upper bound on `occurred_at`.
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    /// Page size, clamped server-side to [1, 200].
    #[serde(default)]
    pub limit: Option<i64>,
    /// Offset for pagination. Cursor-based pagination is a future
    /// wave once volume warrants it.
    #[serde(default)]
    pub offset: Option<i64>,
}

const AUDIT_PAGE_DEFAULT: i64 = 50;
const AUDIT_PAGE_MAX: i64 = 200;

/// GET /v1/admin/audit — paginated audit-log viewer. Gated on
/// moderator; admins inherit. Returns the most recent rows first
/// because the timeline expectation on the admin UI is "what just
/// happened?".
#[utoipa::path(
    get,
    path = "/v1/admin/audit",
    tag = "admin",
    params(AuditListParams),
    responses(
        (status = 200, description = "Audit entries (most recent first)", body = AuditListResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_audit(
    _: RequireModerator,
    Extension(audit_query): Extension<Arc<dyn AuditQuery>>,
    Query(params): Query<AuditListParams>,
) -> Response {
    let limit = params
        .limit
        .unwrap_or(AUDIT_PAGE_DEFAULT)
        .clamp(1, AUDIT_PAGE_MAX);
    let offset = params.offset.unwrap_or(0).max(0);

    // Trim/normalize before handing to the store so blank inputs
    // become Nones (matches the UI's "leave field empty = no filter"
    // expectation).
    let actor_handle = params
        .actor
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let action = params
        .action
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let filters = AuditFilters {
        actor_handle,
        action,
        since: params.since,
        until: params.until,
        limit,
        offset,
    };

    let rows = match audit_query.list(filters).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "audit list failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "audit_query_failed"})),
            )
                .into_response();
        }
    };

    let has_more = rows.len() as i64 >= limit;
    let entries: Vec<AuditEntryDto> = rows
        .into_iter()
        .map(|r| AuditEntryDto {
            seq: r.seq,
            occurred_at: r.occurred_at,
            actor_sub: r.actor_sub,
            actor_handle: r.actor_handle,
            action: r.action,
            payload: r.payload,
        })
        .collect();

    (
        StatusCode::OK,
        Json(AuditListResponse { entries, has_more }),
    )
        .into_response()
}

/// Builds the admin sub-router rooted at `/v1/admin`. Each handler
/// uses its own role-gate extractor (`RequireModerator` /
/// `RequireAdmin`) so the role check lives next to the code being
/// protected, not on the router builder.
///
/// Kept parameterless: per-feature stores travel as Extension layers
/// installed by `main.rs`. Adding generics here would force every
/// caller to know about every store the gates need.
pub fn router() -> Router {
    Router::new().route("/v1/admin/audit", get(list_audit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::AuthVerifier;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::StaffRoleStore;
    use crate::users::test_support::MemoryUserStore;
    use crate::users::UserStore;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::{Extension, Router};
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Handler that returns 200 + the JWT sub iff the gate accepts.
    async fn protected_mod(RequireModerator(user): RequireModerator) -> String {
        user.sub
    }

    async fn protected_admin(RequireAdmin(user): RequireAdmin) -> String {
        user.sub
    }

    /// Build the test app. Returns the router, the issuer (for minting
    /// tokens), the user store (for seeding accounts) and the role
    /// store (for granting roles in-test).
    async fn build_app() -> (
        Router,
        crate::auth::TokenIssuer,
        Arc<MemoryUserStore>,
        Arc<MemoryStaffRoleStore>,
    ) {
        let (issuer, verifier) = fresh_pair();
        let verifier_arc: Arc<AuthVerifier> = Arc::new(verifier);
        let users = Arc::new(MemoryUserStore::new());
        let staff_mem = Arc::new(MemoryStaffRoleStore::new());
        let staff_dyn: Arc<dyn StaffRoleStore> = staff_mem.clone();

        let app = Router::new()
            .route("/protected/mod", get(protected_mod))
            .route("/protected/admin", get(protected_admin))
            .layer(Extension(verifier_arc))
            .layer(Extension(staff_dyn));

        (app, issuer, users, staff_mem)
    }

    /// Build an app WITHOUT the StaffRoleStore extension installed --
    /// proves the gate fails closed instead of silently letting the
    /// request through.
    async fn build_app_missing_store() -> (Router, crate::auth::TokenIssuer, Arc<MemoryUserStore>) {
        let (issuer, verifier) = fresh_pair();
        let verifier_arc: Arc<AuthVerifier> = Arc::new(verifier);
        let users = Arc::new(MemoryUserStore::new());

        let app = Router::new()
            .route("/protected/admin", get(protected_admin))
            .layer(Extension(verifier_arc));

        (app, issuer, users)
    }

    fn auth_request(uri: &str, token: &str) -> Request<axum::body::Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap()
    }

    async fn seed_user(users: &MemoryUserStore, handle: &str) -> crate::users::User {
        users
            .create(&format!("{handle}@example.com"), "phc$dummy", handle)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn require_moderator_rejects_user_with_no_role() {
        let (app, issuer, users, _staff) = build_app().await;
        let user = seed_user(&users, "alice").await;
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();

        let resp = app
            .oneshot(auth_request("/protected/mod", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn require_moderator_accepts_moderator() {
        let (app, issuer, users, staff) = build_app().await;
        let user = seed_user(&users, "bob").await;
        staff
            .grant(user.id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();

        let resp = app
            .oneshot(auth_request("/protected/mod", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn require_moderator_accepts_admin() {
        // Admins inherit moderator -- the StaffRoleSet::has impl
        // handles this; the test pins the behaviour at the extractor.
        let (app, issuer, users, staff) = build_app().await;
        let user = seed_user(&users, "carol").await;
        staff
            .grant(user.id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();

        let resp = app
            .oneshot(auth_request("/protected/mod", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn require_admin_rejects_moderator() {
        let (app, issuer, users, staff) = build_app().await;
        let user = seed_user(&users, "dave").await;
        staff
            .grant(user.id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();

        let resp = app
            .oneshot(auth_request("/protected/admin", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn require_admin_accepts_admin() {
        let (app, issuer, users, staff) = build_app().await;
        let user = seed_user(&users, "erin").await;
        staff
            .grant(user.id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();

        let resp = app
            .oneshot(auth_request("/protected/admin", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn require_admin_returns_500_when_store_extension_missing() {
        // Misconfigured router: StaffRoleStore extension is absent.
        // The gate must fail closed (500) rather than silently let the
        // request through. This is the load-bearing test for the
        // "fail loud, not silent" stance in the module docstring.
        let (app, issuer, users) = build_app_missing_store().await;
        let user = seed_user(&users, "frank").await;
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();

        let resp = app
            .oneshot(auth_request("/protected/admin", &token))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
