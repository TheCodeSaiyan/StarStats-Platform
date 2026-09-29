//! Admin surface for the tier-based data retention purge.
//!
//! Two endpoints, both gated on `RequireAdmin`:
//! - `GET  /v1/admin/retention/policies` — list the seeded tiers and
//!   their windows (NULL = unlimited).
//! - `POST /v1/admin/retention/purge`    — kick off a sweep now,
//!   out-of-band from the scheduled tokio loop.
//!
//! The scheduled loop in main.rs is the load-bearing path; this
//! endpoint exists so operators can force a sweep in non-prod and so
//! the admin UI has a "run now" button.

use crate::admin_routes::RequireAdmin;
use crate::audit::{AuditEntry, AuditLog};
use crate::retention::{self, RetentionPolicyStore, Tier};
use axum::extract::Path;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetentionPolicyDto {
    /// `free` | `supporter` (closed vocabulary; see retention::Tier).
    pub tier: String,
    /// Number of days events are kept for users on this tier. `None`
    /// (omitted in JSON) means unlimited retention -- no purge runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetentionPoliciesResponse {
    pub policies: Vec<RetentionPolicyDto>,
}

/// A tier's new window. `retention_days` null or absent means unlimited:
/// nothing is purged for that tier.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetRetentionPolicy {
    #[serde(default)]
    pub retention_days: Option<i32>,
}

/// The longest window that is still a window: ten years. Anything longer
/// is what unlimited (null) is for.
pub const MAX_RETENTION_DAYS: i32 = 3650;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetentionPurgeResponse {
    pub users_considered: u64,
    pub users_unlimited: u64,
    pub users_purged: u64,
    pub events_deleted: u64,
    pub users_truncated: u64,
}

/// GET /v1/admin/retention/policies -- list seeded tiers + windows.
#[utoipa::path(
    get,
    path = "/v1/admin/retention/policies",
    tag = "admin",
    responses(
        (status = 200, description = "Current retention policies", body = RetentionPoliciesResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_policies(
    _: RequireAdmin,
    Extension(store): Extension<Arc<dyn RetentionPolicyStore>>,
) -> Response {
    match store.list_all().await {
        Ok(policies) => {
            let dtos: Vec<RetentionPolicyDto> = policies
                .into_iter()
                .map(|p| RetentionPolicyDto {
                    tier: p.tier.as_str().to_string(),
                    retention_days: p.retention_days,
                })
                .collect();
            (
                StatusCode::OK,
                Json(RetentionPoliciesResponse { policies: dtos }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "retention policy list failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "retention_policy_list_failed"})),
            )
                .into_response()
        }
    }
}

/// POST /v1/admin/retention/purge -- run a sweep right now.
/// Synchronous: returns the sweep summary when the pass completes.
/// The scheduled loop in main.rs runs the same code on a 24h cadence;
/// this endpoint exists for ad-hoc operator runs.
/// PUT /v1/admin/retention/policies/{tier} -- change a tier's window.
/// Admin only, audited as `retention.policy_changed`. The next sweep uses
/// it; shortening a window deletes events that fall outside it then.
#[utoipa::path(
    put,
    path = "/v1/admin/retention/policies/{tier}",
    tag = "admin",
    params(("tier" = String, Path, description = "free | supporter")),
    request_body = SetRetentionPolicy,
    responses(
        (status = 200, description = "The policies after the change", body = RetentionPoliciesResponse),
        (status = 400, description = "invalid_retention_days"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 404, description = "unknown_tier"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn set_policy(
    RequireAdmin(admin): RequireAdmin,
    Path(tier): Path<String>,
    Extension(store): Extension<Arc<dyn RetentionPolicyStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(body): Json<SetRetentionPolicy>,
) -> Response {
    let Some(tier) = Tier::parse(&tier) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "unknown_tier"})),
        )
            .into_response();
    };
    if body
        .retention_days
        .is_some_and(|d| !(1..=MAX_RETENTION_DAYS).contains(&d))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid_retention_days"})),
        )
            .into_response();
    }
    let before = match store.list_all().await {
        Ok(p) => p
            .into_iter()
            .find(|p| p.tier == tier)
            .and_then(|p| p.retention_days),
        Err(e) => {
            tracing::error!(error = %e, "retention policy read failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "retention_policy_update_failed"})),
            )
                .into_response();
        }
    };
    if let Err(e) = store.set(tier, body.retention_days).await {
        tracing::error!(error = %e, "retention policy update failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "retention_policy_update_failed"})),
        )
            .into_response();
    }
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(admin.sub.clone()),
            actor_handle: Some(admin.preferred_username.clone()),
            action: "retention.policy_changed".into(),
            payload: serde_json::json!({
                "tier": tier.as_str(),
                "from_days": before,
                "to_days": body.retention_days,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed");
    }
    list_policies(RequireAdmin(admin), Extension(store)).await
}

