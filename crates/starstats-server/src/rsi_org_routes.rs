//! RSI organisation-membership snapshot endpoints.
//!
//! Once a user has proven ownership of their RSI handle (see
//! [`rsi_verify_routes`]), we can periodically scrape the public
//! `/citizens/{handle}/organizations` page and persist the resulting
//! list of orgs the user belongs to. StarStats then surfaces those
//! affiliations alongside the player's stats / profile snapshot.
//!
//! Three endpoints, intentionally mirroring [`rsi_profile_routes`]:
//!  - `POST /v1/auth/rsi/orgs/refresh` — user-authenticated.
//!    Hits RSI, persists a fresh snapshot, returns it. Rate-limited
//!    to one refresh per hour per user via inline cooldown check.
//!  - `GET /v1/me/rsi-orgs` — user-authenticated. Returns the latest
//!    stored snapshot for the caller, or 404 if none captured yet.
//!  - `GET /v1/public/u/{handle}/orgs` — unauthenticated.
//!    Returns the latest snapshot for `handle` if (and only if) the
//!    owner has flipped public visibility on their `stats_record`.
//!    Visibility resolution mirrors `rsi_profile_routes::public_profile`:
//!    SpiceDB `view@public_view` on `stats_record:<handle>`. A failed
//!    visibility check returns 404 — never leak handle existence.
//!
//! All upstream-facing failure modes (RSI 404, RSI down, SpiceDB
//! down) map to the same `ApiErrorBody { error, detail }` envelope as
//! the rest of the API.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::rsi_org_store::{PostgresRsiOrgStore, RsiOrgStore};
use crate::rsi_verify::{RsiClient, RsiOrg, RsiOrgsOutcome};
use crate::spicedb::SpicedbClient;
use crate::users::{PostgresUserStore, UserStore};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use chrono::Utc;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use uuid::Uuid;

/// Minimum interval between org-membership refreshes for a single
/// user. One hour, mirroring `rsi_profile_routes::PROFILE_REFRESH_COOLDOWN`:
/// org membership changes far less often than once an hour, and the
/// upstream is a public HTML page we don't want to hammer. Clients
/// that hit the limit get a 429 with a `wait Nm` retry hint.
pub const RSI_ORGS_REFRESH_COOLDOWN: chrono::Duration = chrono::Duration::hours(1);

/// Per-string cap applied to every field of an `RsiOrg`. Mirrors the
/// `MAX_FIELD_CHARS` posture in `hangar_routes`: the upstream is
/// public HTML, so megabyte fields are a realistic bloat vector.
const MAX_FIELD_CHARS: usize = 200;

/// Build the `/v1/auth/rsi/orgs/refresh`, `/v1/me/rsi-orgs`, and
/// `/v1/public/u/:handle/orgs` sub-router.
///
/// Three internal sub-routers because the `State<_>` shape diverges,
/// matching `rsi_profile_routes::routes`:
///  - `refresh` needs the user store (handle + verification flag) and
///    the org store (cooldown read + persist).
///  - `me` needs only the org store (uses `auth.sub` directly).
///  - `public_orgs` needs both: user store to resolve handle to user
///    id (and to get canonical-cased handle for SpiceDB), org store
///    to fetch the snapshot.
///
/// `Arc<dyn RsiClient>` and `Arc<Option<SpicedbClient>>` come in via
/// `Extension<_>` so they're shared with `rsi_verify_routes` /
/// `sharing_routes` rather than duplicated per sub-router.
pub fn routes(users: Arc<PostgresUserStore>, orgs: Arc<PostgresRsiOrgStore>) -> Router {
    let refresh_router = Router::new()
        .route(
            "/v1/auth/rsi/orgs/refresh",
            post(refresh::<PostgresUserStore, PostgresRsiOrgStore>),
        )
        .with_state((users.clone(), orgs.clone()));

    let me_router = Router::new()
        .route("/v1/me/rsi-orgs", get(me::<PostgresRsiOrgStore>))
        .with_state(orgs.clone());

    // Per-IP throttle on the unauthenticated public endpoint, same
    // configuration as `rsi_profile_routes::public_profile`: SpiceDB +
    // DB lookup per request, a scanner could otherwise enumerate
    // memberships at line rate. 5/s sustained + 20-request burst is
    // generous for normal browsing (~1 page per 200ms).
    let public_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(5)
            .burst_size(20)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("public rsi-orgs governor config builder produced no config"),
    );
    let public_router = Router::new()
        .route(
            "/v1/public/u/:handle/orgs",
            get(public_orgs::<PostgresUserStore, PostgresRsiOrgStore>),
        )
        .with_state((users, orgs))
        .layer(GovernorLayer {
            config: public_governor,
        });

    refresh_router.merge(me_router).merge(public_router)
}

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

