//! RSI public-profile snapshot endpoints.
//!
//! Once a user has proven ownership of their RSI handle (see
//! [`rsi_verify_routes`]), we can periodically snapshot the public
//! profile page (display name, enlistment date, location, badges,
//! short bio, primary org summary) and surface that snapshot through
//! StarStats so it can render alongside the player's stats.
//!
//! Three endpoints:
//!  - `POST /v1/auth/rsi/profile/refresh` — user-authenticated.
//!    Hits RSI, persists a fresh snapshot, returns it. Rate-limited to
//!    one refresh per hour per user; the limit is enforced inline by
//!    looking at the previous snapshot's `captured_at` because the
//!    upstream is a public HTML page that we politely shouldn't
//!    hammer.
//!  - `GET /v1/me/profile` — user-authenticated. Returns the latest
//!    stored snapshot for the caller, or 404 if none has been
//!    captured yet.
//!  - `GET /v1/public/u/{handle}/profile` — unauthenticated.
//!    Returns the latest snapshot for `handle` if (and only if) the
//!    owner has flipped public visibility on their `stats_record`.
//!    Permission resolution mirrors `sharing_routes::public_summary`:
//!    SpiceDB `view@public_view` on `stats_record:<handle>`. A failed
//!    visibility check returns 404 — never leak existence.
//!
//! All upstream-facing failure modes (RSI 404, RSI down, SpiceDB
//! down) map to the same envelope shape as the rest of the API:
//! `ApiErrorBody { error, detail }`. The `error` strings are the
//! ones the SDK + frontend already key off, so a future migration
//! away from utoipa-derived clients keeps working.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::profile_store::{
    PostgresProfileStore, ProfileSnapshot, ProfileStore, ProfileStoreError,
};
use crate::profile_view_stats::{ProfileViewSource, ProfileViewStats, ProfileViewStatsStore};
use crate::rsi_verify::{Badge, RsiClient, RsiProfileOutcome};
use crate::spicedb::SpicedbClient;
use crate::users::{PostgresUserStore, UserStore};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use utoipa::ToSchema;
use uuid::Uuid;

/// Minimum interval between refreshes for a single user. Set to one
/// hour: an RSI bio doesn't change minute-to-minute, and the upstream
/// is a public HTML page that we don't want to hammer. Clients that
/// hit the limit get 429 with a hint at how long to wait.
pub const PROFILE_REFRESH_COOLDOWN: chrono::Duration = chrono::Duration::hours(1);

/// Build the `/v1/auth/rsi/profile/*`, `/v1/me/profile`, and
/// `/v1/public/u/:handle/profile` sub-router.
///
/// Three internal sub-routers because the `State<_>` shape diverges:
///  - `refresh` needs both the user store (to look up the handle +
///    verification flag) and the profile store (to read the previous
///    snapshot for cooldown + write the fresh one).
///  - `me` needs only the profile store (with a `auth.sub`-derived
///    user id).
///  - `public_profile` needs both: user store to resolve the handle
///    to the live `claimed_handle` casing, profile store to fetch
///    the snapshot.
///
/// `Arc<dyn RsiClient>` and `Arc<Option<SpicedbClient>>` come in via
/// `Extension<_>` so they're shared with `rsi_verify_routes` /
/// `sharing_routes` rather than duplicated per sub-router.
pub fn routes(
    users: Arc<PostgresUserStore>,
    profiles: Arc<PostgresProfileStore>,
    view_stats: Arc<dyn ProfileViewStatsStore>,
) -> Router {
    let refresh_router = Router::new()
        .route(
            "/v1/auth/rsi/profile/refresh",
            post(refresh::<PostgresUserStore, PostgresProfileStore>),
        )
        .with_state((users.clone(), profiles.clone()));

    let me_router = Router::new()
        .route("/v1/me/profile", get(me::<PostgresProfileStore>))
        .with_state(profiles.clone());

    // Owner-only read of the per-day profile-view counters. Auth uses
    // the bearer token's `preferred_username` claim directly — we do
    // NOT accept a `?handle=` param so a token holder can't fish for
    // other users' counters. Same posture as the other `/v1/me/*`
    // endpoints in this module.
    let me_views_router = Router::new()
        .route("/v1/me/profile-views", get(profile_views_me))
        .with_state(view_stats.clone());

    // Per-IP throttle on the unauthenticated public endpoint. SpiceDB
    // and a DB lookup run per request, and a scanner with a list of
    // valid handles can otherwise enumerate snapshots at line rate.
    // Generous enough for normal browsing (≈1 page per 200 ms with
    // sustained 5/s + a 20-request burst).
    let public_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(5)
            .burst_size(20)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("public profile governor config builder produced no config"),
    );
    // State tuple carries the view-stats store alongside the existing
    // user / profile stores so the public-profile handler can fire a
    // background recorder for every successful read.
    let public_router = Router::new()
        .route(
            "/v1/public/u/:handle/profile",
            get(public_profile::<PostgresUserStore, PostgresProfileStore>),
        )
        .with_state((users, profiles, view_stats))
        .layer(GovernorLayer {
            config: public_governor,
        });

    refresh_router
        .merge(me_router)
        .merge(me_views_router)
        .merge(public_router)
}

