//! Routes for per-widget sharing toggles — Plan 3b Option A.
//!
//! Three endpoints:
//!
//! - `GET /v1/users/me/share-scopes`  — owner reads own scopes.
//! - `PUT /v1/users/me/share-scopes`  — owner writes own scopes + audit row.
//! - `GET /v1/public/:handle/share-scopes` — visitor reads the owner's scopes
//!   after a SpiceDB public-visibility check. Used by the web layer to thread
//!   `shareScopes` into `ViewerCtx` before `isAvailable` runs.

use crate::audit::{AuditEntry, AuditLog, ACTION_SHARE_SCOPES_UPDATED};
use crate::auth::AuthenticatedUser;
use crate::share_scopes::{ShareScopesStore, WidgetShareScopes};
use crate::spicedb::PublicAccessChecker;
use crate::validation::validate_handle;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the share-scopes sub-router. Merge into the main router.
pub fn routes() -> Router {
    Router::new()
        .route(
            "/v1/users/me/share-scopes",
            get(get_share_scopes).put(put_share_scopes),
        )
        .route("/v1/public/:handle/share-scopes", get(public_share_scopes))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /v1/users/me/share-scopes
///
/// Owner-only. Returns the current per-widget visibility toggles.
/// All fields default to `false` (private) if the owner has never saved.
#[utoipa::path(
    get,
    path = "/v1/users/me/share-scopes",
    responses(
        (status = 200, description = "Current widget share scopes", body = WidgetShareScopes),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = [])),
    tag = "share-scopes",
)]
pub async fn get_share_scopes(
    auth: AuthenticatedUser,
    Extension(store): Extension<Arc<dyn ShareScopesStore>>,
) -> Result<Json<WidgetShareScopes>, StatusCode> {
    let scopes = store.get(&auth.preferred_username).await.map_err(|e| {
        tracing::warn!(err = %e, "share_scopes get failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(scopes))
}

/// PUT /v1/users/me/share-scopes
///
/// Owner-only. Validates + writes the new scopes and emits an audit row.
/// Validation errors return **400** — never 401, which would trigger the
/// client auto-logout interceptor.
#[utoipa::path(
    put,
    path = "/v1/users/me/share-scopes",
    request_body = WidgetShareScopes,
    responses(
        (status = 200, description = "Scopes written; response carries the canonical form", body = WidgetShareScopes),
        (status = 400, description = "Malformed request body"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = [])),
    tag = "share-scopes",
)]
pub async fn put_share_scopes(
    auth: AuthenticatedUser,
    Extension(store): Extension<Arc<dyn ShareScopesStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(req): Json<WidgetShareScopes>,
) -> Result<Json<WidgetShareScopes>, StatusCode> {
    // Fetch prior scopes for the audit diff. Best-effort — a failure here
    // yields a less-useful diff, not a request failure.
    let prev = store.get(&auth.preferred_username).await.ok();

    store
        .put(&auth.preferred_username, &req)
        .await
        .map_err(|e| {
            tracing::warn!(err = %e, "share_scopes put failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Best-effort audit emission per docs/ENGINEERING.md — a hiccup here must
    // never poison the response.
    let diff = render_diff(prev.as_ref(), &req);
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: ACTION_SHARE_SCOPES_UPDATED.to_string(),
            payload: json!({ "diff": diff }),
        })
        .await
    {
        tracing::warn!(err = %e, "audit emit failed for share_scopes.updated");
    }

    Ok(Json(req))
}

/// GET /v1/public/:handle/share-scopes
///
/// Visitor endpoint. Returns the owner's widget share scopes after
/// verifying that the owner's public-visibility toggle is on (SpiceDB
/// `view` permission check). Anonymous callers are welcome — the check
/// is against the public `user:*` subject.
///
/// Returns 404 when the profile is not public (same policy as
/// `/v1/public/:handle/summary`). Returns 503 when SpiceDB is unavailable.
#[utoipa::path(
    get,
    path = "/v1/public/{handle}/share-scopes",
    params(
        ("handle" = String, Path, description = "RSI handle whose share scopes to fetch"),
    ),
    responses(
        (status = 200, description = "Owner's widget share scopes", body = WidgetShareScopes),
        (status = 404, description = "Profile not public or handle not found"),
        (status = 503, description = "SpiceDB unavailable"),
    ),
    tag = "share-scopes",
)]
pub async fn public_share_scopes(
    Path(handle): Path<String>,
    Extension(checker): Extension<Arc<Option<Arc<dyn PublicAccessChecker>>>>,
    Extension(store): Extension<Arc<dyn ShareScopesStore>>,
) -> Response {
    // Reject malformed handles before they reach SpiceDB or Postgres.
    // Mirrors the gate used by public_summary / public_timeline in sharing_routes.rs.
    if !validate_handle(&handle) {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    let checker = match checker.as_ref().as_ref() {
        Some(c) => c,
        None => {
            tracing::warn!("public_share_scopes: SpiceDB not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "spicedb_unavailable" })),
            )
                .into_response();
        }
    };

    let allowed = match checker.check_public_access(&handle).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(err = %e, handle = %handle, "SpiceDB check_public_access failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "spicedb_unavailable" })),
            )
                .into_response();
        }
    };

    if !allowed {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response();
    }

    match store.get(&handle).await {
        Ok(scopes) => (StatusCode::OK, Json(scopes)).into_response(),
        Err(e) => {
            tracing::warn!(err = %e, handle = %handle, "share_scopes get failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error" })),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Audit diff helper
