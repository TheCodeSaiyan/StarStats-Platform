//! RSI handle rename endpoints.
//!
//!  - `GET  /v1/auth/rsi/handle` (user JWT): the handle, verification
//!    state, and when the next rename is allowed.
//!  - `POST /v1/auth/rsi/handle` (user JWT): change the handle.
//!
//! The current handle always comes from the account, never the client;
//! the client supplies only the handle it wants. A rename resets RSI
//! verification, so the web sends the user straight into the bio-code
//! flow (`/v1/auth/rsi/start`) afterwards. The response carries a fresh
//! user token because the one the caller holds names the old handle.

use crate::audit::{AuditEntry, AuditLog};
use crate::auth::{AuthenticatedUser, TokenIssuer, TokenType};
use crate::handle_renames::{
    HandleIndex, HandleRenameError, HandleRenameStore, RenameRefusal, COOLDOWN, RESERVATION,
};
use crate::spicedb::SpicedbClient;
use crate::users::UserStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes<U: UserStore>(users: Arc<U>) -> Router {
    Router::new()
        .route("/v1/auth/rsi/handle", get(status::<U>).post(change))
        .route("/v1/handles/{handle}/current", get(current_handle))
        .with_state(users)
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HandleStatusResponse {
    pub claimed_handle: String,
    /// Whether the current handle is proven by the RSI bio check.
    pub verified: bool,
    /// When the next rename is allowed. Absent when one is allowed now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    /// True while the account has never been verified and never renamed:
    /// its first rename skips the cooldown.
    pub free_change_available: bool,
    /// The handle before the most recent rename, while it is still held
    /// for this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_handle_reserved_until: Option<DateTime<Utc>>,
    /// Days between renames, and days an old handle is held, so the UI
    /// states the consequences from the server's numbers.
    pub cooldown_days: i64,
    pub reservation_days: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct HandleChangeRequest {
    pub new_handle: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HandleChangeResponse {
    pub claimed_handle: String,
    pub previous_handle: String,
    /// A user token naming the new handle. The caller's token still names
    /// the old one; the web stores this in the session.
    pub token: String,
    /// Always false: the new handle is unproven until the bio check.
    pub verified: bool,
    pub free_change: bool,
    pub cooldown_until: DateTime<Utc>,
    pub previous_handle_reserved_until: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    detail: Option<String>,
}

fn error(status: StatusCode, code: &'static str, detail: Option<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: code,
            detail,
        }),
    )
        .into_response()
}

fn user_id_of(auth: &AuthenticatedUser) -> Result<Uuid, Response> {
    if !matches!(auth.token_type, TokenType::User) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "user_token_required",
            Some("device tokens cannot change the handle".into()),
        ));
    }
    Uuid::parse_str(&auth.sub)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None))
}