/// Default `days` window when the caller doesn't supply one. Sized to
/// the same 30-day card the /sharing page renders.
pub const PROFILE_VIEWS_DAYS_DEFAULT: u16 = 30;
/// Hard upper bound — beyond 90 days the per-day SQL projection grows
/// fast and the card UI tops out at this many bars anyway.
pub const PROFILE_VIEWS_DAYS_MAX: u16 = 90;

#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
pub struct ProfileViewsQuery {
    /// Per-day window size. Clamped to `[1, 90]`; defaults to 30.
    #[serde(default)]
    pub days: Option<u16>,
}

#[utoipa::path(
    get,
    path = "/v1/me/profile-views",
    tag = "rsi-profile",
    operation_id = "rsi_profile_views_me",
    params(ProfileViewsQuery),
    responses(
        (status = 200, description = "Per-day + aggregate view counters for the caller", body = ProfileViewStats),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn profile_views_me(
    State(view_stats): State<Arc<dyn ProfileViewStatsStore>>,
    auth: AuthenticatedUser,
    Query(query): Query<ProfileViewsQuery>,
) -> Response {
    // Clamp the window: 0 -> 1 (silly but defensible), >90 -> 90. We
    // don't 400 because the card UI can pass an environment-driven
    // default and a misconfigured one shouldn't blow up.
    let days = query
        .days
        .unwrap_or(PROFILE_VIEWS_DAYS_DEFAULT)
        .clamp(1, PROFILE_VIEWS_DAYS_MAX);
    match view_stats.read_stats(&auth.preferred_username, days).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "profile views read failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

/// JSON shape for every successful response on these endpoints.
/// Mirrors [`ProfileSnapshot`] but trims off the internal `user_id`
/// (clients identify the profile by handle / by being authenticated)
/// and renames the field set to the wire spec the frontend expects.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProfileResponse {
    /// When this snapshot was captured. ISO-8601 UTC. Clients display
    /// this verbatim ("Last refreshed 5 minutes ago") so it's the
    /// stored timestamp, not "now" — a cached `me` response and a
    /// fresh `refresh` response with the same body must agree.
    pub captured_at: DateTime<Utc>,
    /// Display name as it appears on the RSI profile page (may differ
    /// from the user's `claimed_handle` URL slug).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// "Enlisted" date from the citizen card, parsed to a `NaiveDate`
    /// because RSI publishes it without a timezone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enlistment_date: Option<chrono::NaiveDate>,
    /// Free-form location string from the profile (often a country
    /// name or "Unknown"). Surfaced verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Badges shown on the citizen card. Order is the order RSI
    /// rendered them.
    pub badges: Vec<Badge>,
    /// Short bio paragraph from the profile page. Surfaced verbatim;
    /// callers are responsible for HTML-escaping if they render it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    /// Primary org summary string (`"<org name> [<rank>]"` style) if
    /// the user has a non-redacted main org. Absent when the user
    /// has no main org or has marked it redacted on RSI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_org_summary: Option<String>,
}

impl From<ProfileSnapshot> for ProfileResponse {
    fn from(s: ProfileSnapshot) -> Self {
        ProfileResponse {
            captured_at: s.captured_at,
            display_name: s.display_name,
            enlistment_date: s.enlistment_date,
            location: s.location,
            badges: s.badges,
            bio: s.bio,
            primary_org_summary: s.primary_org_summary,
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
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

// `require_user_token` (removed) used to 403 anything but user-session
// JWTs. The original comment said "the desktop client doesn't surface
// profile management" — but the tray DOES surface RSI profile pushes
// (see CHANGELOG v0.3.7-alpha: public-profile snapshot handle / citizen
// number / enlisted-since). Pairing only mints device JWTs, so the gate
// left the tray unable to deliver any of that data. Gate removed; both
// endpoints now accept any authenticated JWT for the caller's own user.
// Same posture and reasoning as the hangar_routes fix.

use crate::users::validate_handle;

#[utoipa::path(
    post,
    path = "/v1/auth/rsi/profile/refresh",
    tag = "rsi-profile",
    operation_id = "rsi_profile_refresh",
    responses(
        (status = 200, description = "Snapshot refreshed", body = ProfileResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "RSI returned 404 for the claimed handle", body = ApiErrorBody),
        (status = 422, description = "Caller has not yet verified their RSI handle", body = ApiErrorBody),
        (status = 429, description = "Cooldown not elapsed; try again later", body = ApiErrorBody),
        (status = 503, description = "RSI upstream unreachable", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn refresh<U: UserStore, P: ProfileStore>(
    State((users, profiles)): State<(Arc<U>, Arc<P>)>,
    Extension(rsi): Extension<Arc<dyn RsiClient>>,
    auth: AuthenticatedUser,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in rsi/profile/refresh");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    if user.rsi_verified_at.is_none() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rsi_handle_not_verified",
            Some(
                "verify your RSI handle (POST /v1/auth/rsi/start) before refreshing the profile"
                    .into(),
            ),
        );
    }

    // Cooldown check. We do this BEFORE hitting RSI: a hammered
    // upstream would happily 429 us, but checking locally is cheaper
    // and gives the client a clean machine-readable retry hint.
    let now = Utc::now();
    let prior = match profiles.latest_for_user(user_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "latest_for_user failed in rsi/profile/refresh");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    if let Some(prev) = prior.as_ref() {
        let next_allowed = prev.captured_at + PROFILE_REFRESH_COOLDOWN;
        if next_allowed > now {
            let remaining = next_allowed - now;
            // Clamp to the nearest minute (rounded up) so a 1-second
            // residue still surfaces as "wait 1m" rather than "wait
            // 0m" — clients display this verbatim.
            let mins = remaining.num_seconds().div_euclid(60).max(0) + 1;
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "refresh_too_soon",
                Some(format!("wait {mins}m")),
            );
        }
    }

    match rsi.fetch_profile(&user.claimed_handle).await {
        RsiProfileOutcome::Found(profile) => {
            let snapshot = ProfileSnapshot {
                user_id,
                captured_at: now,
                display_name: profile.display_name,
                enlistment_date: profile.enlistment_date,
                location: profile.location,
                badges: profile.badges,
                bio: profile.bio,
                primary_org_summary: profile.primary_org_summary,
            };
            // `ProfileSnapshot: Clone`, so render the response from a
            // clone before handing the original to the store. Keeps
            // the response and the persisted row identical without a
            // re-fetch round trip.
            let response = ProfileResponse::from(snapshot.clone());
            if let Err(e) = profiles.save(snapshot).await {
                tracing::error!(error = %e, "profile save failed");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
            }
            (StatusCode::OK, Json(response)).into_response()
        }
        RsiProfileOutcome::HandleNotFound => error(
            StatusCode::NOT_FOUND,
            "rsi_handle_not_found",
            Some("RSI returned 404 for that handle".into()),
        ),
        RsiProfileOutcome::UpstreamUnavailable => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "rsi_unavailable",
            Some("RSI is unreachable; please try again shortly".into()),
        ),
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/profile",
    tag = "rsi-profile",
    operation_id = "rsi_profile_me",
    responses(
        (status = 200, description = "Latest snapshot for the caller", body = ProfileResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No snapshot has been captured yet", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn me<P: ProfileStore>(
    State(profiles): State<Arc<P>>,
    auth: AuthenticatedUser,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None),
    };

    match profiles.latest_for_user(user_id).await {
        Ok(Some(snapshot)) => {
            (StatusCode::OK, Json(ProfileResponse::from(snapshot))).into_response()
        }
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            "no_profile_yet",
            Some("call POST /v1/auth/rsi/profile/refresh to capture a snapshot".into()),
        ),
        Err(e) => {
            tracing::error!(error = %e, "latest_for_user failed in /v1/me/profile");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

/// Query string for `/v1/public/u/{handle}/profile`. The optional
/// `source` lets the discover page / share-link UI tag its outbound
/// link so we don't have to infer everything from the Referer header.
/// Unknown values are ignored (we fall back to header sniffing) rather
/// than 400ing — the endpoint is unauthenticated public surface and a
/// stale link with a typoed source shouldn't break.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PublicProfileQuery {
    #[serde(default)]
    pub source: Option<String>,
}

#[utoipa::path(
    get,
    path = "/v1/public/u/{handle}/profile",
    tag = "rsi-profile",
    operation_id = "rsi_profile_public",
    params(("handle" = String, Path, description = "RSI handle to fetch the public profile snapshot for")),
    responses(
        (status = 200, description = "Latest public snapshot", body = ProfileResponse),
        (status = 404, description = "Handle unknown, not public, or no snapshot captured"),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
)]
pub async fn public_profile<U: UserStore, P: ProfileStore>(
    State((users, profiles, view_stats)): State<(Arc<U>, Arc<P>, Arc<dyn ProfileViewStatsStore>)>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Path(handle): Path<String>,
    Query(query): Query<PublicProfileQuery>,
    headers: HeaderMap,
) -> Response {
    // Same posture as `sharing_routes::public_summary`: a malformed
    // handle is indistinguishable from an unknown one, both 404.
    if !validate_handle(&handle) {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    // Resolve the handle to a user row first. `latest_for_handle` could
    // do this in one query, but we still need to consult SpiceDB on
    // the canonical-cased handle from the user row, and rejecting
    // unknown handles before that call avoids an unnecessary
    // permission lookup.
    let user = match users.find_by_handle(&handle).await {
        Ok(Some(u)) => u,
        Ok(None) => return (StatusCode::NOT_FOUND, ()).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "find_by_handle failed in public profile");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    let Some(client) = spicedb.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiErrorBody {
                error: "spicedb_unavailable".into(),
                detail: None,
            }),
        )
            .into_response();
    };

    // Probe the `public_view` relation directly via ReadRelationships
    // (SpiceDB rejects CheckPermission against `user:*` — see
    // sharing_routes::check_public). Existence of the tuple IS the
    // truth for "publicly visible"; no permission-graph traversal
    // needed. v1.5.4 / v1.7.1 incident pattern, restored 2026-05-24.
    let public = match client.has_public_view(&user.claimed_handle).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "spicedb public profile check failed");
            return error(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable", None);
        }
    };
    if !public {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    match profiles.latest_for_handle(&user.claimed_handle).await {
        Ok(Some(snapshot)) => {
            // Fire-and-forget the view counter. The recorder runs in a
            // detached `tokio::spawn` so a slow UPSERT (or a transient
            // DB hiccup) never holds up the response — owners care
            // about per-day fidelity, not per-request consistency.
            let referer = headers
                .get(axum::http::header::REFERER)
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string());
            let source = classify_source(query.source.as_deref(), referer.as_deref(), &handle);
            let store = view_stats.clone();
            let handle_for_record = user.claimed_handle.clone();
            tokio::spawn(async move {
                if let Err(e) = store.record_view(&handle_for_record, source).await {
                    tracing::warn!(error = %e, "failed to record profile view");
                }
            });
            (StatusCode::OK, Json(ProfileResponse::from(snapshot))).into_response()
        }
        // Same 404 as "not public" — don't disclose to anonymous
        // callers that a public user simply hasn't refreshed yet.
        Ok(None) => (StatusCode::NOT_FOUND, ()).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "latest_for_handle failed in public profile");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