// ---------------------------------------------------------------------------

fn render_diff(prev: Option<&WidgetShareScopes>, next: &WidgetShareScopes) -> String {
    // Iterate over the canonical widget-key list from `WidgetShareScopes::iter()`.
    // Adding a 6th field to that method is the only source-of-truth update needed.
    let prev_default = WidgetShareScopes::default();
    let prev = prev.unwrap_or(&prev_default);

    let parts: Vec<String> = prev
        .iter()
        .zip(next.iter())
        .filter_map(|((name, prev_val), (_, next_val))| {
            if prev_val != next_val {
                Some(format!(
                    "{}:{}",
                    name,
                    if next_val { "shared" } else { "private" }
                ))
            } else {
                None
            }
        })
        .collect();

    if parts.is_empty() {
        "no-op".to_string()
    } else {
        parts.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::share_scopes::test_support::MemoryShareScopesStore;
    use crate::spicedb::test_support::StubAccessChecker;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Build a minimal router wiring the public route only.
    ///
    /// Pass `None` to simulate SpiceDB being unconfigured (→ 503 branch).
    /// Pass `Some(Arc::new(StubAccessChecker::allowed()))` etc. to exercise
    /// the SpiceDB-allowed/denied paths without a live sidecar.
    fn public_app(
        checker: Arc<Option<Arc<dyn PublicAccessChecker>>>,
        store: Arc<dyn ShareScopesStore>,
    ) -> axum::Router {
        Router::new()
            .route("/v1/public/:handle/share-scopes", get(public_share_scopes))
            .layer(Extension(checker))
            .layer(Extension(store))
    }

    fn mint_token(issuer: &crate::auth::TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&Uuid::new_v4().to_string(), handle)
            .expect("sign_user")
    }

    async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    fn owner_app(
        store: Arc<dyn ShareScopesStore>,
        audit: Arc<dyn AuditLog>,
        verifier: Arc<crate::auth::AuthVerifier>,
    ) -> axum::Router {
        // Only wire the owner routes (/v1/users/me/share-scopes) for the
        // owner tests — public route requires a SpiceDB extension we skip here.
        Router::new()
            .route(
                "/v1/users/me/share-scopes",
                get(get_share_scopes).put(put_share_scopes),
            )
            .layer(Extension(store))
            .layer(Extension(audit))
            .layer(Extension(verifier))
    }

    // -- Test 1: GET returns all-false for a fresh user ----------------

    #[tokio::test]
    async fn get_returns_defaults_when_unset() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = owner_app(store, audit, Arc::new(verifier));

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/share-scopes")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["combat_mission"], false);
        assert_eq!(body["economy"], false);
        assert_eq!(body["travel"], false);
        assert_eq!(body["records"], false);
        assert_eq!(body["recent_activity"], false);
    }

    // -- Test 2: PUT then GET roundtrips -------------------------------

    #[tokio::test]
    async fn put_then_get_roundtrips() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let verifier = Arc::new(verifier);
        let app = owner_app(store, audit, verifier.clone());

        let put_body = serde_json::to_vec(&serde_json::json!({
            "combat_mission": true,
            "economy": false,
            "travel": true,
            "records": false,
            "recent_activity": false,
        }))
        .unwrap();

        let put_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/share-scopes")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(put_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_res.status(), StatusCode::OK);

        let get_res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/share-scopes")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let (status, body) = read_body(get_res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["combat_mission"], true);
        assert_eq!(body["travel"], true);
        assert_eq!(body["economy"], false);
    }

    // -- Test 3: PUT emits an audit row --------------------------------

    #[tokio::test]
    async fn put_emits_audit_row() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = owner_app(
            store,
            audit.clone() as Arc<dyn AuditLog>,
            Arc::new(verifier),
        );

        let put_body = serde_json::to_vec(&serde_json::json!({
            "combat_mission": true,
            "economy": false,
            "travel": false,
            "records": false,
            "recent_activity": false,
        }))
        .unwrap();

        let put_res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/share-scopes")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(put_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(put_res.status(), StatusCode::OK);

        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, ACTION_SHARE_SCOPES_UPDATED);
        assert_eq!(entries[0].actor_handle.as_deref(), Some("Alice"));

        let diff_str = entries[0]
            .payload
            .get("diff")
            .and_then(|v| v.as_str())
            .expect("diff must be a string in payload");
        assert!(
            diff_str.contains("combat_mission:shared"),
            "diff should mention combat_mission toggle, got: {diff_str}"
        );
    }

    // -- Test 4: GET without token returns 401 -------------------------

    #[tokio::test]
    async fn get_without_token_returns_401() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (_issuer, verifier) = fresh_pair();
        let app = owner_app(store, audit, Arc::new(verifier));

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/share-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    // -- Test 5: render_diff produces "no-op" when nothing changes -----

    #[test]
    fn render_diff_no_op_when_unchanged() {
        let scopes = WidgetShareScopes::default();
        let diff = render_diff(Some(&scopes), &scopes);
        assert_eq!(diff, "no-op");
    }

    // -- Test 6: public route rejects malformed handle (validate_handle gate) --
    //
    // A handle containing a space ("in valid") never reaches SpiceDB.
    // Returns 404 immediately — same posture as public_summary.

    #[tokio::test]
    async fn public_route_rejects_invalid_handle() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        // No SpiceDB configured — handler should 404 before ever inspecting it.
        let app = public_app(Arc::new(None), store);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/public/in%20valid/share-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    // -- Test 7: public route returns 503 when SpiceDB is not configured -------
    //
    // `Arc<Option<SpicedbClient>>` = `Arc::new(None)` simulates the
    // degraded-mode path where SpiceDB was not reachable at startup.
    // A valid handle is used so the validate_handle gate is passed.

    #[tokio::test]
    async fn public_route_503_when_spicedb_unconfigured() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        let app = public_app(Arc::new(None), store);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/public/SomeUser/share-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "spicedb_unavailable");
    }

    // -- Test 8: render_diff uses iter() — adding a 6th field only requires
    //    updating WidgetShareScopes::iter(), not render_diff itself.

    #[test]
    fn render_diff_uses_canonical_iter() {
        // Verify that render_diff picks up all fields from iter() by checking
        // that a full flip from all-false to all-true produces 5 entries.
        let prev = WidgetShareScopes::default();
        let next = WidgetShareScopes {
            combat_mission: true,
            economy: true,
            travel: true,
            records: true,
            recent_activity: true,
        };
        let diff = render_diff(Some(&prev), &next);
        // All 5 widgets should appear in the diff string.
        assert!(
            diff.contains("combat_mission:shared"),
            "missing combat_mission"
        );
        assert!(diff.contains("economy:shared"), "missing economy");
        assert!(diff.contains("travel:shared"), "missing travel");
        assert!(diff.contains("records:shared"), "missing records");
        assert!(
            diff.contains("recent_activity:shared"),
            "missing recent_activity"
        );
        // Exactly 5 comma-separated entries.
        assert_eq!(
            diff.split(", ").count(),
            5,
            "expected 5 diff entries, got: {diff}"
        );
    }

    // -- Test 9: SpiceDB denies → 404 (does not leak whether the owner exists
    //    or whether the column is null vs populated). Uses StubAccessChecker::denied()
    //    so no live SpiceDB sidecar is needed.

    #[tokio::test]
    async fn public_route_404_when_spicedb_denies() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        // Pre-populate so we can confirm the gate fires BEFORE the store
        // would have returned the toggled-on scopes.
        store
            .put(
                "Alice",
                &WidgetShareScopes {
                    combat_mission: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let checker: Arc<dyn PublicAccessChecker> = Arc::new(StubAccessChecker::denied());
        let app = public_app(Arc::new(Some(checker)), store);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/public/Alice/share-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    // -- Test 10: SpiceDB allows → 200 with the stored scopes JSON. Verifies
    //    the happy-path read flow end-to-end, again without a live sidecar.

    #[tokio::test]
    async fn public_route_200_with_scopes_when_spicedb_allows() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        store
            .put(
                "Alice",
                &WidgetShareScopes {
                    combat_mission: true,
                    economy: false,
                    travel: true,
                    records: false,
                    recent_activity: false,
                },
            )
            .await
            .unwrap();
        let checker: Arc<dyn PublicAccessChecker> = Arc::new(StubAccessChecker::allowed());
        let app = public_app(Arc::new(Some(checker)), store);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/public/Alice/share-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let body: WidgetShareScopes = serde_json::from_slice(&bytes).unwrap();
        assert!(body.combat_mission, "combat_mission should be true");
        assert!(!body.economy, "economy should be false");
        assert!(body.travel, "travel should be true");
        assert!(!body.records, "records should be false");
        assert!(!body.recent_activity, "recent_activity should be false");
    }

    // -- Test 11: SpiceDB is configured but the RPC fails (network blip,
    //    sidecar restart, transient unavailability). Covers the Err(_) arm of
    //    the match in public_share_scopes — semantically equivalent to the
    //    503 path but reached through a different control-flow branch than
    //    the "extension is None" case in test 7.

    #[tokio::test]
    async fn public_route_503_when_spicedb_rpc_fails() {
        let store: Arc<dyn ShareScopesStore> = Arc::new(MemoryShareScopesStore::default());
        let checker: Arc<dyn PublicAccessChecker> = Arc::new(StubAccessChecker::failing());
        let app = public_app(Arc::new(Some(checker)), store);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/public/Alice/share-scopes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "spicedb_unavailable");
    }
}
