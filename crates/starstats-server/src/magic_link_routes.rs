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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use utoipa::ToSchema;
use uuid::Uuid;

/// How many magic links one account may have mailed within
/// [`MAGIC_SEND_WINDOW`]. Per-IP limiting alone cannot stop a
/// distributed mail-bomb aimed at a single address; this caps what any
/// number of sources can do to one inbox.
const MAGIC_SENDS_PER_USER: u32 = 3;

/// Window for [`MAGIC_SENDS_PER_USER`]. Matches the link TTL: three
/// live links at once is already more than a person needs.
const MAGIC_SEND_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Process-wide per-account cap on magic-link sends.
///
/// The governor below is keyed by IP, which stops one host hammering
/// `start` and does nothing at all about many hosts aimed at one
/// address — and every one of those requests puts real mail in a real
/// inbox. This caps the inbox side. Mirrors `TotpAttemptLimiter`:
/// in-memory and per-process, so a multi-replica deploy allows the cap
/// per replica. That is a deliberate floor, not a ceiling — the
/// alternative is a round trip to shared state on a login path.
#[derive(Default)]
pub struct MagicSendLimiter {
    sends: Mutex<HashMap<Uuid, SendState>>,
}

struct SendState {
    count: u32,
    window_start: Instant,
}

impl MagicSendLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a send and report whether it may go out. Returns false
    /// once the account has burned [`MAGIC_SENDS_PER_USER`] within the
    /// window; a stale window resets first.
    fn try_send(&self, user_id: Uuid) -> bool {
        let mut map = self.sends.lock().unwrap();
        let entry = map.entry(user_id).or_insert(SendState {
            count: 0,
            window_start: Instant::now(),
        });
        if entry.window_start.elapsed() >= MAGIC_SEND_WINDOW {
            entry.count = 0;
            entry.window_start = Instant::now();
        }
        if entry.count >= MAGIC_SENDS_PER_USER {
            return false;
        }
        entry.count += 1;
        true
    }
}

/// Sustained per-IP allowance for `/v1/auth/magic/*`, in requests
/// per second.
const MAGIC_PER_SECOND: u64 = 1;

/// Per-IP burst allowance for `/v1/auth/magic/*`. Tighter than the
/// `/v1/auth/*` router's burst of 10 because a `start` hit sends real
/// mail; a human signing in needs one request, not five.
const MAGIC_BURST_SIZE: u32 = 5;

/// Build the `/v1/auth/magic/*` sub-router, with its own per-IP rate
/// limit.
///
/// Two sub-routers because the start handler needs the user store +
/// magic-link store, and the redeem handler needs both *plus* the
/// issuer (which lives in Extensions). Bundling them with a shared
/// state tuple keeps `main.rs` clean.
///
/// The limiter is built HERE and not inherited: `auth_routes::routes()`
/// layers its governor onto its own `Router`, and `main.rs` merges this
/// one in as a sibling. A tower layer is scoped to the routes of the
/// router it was applied to, so sharing the `/v1/auth/` path prefix
/// grants no coverage — these two routes went unmetered from the day
/// they were split out. `start` mails a real link to any address on
/// file, so unmetered it is an email cannon aimed at any guessable
/// user. Regression guard: `magic_start_is_rate_limited_per_ip`.
pub fn routes(users: Arc<PostgresUserStore>, magic: Arc<PostgresMagicLinkStore>) -> Router {
    let governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(MAGIC_PER_SECOND)
            .burst_size(MAGIC_BURST_SIZE)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("magic-link governor config builder produced no config"),
    );

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
        .layer(Extension(Arc::new(MagicSendLimiter::new())))
        .layer(GovernorLayer { config: governor })
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
    Extension(send_limiter): Extension<Arc<MagicSendLimiter>>,
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

    // Cap per account, and answer exactly as the happy path does —
    // a distinguishable "throttled" response would turn the limiter
    // into the enumeration oracle the 200-always policy exists to
    // prevent.
    if !send_limiter.try_send(user.id) {
        tracing::warn!(user_id = %user.id, "magic link send throttled for this account");
        return Json(MagicLinkStartResponse { sent: true }).into_response();
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Build the real `/v1/auth/magic/*` router over a lazy pool.
    ///
    /// No Postgres is needed: the rate limiter rejects before the
    /// handler runs, and the handful of requests that DO reach a
    /// handler fail their connection fast (50 ms acquire timeout) and
    /// surface as 500 — which is not the status under test.
    fn test_router() -> Router {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy pool");
        routes(
            Arc::new(PostgresUserStore::new(pool.clone())),
            Arc::new(PostgresMagicLinkStore::new(pool)),
        )
    }

    async fn post_start(app: &Router) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/magic/start")
                    .header("content-type", "application/json")
                    .header("x-forwarded-for", "203.0.113.7")
                    .body(Body::from(r#"{"email":"probe@example.com"}"#))
                    .unwrap(),
            )
            .await
            .expect("router responds")
            .status()
    }

    /// `/v1/auth/magic/start` mails a real link to any address on file
    /// and needs no auth, so an unmetered endpoint is an email cannon
    /// pointed at any user whose address an attacker can guess.
    ///
    /// Regression guard for the gap found 2026-09-09: the governor
    /// layer built in `auth_routes::routes()` covers only that
    /// router's own routes. This router is merged into the app as a
    /// sibling, so sharing the `/v1/auth/` prefix bought it nothing —
    /// it has to carry its own layer.
    #[tokio::test]
    async fn magic_start_is_rate_limited_per_ip() {
        let app = test_router();
        let mut saw_429 = false;
        for _ in 0..(MAGIC_BURST_SIZE + 4) {
            if post_start(&app).await == StatusCode::TOO_MANY_REQUESTS {
                saw_429 = true;
                break;
            }
        }
        assert!(
            saw_429,
            "POST /v1/auth/magic/start was never rate limited — an \
             unauthenticated caller can mail-bomb any known address"
        );
    }
}

