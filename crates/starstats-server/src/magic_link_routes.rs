//! Magic-link sign-in endpoints.
//!
//!  - `POST /v1/auth/magic/start`  (no auth) — body `{ email }`,
//!    always returns 200 (anti-enumeration). On a hit, mails the
//!    user a one-shot link.
//!  - `POST /v1/auth/magic/redeem` (no auth) — body `{ token }`,
//!    consumes the token and returns an [`auth_routes::AuthResponse`].
//!    If the user has TOTP enabled, the response carries an interim
//!    token + `totp_required: true` instead of a full session JWT —
//!    same fork as the password-login path.
//!
//! Failure posture mirrors the password-reset flow: any unmappable
//! error is logged and surfaces as 500. Anti-enumeration responses
//! sleep ~50ms on miss so a probe can't time the difference between
//! "email known" and "email unknown."

use crate::api_error::ApiErrorBody;
use crate::auth::TokenIssuer;
use crate::auth_routes::{issue_login_interim, AuthResponse};
use crate::magic_link::{MagicLinkStore, PostgresMagicLinkStore};
use crate::mail::Mailer;
use crate::users::{PostgresUserStore, UserStore};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::post,
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use utoipa::ToSchema;

/// Build the `/v1/auth/magic/*` sub-router.
///
/// Two sub-routers because the start handler needs the user store +
/// magic-link store, and the redeem handler needs both *plus* the
/// issuer (which lives in Extensions). Bundling them with a shared
/// state tuple keeps `main.rs` clean.
pub fn routes(users: Arc<PostgresUserStore>, magic: Arc<PostgresMagicLinkStore>) -> Router {
    Router::new()
        .route(
            "/v1/auth/magic/start",
            post(start::<PostgresUserStore, PostgresMagicLinkStore>),
        )
        .route(
            "/v1/auth/magic/redeem",
            post(redeem::<PostgresUserStore, PostgresMagicLinkStore>),
        )
        .with_state((users, magic))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MagicLinkStartRequest {
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MagicLinkStartResponse {
    /// Always `true`. Anti-enumeration: the response shape doesn't
    /// distinguish "your email isn't on file" from "we sent you a
    /// link" — a probe can't tell.
    pub sent: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MagicLinkRedeemRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    detail: Option<String>,
}

fn error(
    status: StatusCode,
    code: &'static str,
    detail: Option<String>,
) -> axum::response::Response {
    (
        status,
        Json(ErrorBody {
            error: code,
            detail,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/magic/start",
    tag = "auth",
    operation_id = "magic_link_start",
    request_body = MagicLinkStartRequest,
    responses(
        (status = 200, description = "Always returned (anti-enumeration); a link is mailed only if the email is on file", body = MagicLinkStartResponse),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn start<U: UserStore, M: MagicLinkStore>(
    State((users, magic)): State<(Arc<U>, Arc<M>)>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Json(req): Json<MagicLinkStartRequest>,
) -> impl IntoResponse {
    let email = req.email.trim().to_lowercase();

    let user = match users.find_by_email(&email).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "find_by_email in magic/start");
            // Still return 200 — leaking 500 vs 200 here would defeat
            // the anti-enumeration guarantee.
            tokio::time::sleep(Duration::from_millis(50)).await;
            return Json(MagicLinkStartResponse { sent: true }).into_response();
        }
    };

    let Some(user) = user else {
        // Sleep so the timing of the response doesn't reveal whether
        // an email matched. ~50ms covers the user-creation + email
        // dispatch jitter on the hot path.
        tokio::time::sleep(Duration::from_millis(50)).await;
        return Json(MagicLinkStartResponse { sent: true }).into_response();
    };

    let token = match magic.issue(user.id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "issue magic link failed");
            return Json(MagicLinkStartResponse { sent: true }).into_response();
        }
    };

    if let Err(e) = mailer
        .send_magic_link(&user.email, &user.claimed_handle, &token)
        .await
    {
        tracing::warn!(error = %format!("{e:#}"), "send magic link failed (best-effort)");
    }

    Json(MagicLinkStartResponse { sent: true }).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/magic/redeem",
    tag = "auth",
    operation_id = "magic_link_redeem",
    request_body = MagicLinkRedeemRequest,
    responses(
        (status = 200, description = "Token consumed; session JWT or TOTP-required interim returned", body = AuthResponse),
        (status = 401, description = "Token unknown, expired, or already used", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn redeem<U: UserStore, M: MagicLinkStore>(
    State((users, magic)): State<(Arc<U>, Arc<M>)>,
    Extension(issuer): Extension<Arc<TokenIssuer>>,
    Json(req): Json<MagicLinkRedeemRequest>,
) -> impl IntoResponse {
    let redeemed = match magic.redeem(&req.token).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return error(StatusCode::UNAUTHORIZED, "invalid_or_expired", None);
        }
        Err(e) => {
            tracing::error!(error = %e, "magic link redeem failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    let user = match users.find_by_id(redeemed.user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::error!(
                user_id = %redeemed.user_id,
                "user disappeared between redeem and lookup"
            );
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
        Err(e) => {
            tracing::error!(error = %e, "find_by_id in magic redeem");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // Same TOTP fork as the password-login path: if the account has
    // a second factor enabled, the magic link only gets the user as
    // far as the interim token. They still owe us a 6-digit code.
    if user.totp_enabled_at.is_some() {
        return issue_login_interim(issuer.as_ref(), &user.id.to_string(), &user.claimed_handle);
    }

    match issuer.sign_user(&user.id.to_string(), &user.claimed_handle) {
        Ok(token) => (
            StatusCode::OK,
            Json(AuthResponse {
                token,
                user_id: user.id.to_string(),
                claimed_handle: user.claimed_handle,
                totp_required: false,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "sign user token failed in magic redeem");
            error(StatusCode::INTERNAL_SERVER_ERROR, "sign_failed", None)
        }
    }
}