/// Sanity-check the upstream parser output before persisting:
///
/// 1. Cap every string field at `MAX_FIELD_CHARS` (drops oversize entries
///    rather than rejecting the whole snapshot — public HTML is untrusted
///    and one bad row shouldn't blank the user's hangar list).
/// 2. Enforce at most one `is_main = true`: RSI guarantees a single main
///    org, but a markup change or hostile MitM could surface several.
///    Demote everything past the first to affiliation and log a warn so
///    ops can spot the upstream change.
fn sanitise_orgs(orgs: Vec<RsiOrg>) -> Vec<RsiOrg> {
    let mut seen_main = false;
    orgs.into_iter()
        .filter(|o| {
            o.sid.chars().count() <= MAX_FIELD_CHARS
                && o.name.chars().count() <= MAX_FIELD_CHARS
                && o.rank
                    .as_deref()
                    .map_or(true, |s| s.chars().count() <= MAX_FIELD_CHARS)
        })
        .map(|mut o| {
            if o.is_main {
                if seen_main {
                    tracing::warn!(
                        sid = %o.sid,
                        "rsi orgs: multiple is_main entries; demoting"
                    );
                    o.is_main = false;
                } else {
                    seen_main = true;
                }
            }
            o
        })
        .collect()
}

// `require_user_token` (removed) used to 403 anything but user-session
// JWTs. Pairing only mints device JWTs, so the gate left the tray
// unable to push or read org membership snapshots (org affiliation
// shipped alongside the v0.3.7-alpha public-profile work). Gate removed;
// both endpoints now accept any authenticated JWT for the caller's own
// user. Same posture and reasoning as the hangar_routes /
// rsi_profile_routes fixes.

use crate::users::validate_handle;

#[utoipa::path(
    post,
    path = "/v1/auth/rsi/orgs/refresh",
    tag = "rsi-orgs",
    operation_id = "rsi_orgs_refresh",
    responses(
        (status = 200, description = "Snapshot refreshed", body = crate::rsi_org_store::RsiOrgsSnapshot),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "RSI returned 404 for the claimed handle", body = ApiErrorBody),
        (status = 422, description = "Caller has not yet verified their RSI handle", body = ApiErrorBody),
        (status = 429, description = "Cooldown not elapsed; try again later", body = ApiErrorBody),
        (status = 503, description = "RSI upstream unreachable", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn refresh<U: UserStore, S: RsiOrgStore>(
    State((users, store)): State<(Arc<U>, Arc<S>)>,
    Extension(rsi): Extension<Arc<dyn RsiClient>>,
    auth: AuthenticatedUser,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => {
            tracing::error!(sub = %auth.sub, "auth.sub not a valid uuid in rsi/orgs/refresh");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None);
        }
    };

    let user = match users.find_by_id(user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized", None),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed in rsi/orgs/refresh");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    if user.rsi_verified_at.is_none() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rsi_handle_not_verified",
            Some("verify your RSI handle (POST /v1/auth/rsi/start) before refreshing orgs".into()),
        );
    }

    // Cooldown check before hitting RSI: a hammered upstream would
    // happily 429 us, but checking locally is cheaper and gives the
    // client a clean machine-readable retry hint.
    let now = Utc::now();
    let prior = match store.latest_for_user(user_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "latest_for_user failed in rsi/orgs/refresh");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    if let Some(prev) = prior.as_ref() {
        let next_allowed = prev.captured_at + RSI_ORGS_REFRESH_COOLDOWN;
        if next_allowed > now {
            let remaining = next_allowed - now;
            // Clamp to nearest minute (rounded up) — a 1-second
            // residue still surfaces as "wait 1m" rather than "0m".
            let mins = remaining.num_seconds().div_euclid(60).max(0) + 1;
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "refresh_too_soon",
                Some(format!("wait {mins}m")),
            );
        }
    }

    match rsi.fetch_orgs(&user.claimed_handle).await {
        RsiOrgsOutcome::Found(orgs) => {
            let orgs = sanitise_orgs(orgs);
            match store.save(user_id, &orgs).await {
                Ok(snapshot) => (StatusCode::OK, Json(snapshot)).into_response(),
                Err(e) => {
                    tracing::error!(error = %e, "rsi orgs save failed");
                    error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
                }
            }
        }
        RsiOrgsOutcome::HandleNotFound => error(
            StatusCode::NOT_FOUND,
            "rsi_handle_not_found",
            Some("RSI returned 404 for that handle".into()),
        ),
        RsiOrgsOutcome::UpstreamUnavailable => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "rsi_unavailable",
            Some("RSI is unreachable; please try again shortly".into()),
        ),
    }
}

