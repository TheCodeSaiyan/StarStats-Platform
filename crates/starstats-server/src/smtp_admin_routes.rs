//! Admin endpoints for editing the SMTP configuration at runtime.
//!
//! All three endpoints are gated by [`RequireAdmin`]: only an
//! authenticated user with the platform-level `admin` role can read,
//! write, or test the SMTP config.
//!
//!   * `GET    /v1/admin/smtp`         — current config minus password
//!                                       (with `password_set: bool`).
//!   * `PUT    /v1/admin/smtp`         — persist + hot-swap the mailer.
//!   * `POST   /v1/admin/smtp/test`    — send a diagnostic email to
//!                                       the calling admin's address.
//!
//! Hot reload: a successful `PUT` swaps the [`SwappableMailer`] inner
//! handle. Existing send-in-flight calls keep using the old transport
//! until they drop; new calls (including the immediately-following
//! test send) go through the freshly built `LettreMailer`. When the
//! new record has `enabled: false`, the mailer is swapped to a
//! `NoopMailer` so old credentials stop being held in memory.

use crate::admin_routes::RequireAdmin;
use crate::api_error::ApiErrorBody;
use crate::kek::Kek;
use crate::mail::{build_mailer_from_record, Mailer, NoopMailer, SwappableMailer};
use crate::smtp_config_store::{SmtpConfigRecord, SmtpConfigStore};
use crate::users::UserStore;
use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

// -- DTOs ------------------------------------------------------------

/// Public-facing shape returned by `GET /v1/admin/smtp`. Password is
/// never serialised — the form indicates presence via `password_set`
/// and edits send the desired plaintext (or null = keep) on PUT.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SmtpConfigResponse {
    pub host: String,
    pub port: i32,
    pub username: String,
    pub password_set: bool,
    pub secure: bool,
    pub from_addr: String,
    pub from_name: String,
    pub web_origin: String,
    pub enabled: bool,
}

/// Request body for `PUT /v1/admin/smtp`. `password` semantics:
///
///  * `null` (field absent) — keep the existing encrypted password.
///  * `""` (empty string)    — clear authentication entirely.
///  * non-empty string       — encrypt and store as the new password.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SmtpConfigRequest {
    pub host: String,
    pub port: i32,
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    pub secure: bool,
    pub from_addr: String,
    pub from_name: String,
    pub web_origin: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TestSendResponse {
    pub sent_to: String,
}

/// Body for `POST /v1/admin/smtp/test`.
///
/// `to_address` lets an admin bootstrap from a not-yet-configured
/// state: when SMTP has never worked, the admin's own email is by
/// definition unverified, and the original "must be verified" gate
/// becomes a deadlock. Letting them direct the test to a known-
/// working external address (a personal mailbox, etc.) breaks the
/// cycle. When omitted, behaviour is unchanged: send to the admin's
/// own verified email, refuse with `email_unverified` otherwise.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct TestSendRequest {
    #[serde(default)]
    pub to_address: Option<String>,
}

// -- Helpers ---------------------------------------------------------

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

fn validate(req: &SmtpConfigRequest) -> Result<(), Response> {
    if req.host.trim().is_empty() && req.enabled {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_smtp_config",
            Some("host is required when enabled".into()),
        ));
    }
    if !(1..=65535).contains(&req.port) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_smtp_config",
            Some(format!("port {} out of range 1..=65535", req.port)),
        ));
    }
    if req.from_addr.trim().is_empty() || !req.from_addr.contains('@') {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_smtp_config",
            Some("from_addr must contain '@'".into()),
        ));
    }
    if req.from_name.trim().is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_smtp_config",
            Some("from_name is required".into()),
        ));
    }
    if !req.web_origin.starts_with("http://") && !req.web_origin.starts_with("https://") {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "invalid_smtp_config",
            Some("web_origin must start with http:// or https://".into()),
        ));
    }
    Ok(())
}

fn to_response(rec: &SmtpConfigRecord) -> SmtpConfigResponse {
    SmtpConfigResponse {
        host: rec.host.clone(),
        port: rec.port,
        username: rec.username.clone(),
        password_set: rec
            .password
            .as_deref()
            .map(|p| !p.is_empty())
            .unwrap_or(false),
        secure: rec.secure,
        from_addr: rec.from_addr.clone(),
        from_name: rec.from_name.clone(),
        web_origin: rec.web_origin.clone(),
        enabled: rec.enabled,
    }
}

