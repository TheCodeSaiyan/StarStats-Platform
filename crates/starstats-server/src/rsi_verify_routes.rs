//! RSI handle verification endpoints.
//!
//!  - `POST /v1/auth/rsi/start`  (user JWT) — issue a verification
//!    code (or return the still-valid one already issued).
//!  - `POST /v1/auth/rsi/verify` (user JWT) — re-fetch the user's RSI
//!    profile and check the bio for the issued code.
//!  - `POST /v1/auth/rsi/reverify` (user JWT) — issue a fresh code even
//!    to a verified account, so the user can re-prove their handle at
//!    any time. Verification never expires and a failed re-check leaves
//!    it in place.
//!
//! Both endpoints are authenticated with a user (not device) JWT
//! because the desktop client doesn't surface bio editing — the user
//! does it from the web. Verification is a per-account property, not
//! a per-device one.
//!
//! The flow is idempotent in both directions:
//!  - calling `start` while a code is still in flight returns that
//!    same code (so a refresh doesn't force the user to re-paste);
//!  - calling `verify` while already verified returns `verified: true`
//!    without re-hitting RSI.

use crate::api_error::ApiErrorBody;
use crate::auth::{AuthenticatedUser, TokenType};
use crate::rsi_verify::{generate_verify_code, RsiCheckOutcome, RsiClient};
use crate::users::{PostgresUserStore, UserError, UserStore};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::post,
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// How long a freshly-issued verification code stays usable. Long
/// enough for the user to switch tabs, edit their RSI bio, save, and
/// come back; short enough that an abandoned flow doesn't leave a
/// usable code lying around indefinitely.
pub const RSI_VERIFY_TTL: chrono::Duration = chrono::Duration::minutes(30);

