//! Sitewide appearance config: a public read (so the signed-out shell
//! can stamp the default wave speed before any auth exists) plus a
//! moderator-gated admin console to change it. Mirrors
//! `waitlist_routes.rs`'s public-status / admin-config split.

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::appearance::AppearanceStore;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Theme-switch wave animation speed allowlist. Must stay in sync
/// with `preferences_routes::ALLOWED_WAVE_SPEEDS` (the per-user
/// override) and the web's `WAVE_SPEED_MS` map.
const ALLOWED_WAVE_SPEEDS: &[&str] = &["fast", "normal", "off", "slow"];

/// Build the appearance sub-router. `AppearanceStore` extension is
/// layered on the outer router in `main`.
pub fn routes() -> Router {
    Router::new()
        .route("/v1/appearance", get(public_get))
        .route("/v1/admin/appearance", get(admin_get).put(admin_put))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AppearanceConfigApi {
    /// One of `off`, `slow`, `normal`, `fast`.
    pub theme_wave_speed: String,
}

fn error(
    status: StatusCode,
    code: &'static str,
    detail: Option<String>,
) -> axum::response::Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/appearance",
    tag = "appearance",
    operation_id = "appearance_get_public",
    responses(
        (status = 200, description = "Sitewide appearance defaults", body = AppearanceConfigApi),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn public_get(
    Extension(store): Extension<Arc<dyn AppearanceStore>>,
) -> impl IntoResponse {
    match store.get_wave_speed().await {
        Ok(theme_wave_speed) => Json(AppearanceConfigApi { theme_wave_speed }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "appearance get_wave_speed failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "appearance_failed", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/appearance",
    tag = "appearance",
    operation_id = "appearance_admin_get",
    responses(
        (status = 200, description = "Sitewide appearance defaults", body = AppearanceConfigApi),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_get(
    RequireModerator(_user): RequireModerator,
    Extension(store): Extension<Arc<dyn AppearanceStore>>,
) -> impl IntoResponse {
    match store.get_wave_speed().await {
        Ok(theme_wave_speed) => Json(AppearanceConfigApi { theme_wave_speed }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "appearance admin get failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "appearance_failed", None)
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1/admin/appearance",
    tag = "appearance",
    operation_id = "appearance_admin_put",
    request_body = AppearanceConfigApi,
    responses(
        (status = 200, description = "Saved config as stored", body = AppearanceConfigApi),
        (status = 400, description = "Invalid theme_wave_speed", body = ApiErrorBody),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_put(
    RequireModerator(user): RequireModerator,
    Extension(store): Extension<Arc<dyn AppearanceStore>>,
    Json(cfg): Json<AppearanceConfigApi>,
) -> impl IntoResponse {
    if !ALLOWED_WAVE_SPEEDS.contains(&cfg.theme_wave_speed.as_str()) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_wave_speed",
            Some(format!(
                "theme_wave_speed must be one of {ALLOWED_WAVE_SPEEDS:?}; got {:?}",
                cfg.theme_wave_speed
            )),
        );
    }

    let by = Uuid::parse_str(&user.sub).ok();
    match store.set_wave_speed(&cfg.theme_wave_speed, by).await {
        Ok(()) => {
            tracing::info!(
                theme_wave_speed = %cfg.theme_wave_speed,
                "appearance config changed"
            );
            Json(cfg).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "appearance set_wave_speed failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::test_support::MemoryAppearanceStore;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    /// Public route only, no auth extension needed.
    fn public_app(store: Arc<dyn AppearanceStore>) -> Router {
        Router::new()
            .route("/v1/appearance", get(public_get))
            .layer(Extension(store))
    }

    /// Admin routes with the full extension stack: verifier (so the
    /// JWT extractor works) + staff-role store (so RequireModerator
    /// can evaluate the gate) — same fixture shape as
    /// `admin_submission_routes.rs::build_app`.
    fn admin_app(
        store: Arc<dyn AppearanceStore>,
        staff: Arc<dyn StaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        Router::new()
            .route("/v1/admin/appearance", get(admin_get).put(admin_put))
            .layer(Extension(store))
            .layer(Extension(staff))
            .layer(Extension(verifier))
    }

    fn issue_token(issuer: &TokenIssuer, user_id: Uuid, handle: &str) -> String {
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign user token")
    }

    /// Store + staff-role fixture with one moderator and one plain
    /// user, mirroring `admin_submission_routes.rs::fixture`.
    async fn fixture() -> (
        Arc<dyn AppearanceStore>,
        Router,
        TokenIssuer,
        Uuid, // mod_user
        Uuid, // plain_user
    ) {
        let store: Arc<dyn AppearanceStore> = Arc::new(MemoryAppearanceStore::new());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();

        let mod_user = Uuid::now_v7();
        let plain_user = Uuid::now_v7();
        staff
            .grant(mod_user, StaffRole::Moderator, None, None)
            .await
            .unwrap();

        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let app = admin_app(store.clone(), staff_dyn, Arc::new(verifier));
        (store, app, issuer, mod_user, plain_user)
    }

    // -- Test 1: public read returns the default ----------------------

    #[tokio::test]
    async fn public_get_returns_default_normal() {
        let store: Arc<dyn AppearanceStore> = Arc::new(MemoryAppearanceStore::new());
        let app = public_app(store);
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/appearance")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["theme_wave_speed"], "normal");
    }

    // -- Test 2: public read reflects an admin-set value ---------------

    #[tokio::test]
    async fn public_get_reflects_stored_value() {
        let store = Arc::new(MemoryAppearanceStore::new());
        store.set_wave_speed("fast", None).await.unwrap();
        let dyn_store: Arc<dyn AppearanceStore> = store;
        let app = public_app(dyn_store);
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/appearance")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (_, body) = read_body(res).await;
        assert_eq!(body["theme_wave_speed"], "fast");
    }

    // -- Test 3: public route has no admin surface (no PUT route) -----

    #[tokio::test]
    async fn public_route_does_not_expose_put() {
        let store: Arc<dyn AppearanceStore> = Arc::new(MemoryAppearanceStore::new());
        let app = public_app(store);
        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/appearance")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"theme_wave_speed":"fast"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    // -- Test 4: moderator PUT stores a valid value --------------------

    #[tokio::test]
    async fn moderator_put_stores_a_valid_value() {
        let (store, app, issuer, mod_user, _plain) = fixture().await;
        let token = issue_token(&issuer, mod_user, "mod");

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/admin/appearance")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"theme_wave_speed":"slow"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["theme_wave_speed"], "slow");
        assert_eq!(store.get_wave_speed().await.unwrap(), "slow");
    }

    // -- Test 5: moderator PUT rejects an invalid value ----------------

    #[tokio::test]
    async fn moderator_put_rejects_invalid_speed() {
        let (store, app, issuer, mod_user, _plain) = fixture().await;
        let token = issue_token(&issuer, mod_user, "mod");

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/admin/appearance")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"theme_wave_speed":"ludicrous"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_wave_speed");
        // The bad write must not have landed.
        assert_eq!(store.get_wave_speed().await.unwrap(), "normal");
    }

    // -- Test 6: a non-moderator is refused ----------------------------

    #[tokio::test]
    async fn non_moderator_is_refused() {
        let (_store, app, issuer, _mod_user, plain_user) = fixture().await;
        let token = issue_token(&issuer, plain_user, "rando");

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/admin/appearance")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    // -- Test 7: admin GET reflects the current value ------------------

    #[tokio::test]
    async fn admin_get_reflects_current_value() {
        let (store, app, issuer, mod_user, _plain) = fixture().await;
        store.set_wave_speed("off", None).await.unwrap();
        let token = issue_token(&issuer, mod_user, "mod");

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/admin/appearance")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["theme_wave_speed"], "off");
    }

    // -- Test 8: an unauthenticated request is rejected ----------------

    #[tokio::test]
    async fn missing_bearer_token_is_rejected() {
        let (_store, app, _issuer, _mod_user, _plain) = fixture().await;

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/admin/appearance")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_client_error());
    }
}