fn merge(req: SmtpConfigRequest) -> SmtpConfigRecord {
    SmtpConfigRecord {
        host: req.host,
        port: req.port,
        username: req.username,
        password: req.password,
        secure: req.secure,
        from_addr: req.from_addr,
        from_name: req.from_name,
        web_origin: req.web_origin,
        enabled: req.enabled,
    }
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/smtp",
    tag = "admin-smtp",
    operation_id = "admin_smtp_get",
    responses(
        (status = 200, description = "Current SMTP config (password redacted)", body = SmtpConfigResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is not an admin", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_smtp<C: SmtpConfigStore, U: UserStore>(
    State((cfg_store, _users)): State<(Arc<C>, Arc<U>)>,
    Extension(kek): Extension<Arc<Kek>>,
    _admin: RequireAdmin,
) -> Response {
    match cfg_store.get(&kek).await {
        Ok(rec) => (StatusCode::OK, Json(to_response(&rec))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "smtp_config get failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1/admin/smtp",
    tag = "admin-smtp",
    operation_id = "admin_smtp_put",
    request_body = SmtpConfigRequest,
    responses(
        (status = 200, description = "Config persisted and mailer reloaded", body = SmtpConfigResponse),
        (status = 400, description = "Validation failed", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is not an admin", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn put_smtp<C: SmtpConfigStore, U: UserStore>(
    State((cfg_store, _users)): State<(Arc<C>, Arc<U>)>,
    Extension(kek): Extension<Arc<Kek>>,
    Extension(mailer): Extension<Arc<SwappableMailer>>,
    RequireAdmin(admin): RequireAdmin,
    Json(req): Json<SmtpConfigRequest>,
) -> Response {
    if let Err(resp) = validate(&req) {
        return resp;
    }

    let admin_id = match Uuid::parse_str(&admin.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    let rec = merge(req);
    if let Err(e) = cfg_store.put(rec, &kek, Some(admin_id)).await {
        tracing::error!(error = %e, "smtp_config put failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    // Re-read so the response reflects exactly what's on disk (and so
    // the password-keep semantics are observable: if the PUT passed
    // null, the response's `password_set` matches the prior value).
    let stored = match cfg_store.get(&kek).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "smtp_config re-read after put failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    let new_mailer: Arc<dyn Mailer> = if stored.enabled {
        build_mailer_from_record(&stored)
    } else {
        // Swap to noop so old credentials don't stay in memory when
        // an admin disables the integration.
        tracing::info!("smtp disabled by admin; swapping to noop mailer");
        Arc::new(NoopMailer)
    };
    mailer.swap(new_mailer);

    (StatusCode::OK, Json(to_response(&stored))).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/smtp/test",
    tag = "admin-smtp",
    operation_id = "admin_smtp_test",
    request_body = TestSendRequest,
    responses(
        (status = 200, description = "Test email sent (to caller's own email or to_address override)", body = TestSendResponse),
        (status = 400, description = "Caller's email is not verified, or to_address is malformed", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is not an admin", body = ApiErrorBody),
        (status = 502, description = "Mailer send failed", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn test_smtp<C: SmtpConfigStore, U: UserStore>(
    State((_cfg_store, users)): State<(Arc<C>, Arc<U>)>,
    Extension(mailer): Extension<Arc<SwappableMailer>>,
    RequireAdmin(admin): RequireAdmin,
    body: Option<Json<TestSendRequest>>,
) -> Response {
    let admin_id = match Uuid::parse_str(&admin.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    let user = match users.find_by_id(admin_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in /v1/admin/smtp/test");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // Resolve recipient. Optional `to_address` lets an admin
    // bootstrap from a state where their own email is unverified
    // (the gate below would otherwise deadlock first-time setup).
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let override_to = req
        .to_address
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let recipient: String = if let Some(to) = override_to {
        if !to.contains('@') || to.len() > 320 {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid_to_address",
                Some("to_address must be a valid email address".into()),
            );
        }
        to.to_string()
    } else {
        if user.email_verified_at.is_none() {
            return error(
                StatusCode::BAD_REQUEST,
                "email_unverified",
                Some(
                    "verify your email or pass `to_address` in the request body \
                     to send the test to a known-working external mailbox"
                        .into(),
                ),
            );
        }
        user.email.clone()
    };

    if let Err(e) = mailer
        .send_test_email(&recipient, &user.claimed_handle)
        .await
    {
        // `{e:#}` flattens anyhow's full context chain into the message —
        // without it the user only sees the outermost wrap ("SMTP send
        // failed") and the actual cause (auth rejected, TLS handshake,
        // connection refused, …) is invisible.
        tracing::warn!(error = %format!("{e:#}"), "smtp test send failed");
        return error(
            StatusCode::BAD_GATEWAY,
            "smtp_send_failed",
            Some(format!("{e:#}")),
        );
    }

    (
        StatusCode::OK,
        Json(TestSendResponse { sent_to: recipient }),
    )
        .into_response()
}

// -- Router ----------------------------------------------------------

/// Build the SMTP-admin sub-router. Caller is responsible for adding
/// the request-level extensions this module reads: `Arc<Kek>` and
/// `Arc<SwappableMailer>` (in addition to the admin-routes baseline of
/// `Arc<AuthVerifier>`, `Arc<dyn StaffRoleStore>`, `Arc<dyn UserStore>`).
pub fn router<C: SmtpConfigStore, U: UserStore>(
    config_store: Arc<C>,
    user_store: Arc<U>,
) -> Router {
    Router::new()
        .route(
            "/v1/admin/smtp",
            get(get_smtp::<C, U>).put(put_smtp::<C, U>),
        )
        .route("/v1/admin/smtp/test", post(test_smtp::<C, U>))
        .with_state((config_store, user_store))
}
