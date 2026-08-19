//! Voting + subscribe routes for the roadmap pipeline (Phase 6).
//!
//! Four routes, all authenticated:
//! - `POST   /v1/roadmap/:slug/vote`       — cast (idempotent)
//! - `DELETE /v1/roadmap/:slug/vote`       — retract (idempotent)
//! - `POST   /v1/roadmap/:slug/subscribe`  — subscribe (idempotent)
//! - `DELETE /v1/roadmap/:slug/subscribe`  — unsubscribe (idempotent)
//!
//! Per spec §6.1 anonymous voting is not allowed -- the user identity
//! drives sybil resistance. The rate-limit (30 votes/min/user, §6.1)
//! is enforced inline via a per-user token bucket on a `Mutex<HashMap>`.
//! `tower_governor` would let us share the same governor layer used by
//! `preferences_routes`, but its key extractor is per-IP -- swapping in
//! a per-user-subject extractor requires the auth claim to be parsed
//! BEFORE the layer runs, which fights axum's extractor ordering. The
//! inline bucket is small enough to read end-to-end and stays in this
//! one file.
//!
//! Subscriber privacy (spec §7.2): nothing in this module returns the
//! set of subscribers. Subscriber-COUNT bubbles out via the writeback
//! worker -> GitHub Project field, never via a public list endpoint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::store::RoadmapStore;
use crate::auth::AuthenticatedUser;

// ---------- DTOs -----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VoteResponse {
    /// True after `POST /vote`, false after `DELETE /vote`. Mirrors
    /// the action just performed so the client doesn't need a second
    /// round-trip to confirm state.
    pub voted: bool,
    pub votes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubscribeResponse {
    /// True after `POST /subscribe`, false after `DELETE /subscribe`.
    pub subscribed: bool,
}

// ---------- rate limiter ---------------------------------------------------

/// Per-user token bucket. Bucket capacity = `VOTE_BURST`; refilled at
/// `VOTE_REFILL_PER_SEC` tokens/sec. Spec §6.1 calls for 30/min, so we
/// settle on 30 burst with steady refill of 0.5/sec (=30/min). This is
/// "well above any human" per the spec.
///
/// Applied to BOTH vote and subscribe routes -- subscribing isn't
/// spec'd separately, so we use the same bucket. A subscribe burst is
/// not realistically distinguishable from a vote burst in terms of
/// "is this a script".
const VOTE_BURST: f64 = 30.0;
const VOTE_REFILL_PER_SEC: f64 = 30.0 / 60.0;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new() -> Self {
        Self {
            tokens: VOTE_BURST,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token. Returns `true` if the request is
    /// allowed, `false` if rate-limited.
    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * VOTE_REFILL_PER_SEC).min(VOTE_BURST);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Process-wide per-user rate limiter. Lives behind an `Arc` and is
/// wired in as an axum `Extension` by the caller (or constructed
/// inline by `router()` if the caller doesn't care). A `Mutex` is fine
/// for the contention level expected here -- contention is per-user
/// not global, and the critical section is a few arithmetic ops.
#[derive(Default)]
pub struct VoteRateLimiter {
    buckets: Mutex<HashMap<Uuid, Bucket>>,
}

impl VoteRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    fn check(&self, user_id: Uuid) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets.entry(user_id).or_insert_with(Bucket::new);
        bucket.try_consume()
    }
}

// ---------- helpers --------------------------------------------------------

/// Parse the `sub` claim into a UUID. Mirrors the pattern in
/// `submission_routes::parse_user_id` and `totp_routes::parse_user_id`.
fn parse_user_id(auth: &AuthenticatedUser) -> Result<Uuid, Response> {
    Uuid::parse_str(&auth.sub)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid_subject").into_response())
}