#[cfg(test)]
mod send_throttle_tests {
    use super::*;
    use crate::magic_link::test_support::MemoryMagicLinkStore;
    use crate::users::test_support::MemoryUserStore;
    use crate::users::UserStore;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    /// Counts magic-link sends; every other send is a no-op.
    #[derive(Default)]
    struct CountingMailer {
        magic_sends: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Mailer for CountingMailer {
        async fn send_verification(&self, _a: &str, _n: &str, _t: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn send_password_reset(&self, _a: &str, _n: &str, _t: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn send_email_change_verify(
            &self,
            _a: &str,
            _n: &str,
            _t: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn send_magic_link(&self, _a: &str, _n: &str, _t: &str) -> anyhow::Result<()> {
            self.magic_sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn send_test_email(&self, _a: &str, _n: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn send_waitlist_invite(&self, _a: &str, _t: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// The per-IP governor stops one host hammering `start`. It does
    /// nothing about a thousand hosts aimed at one address, and every
    /// one of those requests puts real mail in a real inbox. Cap the
    /// sends per ACCOUNT as well. The response stays 200 either way —
    /// throttling must not become an enumeration oracle.
    #[tokio::test]
    async fn one_account_cannot_be_mailed_without_limit() {
        let users = Arc::new(MemoryUserStore::new());
        users
            .create("victim@example.com", "argon2-placeholder", "Victim")
            .await
            .expect("seed user");
        let magic = Arc::new(MemoryMagicLinkStore::new());
        let mailer = Arc::new(CountingMailer::default());
        let mailer_dyn: Arc<dyn Mailer> = mailer.clone();

        let app = Router::new()
            .route(
                "/v1/auth/magic/start",
                post(start::<MemoryUserStore, MemoryMagicLinkStore>),
            )
            .layer(Extension(mailer_dyn))
            .layer(Extension(Arc::new(MagicSendLimiter::new())))
            .with_state((users, magic));

        let attempts = MAGIC_SENDS_PER_USER + 5;
        for _ in 0..attempts {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/auth/magic/start")
                        .header("content-type", "application/json")
                        .body(Body::from(r#"{"email":"victim@example.com"}"#))
                        .unwrap(),
                )
                .await
                .expect("router responds");
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "throttling must stay invisible to the caller"
            );
        }

        let sent = mailer.magic_sends.load(Ordering::SeqCst);
        assert!(
            sent as u32 <= MAGIC_SENDS_PER_USER,
            "{sent} magic links mailed to one address in {attempts} requests;              cap is {MAGIC_SENDS_PER_USER}"
        );
    }
}
