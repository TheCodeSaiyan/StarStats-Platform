//! Public-profile listing endpoint (`GET /v1/discover/profiles`).
//!
//! Piece 3 of the public-profile UX work. The endpoint surfaces every
//! profile whose owner has flipped the SpiceDB `public_view` wildcard
//! AND hasn't opted out of the listing via Piece 4's `listing_opt_out`
//! column on `users`. No auth required — the same data is freely
//! readable at `/v1/public/{handle}/*`, so collecting it into a
//! browsable index doesn't change the trust posture.
//!
//! ## Query strategy
//!
//! Two-step compose:
//!  1. Ask SpiceDB for every `stats_record` resource ID where the
//!     wildcard `user:*` has the `view` permission. This is the
//!     ground truth for "public profile". Capped at
//!     [`MAX_PUBLIC_PROFILES_LOOKUP`] so a runaway listing can't open-
//!     loop SpiceDB's gRPC stream.
//!  2. Intersect the resulting handle set against the `users` table
//!     (and supporting joins) inside a single SQL round-trip, filter
//!     out `listing_opt_out = TRUE`, sort lexicographically (lower-
//!     cased handle), and apply the request's `after` cursor and
//!     `limit` cap.
//!
//! ## Why the cap matters
//!
//! `LookupResources` streams every match; for a deployment with a
//! large public-profile cohort the stream's memory and latency cost
//! grows linearly. [`MAX_PUBLIC_PROFILES_LOOKUP`] is the deliberate
//! ceiling on what one request will surface. When the cap fires:
//!  * the handler emits a `tracing::warn!` with the actual count so
//!    ops sees the saturation,
//!  * the response carries `next_after = None` rather than a stale
//!    cursor (the browse stops cleanly; we don't pretend the cap is
//!    a paging boundary).
//!
//! Tuning this cap is a one-line change here — `/discover` is the
//! only caller, and the value is documented inline.
//!
//! ## Store split
//!
//! [`DiscoverStore`] owns the SQL side; the SpiceDB call lives in the
//! route handler because the SpiceDB client is an outbound dependency
//! plumbed through axum's Extensions, not a "store" in the project's
//! trait + Postgres + Memory sense. The store trait carries one
//! method ([`DiscoverStore::list_public_profiles_filtered`]) and has
//! a Postgres + Memory pair so route-layer tests stay self-contained.

use crate::api_error::ApiErrorBody;
use crate::sharing_routes::PublicSupporterInfo;
use crate::spicedb::SpicedbClient;
use crate::supporters::{SupporterState, SupporterStore};
use async_trait::async_trait;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Server-side hard cap on a single `LookupResources` call against
/// SpiceDB. Public profiles are unlikely to exceed this in any
/// realistic StarStats deployment, but the cap stops a runaway
/// schema or a future-feature mistake (e.g. accidentally writing
/// `public_view` rows in bulk) from open-looping the stream.
///
/// When this fires, `/v1/discover/profiles` returns whatever it
/// already collected with `next_after = None` and emits a `warn!`
/// log so the saturation is visible to ops.
pub const MAX_PUBLIC_PROFILES_LOOKUP: u32 = 1000;

/// Default page size for the listing. Picked to fill the typical
/// /discover grid (4-column desktop * ~12 rows) plus a little
/// breathing room, so the first request usually fits without an
/// immediate "Load more" fetch.
pub const DEFAULT_LIMIT: u32 = 50;

/// Hard upper bound on per-request limit. Clients asking for more
/// (e.g. an admin dashboard) get capped here; the cursor still lets
/// them walk the full set across multiple requests.
pub const MAX_LIMIT: u32 = 200;