#[utoipa::path(
    get,
    path = "/v1/auth/rsi/handle",
    tag = "rsi-verify",
    responses(
        (status = 200, description = "Handle and rename state", body = HandleStatusResponse),
        (status = 401, description = "Not signed in", body = crate::api_error::ApiErrorBody),
        (status = 403, description = "Device token", body = crate::api_error::ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn status<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(renames): Extension<Arc<dyn HandleRenameStore>>,
    auth: AuthenticatedUser,
) -> Response {
    let user_id = match user_id_of(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in rsi/handle status");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let latest = match renames.latest_for_user(user_id).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "latest_for_user failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let now = Utc::now();
    let verified = user.rsi_verified_at.is_some();
    let cooldown_until = latest
        .as_ref()
        .map(|r| r.renamed_at + COOLDOWN)
        .filter(|until| *until > now);
    let held = latest.as_ref().filter(|r| {
        r.reserved_until > now && !r.old_handle.eq_ignore_ascii_case(&user.claimed_handle)
    });
    Json(HandleStatusResponse {
        claimed_handle: user.claimed_handle,
        verified,
        cooldown_until,
        free_change_available: latest.is_none() && !verified,
        previous_handle: held.map(|r| r.old_handle.clone()),
        previous_handle_reserved_until: held.map(|r| r.reserved_until),
        cooldown_days: COOLDOWN.num_days(),
        reservation_days: RESERVATION.num_days(),
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/rsi/handle",
    tag = "rsi-verify",
    request_body = HandleChangeRequest,
    responses(
        (status = 200, description = "Handle changed; verification reset", body = HandleChangeResponse),
        (status = 400, description = "invalid_handle or handle_unchanged", body = crate::api_error::ApiErrorBody),
        (status = 401, description = "Not signed in", body = crate::api_error::ApiErrorBody),
        (status = 403, description = "Device token", body = crate::api_error::ApiErrorBody),
        (status = 409, description = "handle_taken or handle_reserved", body = crate::api_error::ApiErrorBody),
        (status = 429, description = "rename_cooldown; detail carries the time it ends", body = crate::api_error::ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn change(
    Extension(renames): Extension<Arc<dyn HandleRenameStore>>,
    Extension(issuer): Extension<Arc<TokenIssuer>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    handle_index: Option<Extension<Arc<HandleIndex>>>,
    auth: AuthenticatedUser,
    Json(req): Json<HandleChangeRequest>,
) -> Response {
    let user_id = match user_id_of(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let new_handle = req.new_handle.trim();
    let now = Utc::now();

    let rename = match renames.rename(user_id, new_handle, now).await {
        Ok(r) => r,
        Err(HandleRenameError::Refused(refusal)) => return refusal_response(refusal),
        Err(HandleRenameError::UserNotFound) => {
            return error(StatusCode::UNAUTHORIZED, "unauthorized", None)
        }
        Err(e) => {
            tracing::error!(error = %e, "handle rename failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // Old tokens on this instance act as the new handle from now on; other
    // instances catch up on their next index refresh.
    if let Some(Extension(index)) = handle_index.as_ref() {
        index.record(&rename);
    }

    // Postgres has committed. SpiceDB cannot share that transaction, so
    // the move runs now and the reconciler retries it if this fails.
    if let Some(client) = spicedb.as_ref().as_ref() {
        match client
            .move_handle(&rename.old_handle, &rename.new_handle)
            .await
        {
            Ok(_) => {
                if let Err(e) = renames.mark_spicedb_moved(rename.id, Utc::now()).await {
                    tracing::warn!(error = %e, rename_id = %rename.id, "mark_spicedb_moved failed");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, rename_id = %rename.id,
                    "SpiceDB handle move failed; the reconciler will retry");
            }
        }
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(user_id.to_string()),
            actor_handle: Some(rename.new_handle.clone()),
            action: "user.handle_changed".to_string(),
            payload: serde_json::json!({
                "old_handle": rename.old_handle,
                "new_handle": rename.new_handle,
                "was_verified": rename.was_verified,
                "free_change": rename.free_change,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit append failed for user.handle_changed");
    }

    let token = match issuer.sign_user(&user_id.to_string(), &rename.new_handle) {
        Ok(t) => t,
        Err(e) => {
            // The rename stands; the caller's old token is still accepted
            // and resolves to the new handle, so signing in again recovers.
            tracing::error!(error = %e, "sign_user failed after rename");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    Json(HandleChangeResponse {
        claimed_handle: rename.new_handle,
        previous_handle: rename.old_handle,
        token,
        verified: false,
        free_change: rename.free_change,
        cooldown_until: rename.renamed_at + COOLDOWN,
        previous_handle_reserved_until: rename.reserved_until,
    })
    .into_response()
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CurrentHandleResponse {
    pub handle: String,
}

#[utoipa::path(
    get,
    path = "/v1/handles/{handle}/current",
    tag = "rsi-verify",
    params(("handle" = String, Path, description = "A handle that may have been renamed away from")),
    responses(
        (status = 200, description = "The handle's owner renamed within the last 90 days; this is their handle now", body = CurrentHandleResponse),
        (status = 404, description = "No redirect: the handle is live, unknown, or its hold has ended", body = crate::api_error::ApiErrorBody),
    )
)]
/// Where a profile or share link built on an old handle should go. Public:
/// the rename is already visible to anyone who follows such a link, and
/// the answer only exists for the 90 days the old handle is held.
pub async fn current_handle(
    Extension(renames): Extension<Arc<dyn HandleRenameStore>>,
    Path(handle): Path<String>,
) -> Response {
    if !crate::validation::validate_handle(&handle) {
        return error(StatusCode::NOT_FOUND, "not_found", None);
    }
    match renames.redirect_for(&handle, Utc::now()).await {
        Ok(Some(current)) => Json(CurrentHandleResponse { handle: current }).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found", None),
        Err(e) => {
            tracing::error!(error = %e, "redirect_for failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

fn refusal_response(refusal: RenameRefusal) -> Response {
    match refusal {
        RenameRefusal::InvalidHandle => error(
            StatusCode::BAD_REQUEST,
            "invalid_handle",
            Some("handles are letters, digits, '_' and '-', up to 64 characters".into()),
        ),
        RenameRefusal::Unchanged => error(StatusCode::BAD_REQUEST, "handle_unchanged", None),
        RenameRefusal::Taken => error(StatusCode::CONFLICT, "handle_taken", None),
        RenameRefusal::Reserved => error(
            StatusCode::CONFLICT,
            "handle_reserved",
            Some("this handle was recently used by another account".into()),
        ),
        RenameRefusal::Cooldown { until } => error(
            StatusCode::TOO_MANY_REQUESTS,
            "rename_cooldown",
            Some(until.to_rfc3339()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::handle_renames::test_support::MemoryHandleRenameStore;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    struct Fixture {
        app: Router,
        users: Arc<MemoryUserStore>,
        audit: Arc<MemoryAuditLog>,
        token: String,
        user_id: Uuid,
    }

    async fn fixture() -> Fixture {
        let users = Arc::new(MemoryUserStore::new());
        let user = users
            .create("pilot@example.com", "phc", "OldPilot")
            .await
            .unwrap();
        let renames: Arc<dyn HandleRenameStore> =
            Arc::new(MemoryHandleRenameStore::new(users.clone()));
        let (issuer, verifier) = fresh_pair();
        let token = issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .unwrap();
        let audit = Arc::new(MemoryAuditLog::default());
        let audit_dyn: Arc<dyn AuditLog> = audit.clone();
        let app = routes(users.clone())
            .layer(Extension(renames))
            .layer(Extension(Arc::new(issuer)))
            .layer(Extension(audit_dyn))
            .layer(Extension(Arc::new(None::<SpicedbClient>)))
            .layer(Extension(Arc::new(verifier)));
        Fixture {
            app,
            users,
            audit,
            token,
            user_id: user.id,
        }
    }

    async fn call(
        app: &Router,
        method: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder()
            .method(method)
            .uri("/v1/auth/rsi/handle")
            .header("authorization", format!("Bearer {token}"));
        let body = match body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn rename_returns_a_token_for_the_new_handle_and_audits() {
        let f = fixture().await;
        let (status, body) = call(
            &f.app,
            "POST",
            &f.token,
            Some(serde_json::json!({ "new_handle": "NewPilot" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["claimed_handle"], "NewPilot");
        assert_eq!(body["previous_handle"], "OldPilot");
        assert_eq!(body["verified"], false);
        assert_eq!(body["free_change"], true);

        let (status, after) = call(&f.app, "GET", body["token"].as_str().unwrap(), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(after["claimed_handle"], "NewPilot");
        assert_eq!(after["previous_handle"], "OldPilot");
        assert_eq!(after["free_change_available"], false);
        assert!(after["cooldown_until"].is_string());

        let entries = f.audit.snapshot();
        assert!(entries
            .iter()
            .any(|e| e.action == "user.handle_changed" && e.payload["old_handle"] == "OldPilot"));
        let user = f.users.find_by_id(f.user_id).await.unwrap().unwrap();
        assert_eq!(user.claimed_handle, "NewPilot");
    }

    #[tokio::test]
    async fn a_second_rename_inside_the_cooldown_is_429() {
        let f = fixture().await;
        let (status, _) = call(
            &f.app,
            "POST",
            &f.token,
            Some(serde_json::json!({ "new_handle": "NewPilot" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = call(
            &f.app,
            "POST",
            &f.token,
            Some(serde_json::json!({ "new_handle": "ThirdPilot" })),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error"], "rename_cooldown");
    }

    #[tokio::test]
    async fn a_taken_handle_is_409_and_nothing_changes() {
        let f = fixture().await;
        f.users
            .create("other@example.com", "phc", "Someone")
            .await
            .unwrap();
        let (status, body) = call(
            &f.app,
            "POST",
            &f.token,
            Some(serde_json::json!({ "new_handle": "someone" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "handle_taken");
        let user = f.users.find_by_id(f.user_id).await.unwrap().unwrap();
        assert_eq!(user.claimed_handle, "OldPilot");
        assert!(f.audit.snapshot().is_empty());
    }

    #[tokio::test]
    async fn the_old_handle_resolves_to_the_new_one_without_signing_in() {
        let f = fixture().await;
        let (status, _) = call(
            &f.app,
            "POST",
            &f.token,
            Some(serde_json::json!({ "new_handle": "NewPilot" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let get = |uri: &'static str| {
            let app = f.app.clone();
            async move {
                let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
                let resp = app.oneshot(req).await.unwrap();
                let status = resp.status();
                let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
                (
                    status,
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                )
            }
        };
        let (status, body) = get("/v1/handles/oldpilot/current").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["handle"], "NewPilot");
        let (status, _) = get("/v1/handles/NewPilot/current").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a live handle has no redirect"
        );
    }

    #[tokio::test]
    async fn an_invalid_handle_is_400() {
        let f = fixture().await;
        let (status, body) = call(
            &f.app,
            "POST",
            &f.token,
            Some(serde_json::json!({ "new_handle": "no spaces!" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_handle");
    }

    #[tokio::test]
    async fn status_offers_the_free_change_to_a_never_verified_account() {
        let f = fixture().await;
        let (status, body) = call(&f.app, "GET", &f.token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["claimed_handle"], "OldPilot");
        assert_eq!(body["free_change_available"], true);
        assert_eq!(body["cooldown_days"], 30);
        assert_eq!(body["reservation_days"], 90);
        assert!(body.get("cooldown_until").is_none());
    }
}
