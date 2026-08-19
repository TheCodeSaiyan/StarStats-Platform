//! TOTP 2FA endpoints.
//!
//!  - `POST /v1/auth/totp/setup`              (user JWT) — generate
//!    a fresh secret, encrypt with KEK, return the plaintext secret
//!    + provisioning URI for the user's authenticator app.
//!  - `POST /v1/auth/totp/confirm`            (user JWT) — submit
//!    the first 6-digit code; on match, mark TOTP enabled and return
//!    a fresh batch of 10 recovery codes (one-time display).
//!  - `POST /v1/auth/totp/disable`            (user JWT) — wipe
//!    TOTP secret + recovery codes after re-confirming the password.
//!  - `POST /v1/auth/totp/recovery/regenerate` (user JWT) — replace
//!    the recovery-code set with a fresh batch.
//!  - `POST /v1/auth/totp/verify-login`       (interim JWT) — second
//!    leg of the login flow. Accepts either a 6-digit TOTP code or
//!    a recovery code, returns a real session JWT on success.

use crate::api_error::ApiErrorBody;
use crate::auth::{AuthenticatedUser, TokenIssuer, TokenType};
use crate::auth_routes::AuthResponse;
use crate::kek::Kek;
use crate::recovery_codes::{PostgresRecoveryCodeStore, RecoveryCodeStore};
use crate::totp::{generate_secret, provisioning_uri, secret_to_base32, verify_now};
use crate::users::{verify_password, PostgresUserStore, UserStore};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(users: Arc<PostgresUserStore>, recovery: Arc<PostgresRecoveryCodeStore>) -> Router {
    // Per-user brute-force lockout on verify-login. In-memory, so a
    // multi-replica deployment needs a shared store to be authoritative —
    // tracked as a follow-up; the GovernorLayer below still throttles
    // per-IP across replicas at the load balancer's granularity.
    let limiter = Arc::new(TotpAttemptLimiter::new());

    // Per-IP rate limit mirroring `auth_routes::routes()` — the 2FA leg is
    // a prime credential-stuffing / code-guessing target.
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(10)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("totp governor config builder produced no config"),
    );
    let governor_layer = GovernorLayer {
        config: governor_conf,
    };

    Router::new()
        .route("/v1/auth/totp/setup", post(setup::<PostgresUserStore>))
        .route("/v1/auth/totp/qr", post(render_qr))
        .route(
            "/v1/auth/totp/confirm",
            post(confirm::<PostgresUserStore, PostgresRecoveryCodeStore>),
        )
        .route(
            "/v1/auth/totp/disable",
            post(disable::<PostgresUserStore, PostgresRecoveryCodeStore>),
        )
        .route(
            "/v1/auth/totp/recovery/regenerate",
            post(regenerate_recovery::<PostgresUserStore, PostgresRecoveryCodeStore>),
        )
        .route(
            "/v1/auth/totp/verify-login",
            post(verify_login::<PostgresUserStore, PostgresRecoveryCodeStore>),
        )
        .with_state((users, recovery))
        .layer(Extension(limiter))
        .layer(governor_layer)
}

/// Maximum failed `verify-login` attempts per user within
/// [`TOTP_LOCKOUT_WINDOW`] before the account is locked out.
const TOTP_MAX_FAILS: u32 = 5;

/// Lockout window — sized to the interim-token TTL (300s, see
/// [`crate::auth::TokenIssuer::sign_login_interim`]). Once the window
/// elapses the counter resets, so a genuine user who fat-fingered the
/// code just waits out their (already short-lived) interim token.
const TOTP_LOCKOUT_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
struct AttemptState {
    fails: u32,
    window_start: Instant,
}

/// Process-wide per-user brute-force limiter for the 2FA verify leg.
/// Wired in as an axum `Extension` by [`routes`]. `Mutex` contention is
/// per-user and the critical section is a couple of field writes, so a
/// plain mutex is fine. Mirrors the `VoteRateLimiter` shape.
#[derive(Default)]
pub struct TotpAttemptLimiter {
    attempts: Mutex<HashMap<Uuid, AttemptState>>,
}

