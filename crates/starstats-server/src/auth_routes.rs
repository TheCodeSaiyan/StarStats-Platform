//! First-party account endpoints.
//!
//!  - `POST /v1/auth/signup` — email + password + RSI handle → user JWT.
//!  - `POST /v1/auth/login`  — email + password → user JWT.
//!
//! Both endpoints respond with the same shape so the web client has a
//! single happy-path. The token returned is identical to what gets
//! handed back from any future OAuth completion handler — there's
//! one canonical "user JWT" that all browser sessions speak.
//!
//! Out of scope for this slice (intentional):
//!  - password reset
//!  - rate limiting (lives at the proxy / Slice 4 hardening)
//!  - device pairing (Slice 3)

use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::{AuthenticatedUser, TokenIssuer};
use crate::devices::DeviceStore;
use crate::mail::Mailer;
use crate::staff_roles::{StaffRoleSet, StaffRoleStore};
use crate::users::{hash_password, verify_password, PostgresUserStore, UserError, UserStore};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Extension, Router,
};
use chrono::{Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use utoipa::ToSchema;
use uuid::Uuid;

/// Build the `/v1/auth/*` sub-router with a per-IP rate limit.
///
/// ~1 req/s sustained with a burst of 10, keyed by `X-Forwarded-For`
/// (Traefik in prod) with peer addr fallback for direct dev hits.
/// Bearer-authenticated routes (`/v1/me/*`, `/v1/ingest`) sit outside
/// this layer — they're already gated by token validation.
pub fn routes(users: Arc<PostgresUserStore>) -> Router {
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(10)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("auth governor config builder produced no config"),
    );
    let governor_layer = GovernorLayer {
        config: governor_conf,
    };

    Router::new()
        .route("/v1/auth/signup", post(signup::<PostgresUserStore>))
        .route("/v1/auth/login", post(login::<PostgresUserStore>))
        .route(
            "/v1/auth/email/verify",
            post(verify_email::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/email/resend",
            post(resend_verification::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/me/password",
            post(change_password::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/me",
            get(get_me::<PostgresUserStore>).delete(delete_account::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/password/reset/start",
            post(password_reset_start::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/password/reset/complete",
            post(password_reset_complete::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/email/change/start",
            post(email_change_start::<PostgresUserStore>),
        )
        .route(
            "/v1/auth/email/change/verify",
            post(email_change_verify::<PostgresUserStore>),
        )
        .with_state(users)
        .layer(governor_layer)
}

/// Verification tokens live for 24 hours. After that, the user has to
/// request a fresh one (resend flow not yet implemented — Slice 4).
const VERIFICATION_TOKEN_TTL_HOURS: i64 = 24;

const MIN_PASSWORD_LEN: usize = 12;

#[derive(Debug, Deserialize, ToSchema)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub claimed_handle: String,
    /// Beta invite, minted by the waitlist on admission. Required only
    /// while `waitlist_config.gate_enabled` is true.
    ///
    /// `serde(default)` keeps every already-shipped client
    /// wire-compatible: a tray or web build that predates the gate simply
    /// omits the field, and while the gate is off that is still a valid
    /// signup. Adding a non-defaulted field here would 400 every old
    /// client the moment this deploys.
    #[serde(default)]
    pub invite_token: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthResponse {
    /// Either a full user JWT (when `totp_required` is false) or a
    /// 5-minute interim token the client trades for a real JWT via
    /// `POST /v1/auth/totp/verify-login` (when `totp_required` is
    /// true). Single field instead of an enum so client code treats
    /// the success path uniformly until it actually needs to branch.
    pub token: String,
    pub user_id: String,
    pub claimed_handle: String,
    /// `true` when the account has TOTP enabled and the client must
    /// prompt for a 6-digit code before the session is usable.
    /// Defaults to `false` for backwards-compat with existing
    /// callers — signup never sets this.
    #[serde(default)]
    pub totp_required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VerifyEmailResponse {
    pub verified: bool,
    pub email: String,
    pub claimed_handle: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChangePasswordResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ResendVerificationResponse {
    pub sent: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAccountRequest {
    pub confirm_handle: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeleteAccountResponse {
    pub deleted: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MeResponse {
    pub user_id: String,
    pub email: String,
    pub claimed_handle: String,
    pub email_verified: bool,
    /// Address the user has requested to switch *to* — absent when no
    /// change is in flight. The settings page surfaces this so the
    /// user knows a verification email is outstanding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_email: Option<String>,
    /// `true` once the user has proven ownership of `claimed_handle`
    /// via the RSI bio flow. Public profiles + org shares are gated
    /// on this flag.
    pub rsi_verified: bool,
    /// `true` once the user has confirmed a TOTP secret. The settings
    /// page uses this to branch the 2FA wizard between the "enable"
    /// and "manage" states without an extra round trip.
    pub totp_enabled: bool,
    /// Site-wide staff grants the user holds, sorted alphabetically.
    /// Empty for normal users; populated for moderators / admins.
    /// The web client mirrors this into the session cookie so /admin
    /// gating doesn't need an extra round trip per page nav.
    /// Older clients tolerate the field via `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staff_roles: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    detail: Option<String>,
}

/// Give a spent invite back after a signup fails downstream.
///
/// Signup redeems the invite before creating the user, so any failure
/// after that point (taken handle, taken email, argon2 blowing up) would
/// otherwise burn it and eject someone from the beta over a typo.
///
/// Best-effort by design: a failure here must not mask the real error the
/// caller needs to see. It is logged at ERROR because the consequence —
/// a person locked out holding a dead link — is invisible from every
/// other angle.
async fn release_invite(waitlist: &Arc<dyn crate::waitlist::WaitlistStore>, token: Option<&str>) {
    let Some(t) = token else {
        return;
    };
    if let Err(e) = waitlist.release_invite(t).await {
        tracing::error!(error = %e, "release_invite failed — invite burnt, user locked out");
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/signup",
    tag = "auth",
    request_body = SignupRequest,
    responses(
        (status = 200, description = "User created; bearer token returned", body = AuthResponse),
        (status = 400, description = "Validation failure", body = ApiErrorBody),
        (status = 403, description = "Invite required or invalid (public-beta gate on)", body = ApiErrorBody),
        (status = 409, description = "Email or handle already in use", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    )
)]
pub async fn signup<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(issuer): Extension<Arc<TokenIssuer>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Extension(spicedb): Extension<Arc<Option<crate::spicedb::SpicedbClient>>>,
    Extension(waitlist): Extension<Arc<dyn crate::waitlist::WaitlistStore>>,
    Json(req): Json<SignupRequest>,
) -> impl IntoResponse {
    if let Some(resp) = validate_signup(&req) {
        return resp;
    }

    // Public-beta gate. Checked before the argon2 hash so a rejected
    // signup costs no CPU.
    //
    // Fails CLOSED: if the switch cannot be read we refuse rather than
    // silently open signups to the world. An unreadable gate that admits
    // everyone is the exact failure this project keeps hitting — the
    // mechanism looks fine and does nothing.
    let gate_on = match waitlist.gate_enabled().await {
        Ok(on) => on,
        Err(e) => {
            tracing::error!(error = %e, "waitlist gate_enabled read failed — refusing signup");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "gate_unavailable", None);
        }
    };

    // Held so the invite can be released if user creation then fails.
    let mut redeemed_invite: Option<&str> = None;

    if gate_on {
        let Some(token) = req.invite_token.as_deref().filter(|t| !t.is_empty()) else {
            return error(
                StatusCode::FORBIDDEN,
                "invite_required",
                Some("StarStats is in invite-only beta — join the waitlist".into()),
            );
        };
        match waitlist.redeem_invite(token).await {
            Ok(Some(_)) => redeemed_invite = Some(token),
            Ok(None) => {
                return error(
                    StatusCode::FORBIDDEN,
                    "invite_invalid",
                    Some("that invite is not valid, or has already been used".into()),
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "waitlist redeem_invite failed — refusing signup");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "gate_unavailable", None);
            }
        }
    }

    // The invite is spent from here on. Every path that fails below MUST
    // release it, or a typo'd handle ejects someone from the beta with no
    // way back in.
    let phc = match hash_password(&req.password) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "argon2 hash failed");
            release_invite(&waitlist, redeemed_invite).await;
            return error(StatusCode::INTERNAL_SERVER_ERROR, "hash_failed", None);
        }
    };

    let user = match users.create(&req.email, &phc, &req.claimed_handle).await {
        Ok(u) => u,
        Err(UserError::EmailTaken) => {
            release_invite(&waitlist, redeemed_invite).await;
            return error(
                StatusCode::CONFLICT,
                "email_taken",
                Some("an account with that email already exists".into()),
            );
        }
        Err(UserError::HandleTaken) => {
            release_invite(&waitlist, redeemed_invite).await;
            return error(
                StatusCode::CONFLICT,
                "handle_taken",
                Some("that RSI handle is already claimed".into()),
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "create user failed");
            release_invite(&waitlist, redeemed_invite).await;
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // Best-effort: register the new user as the owner of their own
    // stats_record in SpiceDB. Without this relation, any future
    // SpiceDB-gated read of the user's own data would 403 (the gate
    // sees an empty `view` permission set). Failure here is logged
    // and ignored — we don't block signup on a SpiceDB blip, and a
    // missing relation is fixed by any subsequent call that re-issues
    // the TOUCH (e.g. the user re-saving public_view from /settings).
    if let Some(client) = spicedb.as_ref() {
        if let Err(e) = client.write_owner(&user.claimed_handle).await {
            tracing::warn!(
                error = %e,
                user_id = %user.id,
                handle = %user.claimed_handle,
                "SpiceDB owner relation write failed on signup"
            );
        }
    }

    // Best-effort verification email. Token persistence and mail send
    // are both swallowed on failure — the user account exists, the
    // session is good, and they can request another link via the
    // resend flow (Slice 4). Never block signup on a flaky mail relay.
    let token = generate_verification_token();
    let expires_at = Utc::now() + Duration::hours(VERIFICATION_TOKEN_TTL_HOURS);
    if let Err(e) = users
        .set_verification_token(user.id, &token, expires_at)
        .await
    {
        tracing::warn!(error = %e, user_id = %user.id, "persist verification token failed");
    } else if let Err(e) = mailer
        .send_verification(&user.email, &user.claimed_handle, &token)
        .await
    {
        tracing::warn!(error = %format!("{e:#}"), user_id = %user.id, "send verification email failed");
    }

    issue_token(&issuer, &user.id.to_string(), &user.claimed_handle)
}

#[utoipa::path(
    post,
    path = "/v1/auth/email/verify",
    tag = "auth",
    request_body = VerifyEmailRequest,
    responses(
        (status = 200, description = "Email verified", body = VerifyEmailResponse),
        (status = 400, description = "Token invalid or expired", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    )
)]
pub async fn verify_email<U: UserStore>(
    State(users): State<Arc<U>>,
    Json(req): Json<VerifyEmailRequest>,
) -> impl IntoResponse {
    if req.token.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
    }

    let user = match users.find_by_verification_token(&req.token).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
        }
        Err(e) => {
            tracing::error!(error = %e, "verification token lookup failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // Expiry check: a token without an expiry is a corrupted row
    // (shouldn't happen — set_verification_token always sets both).
    // Treat that the same as expired so the client message stays
    // consistent.
    let expired = user
        .email_verification_expires_at
        .map(|exp| Utc::now() >= exp)
        .unwrap_or(true);
    if expired {
        return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
    }

    if let Err(e) = users.mark_email_verified(user.id).await {
        tracing::error!(error = %e, user_id = %user.id, "mark verified failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    (
        StatusCode::OK,
        Json(VerifyEmailResponse {
            verified: true,
            email: user.email,
            claimed_handle: user.claimed_handle,
        }),
    )
        .into_response()
}

// -- Account management ----------------------------------------------

/// Resolve `user.sub` to a `Uuid` and fetch the row. Centralises the
/// "bad subject" / "missing user" handling so each account-management
/// handler stays focused on its happy path.
async fn resolve_user<U: UserStore>(
    users: &U,
    user: &AuthenticatedUser,
) -> Result<crate::users::User, axum::response::Response> {
    let user_id = match Uuid::parse_str(&user.sub) {
        Ok(id) => id,
        Err(_) => {
            tracing::error!(sub = %user.sub, "user JWT sub is not a UUID");
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "bad_subject",
                None,
            ));
        }
    };
    match users.find_by_id(user_id).await {
        Ok(Some(u)) => Ok(u),
        Ok(None) => {
            // Authenticated against a now-missing row — token is stale
            // (e.g. another tab deleted the account). Treat as unauth.
            Err(error(StatusCode::UNAUTHORIZED, "user_not_found", None))
        }
        Err(e) => {
            tracing::error!(error = %e, "find user by id failed");
            Err(error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None))
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/me/password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password updated", body = ChangePasswordResponse),
        (status = 400, description = "New password too short", body = ApiErrorBody),
        (status = 401, description = "Current password wrong, or missing/invalid bearer token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn change_password<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
    Json(req): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    let user = match resolve_user(users.as_ref(), &auth).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    if !verify_password(&req.current_password, &user.password_hash) {
        return error(StatusCode::UNAUTHORIZED, "invalid_credentials", None);
    }

    if req.new_password.len() < MIN_PASSWORD_LEN {
        return error(
            StatusCode::BAD_REQUEST,
            "password_too_short",
            Some(format!("minimum {MIN_PASSWORD_LEN} characters")),
        );
    }

    let new_phc = match hash_password(&req.new_password) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "argon2 hash failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    if let Err(e) = users.update_password(user.id, &new_phc).await {
        tracing::error!(error = %e, user_id = %user.id, "update password failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    // Audit AFTER the change so a row only exists when the password
    // actually rolled. Payload deliberately carries the user_id only —
    // never any password material, plaintext or hashed.
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(user.id.to_string()),
            actor_handle: Some(user.claimed_handle.clone()),
            action: "user.password_changed".to_string(),
            payload: serde_json::json!({ "user_id": user.id.to_string() }),
        })
        .await
    {
        tracing::warn!(error = %e, user_id = %user.id, "audit log append failed (password change)");
    }

    (StatusCode::OK, Json(ChangePasswordResponse { ok: true })).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/email/resend",
    tag = "auth",
    responses(
        (status = 200, description = "Verification email queued (best-effort)", body = ResendVerificationResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 409, description = "Email already verified", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn resend_verification<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
) -> impl IntoResponse {
    let user = match resolve_user(users.as_ref(), &auth).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    if user.email_verified_at.is_some() {
        return error(StatusCode::CONFLICT, "already_verified", None);
    }

    let token = generate_verification_token();
    let expires_at = Utc::now() + Duration::hours(VERIFICATION_TOKEN_TTL_HOURS);
    if let Err(e) = users
        .set_verification_token(user.id, &token, expires_at)
        .await
    {
        tracing::error!(error = %e, user_id = %user.id, "persist verification token failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    // Mail send failure is logged but doesn't propagate — same posture
    // as signup. Returning 200 here keeps SMTP state out of the API
    // contract; the user simply requests another resend if nothing
    // arrives.
    if let Err(e) = mailer
        .send_verification(&user.email, &user.claimed_handle, &token)
        .await
    {
        tracing::warn!(error = %format!("{e:#}"), user_id = %user.id, "send verification email failed");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(user.id.to_string()),
            actor_handle: Some(user.claimed_handle.clone()),
            action: "user.email_verification_resent".to_string(),
            payload: serde_json::json!({ "user_id": user.id.to_string() }),
        })
        .await
    {
        tracing::warn!(error = %e, user_id = %user.id, "audit log append failed (resend)");
    }

    (
        StatusCode::OK,
        Json(ResendVerificationResponse { sent: true }),
    )
        .into_response()
}

#[utoipa::path(
    delete,
    path = "/v1/auth/me",
    tag = "auth",
    request_body = DeleteAccountRequest,
    responses(
        (status = 200, description = "Account deleted", body = DeleteAccountResponse),
        (status = 400, description = "Confirmation handle did not match", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn delete_account<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    auth: AuthenticatedUser,
    Json(req): Json<DeleteAccountRequest>,
) -> impl IntoResponse {
    let user = match resolve_user(users.as_ref(), &auth).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    if !req
        .confirm_handle
        .eq_ignore_ascii_case(&user.claimed_handle)
    {
        return error(
            StatusCode::BAD_REQUEST,
            "confirm_mismatch",
            Some("confirm_handle must match the account handle".into()),
        );
    }

    // Audit BEFORE the delete: the audit row references the user_id we
    // are about to remove; emitting after risks a FK-style orphan
    // (the audit table doesn't FK users, but the chain still records
    // the actor, and we want that record to predate the disappearance).
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(user.id.to_string()),
            actor_handle: Some(user.claimed_handle.clone()),
            action: "user.account_deleted".to_string(),
            payload: serde_json::json!({
                "user_id": user.id.to_string(),
                "claimed_handle": user.claimed_handle,
            }),
        })
        .await
    {
        // A failed audit append is *not* a green light to delete — if
        // we can't record the action we shouldn't perform it.
        tracing::error!(error = %e, user_id = %user.id, "audit append failed; aborting delete");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    if let Err(e) = users.delete_user(user.id).await {
        tracing::error!(error = %e, user_id = %user.id, "delete user failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    (
        StatusCode::OK,
        Json(DeleteAccountResponse { deleted: true }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/auth/me",
    tag = "auth",
    responses(
        (status = 200, description = "Authenticated user info", body = MeResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn get_me<U: UserStore>(
    State(users): State<Arc<U>>,
    staff_roles: Option<Extension<Arc<dyn StaffRoleStore>>>,
    auth: AuthenticatedUser,
) -> impl IntoResponse {
    let user = match resolve_user(users.as_ref(), &auth).await {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    // Roles: only looked up if the StaffRoleStore extension is
    // installed. Tests that don't wire the extension see an empty
    // vec; production wires PostgresStaffRoleStore in main.
    let roles = match staff_roles {
        Some(Extension(store)) => match store.list_active_for_user(user.id).await {
            Ok(set) => set.as_strings(),
            Err(e) => {
                tracing::error!(user_id = %user.id, err = ?e, "get_me: staff role lookup failed");
                StaffRoleSet::new().as_strings()
            }
        },
        None => StaffRoleSet::new().as_strings(),
    };

    (
        StatusCode::OK,
        Json(MeResponse {
            user_id: user.id.to_string(),
            email: user.email,
            claimed_handle: user.claimed_handle,
            email_verified: user.email_verified_at.is_some(),
            pending_email: user.pending_email,
            rsi_verified: user.rsi_verified_at.is_some(),
            totp_enabled: user.totp_enabled_at.is_some(),
            staff_roles: roles,
        }),
    )
        .into_response()
}

/// 32 random bytes -> 64-char hex token. The partial unique index on
/// `email_verification_token` enforces collision-freeness at the DB
/// layer — practically irrelevant at 256 bits of entropy, but it
/// catches the bug where a hash function drift makes us re-issue a
/// stale token verbatim.
fn generate_verification_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Authenticated; bearer token returned", body = AuthResponse),
        (status = 401, description = "Invalid credentials", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    )
)]
pub async fn login<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(issuer): Extension<Arc<TokenIssuer>>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let lookup = match users.find_by_email(&req.email).await {
        Ok(opt) => opt,
        Err(e) => {
            tracing::error!(error = %e, "lookup user failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    // Constant-ish response shape regardless of whether the email
    // exists — don't leak account existence to a casual probe.
    let Some(user) = lookup else {
        return error(StatusCode::UNAUTHORIZED, "invalid_credentials", None);
    };

    if !verify_password(&req.password, &user.password_hash) {
        return error(StatusCode::UNAUTHORIZED, "invalid_credentials", None);
    }

    if user.totp_enabled_at.is_some() {
        return issue_login_interim(&issuer, &user.id.to_string(), &user.claimed_handle);
    }

    issue_token(&issuer, &user.id.to_string(), &user.claimed_handle)
}

// -- helpers ---------------------------------------------------------

/// Returns `Some(response)` when the request fails validation,
/// `None` when it's good to proceed. Phrased this way (instead of
/// `Result<(), Response>`) because clippy flags 128-byte `Err`
/// variants and `Option` carries the same intent.
fn validate_signup(req: &SignupRequest) -> Option<axum::response::Response> {
    if !looks_like_email(&req.email) {
        return Some(error(
            StatusCode::BAD_REQUEST,
            "invalid_email",
            Some("email must contain @ and a domain".into()),
        ));
    }
    if req.password.len() < MIN_PASSWORD_LEN {
        return Some(error(
            StatusCode::BAD_REQUEST,
            "password_too_short",
            Some(format!("minimum {MIN_PASSWORD_LEN} characters")),
        ));
    }
    if req.claimed_handle.trim().is_empty() {
        return Some(error(
            StatusCode::BAD_REQUEST,
            "missing_handle",
            Some("claimed_handle is required".into()),
        ));
    }
    if !crate::users::validate_handle(req.claimed_handle.trim()) {
        return Some(error(
            StatusCode::BAD_REQUEST,
            "invalid_handle",
            Some("handle must be 1-64 ASCII letters, digits, '_' or '-'".into()),
        ));
    }
    None
}

fn looks_like_email(s: &str) -> bool {
    // Deliberately loose — RFC-correct validation requires a parser
    // and most real validation happens via the verification email.
    let trimmed = s.trim();
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
}

fn issue_token(
    issuer: &TokenIssuer,
    user_id: &str,
    claimed_handle: &str,
) -> axum::response::Response {
    match issuer.sign_user(user_id, claimed_handle) {
        Ok(token) => (
            StatusCode::OK,
            Json(AuthResponse {
                token,
                user_id: user_id.to_owned(),
                claimed_handle: claimed_handle.to_owned(),
                totp_required: false,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "sign user token failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "sign_failed", None)
        }
    }
}

/// Mint a 5-minute interim token for the TOTP-required login leg.
/// Wire-compatible with [`AuthResponse`] — clients see the same
/// shape, but `totp_required: true` tells them to prompt for a
/// 6-digit code and post to `/v1/auth/totp/verify-login` rather
/// than store the token as a session.
pub fn issue_login_interim(
    issuer: &TokenIssuer,
    user_id: &str,
    claimed_handle: &str,
) -> axum::response::Response {
    match issuer.sign_login_interim(user_id, claimed_handle) {
        Ok(token) => (
            StatusCode::OK,
            Json(AuthResponse {
                token,
                user_id: user_id.to_owned(),
                claimed_handle: claimed_handle.to_owned(),
                totp_required: true,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "sign login-interim token failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "sign_failed", None)
        }
    }
}

// -- Password reset --------------------------------------------------
//
// Two endpoints:
//   POST /v1/auth/password/reset/start    body: { email }
//   POST /v1/auth/password/reset/complete body: { token, new_password }
//
// `start` is intentionally enumeration-safe: every call returns
// `{ sent: true }` regardless of whether the email matches an
// account, and the email itself is best-effort. `complete` validates
// the token, rotates the hash, bumps `password_changed_at`, and
// revokes every paired device — forcing re-pair after a credential
// change is the single biggest mitigation against a stolen token.

const PASSWORD_RESET_TTL_MIN: i64 = 30;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordResetStartRequest {
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PasswordResetStartResponse {
    /// Always `true`. Anti-enumeration: callers can't tell whether
    /// the email mapped to a real account from this field alone.
    pub sent: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PasswordResetCompleteRequest {
    pub token: String,
    pub new_password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PasswordResetCompleteResponse {
    pub ok: bool,
}

#[utoipa::path(
    post,
    path = "/v1/auth/password/reset/start",
    tag = "auth",
    request_body = PasswordResetStartRequest,
    responses(
        (status = 200, description = "If the email matches an account, a reset link has been emailed",
         body = PasswordResetStartResponse),
        (status = 400, description = "Malformed email", body = ApiErrorBody),
    )
)]
pub async fn password_reset_start<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Json(req): Json<PasswordResetStartRequest>,
) -> impl IntoResponse {
    let email = req.email.trim();
    if email.is_empty() || !email.contains('@') {
        return error(StatusCode::BAD_REQUEST, "invalid_email", None);
    }
    // Always return 200 sent: true regardless of outcome below. Any
    // branch that observes "user not found" or a mailer failure must
    // produce the same response so the public can't enumerate
    // accounts via this endpoint.
    if let Ok(Some(user)) = users.find_by_email(email).await {
        let token = generate_verification_token();
        let expires_at = Utc::now() + Duration::minutes(PASSWORD_RESET_TTL_MIN);
        if let Err(e) = users
            .set_password_reset_token(user.id, &token, expires_at)
            .await
        {
            tracing::warn!(error = %e, user_id = %user.id, "persist reset token failed");
        } else if let Err(e) = mailer
            .send_password_reset(&user.email, &user.claimed_handle, &token)
            .await
        {
            tracing::warn!(error = %format!("{e:#}"), user_id = %user.id, "send reset email failed");
        }
    } else {
        // Sleep briefly to flatten the timing oracle — a fast
        // 200 = "no account", a slow 200 = "account found, hashing".
        // 50ms is well below user-perceptible latency but smudges the
        // signal enough to defeat naive enumeration.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    (
        StatusCode::OK,
        Json(PasswordResetStartResponse { sent: true }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/password/reset/complete",
    tag = "auth",
    request_body = PasswordResetCompleteRequest,
    responses(
        (status = 200, description = "Password rotated; all devices revoked", body = PasswordResetCompleteResponse),
        (status = 400, description = "Token unknown / expired or password too short", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    )
)]
pub async fn password_reset_complete<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(devices): Extension<Arc<dyn DeviceStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(req): Json<PasswordResetCompleteRequest>,
) -> impl IntoResponse {
    if req.token.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
    }
    if req.new_password.len() < MIN_PASSWORD_LEN {
        return error(
            StatusCode::BAD_REQUEST,
            "password_too_short",
            Some(format!(
                "password must be at least {MIN_PASSWORD_LEN} characters"
            )),
        );
    }

    let user = match users.find_by_password_reset_token(&req.token).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None),
        Err(e) => {
            tracing::error!(error = %e, "reset token lookup failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let expired = user
        .password_reset_expires_at
        .map(|exp| Utc::now() > exp)
        .unwrap_or(true);
    if expired {
        return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
    }

    let phc = match hash_password(&req.new_password) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "argon2 hash failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "hash_failed", None);
        }
    };
    if let Err(e) = users.complete_password_reset(user.id, &phc).await {
        tracing::error!(error = %e, "complete_password_reset failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }

    // Force re-pair on every device. A failure here is logged but
    // doesn't fail the request — the password change itself was the
    // critical step. Stale device tokens still bear an `iat` that
    // predates the new `password_changed_at`; once we wire the JWT
    // iat check (see comment in users.rs), they become unverifiable
    // even without server-side revocation.
    if let Err(e) = devices.revoke_all_for_user(user.id).await {
        tracing::warn!(error = %e, user_id = %user.id, "device revocation after reset failed");
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(user.id.to_string()),
            actor_handle: Some(user.claimed_handle.clone()),
            action: "user.password_reset_completed".to_string(),
            payload: serde_json::json!({}),
        })
        .await
    {
        tracing::warn!(error = %e, "audit append failed (password reset)");
    }

    (
        StatusCode::OK,
        Json(PasswordResetCompleteResponse { ok: true }),
    )
        .into_response()
}

// -- Email change ----------------------------------------------------

const EMAIL_CHANGE_TTL_HOURS: i64 = 24;

#[derive(Debug, Deserialize, ToSchema)]
pub struct EmailChangeStartRequest {
    pub new_email: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EmailChangeStartResponse {
    pub pending_email: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EmailChangeVerifyRequest {
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EmailChangeVerifyResponse {
    pub email: String,
}

#[utoipa::path(
    post,
    path = "/v1/auth/email/change/start",
    tag = "auth",
    request_body = EmailChangeStartRequest,
    responses(
        (status = 200, description = "Pending email staged; verification link emailed",
         body = EmailChangeStartResponse),
        (status = 400, description = "Malformed email", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 409, description = "Email already in use", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn email_change_start<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    auth: AuthenticatedUser,
    Json(req): Json<EmailChangeStartRequest>,
) -> impl IntoResponse {
    let new_email = req.new_email.trim().to_lowercase();
    if new_email.is_empty() || !new_email.contains('@') {
        return error(StatusCode::BAD_REQUEST, "invalid_email", None);
    }

    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    // Anti-enumeration is less critical for an authenticated flow,
    // but we still don't want to leak whether the staged address
    // belongs to another user. find_by_email here gates the 409.
    if let Ok(Some(other)) = users.find_by_email(&new_email).await {
        if other.id != user_id {
            return error(StatusCode::CONFLICT, "email_taken", None);
        }
    }

    let token = generate_verification_token();
    let expires_at = Utc::now() + Duration::hours(EMAIL_CHANGE_TTL_HOURS);
    if let Err(e) = users
        .set_pending_email(user_id, &new_email, &token, expires_at)
        .await
    {
        tracing::error!(error = %e, "set_pending_email failed");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
    }
    if let Err(e) = mailer
        .send_email_change_verify(&new_email, &auth.preferred_username, &token)
        .await
    {
        tracing::warn!(error = %format!("{e:#}"), "send email-change verify failed");
    }
    (
        StatusCode::OK,
        Json(EmailChangeStartResponse {
            pending_email: new_email,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/auth/email/change/verify",
    tag = "auth",
    request_body = EmailChangeVerifyRequest,
    responses(
        (status = 200, description = "Email rotated", body = EmailChangeVerifyResponse),
        (status = 400, description = "Token unknown or expired", body = ApiErrorBody),
        (status = 409, description = "Staged email is taken", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    )
)]
pub async fn email_change_verify<U: UserStore>(
    State(users): State<Arc<U>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(req): Json<EmailChangeVerifyRequest>,
) -> impl IntoResponse {
    if req.token.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
    }
    let user = match users.find_by_pending_email_token(&req.token).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None),
        Err(e) => {
            tracing::error!(error = %e, "pending email token lookup failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let expired = user
        .pending_email_expires_at
        .map(|exp| Utc::now() > exp)
        .unwrap_or(true);
    if expired {
        return error(StatusCode::BAD_REQUEST, "invalid_or_expired", None);
    }
    let new_email = user.pending_email.clone().unwrap_or_default();
    match users.commit_pending_email(user.id).await {
        Ok(()) => {}
        Err(UserError::EmailTaken) => {
            return error(StatusCode::CONFLICT, "email_taken", None);
        }
        Err(e) => {
            tracing::error!(error = %e, "commit_pending_email failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(user.id.to_string()),
            actor_handle: Some(user.claimed_handle.clone()),
            action: "user.email_changed".to_string(),
            payload: serde_json::json!({ "new_email": new_email }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit append failed (email_changed)");
    }

    (
        StatusCode::OK,
        Json(EmailChangeVerifyResponse { email: new_email }),
    )
        .into_response()
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

// -- tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::mail::NoopMailer;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::StaffRole;
    use crate::users::test_support::MemoryUserStore;
    use crate::waitlist::test_support::MemoryWaitlistStore;
    use crate::waitlist::{SignupOutcome, WaitlistStore};
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::routing::{delete as delete_route, post};
    use axum::Router;

    use tower::ServiceExt;

    /// Build a router whose verifier matches its issuer so token
    /// round-trips work in tests.
    fn router_with_matching_verifier(users: Arc<MemoryUserStore>) -> (Router, AuthVerifier) {
        router_with_waitlist(users, Arc::new(MemoryWaitlistStore::open()))
    }

    /// As above, but with an explicit waitlist store so the beta-gate
    /// tests can supply one with the gate on.
    fn router_with_waitlist(
        users: Arc<MemoryUserStore>,
        waitlist: Arc<MemoryWaitlistStore>,
    ) -> (Router, AuthVerifier) {
        let (issuer, verifier) = fresh_pair();
        let issuer_arc = Arc::new(issuer);
        let mailer: Arc<dyn Mailer> = Arc::new(NoopMailer);
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let staff_roles_mem = Arc::new(MemoryStaffRoleStore::default());
        let staff_roles_dyn: Arc<dyn StaffRoleStore> = staff_roles_mem.clone();
        let waitlist_dyn: Arc<dyn WaitlistStore> = waitlist;
        let app = Router::new()
            .route("/v1/auth/signup", post(signup::<MemoryUserStore>))
            .route("/v1/auth/login", post(login::<MemoryUserStore>))
            .route(
                "/v1/auth/email/verify",
                post(verify_email::<MemoryUserStore>),
            )
            .layer(Extension(issuer_arc))
            .layer(Extension(mailer))
            .layer(Extension(audit))
            .layer(Extension(staff_roles_dyn))
            // signup requires this. Gate defaults OFF here, matching
            // migration 0050 — these tests assert pre-gate behaviour and
            // the gate must not disturb it until an admin opts in.
            .layer(Extension(waitlist_dyn))
            // signup reads this and skips the SpiceDB call when None.
            // Tests don't stand up a real SpiceDB; mirroring the
            // optional-extension pattern from main.rs keeps the test
            // path identical to a real "SPICEDB_URL not set" deploy.
            .layer(Extension(Arc::new(None::<crate::spicedb::SpicedbClient>)))
            .with_state(users);
        (app, verifier)
    }

    /// Variant that wires the four account-management routes with a
    /// matching verifier. Returns the router, an issuer for minting
    /// bearer tokens, and the in-memory audit log so tests can assert
    /// on the recorded actions.
    fn account_router(
        users: Arc<MemoryUserStore>,
    ) -> (
        Router,
        Arc<TokenIssuer>,
        Arc<MemoryAuditLog>,
        Arc<MemoryStaffRoleStore>,
    ) {
        let (issuer, verifier) = fresh_pair();
        let issuer_arc = Arc::new(issuer);
        let verifier_arc = Arc::new(verifier);
        let mailer: Arc<dyn Mailer> = Arc::new(NoopMailer);
        let audit_mem = Arc::new(MemoryAuditLog::default());
        let audit_dyn: Arc<dyn AuditLog> = audit_mem.clone();
        let staff_roles_mem = Arc::new(MemoryStaffRoleStore::default());
        let staff_roles_dyn: Arc<dyn StaffRoleStore> = staff_roles_mem.clone();

        let app = Router::new()
            .route(
                "/v1/auth/me/password",
                post(change_password::<MemoryUserStore>),
            )
            .route(
                "/v1/auth/email/resend",
                post(resend_verification::<MemoryUserStore>),
            )
            .route(
                "/v1/auth/me",
                delete_route(delete_account::<MemoryUserStore>).get(get_me::<MemoryUserStore>),
            )
            .layer(Extension(issuer_arc.clone()))
            .layer(Extension(verifier_arc))
            .layer(Extension(mailer))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_roles_dyn))
            .with_state(users);

        (app, issuer_arc, audit_mem, staff_roles_mem)
    }

    /// Mint a user JWT for `user` so tests can hit the protected
    /// account endpoints without going through signup/login.
    fn token_for(issuer: &TokenIssuer, user: &crate::users::User) -> String {
        issuer
            .sign_user(&user.id.to_string(), &user.claimed_handle)
            .expect("sign user token")
    }

    async fn request_with_bearer<B: serde::Serialize>(
        app: &Router,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&B>,
    ) -> (StatusCode, axum::body::Bytes) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"));
        let req_body = match body {
            Some(b) => {
                builder = builder.header("content-type", "application/json");
                axum::body::Body::from(serde_json::to_vec(b).unwrap())
            }
            None => axum::body::Body::empty(),
        };
        let req = builder.body(req_body).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, bytes)
    }

    async fn post_json<B: serde::Serialize>(
        app: &Router,
        path: &str,
        body: &B,
    ) -> (StatusCode, axum::body::Bytes) {
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, bytes)
    }

    #[tokio::test]
    async fn signup_creates_user_and_returns_verifiable_token() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, verifier) = router_with_matching_verifier(users);

        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (status, bytes) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::OK);
        let resp: AuthResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp.claimed_handle, "TheCodeSaiyan");
        let claims = verifier.verify(&resp.token).unwrap();
        assert_eq!(claims.preferred_username, "TheCodeSaiyan");
    }

    // -- Public-beta gate --------------------------------------------

    #[tokio::test]
    async fn signup_without_invite_is_rejected_when_gate_is_on() {
        let users = Arc::new(MemoryUserStore::new());
        let waitlist = Arc::new(MemoryWaitlistStore::with_cap(0));
        let (app, _) = router_with_waitlist(users, waitlist);

        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (status, bytes) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"], "invite_required");
    }

    #[tokio::test]
    async fn signup_with_a_valid_invite_succeeds_when_gate_is_on() {
        let users = Arc::new(MemoryUserStore::new());
        let waitlist = Arc::new(MemoryWaitlistStore::with_cap(1));
        let SignupOutcome::Admitted { invite_token } =
            waitlist.signup("daisy@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        let (app, _) = router_with_waitlist(users, waitlist);

        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
            "invite_token": invite_token,
        });
        let (status, _) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_invite_cannot_be_used_twice() {
        let users = Arc::new(MemoryUserStore::new());
        let waitlist = Arc::new(MemoryWaitlistStore::with_cap(1));
        let SignupOutcome::Admitted { invite_token } =
            waitlist.signup("daisy@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        let (app, _) = router_with_waitlist(users, waitlist);

        let first = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
            "invite_token": invite_token,
        });
        let (status, _) = post_json(&app, "/v1/auth/signup", &first).await;
        assert_eq!(status, StatusCode::OK);

        // A forwarded invite email must not mint a second account.
        let second = serde_json::json!({
            "email": "someone-else@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "SomeoneElse",
            "invite_token": invite_token,
        });
        let (status, bytes) = post_json(&app, "/v1/auth/signup", &second).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"], "invite_invalid");
    }

    #[tokio::test]
    async fn an_unknown_invite_is_rejected() {
        let users = Arc::new(MemoryUserStore::new());
        let waitlist = Arc::new(MemoryWaitlistStore::with_cap(1));
        let (app, _) = router_with_waitlist(users, waitlist);

        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
            "invite_token": "not-a-real-token",
        });
        let (status, bytes) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"], "invite_invalid");
    }

    #[tokio::test]
    async fn signup_without_invite_still_works_when_gate_is_off() {
        // Gate off is today's behaviour AND the default after migration
        // 0050. Deploying the gate must lock nobody out.
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_waitlist(users, Arc::new(MemoryWaitlistStore::open()));

        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (status, _) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_failed_signup_does_not_burn_the_invite() {
        let users = Arc::new(MemoryUserStore::new());
        let waitlist = Arc::new(MemoryWaitlistStore::with_cap(2));
        let SignupOutcome::Admitted { invite_token } =
            waitlist.signup("first@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        let (app, _) = router_with_waitlist(users, waitlist.clone());

        // Claim the handle with an unrelated account first.
        let SignupOutcome::Admitted {
            invite_token: other,
        } = waitlist.signup("other@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        let squatter = serde_json::json!({
            "email": "other@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
            "invite_token": other,
        });
        assert_eq!(
            post_json(&app, "/v1/auth/signup", &squatter).await.0,
            StatusCode::OK
        );

        // Now our invitee collides on the handle.
        let body = serde_json::json!({
            "email": "first@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
            "invite_token": invite_token,
        });
        let (status, _) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Picking a taken handle is not a reason to eject someone from
        // the beta — their invite must survive to be retried.
        assert!(waitlist
            .redeem_invite(&invite_token)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn signup_rejects_short_password() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users);
        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "short",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (status, _) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signup_rejects_invalid_email() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users);
        let body = serde_json::json!({
            "email": "not-an-email",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (status, _) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signup_rejects_duplicate_email() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users);
        let body = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (s1, _) = post_json(&app, "/v1/auth/signup", &body).await;
        assert_eq!(s1, StatusCode::OK);
        let body2 = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "OtherHandle",
        });
        let (s2, _) = post_json(&app, "/v1/auth/signup", &body2).await;
        assert_eq!(s2, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn login_returns_token_for_valid_credentials() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, verifier) = router_with_matching_verifier(users);

        let signup = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (s, _) = post_json(&app, "/v1/auth/signup", &signup).await;
        assert_eq!(s, StatusCode::OK);

        let login = serde_json::json!({
            "email": "DAISY@example.com",
            "password": "supersecret-1234",
        });
        let (status, bytes) = post_json(&app, "/v1/auth/login", &login).await;
        assert_eq!(status, StatusCode::OK);
        let resp: AuthResponse = serde_json::from_slice(&bytes).unwrap();
        let claims = verifier.verify(&resp.token).unwrap();
        assert_eq!(claims.preferred_username, "TheCodeSaiyan");
    }

    #[tokio::test]
    async fn login_rejects_wrong_password() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users);

        let signup = serde_json::json!({
            "email": "daisy@example.com",
            "password": "supersecret-1234",
            "claimed_handle": "TheCodeSaiyan",
        });
        let (_, _) = post_json(&app, "/v1/auth/signup", &signup).await;

        let login = serde_json::json!({
            "email": "daisy@example.com",
            "password": "wrong-password",
        });
        let (status, _) = post_json(&app, "/v1/auth/login", &login).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_rejects_nonexistent_email() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users);
        let login = serde_json::json!({
            "email": "ghost@example.com",
            "password": "supersecret-1234",
        });
        let (status, _) = post_json(&app, "/v1/auth/login", &login).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // -- email verification ------------------------------------------

    /// Drive a real signup through the router and pull the freshly-set
    /// token straight out of the in-memory store. Exercising the
    /// router (not just the helper) ensures the `Extension(mailer)`
    /// layer is wired and that the best-effort token persist actually
    /// happened — the same surface a real client would hit.
    async fn signup_and_grab_token(
        app: &Router,
        users: &Arc<MemoryUserStore>,
        email: &str,
        handle: &str,
    ) -> String {
        let body = serde_json::json!({
            "email": email,
            "password": "supersecret-1234",
            "claimed_handle": handle,
        });
        let (status, _) = post_json(app, "/v1/auth/signup", &body).await;
        assert_eq!(status, StatusCode::OK);

        let user = users
            .find_by_email(email)
            .await
            .unwrap()
            .expect("user persisted");
        let tokens = users.tokens.lock().unwrap();
        tokens
            .iter()
            .find_map(|(t, (uid, _))| (uid == &user.id).then(|| t.clone()))
            .expect("verification token persisted")
    }

    #[tokio::test]
    async fn verify_with_valid_token_marks_user_verified() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users.clone());
        let token = signup_and_grab_token(&app, &users, "daisy@example.com", "TheCodeSaiyan").await;

        let body = serde_json::json!({ "token": token });
        let (status, bytes) = post_json(&app, "/v1/auth/email/verify", &body).await;
        assert_eq!(status, StatusCode::OK);
        let resp: VerifyEmailResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(resp.verified);
        assert_eq!(resp.claimed_handle, "TheCodeSaiyan");
        assert_eq!(resp.email, "daisy@example.com");

        // The user row should now carry an `email_verified_at` and the
        // token should be cleared — replaying the same token returns
        // the unified invalid/expired error.
        let user = users
            .find_by_email("daisy@example.com")
            .await
            .unwrap()
            .unwrap();
        assert!(user.email_verified_at.is_some());

        let (replay_status, _) = post_json(&app, "/v1/auth/email/verify", &body).await;
        assert_eq!(replay_status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn verify_rejects_unknown_token() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users);

        let body = serde_json::json!({ "token": "0".repeat(64) });
        let (status, bytes) = post_json(&app, "/v1/auth/email/verify", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"], "invalid_or_expired");
    }

    #[tokio::test]
    async fn verify_rejects_expired_token() {
        let users = Arc::new(MemoryUserStore::new());
        let (app, _) = router_with_matching_verifier(users.clone());

        // Sign up to get a valid user, then forcibly rewind the token's
        // expiry so the handler sees it as stale. We poke the store
        // directly because there's no resend endpoint yet to drive an
        // expired token through the public surface.
        let _ = signup_and_grab_token(&app, &users, "stale@example.com", "ExpiredHandle").await;
        let user = users
            .find_by_email("stale@example.com")
            .await
            .unwrap()
            .unwrap();

        // Replace the in-memory token with a known one that's already
        // expired by an hour.
        let stale_token = "deadbeef".repeat(8); // 64 hex chars
        let stale_when = Utc::now() - Duration::hours(1);
        users
            .set_verification_token(user.id, &stale_token, stale_when)
            .await
            .unwrap();

        let body = serde_json::json!({ "token": stale_token });
        let (status, bytes) = post_json(&app, "/v1/auth/email/verify", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"], "invalid_or_expired");
    }

    // -- Account management ------------------------------------------

    /// Seed a user via the store directly so tests don't need to drive
    /// the signup endpoint just to obtain a working bearer token.
    async fn seed_user(
        users: &MemoryUserStore,
        email: &str,
        password: &str,
        handle: &str,
    ) -> crate::users::User {
        let phc = hash_password(password).expect("hash");
        users.create(email, &phc, handle).await.expect("seed user")
    }

    #[tokio::test]
    async fn change_password_with_correct_current_succeeds() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, audit, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let body = serde_json::json!({
            "current_password": "supersecret-1234",
            "new_password": "another-strong-password",
        });
        let (status, bytes) =
            request_with_bearer(&app, "POST", "/v1/auth/me/password", &token, Some(&body)).await;
        assert_eq!(status, StatusCode::OK);
        let parsed: ChangePasswordResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.ok);

        // Login (manually) with the new password by re-checking the
        // store's hash.
        let updated = users.find_by_id(user.id).await.unwrap().unwrap();
        assert!(verify_password(
            "another-strong-password",
            &updated.password_hash
        ));
        assert!(!verify_password("supersecret-1234", &updated.password_hash));

        // Audit recorded with the right action and no password material.
        let entries = audit.snapshot();
        assert!(entries
            .iter()
            .any(|e| e.action == "user.password_changed"
                && e.payload["user_id"] == user.id.to_string()));
    }

    #[tokio::test]
    async fn change_password_rejects_wrong_current() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, audit, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let body = serde_json::json!({
            "current_password": "wrong-password",
            "new_password": "another-strong-password",
        });
        let (status, bytes) =
            request_with_bearer(&app, "POST", "/v1/auth/me/password", &token, Some(&body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"], "invalid_credentials");

        // Hash unchanged; no audit row.
        let unchanged = users.find_by_id(user.id).await.unwrap().unwrap();
        assert!(verify_password(
            "supersecret-1234",
            &unchanged.password_hash
        ));
        assert!(audit
            .snapshot()
            .iter()
            .all(|e| e.action != "user.password_changed"));
    }

    #[tokio::test]
    async fn change_password_rejects_short_new() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, _, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let body = serde_json::json!({
            "current_password": "supersecret-1234",
            "new_password": "short",
        });
        let (status, bytes) =
            request_with_bearer(&app, "POST", "/v1/auth/me/password", &token, Some(&body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"], "password_too_short");
    }

    #[tokio::test]
    async fn resend_verification_for_unverified_user_returns_sent() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, audit, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let (status, bytes) = request_with_bearer::<serde_json::Value>(
            &app,
            "POST",
            "/v1/auth/email/resend",
            &token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let parsed: ResendVerificationResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.sent);

        // A token was persisted for the user.
        let tokens = users.tokens.lock().unwrap();
        assert!(tokens.values().any(|(uid, _)| *uid == user.id));
        drop(tokens);

        let entries = audit.snapshot();
        assert!(entries
            .iter()
            .any(|e| e.action == "user.email_verification_resent"));
    }

    #[tokio::test]
    async fn resend_verification_for_verified_user_returns_409() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        users.mark_email_verified(user.id).await.unwrap();

        let (app, issuer, _, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let (status, bytes) = request_with_bearer::<serde_json::Value>(
            &app,
            "POST",
            "/v1/auth/email/resend",
            &token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"], "already_verified");
    }

    #[tokio::test]
    async fn delete_account_with_matching_handle_succeeds_and_devices_gone() {
        // Seed a user, mint a device for them via the device store,
        // then hit DELETE /v1/auth/me with a matching confirm_handle.
        // After delete we expect:
        //  - user row gone,
        //  - device row gone (mirrors the Postgres ON DELETE CASCADE
        //    by clearing the Memory device store explicitly).
        use crate::devices::test_support::MemoryDeviceStore;
        use crate::devices::{DeviceStore, PAIRING_TTL};

        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;

        let device_store = Arc::new(MemoryDeviceStore::new());
        let pairing = device_store
            .create_pairing(user.id, "PC", PAIRING_TTL)
            .await
            .unwrap();
        let redeemed = device_store.redeem(&pairing.code).await.unwrap();
        assert_eq!(
            device_store
                .device_auth_status(redeemed.device_id)
                .await
                .unwrap(),
            crate::devices::DeviceAuthStatus::Active,
        );

        let (app, issuer, audit, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let body = serde_json::json!({ "confirm_handle": "thecodesaiyan" });
        let (status, bytes) =
            request_with_bearer(&app, "DELETE", "/v1/auth/me", &token, Some(&body)).await;
        assert_eq!(status, StatusCode::OK);
        let parsed: DeleteAccountResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.deleted);

        assert!(users.find_by_id(user.id).await.unwrap().is_none());

        // The MemoryDeviceStore doesn't FK against MemoryUserStore, so
        // we mimic the Postgres cascade by revoking here. The point is
        // to assert the wiring: the handler must have deleted the user,
        // and that deletion is the trigger for the device cleanup. We
        // leave the explicit device DELETE to the Postgres FK.
        device_store
            .revoke(user.id, redeemed.device_id)
            .await
            .unwrap();
        assert_eq!(
            device_store
                .device_auth_status(redeemed.device_id)
                .await
                .unwrap(),
            crate::devices::DeviceAuthStatus::Revoked,
        );

        // Audit predates the actual delete (i.e. exists in the log).
        let entries = audit.snapshot();
        let entry = entries
            .iter()
            .find(|e| e.action == "user.account_deleted")
            .expect("audit entry recorded");
        assert_eq!(entry.payload["user_id"], user.id.to_string());
        assert_eq!(entry.payload["claimed_handle"], "TheCodeSaiyan");
    }

    #[tokio::test]
    async fn delete_account_with_wrong_handle_returns_400() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, audit, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        let body = serde_json::json!({ "confirm_handle": "WrongHandle" });
        let (status, bytes) =
            request_with_bearer(&app, "DELETE", "/v1/auth/me", &token, Some(&body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["error"], "confirm_mismatch");

        // User row still exists; no audit row for delete.
        assert!(users.find_by_id(user.id).await.unwrap().is_some());
        assert!(audit
            .snapshot()
            .iter()
            .all(|e| e.action != "user.account_deleted"));
    }

    #[tokio::test]
    async fn get_me_returns_email_verified_flag() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, _, _) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        // Unverified — flag should be false.
        let (status, bytes) =
            request_with_bearer::<serde_json::Value>(&app, "GET", "/v1/auth/me", &token, None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let parsed: MeResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.user_id, user.id.to_string());
        assert_eq!(parsed.email, "daisy@example.com");
        assert_eq!(parsed.claimed_handle, "TheCodeSaiyan");
        assert!(!parsed.email_verified);

        // Mark verified — flag flips.
        users.mark_email_verified(user.id).await.unwrap();
        let (status, bytes) =
            request_with_bearer::<serde_json::Value>(&app, "GET", "/v1/auth/me", &token, None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let parsed: MeResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.email_verified);
    }

    #[tokio::test]
    async fn get_me_returns_staff_roles_when_granted() {
        let users = Arc::new(MemoryUserStore::new());
        let user = seed_user(
            users.as_ref(),
            "daisy@example.com",
            "supersecret-1234",
            "TheCodeSaiyan",
        )
        .await;
        let (app, issuer, _, staff_roles_mem) = account_router(users.clone());
        let token = token_for(&issuer, &user);

        // No grants — staff_roles should be empty.
        let (status, bytes) =
            request_with_bearer::<serde_json::Value>(&app, "GET", "/v1/auth/me", &token, None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let parsed: MeResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.staff_roles.is_empty());

        // Grant admin — staff_roles should now contain "admin".
        staff_roles_mem
            .grant(user.id, StaffRole::Admin, None, None)
            .await
            .unwrap();

        let (status, bytes) =
            request_with_bearer::<serde_json::Value>(&app, "GET", "/v1/auth/me", &token, None)
                .await;
        assert_eq!(status, StatusCode::OK);
        let parsed: MeResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.staff_roles, vec!["admin".to_string()]);
    }
}
