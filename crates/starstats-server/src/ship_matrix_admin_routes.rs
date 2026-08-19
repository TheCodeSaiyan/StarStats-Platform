//! Admin endpoint for the Ship Matrix runtime config.
//!
//! Gated by [`RequireAdmin`]: only an authenticated platform `admin` can
//! read or flip the media kill-switch.
//!
//!   * `GET /v1/admin/ship-matrix` — current `media_enabled` flag.
//!   * `PUT /v1/admin/ship-matrix` — persist + hot-swap the flag.
//!
//! Hot reload: a successful `PUT` writes the DB row AND updates the
//! shared [`AtomicBool`] that the media proxy reads on every request, so
//! the change takes effect immediately with no redeploy and no
//! per-request DB hit (mirrors the SMTP `SwappableMailer` hot-swap).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::admin_routes::RequireAdmin;
use crate::api_error::ApiErrorBody;
use crate::ship_matrix_config_store::ShipMatrixConfigStore;

// -- DTOs ------------------------------------------------------------

/// Shape returned by `GET /v1/admin/ship-matrix`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ShipMatrixConfigResponse {
    /// Whether RSI ship images are surfaced (the media proxy + gallery).
    pub media_enabled: bool,
}

/// Request body for `PUT /v1/admin/ship-matrix`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ShipMatrixConfigRequest {
    pub media_enabled: bool,
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/ship-matrix",
    tag = "admin-ship-matrix",
    operation_id = "admin_ship_matrix_get",
    responses(
        (status = 200, description = "Current Ship Matrix config", body = ShipMatrixConfigResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is not an admin", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_config<C: ShipMatrixConfigStore>(
    State((store, _flag)): State<(Arc<C>, Arc<AtomicBool>)>,
    _admin: RequireAdmin,
) -> Response {
    match store.get_media_enabled().await {
        Ok(media_enabled) => (
            StatusCode::OK,
            Json(ShipMatrixConfigResponse { media_enabled }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "ship_matrix_config get failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1/admin/ship-matrix",
    tag = "admin-ship-matrix",
    operation_id = "admin_ship_matrix_put",
    request_body = ShipMatrixConfigRequest,
    responses(
        (status = 200, description = "Config persisted; media flag hot-swapped", body = ShipMatrixConfigResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is not an admin", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn put_config<C: ShipMatrixConfigStore>(
    State((store, flag)): State<(Arc<C>, Arc<AtomicBool>)>,
    RequireAdmin(admin): RequireAdmin,
    Json(req): Json<ShipMatrixConfigRequest>,
) -> Response {
    let admin_id = Uuid::parse_str(&admin.sub).ok();

    if let Err(e) = store.set_media_enabled(req.media_enabled, admin_id).await {
        tracing::error!(error = %e, "ship_matrix_config put failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }

    // Hot-swap the in-memory flag the media proxy reads — change is
    // effective immediately, no redeploy.
    flag.store(req.media_enabled, Ordering::Relaxed);
    tracing::info!(
        media_enabled = req.media_enabled,
        admin = %admin.sub,
        "ship matrix media flag updated by admin"
    );

    (
        StatusCode::OK,
        Json(ShipMatrixConfigResponse {
            media_enabled: req.media_enabled,
        }),
    )
        .into_response()
}

// -- Router ----------------------------------------------------------

/// Build the Ship-Matrix-admin sub-router. The caller supplies the
/// shared `media_flag` [`AtomicBool`] (the same handle the media proxy
/// router reads) so a `PUT` takes effect immediately. Admin-baseline
/// extensions (`AuthVerifier`, `dyn StaffRoleStore`) are layered on the
/// outer app, same as the other admin routers.
pub fn router<C: ShipMatrixConfigStore>(
    config_store: Arc<C>,
    media_flag: Arc<AtomicBool>,
) -> Router {
    Router::new()
        .route(
            "/v1/admin/ship-matrix",
            get(get_config::<C>).put(put_config::<C>),
        )
        .with_state((config_store, media_flag))
}
