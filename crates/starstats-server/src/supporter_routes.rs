//! HTTP handlers for the supporter (donate) read endpoint.
//!
//! Mutations (state transitions, name_plate edits, Revolut webhook
//! sink) land in the Wave 9 follow-up that wires up the actual
//! payment flow. This file covers the read side only — enough to
//! light up the supporter pill on the profile / settings pages.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::supporters::{PostgresSupporterStore, SupporterStore};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(store: Arc<PostgresSupporterStore>) -> Router {
    Router::new()
        .route("/v1/me/supporter", get(get_me::<PostgresSupporterStore>))
        .with_state(store)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SupporterStatusDto {
    pub state: String,
    pub name_plate: Option<String>,
    pub became_supporter_at: Option<DateTime<Utc>>,
    pub last_payment_at: Option<DateTime<Utc>>,
    pub grace_until: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    /// `tier_key` of the most-recent completed donation
    /// (`coffee` / `standard` / `generous`). Used by the web UI to
    /// pick the supporter-chip styling. `None` when the user has
    /// never had a completed order — combined with `state` the
    /// frontend can distinguish "never paid" from "paid (any tier)".
    pub current_tier_key: Option<String>,
}

fn err(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/me/supporter",
    tag = "supporter",
    operation_id = "supporter_get_me",
    responses(
        (status = 200, description = "The caller's supporter status. Defaults to state=none for users with no row.", body = SupporterStatusDto),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_me<S: SupporterStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return err(StatusCode::UNAUTHORIZED, "invalid_subject"),
    };

    match store.get(user_id).await {
        Ok(s) => (
            StatusCode::OK,
            Json(SupporterStatusDto {
                state: s.state.as_str().to_string(),
                name_plate: s.name_plate,
                became_supporter_at: s.became_supporter_at,
                last_payment_at: s.last_payment_at,
                grace_until: s.grace_until,
                cancelled_at: s.cancelled_at,
                current_tier_key: s.current_tier_key,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "supporter get_me failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "supporter_failed").into_response()
        }
    }
}
