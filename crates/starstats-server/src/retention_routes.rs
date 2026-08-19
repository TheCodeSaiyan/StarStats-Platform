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
use crate::audit::AuditLog;
use crate::retention::{self, RetentionPolicyStore};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
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
        .route("/v1/admin/retention/purge", post(trigger_purge))
}