/// Classify the traffic source from the optional `?source=` query
/// param + the `Referer` header. Pure function so it's testable
/// without spinning up a router.
///
/// Precedence:
///   1. `?source=discover` / `?source=shared` win unconditionally — the
///      link emitter knows where the user came from with more certainty
///      than a referer match.
///   2. A Referer URL whose path starts with `/discover` -> Discover.
///   3. A Referer URL of the form `/u/<other-handle>` (a different
///      handle than the one being viewed) -> Shared.
///   4. No referer or a referer that doesn't match any of the above
///      heuristics -> Direct.
///   5. Anything else (a same-handle re-entry, an unrecognised same-site
///      path) -> Other so we still get a counted bucket without
///      misattributing.
pub fn classify_source(
    query_source: Option<&str>,
    referer: Option<&str>,
    viewed_handle: &str,
) -> ProfileViewSource {
    if let Some(q) = query_source {
        match q {
            "discover" => return ProfileViewSource::Discover,
            "shared" => return ProfileViewSource::Shared,
            "direct" => return ProfileViewSource::Direct,
            _ => {}
        }
    }
    let Some(ref_url) = referer else {
        return ProfileViewSource::Direct;
    };
    // Strip the scheme + host so we can examine the path. We accept
    // either `http://host/path` or `https://host/path`; anything else
    // (relative referer, empty string) drops into "Direct".
    let path = if let Some(rest) = ref_url.strip_prefix("https://") {
        rest.split_once('/').map(|(_, p)| p).unwrap_or("")
    } else if let Some(rest) = ref_url.strip_prefix("http://") {
        rest.split_once('/').map(|(_, p)| p).unwrap_or("")
    } else {
        return ProfileViewSource::Direct;
    };
    let path = format!("/{}", path);
    if path.starts_with("/discover") {
        return ProfileViewSource::Discover;
    }
    // `/u/<handle>` shape — strip trailing slash / segments, compare
    // against the viewed handle.
    if let Some(rest) = path.strip_prefix("/u/") {
        let other_handle = rest.split('/').next().unwrap_or("");
        if !other_handle.is_empty() && !other_handle.eq_ignore_ascii_case(viewed_handle) {
            return ProfileViewSource::Shared;
        }
    }
    ProfileViewSource::Other
}