#[utoipa::path(
    post,
    path = "/v1/admin/retention/purge",
    tag = "admin",
    responses(
        (status = 200, description = "Sweep completed", body = RetentionPurgeResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 500, description = "Sweep error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn trigger_purge(
    _: RequireAdmin,
    Extension(pool): Extension<PgPool>,
    Extension(store): Extension<Arc<dyn RetentionPolicyStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
) -> Response {
    match retention::run_sweep(&pool, store.as_ref(), audit.as_ref()).await {
        Ok(summary) => (
            StatusCode::OK,
            Json(RetentionPurgeResponse {
                users_considered: summary.users_considered,
                users_unlimited: summary.users_unlimited,
                users_purged: summary.users_purged,
                events_deleted: summary.events_deleted,
                users_truncated: summary.users_truncated,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "retention sweep (admin-triggered) failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "retention_sweep_failed"})),
            )
                .into_response()
        }
    }
}

/// Parameterless router. The two endpoints read everything they need
/// off Extension layers installed in main.rs (the policy store, the
/// PgPool used for the sweep, the audit log).
pub fn router() -> Router {
    Router::new()
        .route("/v1/admin/retention/policies", get(list_policies))
        .route("/v1/admin/retention/policies/{tier}", put(set_policy))
        .route("/v1/admin/retention/purge", post(trigger_purge))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::retention::test_support::MemoryRetentionPolicyStore;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;
    use uuid::Uuid;

    struct Fixture {
        app: Router,
        store: Arc<MemoryRetentionPolicyStore>,
        audit: Arc<MemoryAuditLog>,
        admin: String,
        moderator: String,
    }

    async fn fixture() -> Fixture {
        let store = Arc::new(MemoryRetentionPolicyStore::with_defaults());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let (admin_id, mod_id) = (Uuid::now_v7(), Uuid::now_v7());
        staff
            .grant(admin_id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        staff
            .grant(mod_id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        let app = router()
            .layer(Extension(Arc::new(verifier)))
            .layer(Extension(store.clone() as Arc<dyn RetentionPolicyStore>))
            .layer(Extension(audit.clone() as Arc<dyn AuditLog>))
            .layer(Extension(staff as Arc<dyn StaffRoleStore>));
        Fixture {
            app,
            store,
            audit,
            admin: issuer.sign_user(&admin_id.to_string(), "Admin").unwrap(),
            moderator: issuer.sign_user(&mod_id.to_string(), "Mod").unwrap(),
        }
    }

    async fn put(
        f: &Fixture,
        token: &str,
        tier: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/v1/admin/retention/policies/{tier}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = f.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    async fn days(f: &Fixture, tier: Tier) -> Option<i32> {
        f.store
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.tier == tier)
            .unwrap()
            .retention_days
    }

    #[tokio::test]
    async fn an_admin_sets_a_window_and_it_is_audited() {
        let f = fixture().await;
        let (s, v) = put(&f, &f.admin, "free", r#"{"retention_days":365}"#).await;
        assert_eq!(s, StatusCode::OK);
        assert!(v["policies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["tier"] == "free" && p["retention_days"] == 365));
        assert_eq!(days(&f, Tier::Free).await, Some(365));
        let e = f.audit.snapshot().pop().unwrap();
        assert_eq!(e.action, "retention.policy_changed");
        assert_eq!(e.payload["from_days"], 90);
        assert_eq!(e.payload["to_days"], 365);
        // Null means unlimited.
        let (s, _) = put(&f, &f.admin, "free", r#"{"retention_days":null}"#).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(days(&f, Tier::Free).await, None);
    }

    #[tokio::test]
    async fn only_admins_and_only_sane_windows() {
        let f = fixture().await;
        let (s, _) = put(&f, &f.moderator, "free", r#"{"retention_days":365}"#).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        for bad in ["0", "-1", "3651"] {
            let (s, v) = put(
                &f,
                &f.admin,
                "free",
                &format!(r#"{{"retention_days":{bad}}}"#),
            )
            .await;
            assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
            assert_eq!(v["error"], "invalid_retention_days");
        }
        let (s, v) = put(&f, &f.admin, "gold", r#"{"retention_days":30}"#).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "unknown_tier");
        // Nothing changed, nothing audited.
        assert_eq!(days(&f, Tier::Free).await, Some(90));
        assert!(f.audit.snapshot().is_empty());
    }
}