// -- Wire DTOs -------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiscoverProfile {
    /// Canonical handle as stored on `users.claimed_handle`. Sort key
    /// is the lowercased form; the original casing is preserved on
    /// the wire so the UI doesn't have to remember it.
    pub handle: String,
    /// Most-recent display name from the latest RSI profile snapshot.
    /// `None` when no snapshot has been captured (verified user but
    /// never refreshed) or the field was empty on the upstream page.
    pub display_name: Option<String>,
    /// `users.created_at` — when the user signed up. ISO 8601 UTC.
    pub joined_at: Option<String>,
    /// Most-recent `event_timestamp` from the events table. `None`
    /// when the user has never ingested an event with a populated
    /// timestamp (every event row carries one or the field is left
    /// NULL for lines that parsed structurally but lacked a stamp).
    pub last_active_at: Option<String>,
    /// Supporter chip data — same public-safe projection used by the
    /// summary endpoint (see `sharing_routes::PublicSupporterInfo`).
    /// `None` for non-supporters; present (with tier + plate) for
    /// `active`/`lapsed` rows. Bulk-fetched alongside the profile
    /// list so the discover page renders chips without N+1 queries.
    pub supporter: Option<crate::sharing_routes::PublicSupporterInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiscoverProfilesResponse {
    pub profiles: Vec<DiscoverProfile>,
    /// Cursor for the next page; pass back as `after`. `None` when
    /// the response exhausted the public-profile set OR when the
    /// SpiceDB-lookup cap fired (saturation, not pagination — see
    /// [`MAX_PUBLIC_PROFILES_LOOKUP`]).
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct DiscoverQuery {
    /// Cursor — return handles whose lowercased form is strictly
    /// greater than this value. Pass the `next_after` returned by
    /// the previous call. Case-insensitive comparison; the client
    /// can echo whatever casing the server returned.
    pub after: Option<String>,
    /// Page size cap. Clamped to `[1, MAX_LIMIT]`; defaults to
    /// [`DEFAULT_LIMIT`] when absent.
    pub limit: Option<u32>,
}

// -- DiscoverStore trait + Postgres impl ----------------------------

#[derive(Debug, thiserror::Error)]
pub enum DiscoverStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub type Result<T> = std::result::Result<T, DiscoverStoreError>;

#[async_trait]
pub trait DiscoverStore: Send + Sync + 'static {
    /// Filter `candidate_handles` against the persisted user set,
    /// drop opted-out rows, sort by `lower(handle)`, apply the
    /// `after` cursor (case-insensitive `>` against
    /// `lower(handle)`), and return up to `limit` rows. The caller
    /// (route handler) is expected to have clamped `limit` already.
    ///
    /// The "candidate" framing is deliberate: the SpiceDB
    /// `LookupResources` call already pre-filtered to public profiles,
    /// so this method's job is purely the SQL-side filter + shape.
    /// Returning an empty `Vec` for an empty input is the documented
    /// behaviour and lets the handler skip a round-trip when SpiceDB
    /// returns nothing.
    async fn list_public_profiles_filtered(
        &self,
        candidate_handles: &[String],
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DiscoverProfile>>;
}

pub struct PostgresDiscoverStore {
    pool: PgPool,
}

impl PostgresDiscoverStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DiscoverStore for PostgresDiscoverStore {
    async fn list_public_profiles_filtered(
        &self,
        candidate_handles: &[String],
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<DiscoverProfile>> {
        if candidate_handles.is_empty() {
            return Ok(Vec::new());
        }

        // Bind the candidate set as a single TEXT[] parameter so
        // PostgreSQL can use `= ANY($1)` against `lower(claimed_handle)`.
        // The candidate set is already lowercased on the SpiceDB side
        // (handles are stored case-insensitively elsewhere), but we
        // lowercase here anyway to defend against any caller passing
        // mixed casing.
        let normalized: Vec<String> = candidate_handles
            .iter()
            .map(|h| h.to_ascii_lowercase())
            .collect();
        let after_lower = after.map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let limit_i64 = limit as i64;

        // LEFT JOIN against the latest profile snapshot (per user) and
        // against the per-handle last-event timestamp. Both feeds are
        // optional — a freshly-signed-up public profile with no
        // snapshots or events still shows up, just with NULL fields.
        // The lateral subqueries keep the index access bounded to one
        // row per (user_id, claimed_handle) regardless of history
        // depth.
        let rows: Vec<(String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)> =
            sqlx::query_as(
                r#"
            SELECT u.claimed_handle,
                   snap.display_name,
                   u.created_at,
                   evt.last_active_at
              FROM users u
              LEFT JOIN LATERAL (
                  SELECT display_name
                    FROM rsi_profile_snapshots
                   WHERE user_id = u.id
                   ORDER BY captured_at DESC
                   LIMIT 1
              ) snap ON TRUE
              LEFT JOIN LATERAL (
                  SELECT MAX(event_timestamp) AS last_active_at
                    FROM events
                   WHERE claimed_handle = lower(u.claimed_handle)
                     AND event_timestamp IS NOT NULL
              ) evt ON TRUE
             WHERE lower(u.claimed_handle) = ANY($1)
               AND u.listing_opt_out = FALSE
               AND lower(u.claimed_handle) > $2
             ORDER BY lower(u.claimed_handle) ASC
             LIMIT $3
            "#,
            )
            .bind(&normalized)
            .bind(&after_lower)
            .bind(limit_i64)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(handle, display_name, created_at, last_active_at)| DiscoverProfile {
                    handle,
                    display_name,
                    joined_at: Some(created_at.to_rfc3339()),
                    last_active_at: last_active_at.map(|t| t.to_rfc3339()),
                    // Supporter info is bulk-fetched by the handler
                    // AFTER the listing query returns — keeps this
                    // store method focused on the user/profile join
                    // and avoids growing this SQL with a third
                    // optional source.
                    supporter: None,
                },
            )
            .collect())
    }
}