// `ProfileStoreError` is referenced via `tracing::error!` interpolation
// above; importing the type keeps the `%e` Display impl resolution
// honest and is also needed for the bin's openapi build (the `mod`
// declaration mirrors the live server's so unused imports are
// suppressed via `#![allow(unused_imports)]` at the bin root).
#[allow(dead_code)]
type _UnusedProfileStoreError = ProfileStoreError;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::profile_view_stats::test_support::MemoryProfileViewStatsStore;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Extension;
    use tower::ServiceExt;

    fn issue_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&uuid::Uuid::now_v7().to_string(), handle)
            .expect("sign user token")
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()
    }

    /// Build a minimal router exposing only `/v1/me/profile-views` with
    /// the supplied memory store. Bypasses the `routes()` builder so a
    /// test can hand-craft fixture data without resolving the
    /// PostgresUserStore / PostgresProfileStore dependencies.
    fn me_views_router(
        verifier: Arc<AuthVerifier>,
        store: Arc<dyn ProfileViewStatsStore>,
    ) -> Router {
        Router::new()
            .route("/v1/me/profile-views", get(profile_views_me))
            .with_state(store)
            .layer(Extension(verifier))
    }

    #[tokio::test]
    async fn profile_views_rejects_without_auth() {
        let (_issuer, verifier) = fresh_pair();
        let store: Arc<dyn ProfileViewStatsStore> = Arc::new(MemoryProfileViewStatsStore::new());
        let app = me_views_router(Arc::new(verifier), store);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/profile-views")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn profile_views_empty_for_user_with_no_views() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let store: Arc<dyn ProfileViewStatsStore> = Arc::new(MemoryProfileViewStatsStore::new());
        let app = me_views_router(Arc::new(verifier), store);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/profile-views")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(body["totals"]["all_time"], 0);
        assert_eq!(body["totals"]["last_7d"], 0);
        assert_eq!(body["totals"]["last_30d"], 0);
        assert!(body["days"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn profile_views_returns_per_source_breakdown() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let memory = Arc::new(MemoryProfileViewStatsStore::new());
        // Seed: 3 direct, 2 discover, 1 shared — all today.
        for _ in 0..3 {
            memory
                .record_view("alice", ProfileViewSource::Direct)
                .await
                .unwrap();
        }
        for _ in 0..2 {
            memory
                .record_view("alice", ProfileViewSource::Discover)
                .await
                .unwrap();
        }
        memory
            .record_view("alice", ProfileViewSource::Shared)
            .await
            .unwrap();
        let store: Arc<dyn ProfileViewStatsStore> = memory;
        let app = me_views_router(Arc::new(verifier), store);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/profile-views")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(body["totals"]["all_time"], 6);
        assert_eq!(body["totals"]["last_30d"], 6);
        assert_eq!(body["totals"]["by_source_30d"]["direct"], 3);
        assert_eq!(body["totals"]["by_source_30d"]["discover"], 2);
        assert_eq!(body["totals"]["by_source_30d"]["shared"], 1);
        assert_eq!(body["days"].as_array().unwrap().len(), 1);
        assert_eq!(body["days"][0]["total"], 6);
    }

    #[tokio::test]
    async fn profile_views_clamps_zero_days_to_one() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let memory = Arc::new(MemoryProfileViewStatsStore::new());
        memory
            .record_view("alice", ProfileViewSource::Direct)
            .await
            .unwrap();
        let store: Arc<dyn ProfileViewStatsStore> = memory;
        let app = me_views_router(Arc::new(verifier), store);
        // days=0 -> clamped to 1; should still include today.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/profile-views?days=0")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(body["days"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn profile_views_clamps_oversized_days_to_max() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let memory = Arc::new(MemoryProfileViewStatsStore::new());
        // Seed something so the request doesn't 200-empty by accident.
        memory
            .record_view("alice", ProfileViewSource::Direct)
            .await
            .unwrap();
        let store: Arc<dyn ProfileViewStatsStore> = memory;
        let app = me_views_router(Arc::new(verifier), store);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/profile-views?days=1000")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // The day list is at most PROFILE_VIEWS_DAYS_MAX entries (90).
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        let day_count = body["days"].as_array().unwrap().len();
        assert!(
            day_count <= PROFILE_VIEWS_DAYS_MAX as usize,
            "day count {day_count} exceeded clamp cap"
        );
    }

    #[tokio::test]
    async fn profile_views_separates_7d_30d_all_time_windows() {
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, "alice");
        let memory = Arc::new(MemoryProfileViewStatsStore::new());
        let today = chrono::Utc::now().date_naive();
        // 1 view today (in 7d), 1 view 10 days ago (in 30d, not 7d),
        // 1 view 45 days ago (in all_time only).
        memory.seed("alice", today, ProfileViewSource::Direct, 1);
        memory.seed(
            "alice",
            today - chrono::Duration::days(10),
            ProfileViewSource::Direct,
            1,
        );
        memory.seed(
            "alice",
            today - chrono::Duration::days(45),
            ProfileViewSource::Direct,
            1,
        );
        let store: Arc<dyn ProfileViewStatsStore> = memory;
        let app = me_views_router(Arc::new(verifier), store);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/me/profile-views")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(body["totals"]["all_time"], 3);
        assert_eq!(body["totals"]["last_7d"], 1);
        assert_eq!(body["totals"]["last_30d"], 2);
    }

    #[test]
    fn classify_source_query_param_takes_precedence_over_referer() {
        // ?source=discover wins even when the Referer points at a
        // /u/<handle> shape — the emitter knows better than the heuristic.
        let s = classify_source(
            Some("discover"),
            Some("https://starstats.app/u/bob"),
            "alice",
        );
        assert_eq!(s, ProfileViewSource::Discover);
    }

    #[test]
    fn classify_source_unknown_query_param_falls_back_to_referer() {
        // A typoed ?source= value mustn't break the heuristic.
        let s = classify_source(Some("xyz"), Some("https://starstats.app/discover"), "alice");
        assert_eq!(s, ProfileViewSource::Discover);
    }

    #[test]
    fn classify_source_no_referer_is_direct() {
        let s = classify_source(None, None, "alice");
        assert_eq!(s, ProfileViewSource::Direct);
    }

    #[test]
    fn classify_source_discover_referer_path() {
        let s = classify_source(None, Some("https://starstats.app/discover"), "alice");
        assert_eq!(s, ProfileViewSource::Discover);
        // Sub-paths under /discover (filtering, pagination) still count.
        let s2 = classify_source(None, Some("https://starstats.app/discover?page=2"), "alice");
        assert_eq!(s2, ProfileViewSource::Discover);
    }

    #[test]
    fn classify_source_other_handle_referer_is_shared() {
        // Coming in from `/u/bob` to view `alice` -> Shared (viral
        // navigation between public profiles).
        let s = classify_source(None, Some("https://starstats.app/u/bob"), "alice");
        assert_eq!(s, ProfileViewSource::Shared);
    }

    #[test]
    fn classify_source_same_handle_referer_is_other() {
        // Same-handle Referer is a re-entry (in-page tab nav etc.) —
        // explicitly NOT Shared.
        let s = classify_source(None, Some("https://starstats.app/u/alice"), "alice");
        assert_eq!(s, ProfileViewSource::Other);
        // Case-insensitive match: handle casing variance shouldn't
        // change the classification.
        let s2 = classify_source(None, Some("https://starstats.app/u/ALICE"), "alice");
        assert_eq!(s2, ProfileViewSource::Other);
    }

    #[test]
    fn classify_source_external_referer_is_direct() {
        // A referer with a path the heuristic doesn't recognise should
        // resolve to Direct (no shape match) for an external domain
        // and Other for an unrecognised same-site path. The function
        // can't easily distinguish without knowing our own host, so we
        // treat both via the `Other` branch when the path doesn't
        // match — except the no-path case which is Direct.
        let s = classify_source(None, Some("https://reddit.com/r/starcitizen"), "alice");
        assert_eq!(s, ProfileViewSource::Other);
    }

    #[test]
    fn classify_source_relative_referer_is_direct() {
        // A non-http(s) Referer (some browsers emit `/path` only)
        // can't be classified; treat as Direct.
        let s = classify_source(None, Some("/some/relative/path"), "alice");
        assert_eq!(s, ProfileViewSource::Direct);
    }
}