#[utoipa::path(
    get,
    path = "/v1/me/rsi-orgs",
    tag = "rsi-orgs",
    operation_id = "rsi_orgs_me",
    responses(
        (status = 200, description = "Latest snapshot for the caller", body = crate::rsi_org_store::RsiOrgsSnapshot),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No snapshot has been captured yet", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn me<S: RsiOrgStore>(State(store): State<Arc<S>>, auth: AuthenticatedUser) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => {
            tracing::error!(sub = %auth.sub, "auth.sub not a valid uuid in /v1/me/rsi-orgs");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "bad_subject", None);
        }
    };

    match store.latest_for_user(user_id).await {
        Ok(Some(snapshot)) => (StatusCode::OK, Json(snapshot)).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            "no_orgs_yet",
            Some("call POST /v1/auth/rsi/orgs/refresh to capture a snapshot".into()),
        ),
        Err(e) => {
            tracing::error!(error = %e, "latest_for_user failed in /v1/me/rsi-orgs");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/public/u/{handle}/orgs",
    tag = "rsi-orgs",
    operation_id = "rsi_orgs_public",
    params(("handle" = String, Path, description = "RSI handle to fetch the public org-membership snapshot for")),
    responses(
        (status = 200, description = "Latest public snapshot", body = crate::rsi_org_store::RsiOrgsSnapshot),
        (status = 404, description = "Handle unknown, not public, or no snapshot captured"),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    ),
)]
pub async fn public_orgs<U: UserStore, S: RsiOrgStore>(
    State((users, store)): State<(Arc<U>, Arc<S>)>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Path(handle): Path<String>,
) -> Response {
    // Same posture as `sharing_routes::public_summary` /
    // `rsi_profile_routes::public_profile`: malformed handle is
    // indistinguishable from unknown — both 404.
    if !validate_handle(&handle) {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    // Resolve the handle to a user row. We need the canonical-cased
    // `claimed_handle` for SpiceDB and the user id for the snapshot
    // lookup. Rejecting unknown handles before SpiceDB avoids an
    // unnecessary permission RPC.
    let user = match users.find_by_handle(&handle).await {
        Ok(Some(u)) => u,
        Ok(None) => return (StatusCode::NOT_FOUND, ()).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "find_by_handle failed in public rsi-orgs");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };

    let Some(client) = spicedb.as_ref() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable", None);
    };

    // Probe the `public_view` relation directly via ReadRelationships
    // (SpiceDB rejects CheckPermission against `user:*` — see
    // sharing_routes::check_public). v1.5.4 / v1.7.1 incident pattern,
    // restored 2026-05-24.
    let public = match client.has_public_view(&user.claimed_handle).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "spicedb public rsi-orgs check failed");
            return error(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable", None);
        }
    };
    if !public {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }

    match store.latest_for_user(user.id).await {
        Ok(Some(snapshot)) => (StatusCode::OK, Json(snapshot)).into_response(),
        // Same 404 as "not public" — don't disclose to anonymous
        // callers that a public user simply hasn't refreshed yet.
        Ok(None) => (StatusCode::NOT_FOUND, ()).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "latest_for_user failed in public rsi-orgs");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}