/// Resolve `:slug` to a live roadmap item id. Returns the standard
/// 404 / 500 responses on miss / error.
async fn resolve_item(store: &dyn RoadmapStore, slug: &str) -> Result<Uuid, Response> {
    match store.get_item_by_slug(slug).await {
        Ok(Some(item)) => Ok(item.id),
        Ok(None) => Err((StatusCode::NOT_FOUND, "not found").into_response()),
        Err(e) => {
            tracing::warn!(error = %e, slug, "roadmap slug lookup failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "lookup failed").into_response())
        }
    }
}

fn rate_limited() -> Response {
    (StatusCode::TOO_MANY_REQUESTS, "rate_limited").into_response()
}

// ---------- handlers -------------------------------------------------------

/// POST /v1/roadmap/{slug}/vote
#[utoipa::path(
    post,
    path = "/v1/roadmap/{slug}/vote",
    tag = "roadmap",
    params(("slug" = String, Path, description = "Roadmap item slug")),
    responses(
        (status = 200, description = "Vote recorded (idempotent)", body = VoteResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Item not found"),
        (status = 429, description = "Rate limited"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn cast_vote(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Extension(limiter): Extension<Arc<VoteRateLimiter>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
) -> Response {
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if !limiter.check(user_id) {
        return rate_limited();
    }
    let item_id = match resolve_item(&*store, &slug).await {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(e) = store.cast_vote(user_id, item_id).await {
        tracing::warn!(error = %e, "cast_vote failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "cast failed").into_response();
    }
    let votes = match store.count_votes(item_id).await {
        Ok(t) => t.votes,
        Err(e) => {
            tracing::warn!(error = %e, "count_votes failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "count failed").into_response();
        }
    };
    (StatusCode::OK, Json(VoteResponse { voted: true, votes })).into_response()
}

/// DELETE /v1/roadmap/{slug}/vote
#[utoipa::path(
    delete,
    path = "/v1/roadmap/{slug}/vote",
    tag = "roadmap",
    params(("slug" = String, Path, description = "Roadmap item slug")),
    responses(
        (status = 200, description = "Vote retracted (idempotent)", body = VoteResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Item not found"),
        (status = 429, description = "Rate limited"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn retract_vote(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Extension(limiter): Extension<Arc<VoteRateLimiter>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
) -> Response {
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if !limiter.check(user_id) {
        return rate_limited();
    }
    let item_id = match resolve_item(&*store, &slug).await {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(e) = store.retract_vote(user_id, item_id).await {
        tracing::warn!(error = %e, "retract_vote failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "retract failed").into_response();
    }
    let votes = match store.count_votes(item_id).await {
        Ok(t) => t.votes,
        Err(e) => {
            tracing::warn!(error = %e, "count_votes failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "count failed").into_response();
        }
    };
    (
        StatusCode::OK,
        Json(VoteResponse {
            voted: false,
            votes,
        }),
    )
        .into_response()
}

/// POST /v1/roadmap/{slug}/subscribe
#[utoipa::path(
    post,
    path = "/v1/roadmap/{slug}/subscribe",
    tag = "roadmap",
    params(("slug" = String, Path, description = "Roadmap item slug")),
    responses(
        (status = 200, description = "Subscribed (idempotent)", body = SubscribeResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Item not found"),
        (status = 429, description = "Rate limited"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn subscribe(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Extension(limiter): Extension<Arc<VoteRateLimiter>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
) -> Response {
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if !limiter.check(user_id) {
        return rate_limited();
    }
    let item_id = match resolve_item(&*store, &slug).await {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(e) = store.subscribe(user_id, item_id).await {
        tracing::warn!(error = %e, "subscribe failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "subscribe failed").into_response();
    }
    (StatusCode::OK, Json(SubscribeResponse { subscribed: true })).into_response()
}

/// DELETE /v1/roadmap/{slug}/subscribe
#[utoipa::path(
    delete,
    path = "/v1/roadmap/{slug}/subscribe",
    tag = "roadmap",
    params(("slug" = String, Path, description = "Roadmap item slug")),
    responses(
        (status = 200, description = "Unsubscribed (idempotent)", body = SubscribeResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Item not found"),
        (status = 429, description = "Rate limited"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn unsubscribe(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Extension(limiter): Extension<Arc<VoteRateLimiter>>,
    auth: AuthenticatedUser,
    Path(slug): Path<String>,
) -> Response {
    let user_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };
    if !limiter.check(user_id) {
        return rate_limited();
    }
    let item_id = match resolve_item(&*store, &slug).await {
        Ok(id) => id,
        Err(r) => return r,
    };
    if let Err(e) = store.unsubscribe(user_id, item_id).await {
        tracing::warn!(error = %e, "unsubscribe failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "unsubscribe failed").into_response();
    }
    (
        StatusCode::OK,
        Json(SubscribeResponse { subscribed: false }),
    )
        .into_response()
}

// ---------- router ---------------------------------------------------------

/// Build the voting + subscribe sub-router. The caller layers
/// `Arc<dyn RoadmapStore>`, `Arc<AuthVerifier>`, and (if not already
/// present) `Arc<VoteRateLimiter>` as Extensions on the parent app.
///
/// Used by `main.rs` to mount under the same prefix as the public
/// roadmap router. Slug-suffix routes (`/vote`, `/subscribe`) MUST be
/// registered BEFORE the public router's `/:slug` catch -- axum's
/// router merges literal paths first, but it's clearer to use distinct
/// suffixes that can't collide.
pub fn router() -> Router {
    Router::new()
        .route(
            "/v1/roadmap/:slug/vote",
            post(cast_vote).delete(retract_vote),
        )
        .route(
            "/v1/roadmap/:slug/subscribe",
            post(subscribe).delete(unsubscribe),
        )
        .layer(Extension(
            Arc::new(VoteRateLimiter::new()) as Arc<VoteRateLimiter>
        ))
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::UpsertRoadmapItem;
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn seed_item(store: &MemoryRoadmapStore, slug: &str) -> Uuid {
        let surfaces: Vec<String> = vec![];
        let item = store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: &format!("PVTI_{slug}"),
                slug,
                title: slug,
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public: true,
            })
            .await
            .unwrap();
        item.id
    }

    fn build_app(store: Arc<dyn RoadmapStore>, verifier: Arc<AuthVerifier>) -> Router {
        router().layer(Extension(store)).layer(Extension(verifier))
    }

    fn issue_token(issuer: &TokenIssuer, user_id: Uuid, handle: &str) -> String {
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign user token")
    }

    #[tokio::test]
    async fn anonymous_vote_is_401() {
        let store: Arc<dyn RoadmapStore> = {
            let s = Arc::new(MemoryRoadmapStore::new());
            seed_item(&s, "feature-x").await;
            s
        };
        let (_issuer, verifier) = fresh_pair();
        let app = build_app(store, Arc::new(verifier));

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/feature-x/vote")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn vote_is_idempotent_per_user() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "feature-y").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "alice");
        let app = build_app(store.clone(), Arc::new(verifier));

        // First vote -> 1.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/feature-y/vote")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: VoteResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(body.voted);
        assert_eq!(body.votes, 1);

        // Second vote from the same user -> still 1.
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/feature-y/vote")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: VoteResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.votes, 1, "duplicate vote does not increment");

        // Confirm via the store directly.
        let tally = store.count_votes(item_id).await.unwrap();
        assert_eq!(tally.votes, 1);
    }

    #[tokio::test]
    async fn retract_then_count_is_zero() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "feature-z").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "bob");
        let app = build_app(store.clone(), Arc::new(verifier));

        // Cast then retract.
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/feature-z/vote")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/roadmap/feature-z/vote")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: VoteResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(!body.voted);
        assert_eq!(body.votes, 0);

        let tally = store.count_votes(item_id).await.unwrap();
        assert_eq!(tally.votes, 0);
    }

    #[tokio::test]
    async fn vote_on_missing_slug_is_404() {
        let store: Arc<dyn RoadmapStore> = Arc::new(MemoryRoadmapStore::new());
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "carol");
        let app = build_app(store, Arc::new(verifier));

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/does-not-exist/vote")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rate_limit_blocks_after_burst() {
        // VOTE_BURST = 30 -- the 31st request inside the same second
        // refills < 1 token and trips the 429. We don't sleep, so the
        // bucket doesn't refill back to 1 mid-test.
        let memory = Arc::new(MemoryRoadmapStore::new());
        seed_item(&memory, "rate-target").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "rate-tester");
        let app = build_app(store, Arc::new(verifier));

        for _ in 0..(VOTE_BURST as usize) {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/roadmap/rate-target/vote")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/rate-target/vote")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn subscribe_and_unsubscribe_do_not_leak_membership() {
        // The public API surface exposed by `router()` has no
        // list-subscribers endpoint. This test pins that contract: a
        // subscribed user can't enumerate other subscribers from
        // anything the router exposes. We exhaustively check that
        // GET on the subscribe path is not routed (405).
        let memory = Arc::new(MemoryRoadmapStore::new());
        seed_item(&memory, "sub-target").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "sub-tester");
        let app = build_app(store.clone(), Arc::new(verifier));

        // Subscribe.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/sub-target/subscribe")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: SubscribeResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(body.subscribed);

        // Idempotent re-subscribe.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/roadmap/sub-target/subscribe")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // No GET endpoint exists -- 405 from axum (route exists for
        // POST + DELETE only).
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/roadmap/sub-target/subscribe")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

        // Unsubscribe.
        let resp = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/roadmap/sub-target/subscribe")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: SubscribeResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(!body.subscribed);
    }
}
