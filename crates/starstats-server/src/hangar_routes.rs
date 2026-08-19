//! Hangar snapshot endpoints.
//!
//! Once the desktop tray has scraped the user's RSI hangar page, it
//! POSTs the parsed list of ships to `POST /v1/me/hangar`. The server
//! stamps `captured_at` and persists the snapshot keyed on the
//! authenticated user. `GET /v1/me/hangar` returns the most-recent
//! snapshot for the caller.
//!
//! Both endpoints are user-token only — device tokens cannot read or
//! push hangars (the hangar belongs to a user, not a paired device).
//! Responses share the `ApiErrorBody` envelope with the rest of the
//! API.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::hangar_store::{HangarSnapshot, HangarStore};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use starstats_core::wire::HangarShip;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use utoipa::ToSchema;
use uuid::Uuid;

/// Defensive ceilings on a single push. RSI hangars top out at a few
/// hundred ships; per-field strings are short identifiers + a name.
/// Validating both keeps a runaway tray-side parser from filling JSONB
/// rows with megabyte payloads.
const MAX_SHIPS_PER_PUSH: usize = 5000;
const MAX_FIELD_CHARS: usize = 200;
/// 1 MB hard cap on the request body. 5000 typical ships ≈ 750 KB; this
/// is roomy enough for normal pushes but bounds the parse cost so the
/// post-deserialise count + per-field checks have something tighter
/// than Axum's 2 MB default sitting in front of them.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Build the `/v1/me/hangar` sub-router. Per-IP rate limited (1/s with
/// burst 5) — the tray pushes once per refresh cycle and the dashboard
/// reads on page load, so the limit is generous for legitimate use
/// while bounding a compromised tray that would otherwise hammer the
/// upsert path.
pub fn routes<S: HangarStore>(store: Arc<S>) -> Router {
    let governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("hangar governor config builder produced no config"),
    );
    Router::new()
        .route("/v1/me/hangar", post(push::<S>).get(me::<S>))
        .with_state(store)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(GovernorLayer { config: governor })
}

// Schema-only mirrors of `starstats_core::wire::{HangarPushRequest,
// HangarShip}`. The real types live in `starstats-core`, which
// intentionally has no `utoipa` dependency (the desktop client pulls
// it in too — we don't want a server-side schema crate leaking down).
//
// **Keep these in sync with the core types — drift here silently
// breaks OpenAPI clients without a compile error.**
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HangarPushRequestSchema {
    pub schema_version: u16,
    pub ships: Vec<HangarShipSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HangarShipSchema {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pledge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Constituent items for a bundle/pack pledge (from the RSI
    /// "Contains:" list). Empty for a plain single-item pledge. Pure
    /// pass-through — persisted verbatim in the snapshot JSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contains: Vec<String>,
}

// `require_user_token` was previously a gate that 403'd anything but
// user-session JWTs. The intent (per its original comment: "the
// desktop client uses its paired user JWT, not its device JWT, for
// these endpoints") was never wired — pairing only mints device
// JWTs, so the tray was the ONLY system that could fetch hangar
// data (via the user's RSI session cookie, held in the OS keychain
// on the tray's host) and the gate left it unable to deliver. Gate
// removed; both endpoints now accept any authenticated JWT for the
// caller's own user. Authentication identity is still verified by
// `AuthenticatedUser`; ingest-adjacent code cross-checks the bearer
// against `claimed_handle` so a token can't push under another
// user's handle.

fn error(status: StatusCode, code: &'static str, detail: Option<String>) -> Response {
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
    post,
    path = "/v1/me/hangar",
    tag = "hangar",
    operation_id = "hangar_push",
    request_body = HangarPushRequestSchema,
    responses(
        (status = 200, description = "Snapshot persisted", body = HangarSnapshot),
        (status = 400, description = "Schema-level rejection", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn push<S: HangarStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
    Json(body): Json<starstats_core::wire::HangarPushRequest>,
) -> Response {
    if body.schema_version != 1 {
        return error(
            StatusCode::BAD_REQUEST,
            "unsupported_schema_version",
            Some(format!("got {}, server speaks 1", body.schema_version)),
        );
    }

    if body.ships.len() > MAX_SHIPS_PER_PUSH {
        return error(
            StatusCode::BAD_REQUEST,
            "ships_too_large",
            Some(format!(
                "got {} ships, limit is {MAX_SHIPS_PER_PUSH}",
                body.ships.len()
            )),
        );
    }

    if let Some(reason) = validate_ships(&body.ships) {
        return error(StatusCode::BAD_REQUEST, "invalid_ship", Some(reason));
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    match store.put_snapshot(user_id, &body.ships).await {
        Ok(snapshot) => (StatusCode::OK, Json(snapshot)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "put_snapshot failed in /v1/me/hangar");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

/// Cap each per-ship string at `MAX_FIELD_CHARS` and require a non-empty
/// `name`. Without this, a single ship with a megabyte `name` would slip
/// past the `MAX_SHIPS_PER_PUSH` count check and bloat both JSONB
/// storage and every subsequent GET.
fn validate_ships(ships: &[HangarShip]) -> Option<String> {
    for (i, ship) in ships.iter().enumerate() {
        if ship.name.trim().is_empty() {
            return Some(format!("ship[{i}].name is empty"));
        }
        if ship.name.chars().count() > MAX_FIELD_CHARS {
            return Some(format!("ship[{i}].name exceeds {MAX_FIELD_CHARS} chars"));
        }
        for (label, val) in [
            ("manufacturer", &ship.manufacturer),
            ("pledge_id", &ship.pledge_id),
            ("kind", &ship.kind),
        ] {
            if let Some(s) = val {
                if s.chars().count() > MAX_FIELD_CHARS {
                    return Some(format!("ship[{i}].{label} exceeds {MAX_FIELD_CHARS} chars"));
                }
            }
        }
    }
    None
}

#[utoipa::path(
    get,
    path = "/v1/me/hangar",
    tag = "hangar",
    operation_id = "hangar_me",
    responses(
        (status = 200, description = "Latest hangar snapshot for the caller", body = HangarSnapshot),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No snapshot has been pushed yet", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn me<S: HangarStore>(State(store): State<Arc<S>>, auth: AuthenticatedUser) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    match store.get_snapshot(user_id).await {
        Ok(Some(snapshot)) => (StatusCode::OK, Json(snapshot)).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            "no_hangar_yet",
            Some("push a snapshot via POST /v1/me/hangar first".into()),
        ),
        Err(e) => {
            tracing::error!(error = %e, "get_snapshot failed in /v1/me/hangar");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}