// -- Route + handler ------------------------------------------------

/// Build the `/v1/discover/profiles` sub-router. Public — no auth
/// required, no Extension dependencies beyond what
/// [`crate::main`] already layers on the outer router (the SpiceDB
/// client).
pub fn routes(store: Arc<PostgresDiscoverStore>) -> Router {
    let store_dyn: Arc<dyn DiscoverStore> = store;
    Router::new()
        .route("/v1/discover/profiles", get(list_discover_profiles))
        .with_state(store_dyn)
}

#[utoipa::path(
    get,
    path = "/v1/discover/profiles",
    tag = "discover",
    params(DiscoverQuery),
    responses(
        (status = 200, description = "Public-profile listing slice", body = DiscoverProfilesResponse),
        (status = 503, description = "SpiceDB not configured", body = ApiErrorBody),
    )
)]
pub async fn list_discover_profiles(
    State(store): State<Arc<dyn DiscoverStore>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(supporters): Extension<Arc<dyn SupporterStore>>,
    Extension(restrictions): Extension<
        Arc<dyn crate::account_restrictions::AccountRestrictionStore>,
    >,
    Query(q): Query<DiscoverQuery>,
) -> Response {
    let Some(client) = spicedb.as_ref() else {
        // No SpiceDB = no way to enumerate public profiles. Same
        // posture as the other SpiceDB-dependent endpoints: 503 with
        // `spicedb_unavailable` so the UI banner is consistent.
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiErrorBody {
                error: "spicedb_unavailable".into(),
                detail: None,
            }),
        )
            .into_response();
    };

    // Clamp limit to the documented bounds. 0 collapses to 1 so we
    // never serve an empty page just because the client passed
    // `?limit=0` by accident.
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let candidates = match client
        .list_public_profile_handles(MAX_PUBLIC_PROFILES_LOOKUP)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "spicedb ReadRelationships failed (discover)");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorBody {
                    error: "spicedb_error".into(),
                    detail: None,
                }),
            )
                .into_response();
        }
    };

    // Drop public-profile-restricted users BEFORE paging, so the cursor
    // and the has-more calculation both operate on the set the caller
    // will actually see.
    //
    // Fails CLOSED: if the restriction lookup errors we 503 rather than
    // serve an unfiltered list. Degrading to "show everything" would
    // leak exactly the profiles a moderator has just hidden, and would
    // do it silently.
    let candidates = {
        let lowered: Vec<String> = candidates.iter().map(|h| h.to_ascii_lowercase()).collect();
        match restrictions.restricted_public_handles(&lowered).await {
            Ok(blocked) if blocked.is_empty() => candidates,
            Ok(blocked) => candidates
                .into_iter()
                .filter(|h| !blocked.contains(&h.to_ascii_lowercase()))
                .collect(),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "restriction lookup failed (discover); refusing to serve unfiltered"
                );
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(ApiErrorBody {
                        error: "restriction_check_unavailable".into(),
                        detail: None,
                    }),
                )
                    .into_response();
            }
        }
    };

    let saturated = candidates.len() as u32 >= MAX_PUBLIC_PROFILES_LOOKUP;
    if saturated {
        tracing::warn!(
            candidates = candidates.len(),
            cap = MAX_PUBLIC_PROFILES_LOOKUP,
            "spicedb LookupResources hit the discover cap; listing truncated"
        );
    }

    // Fetch limit+1 so we can detect "more rows available" without a
    // separate count query. The (limit+1)-th row, if present, is
    // dropped before the response goes out.
    let fetch_limit = limit.saturating_add(1);
    let mut rows = match store
        .list_public_profiles_filtered(&candidates, q.after.as_deref(), fetch_limit)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "discover store query failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorBody {
                    error: "database_error".into(),
                    detail: None,
                }),
            )
                .into_response();
        }
    };

    let next_after = if rows.len() as u32 > limit {
        rows.truncate(limit as usize);
        // The cursor is the lowercased handle of the LAST returned
        // row. Saturation overrides this: if we couldn't see the
        // full set on the SpiceDB side, advancing the cursor would
        // silently skip handles that didn't make the lookup cap, so
        // we report "no more" instead of a misleading next-page link.
        if saturated {
            None
        } else {
            rows.last().map(|p| p.handle.to_ascii_lowercase())
        }
    } else {
        None
    };

    // Bulk-fetch supporter chip data for everyone on this page in a
    // single SQL round-trip. Failing soft means a transient supporter-
    // store error degrades to "no chips on the list" rather than
    // 5xx'ing the whole discover page (chips are decorative; the
    // listing is the load-bearing payload). Logged as a warn so ops
    // sees the degradation.
    if !rows.is_empty() {
        let handles: Vec<String> = rows.iter().map(|p| p.handle.clone()).collect();
        match supporters.get_many_public_by_handle(&handles).await {
            Ok(supporter_map) => {
                for profile in rows.iter_mut() {
                    let key = profile.handle.to_ascii_lowercase();
                    if let Some(status) = supporter_map.get(&key) {
                        // Same projection logic as
                        // sharing_routes::supporter_to_public; we
                        // can't reuse that helper directly because
                        // it's a private fn in sharing_routes (and
                        // adding a pub re-export for one call site
                        // is more coupling than the duplication).
                        let info = match status.state {
                            SupporterState::Active | SupporterState::Lapsed => {
                                Some(PublicSupporterInfo {
                                    state: status.state.as_str().to_string(),
                                    current_tier_key: status.current_tier_key.clone(),
                                    name_plate: status.name_plate.clone(),
                                })
                            }
                            SupporterState::None => None,
                        };
                        profile.supporter = info;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    count = handles.len(),
                    "discover supporter bulk fetch failed; chips will be absent"
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(DiscoverProfilesResponse {
            profiles: rows,
            next_after,
        }),
    )
        .into_response()
}

// -- Test support + tests -------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory implementation used by route-layer tests. Mirrors the
    /// Postgres semantics: filter, opt-out drop, lowercased sort,
    /// case-insensitive cursor compare, limit clamp.
    #[derive(Default)]
    pub struct MemoryDiscoverStore {
        rows: Mutex<HashMap<String, DiscoverProfile>>,
        opt_out: Mutex<HashMap<String, bool>>,
    }

    impl MemoryDiscoverStore {
        pub fn new() -> Self {
            Self::default()
        }

        /// Stand in for `users.claimed_handle`. Keyed by lowercased
        /// handle internally; the stored `DiscoverProfile` preserves
        /// the original casing on the `handle` field.
        pub fn register(&self, profile: DiscoverProfile) {
            let key = profile.handle.to_ascii_lowercase();
            self.rows.lock().unwrap().insert(key, profile);
        }

        /// Toggle the per-user `listing_opt_out` value. Absence
        /// reads as FALSE, matching the Postgres column default.
        pub fn set_opt_out(&self, handle: &str, value: bool) {
            let key = handle.to_ascii_lowercase();
            self.opt_out.lock().unwrap().insert(key, value);
        }
    }

    #[async_trait]
    impl DiscoverStore for MemoryDiscoverStore {
        async fn list_public_profiles_filtered(
            &self,
            candidate_handles: &[String],
            after: Option<&str>,
            limit: u32,
        ) -> Result<Vec<DiscoverProfile>> {
            if candidate_handles.is_empty() {
                return Ok(Vec::new());
            }
            let normalized: Vec<String> = candidate_handles
                .iter()
                .map(|h| h.to_ascii_lowercase())
                .collect();
            let after_lower = after.map(|s| s.to_ascii_lowercase());
            let rows = self.rows.lock().unwrap();
            let opt_out = self.opt_out.lock().unwrap();

            let mut filtered: Vec<DiscoverProfile> = normalized
                .iter()
                .filter_map(|key| rows.get(key).cloned())
                .filter(|p| {
                    !opt_out
                        .get(&p.handle.to_ascii_lowercase())
                        .copied()
                        .unwrap_or(false)
                })
                .filter(|p| match after_lower.as_ref() {
                    None => true,
                    Some(a) => p.handle.to_ascii_lowercase().as_str() > a.as_str(),
                })
                .collect();
            filtered.sort_by(|a, b| {
                a.handle
                    .to_ascii_lowercase()
                    .cmp(&b.handle.to_ascii_lowercase())
            });
            filtered.truncate(limit as usize);
            Ok(filtered)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryDiscoverStore;
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    fn make_profile(handle: &str) -> DiscoverProfile {
        DiscoverProfile {
            handle: handle.to_owned(),
            display_name: Some(format!("{handle} Display")),
            joined_at: Some("2026-01-01T00:00:00+00:00".to_owned()),
            last_active_at: None,
            // No supporter data in these tests — the listing-store
            // tests target the SQL-side filter/sort, not the chip
            // enrichment which is a separate handler step.
            supporter: None,
        }
    }

    fn router(store: Arc<MemoryDiscoverStore>) -> Router {
        // The route handler reads SpiceDB through an Extension to
        // resolve the candidate set; tests intercept that step by
        // wrapping the store call in a fake handler. We rebuild a
        // tiny router here that swaps the SpiceDB step for a
        // test-supplied candidate set so the SQL-side behaviour can
        // be asserted in isolation. The production handler is exercised
        // through the discover.spec.ts Playwright path that goes
        // through the live server build.
        let store_dyn: Arc<dyn DiscoverStore> = store;
        Router::new()
            .route(
                "/v1/discover/profiles",
                get(test_list_profiles_with_candidates),
            )
            .with_state(store_dyn)
    }

    /// Test-only handler — replicates the production handler's
    /// pagination + clamping logic but takes the candidate set from
    /// a query param instead of SpiceDB. Lets the unit tests assert
    /// the SQL-side filter / cursor / clamp behaviour without
    /// standing up a SpiceDB sidecar.
    async fn test_list_profiles_with_candidates(
        State(store): State<Arc<dyn DiscoverStore>>,
        Query(params): Query<TestQuery>,
    ) -> Response {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let candidates: Vec<String> = params
            .candidates
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_owned())
            .collect();
        let fetch_limit = limit.saturating_add(1);
        let mut rows = store
            .list_public_profiles_filtered(&candidates, params.after.as_deref(), fetch_limit)
            .await
            .expect("memory store cannot fail");
        let next_after = if rows.len() as u32 > limit {
            rows.truncate(limit as usize);
            rows.last().map(|p| p.handle.to_ascii_lowercase())
        } else {
            None
        };
        Json(DiscoverProfilesResponse {
            profiles: rows,
            next_after,
        })
        .into_response()
    }

    #[derive(Deserialize)]
    struct TestQuery {
        candidates: String,
        after: Option<String>,
        limit: Option<u32>,
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn empty_candidate_set_yields_empty_listing() {
        let store = Arc::new(MemoryDiscoverStore::new());
        let app = router(store);
        let req = Request::builder()
            .uri("/v1/discover/profiles?candidates=")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["profiles"], serde_json::json!([]));
        assert!(v["next_after"].is_null());
    }

    #[tokio::test]
    async fn filters_out_listing_opt_out_profiles() {
        // alice and bob both have public_view, but alice has opted
        // out of the listing. Only bob should land in the response.
        let store = Arc::new(MemoryDiscoverStore::new());
        store.register(make_profile("Alice"));
        store.register(make_profile("Bob"));
        store.set_opt_out("Alice", true);
        let app = router(store);
        let req = Request::builder()
            .uri("/v1/discover/profiles?candidates=alice,bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let v = body_json(resp).await;
        let handles: Vec<String> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(handles, vec!["Bob"]);
        assert!(v["next_after"].is_null());
    }

    #[tokio::test]
    async fn pagination_cursor_walks_two_pages() {
        // Three handles, limit=2 → first page has [a, b] with
        // next_after = "b"; second page passes after=b and returns
        // [c] with next_after = null.
        let store = Arc::new(MemoryDiscoverStore::new());
        for h in ["aaa", "bbb", "ccc"] {
            store.register(make_profile(h));
        }
        let app = router(store);

        let req1 = Request::builder()
            .uri("/v1/discover/profiles?candidates=aaa,bbb,ccc&limit=2")
            .body(Body::empty())
            .unwrap();
        let v1 = body_json(app.clone().oneshot(req1).await.unwrap()).await;
        let p1: Vec<String> = v1["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(p1, vec!["aaa", "bbb"]);
        assert_eq!(v1["next_after"].as_str(), Some("bbb"));

        let req2 = Request::builder()
            .uri("/v1/discover/profiles?candidates=aaa,bbb,ccc&limit=2&after=bbb")
            .body(Body::empty())
            .unwrap();
        let v2 = body_json(app.oneshot(req2).await.unwrap()).await;
        let p2: Vec<String> = v2["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(p2, vec!["ccc"]);
        assert!(v2["next_after"].is_null());
    }

    #[tokio::test]
    async fn limit_is_clamped_to_max() {
        // limit=1000 must clamp to MAX_LIMIT. Seed MAX_LIMIT+5
        // handles so we can confirm the response carries at most
        // MAX_LIMIT rows.
        let store = Arc::new(MemoryDiscoverStore::new());
        let total = MAX_LIMIT as usize + 5;
        for i in 0..total {
            store.register(make_profile(&format!("user{i:04}")));
        }
        let candidates: Vec<String> = (0..total).map(|i| format!("user{i:04}")).collect();
        let url = format!(
            "/v1/discover/profiles?candidates={}&limit=1000",
            candidates.join(",")
        );
        let app = router(store);
        let req = Request::builder().uri(&url).body(Body::empty()).unwrap();
        let v = body_json(app.oneshot(req).await.unwrap()).await;
        let len = v["profiles"].as_array().unwrap().len();
        assert_eq!(len, MAX_LIMIT as usize);
        // next_after is present because the (limit+1)-th row existed.
        assert!(v["next_after"].is_string());
    }

    #[tokio::test]
    async fn limit_zero_is_clamped_to_one() {
        // limit=0 must clamp to 1 so the page never collapses to
        // empty just because the client passed an off-by-one bound.
        let store = Arc::new(MemoryDiscoverStore::new());
        store.register(make_profile("only"));
        let app = router(store);
        let req = Request::builder()
            .uri("/v1/discover/profiles?candidates=only&limit=0")
            .body(Body::empty())
            .unwrap();
        let v = body_json(app.oneshot(req).await.unwrap()).await;
        let len = v["profiles"].as_array().unwrap().len();
        assert_eq!(len, 1);
    }

    #[tokio::test]
    async fn sort_is_case_insensitive() {
        // Mixed-case input; the response must come back sorted by
        // lowercased handle: alice, Bob, charlie.
        let store = Arc::new(MemoryDiscoverStore::new());
        store.register(make_profile("Bob"));
        store.register(make_profile("alice"));
        store.register(make_profile("charlie"));
        let app = router(store);
        let req = Request::builder()
            .uri("/v1/discover/profiles?candidates=Bob,alice,charlie")
            .body(Body::empty())
            .unwrap();
        let v = body_json(app.oneshot(req).await.unwrap()).await;
        let handles: Vec<String> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(handles, vec!["alice", "Bob", "charlie"]);
    }
}