impl TotpAttemptLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the user has hit [`TOTP_MAX_FAILS`] within the current
    /// window. Checked BEFORE the code is verified so a locked user is
    /// rejected even if they now present the correct code. A stale
    /// (expired-window) entry is cleared and treated as unlocked.
    fn is_locked(&self, user_id: Uuid) -> bool {
        let mut map = self.attempts.lock().unwrap();
        match map.get(&user_id) {
            Some(s) if s.window_start.elapsed() < TOTP_LOCKOUT_WINDOW => s.fails >= TOTP_MAX_FAILS,
            Some(_) => {
                map.remove(&user_id);
                false
            }
            None => false,
        }
    }

    /// Record one failed attempt, resetting the window if the previous
    /// one has elapsed. Returns the running fail count within the window.
    fn record_failure(&self, user_id: Uuid) -> u32 {
        let mut map = self.attempts.lock().unwrap();
        let now = Instant::now();
        let entry = map.entry(user_id).or_insert(AttemptState {
            fails: 0,
            window_start: now,
        });
        if entry.window_start.elapsed() >= TOTP_LOCKOUT_WINDOW {
            entry.fails = 0;
            entry.window_start = now;
        }
        entry.fails += 1;
        entry.fails
    }

    /// Clear a user's failure record on a successful verify.
    fn record_success(&self, user_id: Uuid) {
        self.attempts.lock().unwrap().remove(&user_id);
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TotpSetupResponse {
    /// The base32-encoded TOTP shared secret. Show this once. The
    /// user types it into their authenticator app (or scans the
    /// `provisioning_uri` QR).
    pub secret_base32: String,
    /// `otpauth://totp/...` URI suitable for QR-code rendering.
    pub provisioning_uri: String,
    /// Human-readable label used in the URI, surfaced so the
    /// frontend can echo it back ("This will appear in your
    /// authenticator as 'StarStats:alice@example.com'").
    pub account_label: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpQrRequest {
    /// The `otpauth://` URI to encode. Sent in the BODY, never a query
    /// string: it carries the shared secret, which must not reach an
    /// access log or a `Referer` header.
    pub provisioning_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TotpQrResponse {
    /// `data:image/svg+xml;base64,...`
    pub data_uri: String,
}

/// Render a provisioning URI as a QR code.
///
/// Exists because the enrolment payload lives in a ~4 KB cookie and a
/// QR data URI is roughly 9 KB — storing it there would silently drop
/// the cookie and lose enrolment state. The caller keeps the small URI
/// and asks for the picture when it needs to draw it.
///
/// This replaced a third-party image service (`api.qrserver.com`), which
/// received the shared secret on every enrolment.
#[utoipa::path(
    post,
    path = "/v1/auth/totp/qr",
    tag = "auth",
    request_body = TotpQrRequest,
    responses(
        (status = 200, description = "QR as a data URI", body = TotpQrResponse),
        (status = 400, description = "Not a TOTP provisioning URI"),
    )
)]
pub async fn render_qr(Json(req): Json<TotpQrRequest>) -> Response {
    // Only ever encode an otpauth URI. Without this the endpoint is a
    // free QR generator for arbitrary attacker-supplied content served
    // from our origin.
    if !req.provisioning_uri.starts_with("otpauth://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "not_a_provisioning_uri" })),
        )
            .into_response();
    }
    match crate::totp::provisioning_qr_data_uri(&req.provisioning_uri) {
        Ok(data_uri) => (StatusCode::OK, Json(TotpQrResponse { data_uri })).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "TOTP QR render failed");
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "qr_render_failed" })),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpConfirmRequest {
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TotpConfirmResponse {
    pub enabled: bool,
    /// Plaintext recovery codes — show once, never again. The user
    /// is expected to save these somewhere offline.
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpDisableRequest {
    /// Re-confirm the user's password before tearing down 2FA. The
    /// session bearer alone isn't enough — we want a "you are who
    /// you say you are RIGHT NOW" check, not just "you logged in
    /// some time in the last hour."
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TotpDisableResponse {
    pub disabled: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegenerateRecoveryRequest {
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RegenerateRecoveryResponse {
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyLoginRequest {
    /// Either a 6-digit TOTP code from the authenticator app, or a
    /// recovery code (`XXXX-XXXX-XXXX-XXXX`). The handler
    /// distinguishes by length.
    pub code: String,
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

fn require_user_token(user: &AuthenticatedUser) -> Option<axum::response::Response> {
    if !matches!(user.token_type, TokenType::User) {
        return Some(error(StatusCode::FORBIDDEN, "user_token_required", None));
    }
    None
}

fn parse_user_id(auth: &AuthenticatedUser) -> Result<Uuid, axum::response::Response> {
    Uuid::parse_str(&auth.sub)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None))
}

#[utoipa::path(
    post,
    path = "/v1/auth/totp/setup",
    tag = "totp",
    operation_id = "totp_setup",
    responses(
        (status = 200, description = "Secret issued (un-confirmed)", body = TotpSetupResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device or interim token", body = ApiErrorBody),
        (status = 409, description = "TOTP already enabled — disable first to re-pair", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn setup<U: UserStore>(
    State((users, _recovery)): State<(Arc<U>, Arc<PostgresRecoveryCodeStore>)>,
    Extension(kek): Extension<Arc<Kek>>,
    auth: AuthenticatedUser,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id in totp setup");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    if user.totp_enabled_at.is_some() {
        return error(
            StatusCode::CONFLICT,
            "totp_already_enabled",
            Some("disable TOTP before re-pairing".into()),
        );
    }

    // Reuse the in-flight secret (if any) so refreshing the setup
    // page doesn't churn the QR code under the user's authenticator
    // app. We only generate a fresh secret when there isn't one
    // already, which means a user who abandons setup gets a clean
    // restart on their next visit.
    let secret_bytes = match decrypt_user_secret(users.as_ref(), kek.as_ref(), user_id).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            let fresh = generate_secret();
            let (ct, nonce) = match kek.encrypt(&fresh) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(error = %e, "kek encrypt failed in totp setup");
                    return error(StatusCode::INTERNAL_SERVER_ERROR, "encrypt_failed", None);
                }
            };
            if let Err(e) = users.start_totp_setup(user_id, &ct, &nonce).await {
                tracing::error!(error = %e, "start_totp_setup failed");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
            fresh.to_vec()
        }
        Err(resp) => return resp,
    };

    let secret_b32 = secret_to_base32(&secret_bytes);
    let label = format!("StarStats:{}", user.email);
    let uri = provisioning_uri(&secret_b32, "StarStats", &user.email);
    (
        StatusCode::OK,
        Json(TotpSetupResponse {
            secret_base32: secret_b32,
            provisioning_uri: uri,
            account_label: label,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/totp/confirm",
    tag = "totp",
    operation_id = "totp_confirm",
    request_body = TotpConfirmRequest,
    responses(
        (status = 200, description = "TOTP enabled, recovery codes returned (one-time display)", body = TotpConfirmResponse),
        (status = 400, description = "Invalid code format", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller is a device or interim token", body = ApiErrorBody),
        (status = 409, description = "TOTP already enabled or no setup in flight", body = ApiErrorBody),
        (status = 422, description = "Code did not match", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn confirm<U: UserStore, R: RecoveryCodeStore>(
    State((users, recovery)): State<(Arc<U>, Arc<R>)>,
    Extension(kek): Extension<Arc<Kek>>,
    auth: AuthenticatedUser,
    Json(req): Json<TotpConfirmRequest>,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id in totp confirm");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    if user.totp_enabled_at.is_some() {
        return error(StatusCode::CONFLICT, "totp_already_enabled", None);
    }

    let secret = match decrypt_user_secret(users.as_ref(), kek.as_ref(), user_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return error(
                StatusCode::CONFLICT,
                "no_setup_in_flight",
                Some("call /v1/auth/totp/setup before /confirm".into()),
            );
        }
        Err(resp) => return resp,
    };

    if !verify_now(&secret, &req.code) {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "code_mismatch", None);
    }

    if let Err(e) = users.mark_totp_enabled(user_id).await {
        tracing::error!(error = %e, "mark_totp_enabled failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }
    let codes = match recovery.regenerate_for_user(user_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "regenerate recovery codes failed");
            // TOTP is already on; don't roll back. The user can
            // hit /recovery/regenerate to retry the code issuance.
            Vec::new()
        }
    };

    (
        StatusCode::OK,
        Json(TotpConfirmResponse {
            enabled: true,
            recovery_codes: codes,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/totp/disable",
    tag = "totp",
    operation_id = "totp_disable",
    request_body = TotpDisableRequest,
    responses(
        (status = 200, description = "TOTP disabled", body = TotpDisableResponse),
        (status = 401, description = "Wrong password or invalid bearer", body = ApiErrorBody),
        (status = 403, description = "Caller is a device or interim token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn disable<U: UserStore, R: RecoveryCodeStore>(
    State((users, recovery)): State<(Arc<U>, Arc<R>)>,
    auth: AuthenticatedUser,
    Json(req): Json<TotpDisableRequest>,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id in totp disable");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    if !verify_password(&req.password, &user.password_hash) {
        return error(StatusCode::UNAUTHORIZED, "invalid_credentials", None);
    }

    // Disable order: clear recovery codes first, then the user-row
    // TOTP fields. If the second step fails, a follow-up retry sees
    // no orphan codes.
    if let Err(e) = recovery.clear_for_user(user_id).await {
        tracing::error!(error = %e, "clear recovery codes failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }
    if let Err(e) = users.disable_totp(user_id).await {
        tracing::error!(error = %e, "disable_totp failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    (StatusCode::OK, Json(TotpDisableResponse { disabled: true })).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/totp/recovery/regenerate",
    tag = "totp",
    operation_id = "totp_recovery_regenerate",
    request_body = RegenerateRecoveryRequest,
    responses(
        (status = 200, description = "Fresh recovery codes returned (one-time display)", body = RegenerateRecoveryResponse),
        (status = 401, description = "Wrong password or invalid bearer", body = ApiErrorBody),
        (status = 403, description = "Caller is a device or interim token", body = ApiErrorBody),
        (status = 409, description = "TOTP isn't enabled", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn regenerate_recovery<U: UserStore, R: RecoveryCodeStore>(
    State((users, recovery)): State<(Arc<U>, Arc<R>)>,
    auth: AuthenticatedUser,
    Json(req): Json<RegenerateRecoveryRequest>,
) -> impl IntoResponse {
    if let Some(resp) = require_user_token(&auth) {
        return resp;
    }
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id in regenerate_recovery");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    if !verify_password(&req.password, &user.password_hash) {
        return error(StatusCode::UNAUTHORIZED, "invalid_credentials", None);
    }
    if user.totp_enabled_at.is_none() {
        return error(StatusCode::CONFLICT, "totp_not_enabled", None);
    }

    let codes = match recovery.regenerate_for_user(user_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "regenerate recovery codes failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    (
        StatusCode::OK,
        Json(RegenerateRecoveryResponse {
            recovery_codes: codes,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/totp/verify-login",
    tag = "totp",
    operation_id = "totp_verify_login",
    request_body = VerifyLoginRequest,
    responses(
        (status = 200, description = "Code matched; full session JWT returned", body = AuthResponse),
        (status = 401, description = "Interim token invalid/expired or code mismatch", body = ApiErrorBody),
        (status = 403, description = "Bearer is not an interim token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn verify_login<U: UserStore, R: RecoveryCodeStore>(
    State((users, recovery)): State<(Arc<U>, Arc<R>)>,
    Extension(kek): Extension<Arc<Kek>>,
    Extension(issuer): Extension<Arc<TokenIssuer>>,
    Extension(limiter): Extension<Arc<TotpAttemptLimiter>>,
    auth: AuthenticatedUser,
    Json(req): Json<VerifyLoginRequest>,
) -> impl IntoResponse {
    // The bearer MUST be a LoginInterim token. Reject everything
    // else — letting a real user JWT call this would let an
    // attacker who phishes a 6-digit code mint a fresh JWT after
    // the original one was supposed to expire.
    if !matches!(auth.token_type, TokenType::LoginInterim) {
        return error(
            StatusCode::FORBIDDEN,
            "interim_token_required",
            Some("call /v1/auth/login first to obtain an interim token".into()),
        );
    }
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    // Brute-force lockout (H6): once the user has burned through
    // TOTP_MAX_FAILS within the window, reject every attempt — including
    // one bearing a now-correct code — so the (short-lived) interim token
    // is effectively dead and the attacker can't keep guessing. The user
    // waits out the window and re-runs the password login.
    if limiter.is_locked(user_id) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_attempts",
            Some("too many failed codes; wait, then start login again".into()),
        );
    }
    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id in verify_login");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    if user.totp_enabled_at.is_none() {
        // The user disabled TOTP between login start and verify.
        // Flag it as 401 — the interim token is no longer
        // meaningful — and let them re-login.
        return error(StatusCode::UNAUTHORIZED, "totp_no_longer_required", None);
    }

    let trimmed = req.code.trim();
    let matched = if looks_like_totp(trimmed) {
        let secret = match decrypt_user_secret(users.as_ref(), kek.as_ref(), user_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::error!("totp enabled but no secret on row in verify_login");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
            Err(resp) => return resp,
        };
        verify_now(&secret, trimmed)
    } else {
        match recovery.redeem(user_id, trimmed).await {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "recovery redeem failed");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
        }
    };

    if !matched {
        let fails = limiter.record_failure(user_id);
        if fails >= TOTP_MAX_FAILS {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_attempts",
                Some("too many failed codes; wait, then start login again".into()),
            );
        }
        return error(StatusCode::UNAUTHORIZED, "code_mismatch", None);
    }

    // Successful verify clears the failure record so a legitimate user
    // isn't penalised by earlier typos.
    limiter.record_success(user_id);

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
            tracing::error!(error = %e, "sign user token in verify_login");
            error(StatusCode::INTERNAL_SERVER_ERROR, "sign_failed", None)
        }
    }
}

/// 6-digit shape: TOTP. Anything else (including the hyphenated
/// recovery format) falls through to the recovery-code path.
fn looks_like_totp(s: &str) -> bool {
    s.len() == 6 && s.chars().all(|c| c.is_ascii_digit())
}

/// Pull the AES-GCM-encrypted TOTP secret off the user row and
/// decrypt it. `Ok(None)` means no setup ever happened. Anything
/// else maps to a 500-shaped response — failed decrypt indicates
/// KEK rotation drift or row corruption, neither of which the user
/// can fix.
async fn decrypt_user_secret<U: UserStore>(
    users: &U,
    kek: &Kek,
    user_id: Uuid,
) -> Result<Option<Vec<u8>>, axum::response::Response> {
    let stored = match users.get_totp_secret(user_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "get_totp_secret failed");
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None));
        }
    };
    let Some((ct, nonce)) = stored else {
        return Ok(None);
    };
    match kek.decrypt(&ct, &nonce) {
        Ok(s) => Ok(Some(s)),
        Err(e) => {
            tracing::error!(error = %e, "kek decrypt failed in totp routes");
            Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "decrypt_failed",
                None,
            ))
        }
    }
}

#[cfg(test)]
mod qr_tests {
    use super::*;

    #[tokio::test]
    async fn refuses_anything_but_a_provisioning_uri() {
        // Without this the endpoint is a free QR generator for arbitrary
        // attacker-supplied content, served from our own origin.
        let resp = render_qr(Json(TotpQrRequest {
            provisioning_uri: "https://evil.example/phish".into(),
        }))
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_a_self_contained_data_uri() {
        let uri = crate::totp::provisioning_uri("JBSWY3DPEHPK3PXP", "StarStats", "a@b.c");
        let resp = render_qr(Json(TotpQrRequest {
            provisioning_uri: uri,
        }))
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let data_uri = v["data_uri"].as_str().unwrap();

        assert!(data_uri.starts_with("data:image/svg+xml;base64,"));
        // The whole point: the shared secret never leaves this origin.
        assert!(!data_uri.contains("qrserver"));
        assert!(!data_uri.contains("http"));
    }
}

#[cfg(test)]
mod limiter_tests {
    use super::*;

    #[test]
    fn locks_after_max_failures() {
        let lim = TotpAttemptLimiter::new();
        let u = Uuid::new_v4();
        assert!(!lim.is_locked(u));
        // The first TOTP_MAX_FAILS-1 failures stay below the threshold.
        for i in 1..TOTP_MAX_FAILS {
            assert_eq!(lim.record_failure(u), i);
            assert!(!lim.is_locked(u), "must not lock below the threshold");
        }
        // The TOTP_MAX_FAILS-th failure trips the lock.
        assert_eq!(lim.record_failure(u), TOTP_MAX_FAILS);
        assert!(lim.is_locked(u), "locked at the threshold");
        // Further attempts (the '6th wrong code, and even a correct one')
        // stay rejected.
        lim.record_failure(u);
        assert!(lim.is_locked(u));
    }

    #[test]
    fn success_clears_the_counter() {
        let lim = TotpAttemptLimiter::new();
        let u = Uuid::new_v4();
        for _ in 0..TOTP_MAX_FAILS {
            lim.record_failure(u);
        }
        assert!(lim.is_locked(u));
        lim.record_success(u);
        assert!(!lim.is_locked(u), "a successful verify resets the lock");
        assert_eq!(
            lim.record_failure(u),
            1,
            "the counter starts fresh after success",
        );
    }

    #[test]
    fn lockout_is_per_user() {
        let lim = TotpAttemptLimiter::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        for _ in 0..TOTP_MAX_FAILS {
            lim.record_failure(a);
        }
        assert!(lim.is_locked(a));
        assert!(!lim.is_locked(b), "one user's failures never lock another");
    }
}