/// Build the `/v1/auth/rsi/*` sub-router. `RsiClient` is sourced from
/// the request extensions (`Extension<Arc<dyn RsiClient>>`) so tests
/// can inject a fake without re-plumbing state shapes.
pub fn routes(users: Arc<PostgresUserStore>) -> Router {
    Router::new()
        .route("/v1/auth/rsi/start", post(start::<PostgresUserStore>))
        .route("/v1/auth/rsi/verify", post(verify::<PostgresUserStore>))
        .route("/v1/auth/rsi/reverify", post(reverify::<PostgresUserStore>))
        .with_state(users)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RsiStartResponse {
    /// `true` when the handle is already proven; `code` and
    /// `expires_at` are absent in that case.
    pub verified: bool,
    /// The string the user must paste into their RSI bio. Absent when
    /// `verified` is true, except from `/reverify`, which re-checks a
    /// verified handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// ISO-8601 UTC. Present exactly when `code` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The handle the code is bound to — included so the UI can show
    /// the exact profile URL the user needs to paste it into.
    pub claimed_handle: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RsiVerifyResponse {
    /// `true` when the handle is now (or was already) proven.
    pub verified: bool,
    /// Machine-readable outcome string, mirrors the response body
    /// `error` field on non-2xx for parity with other endpoints. One
    /// of: `verified`, `already_verified`. Failure outcomes are
    /// surfaced as non-2xx with `ApiErrorBody`.
    pub status: &'static str,
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

/// Reject device tokens. Verification is a user-account operation —
/// device clients shouldn't be able to (re)issue codes on behalf of
/// the user.
fn require_user_token(user: &AuthenticatedUser) -> Option<axum::response::Response> {
    if !matches!(user.token_type, TokenType::User) {
        return Some(error(
            StatusCode::FORBIDDEN,
            "user_token_required",
            Some("device tokens cannot run RSI verification".into()),
        ));
    }
    None
}

#[utoipa::path(
    post,
    path = "/v1/auth/rsi/start",
    tag = "rsi-verify",
    operation_id = "rsi_verify_start",
    responses(
        (status = 200, description = "Verification code issued (or already verified)", body = RsiStartResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn start<U: UserStore>(
    State(users): State<Arc<U>>,
    auth: AuthenticatedUser,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in rsi/start");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    if user.rsi_verified_at.is_some() {
        return (
            StatusCode::OK,
            Json(RsiStartResponse {
                verified: true,
                code: None,
                expires_at: None,
                claimed_handle: user.claimed_handle,
            }),
        )
            .into_response();
    }

    // Reuse the in-flight code if it's still valid. The window
    // matters: a user who pastes the code into their bio, leaves the
    // tab open, and comes back 5 minutes later expects "Check now"
    // to validate the same code, not a fresh one.
    let now = Utc::now();
    let reuse = match (user.rsi_verify_code.as_deref(), user.rsi_verify_expires_at) {
        (Some(c), Some(exp)) if exp > now => Some((c.to_owned(), exp)),
        _ => None,
    };

    let (code, expires_at) = match reuse {
        Some(pair) => pair,
        None => {
            let code = generate_verify_code();
            let expires_at = now + RSI_VERIFY_TTL;
            if let Err(e) = users.set_rsi_verify_code(user_id, &code, expires_at).await {
                tracing::error!(error = %e, "set_rsi_verify_code failed");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
            (code, expires_at)
        }
    };

    (
        StatusCode::OK,
        Json(RsiStartResponse {
            verified: false,
            code: Some(code),
            expires_at: Some(expires_at),
            claimed_handle: user.claimed_handle,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/rsi/verify",
    tag = "rsi-verify",
    operation_id = "rsi_verify_check",
    responses(
        (status = 200, description = "Handle verified or already-verified", body = RsiVerifyResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device token", body = ApiErrorBody),
        (status = 404, description = "RSI returned 404 for the claimed handle", body = ApiErrorBody),
        (status = 410, description = "No code in flight or code expired — call /start again", body = ApiErrorBody),
        (status = 422, description = "Code not found in user's RSI bio", body = ApiErrorBody),
        (status = 503, description = "RSI upstream unreachable", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn verify<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(rsi): Extension<Arc<dyn RsiClient>>,
    auth: AuthenticatedUser,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in rsi/verify");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // A verified account with a code in flight asked to be re-checked
    // (`/reverify`), so fall through and check it. Failing that check
    // never removes verification: it does not expire.
    if user.rsi_verified_at.is_some() && user.rsi_verify_code.is_none() {
        return (
            StatusCode::OK,
            Json(RsiVerifyResponse {
                verified: true,
                status: "already_verified",
            }),
        )
            .into_response();
    }

    let (code, expires_at) = match (user.rsi_verify_code.as_deref(), user.rsi_verify_expires_at) {
        (Some(c), Some(exp)) => (c.to_owned(), exp),
        _ => {
            return error(
                StatusCode::GONE,
                "no_code_pending",
                Some("call /v1/auth/rsi/start to issue a code first".into()),
            );
        }
    };

    if expires_at <= Utc::now() {
        return error(
            StatusCode::GONE,
            "code_expired",
            Some("call /v1/auth/rsi/start to issue a fresh code".into()),
        );
    }

    match rsi.check_bio(&user.claimed_handle, &code).await {
        RsiCheckOutcome::BioContains => {
            if let Err(e) = mark_verified(users.as_ref(), user_id).await {
                tracing::error!(error = %e, "mark_rsi_verified failed");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
            (
                StatusCode::OK,
                Json(RsiVerifyResponse {
                    verified: true,
                    status: "verified",
                }),
            )
                .into_response()
        }
        RsiCheckOutcome::BioMissing => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "code_not_in_bio",
            Some(
                "your RSI bio does not contain the verification code. \
                 Paste it into the public bio field and try again."
                    .into(),
            ),
        ),
        RsiCheckOutcome::HandleNotFound => error(
            StatusCode::NOT_FOUND,
            "rsi_handle_not_found",
            Some("RSI returned 404 for that handle".into()),
        ),
        RsiCheckOutcome::UpstreamUnavailable => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "rsi_unavailable",
            Some("RSI is unreachable; please try again shortly".into()),
        ),
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/rsi/reverify",
    tag = "rsi-verify",
    operation_id = "rsi_verify_reverify",
    responses(
        (status = 200, description = "Fresh verification code issued; `verified` is the current state, which a failed re-check does not change", body = RsiStartResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
/// Re-prove the current handle at any time. Unlike `/start`, this issues
/// a code even to a verified account (reusing one still in flight);
/// `/verify` then checks it.
pub async fn reverify<U: UserStore>(
    State(users): State<Arc<U>>,
    auth: AuthenticatedUser,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };
    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in rsi/reverify");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    // Reuse a code still in flight, as `/start` does: the page asks for
    // it on every render of the re-check step, and a new code each time
    // would invalidate the one the user has just pasted.
    let now = Utc::now();
    let (code, expires_at) = match (user.rsi_verify_code.as_deref(), user.rsi_verify_expires_at) {
        (Some(c), Some(exp)) if exp > now => (c.to_owned(), exp),
        _ => {
            let code = generate_verify_code();
            let expires_at = now + RSI_VERIFY_TTL;
            if let Err(e) = users.set_rsi_verify_code(user_id, &code, expires_at).await {
                tracing::error!(error = %e, "set_rsi_verify_code failed in rsi/reverify");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
            (code, expires_at)
        }
    };
    (
        StatusCode::OK,
        Json(RsiStartResponse {
            verified: user.rsi_verified_at.is_some(),
            code: Some(code),
            expires_at: Some(expires_at),
            claimed_handle: user.claimed_handle,
        }),
    )
        .into_response()
}

async fn mark_verified<U: UserStore>(users: &U, user_id: Uuid) -> Result<(), UserError> {
    users.mark_rsi_verified(user_id).await
}

#[cfg(test)]
mod reverify_tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::rsi_verify::{RsiOrgsOutcome, RsiProfileOutcome};
    use crate::users::test_support::MemoryUserStore;
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    /// An RSI whose bio contains whatever code was last issued, or none.
    struct FakeRsi {
        bio_has_code: bool,
    }

    #[async_trait]
    impl RsiClient for FakeRsi {
        async fn check_bio(&self, _handle: &str, _code: &str) -> RsiCheckOutcome {
            if self.bio_has_code {
                RsiCheckOutcome::BioContains
            } else {
                RsiCheckOutcome::BioMissing
            }
        }
        async fn fetch_profile(&self, _handle: &str) -> RsiProfileOutcome {
            RsiProfileOutcome::HandleNotFound
        }
        async fn fetch_orgs(&self, _handle: &str) -> RsiOrgsOutcome {
            RsiOrgsOutcome::HandleNotFound
        }
    }

    async fn app(bio_has_code: bool) -> (Router, Arc<MemoryUserStore>, Uuid, String) {
        let users = Arc::new(MemoryUserStore::new());
        let user = users
            .create("pilot@example.com", "phc", "Pilot")
            .await
            .unwrap();
        users.mark_rsi_verified(user.id).await.unwrap();
        let (issuer, verifier) = fresh_pair();
        let token = issuer.sign_user(&user.id.to_string(), "Pilot").unwrap();
        let rsi: Arc<dyn RsiClient> = Arc::new(FakeRsi { bio_has_code });
        let router = Router::new()
            .route("/v1/auth/rsi/reverify", post(reverify::<MemoryUserStore>))
            .route("/v1/auth/rsi/verify", post(verify::<MemoryUserStore>))
            .with_state(users.clone())
            .layer(Extension(rsi))
            .layer(Extension(Arc::new(verifier)));
        (router, users, user.id, token)
    }

    async fn post_json(app: &Router, path: &str, token: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn reverify_issues_a_code_to_a_verified_account_and_verify_checks_it() {
        let (app, users, id, token) = app(true).await;
        let before = users.find_by_id(id).await.unwrap().unwrap().rsi_verified_at;
        let (status, body) = post_json(&app, "/v1/auth/rsi/reverify", &token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["verified"], true, "still verified while re-checking");
        assert!(body["code"].is_string());
        let (status, body) = post_json(&app, "/v1/auth/rsi/verify", &token).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "verified", "the pending code was checked");
        let after = users.find_by_id(id).await.unwrap().unwrap();
        assert!(after.rsi_verify_code.is_none());
        assert!(after.rsi_verified_at >= before);
    }

    #[tokio::test]
    async fn reverify_hands_back_the_same_code_while_it_is_valid() {
        // The settings page calls reverify on every render of the re-check
        // step; a fresh code each time would invalidate the one the user
        // has just pasted into their bio.
        let (app, _users, _id, token) = app(true).await;
        let (_, first) = post_json(&app, "/v1/auth/rsi/reverify", &token).await;
        let (_, second) = post_json(&app, "/v1/auth/rsi/reverify", &token).await;
        assert_eq!(first["code"], second["code"]);
    }

    #[tokio::test]
    async fn a_failed_reverify_does_not_take_verification_away() {
        let (app, users, id, token) = app(false).await;
        post_json(&app, "/v1/auth/rsi/reverify", &token).await;
        let (status, body) = post_json(&app, "/v1/auth/rsi/verify", &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(users
            .find_by_id(id)
            .await
            .unwrap()
            .unwrap()
            .rsi_verified_at
            .is_some());
    }
}
