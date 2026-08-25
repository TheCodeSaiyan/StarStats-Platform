//! Public read-only endpoints over the cached vehicle/item reference
//! data sourced from `api.star-citizen.wiki`.
//!
//! Two endpoints, both unauthenticated:
//!  - `GET /v1/reference/vehicles` — full list. Clients (the dashboard
//!    in particular) preload this once on first render and key
//!    everything else off `class_name`.
//!  - `GET /v1/reference/vehicles/:class_name` — single lookup.
//!    `class_name` matching is case-insensitive: the in-game prop
//!    names ("ANVL_Hornet_F7C", "anvl_hornet_f7c") differ in casing
//!    across data sources, and forcing clients to remember the
//!    canonical form would be a footgun.
//!
//! Rate-limited per-IP (10/s with a burst of 40) — generous enough
//! that the dashboard's "preload" hit doesn't trip the limiter on
//! cold-load even with the user clicking around fast, but tight
//! enough that a scraper can't pull the full list multiple times
//! per second without slowing down. The data is freshness-tolerant
//! (refreshed once per 24h server-side), so a 429 here is a
//! non-event for normal callers.

use crate::api_error::ApiErrorBody;
use crate::reference_data::{
    build_summary, ReferenceCategory, ReferenceEntry, Summary, VehicleReference,
};
use crate::reference_stats::ReferenceStatsCache;
use crate::reference_store::{PostgresReferenceStore, ReferenceStore, ReferenceStoreError};
use axum::{
    body::Bytes,
    extract::{Extension, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower_governor::{
    governor::GovernorConfigBuilder,
    key_extractor::{KeyExtractor, SmartIpKeyExtractor},
    GovernorError, GovernorLayer,
};
use utoipa::ToSchema;

/// OpenAPI schema mirror of `starstats_core::cohort::Cohort` (core has no
/// utoipa dep). Keep field-for-field in sync.
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CohortSchema {
    pub key: String,
    pub kind: String,
    pub label: String,
}

/// Header carrying the SSR shared secret, and the header carrying the end
/// user's address that it vouches for.
const SSR_TOKEN_HEADER: &str = "x-starstats-ssr";
const SSR_FOR_HEADER: &str = "x-starstats-ssr-for";

/// Environment variable holding the SSR shared secret. Absent → no request
/// can ever be treated as SSR, which is the safe default.
const SSR_TOKEN_ENV: &str = "STARSTATS_SSR_TOKEN";

/// Rate-limit key that understands server-side rendering.
///
/// THE PROBLEM THIS SOLVES. The web frontend is server-rendered, so every
/// reference read for EVERY end user arrives at this limiter from the web
/// container's single IP. One bucket fronts the entire site. A crawler
/// walking KB slugs therefore does not throttle itself — it drains the bucket
/// that every real reader's page render shares, and those renders 429. The
/// limit has already been raised once for this (10/s+40 → 30/s+150) and
/// raising it again only moves the cliff.
///
/// THE FIX IS NOT AN EXEMPTION. Letting the SSR caller past the limiter would
/// also let the crawler past it, because the crawler reaches this API THROUGH
/// the web tier — the abuse would arrive pre-approved. Instead the web tier
/// says who it is rendering for, and that end user gets their own bucket. A
/// crawler is then throttled on its own address and a reader is unaffected by
/// it.
///
/// TRUST. The forwarded address is only believed when the request also
/// carries the shared secret, compared in constant time. Without the secret —
/// or without `STARSTATS_SSR_TOKEN` configured at all — the header is ignored
/// completely and the caller is keyed by its own IP exactly as before. So a
/// third party cannot mint themselves a private bucket, or evade the limit by
/// rotating a forged header, and the mechanism is off entirely until an
/// operator deliberately turns it on.
#[derive(Clone)]
pub struct SsrAwareIpKeyExtractor {
    /// `None` disables SSR keying entirely.
    token: Option<Arc<str>>,
}

impl SsrAwareIpKeyExtractor {
    /// Read the shared secret from the environment. An empty value is treated
    /// as unset so a blank env var cannot accidentally match a blank header.
    pub fn from_env() -> Self {
        let token = std::env::var(SSR_TOKEN_ENV)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .map(|t| Arc::from(t.as_str()));
        if token.is_none() {
            tracing::info!(
                "{SSR_TOKEN_ENV} not set — reference rate limiting keys every                  request by its own IP, so all server-side renders share one bucket"
            );
        }
        Self { token }
    }

    /// Constant-time equality. A short-circuiting compare would leak the
    /// secret's prefix to anyone able to time responses.
    fn token_matches(expected: &str, presented: &str) -> bool {
        if expected.len() != presented.len() {
            return false;
        }
        expected
            .bytes()
            .zip(presented.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl KeyExtractor for SsrAwareIpKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(expected) = self.token.as_deref() {
            let presented = req
                .headers()
                .get(SSR_TOKEN_HEADER)
                .and_then(|v| v.to_str().ok());
            if presented.is_some_and(|p| Self::token_matches(expected, p)) {
                if let Some(on_behalf_of) = req
                    .headers()
                    .get(SSR_FOR_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    // Namespaced so a forwarded value can never collide with a
                    // direct caller's own IP key.
                    return Ok(format!("ssr:{on_behalf_of}"));
                }
                // Authenticated SSR with no end user named (a cron, a warmup):
                // one bucket of its own, still bounded.
                return Ok("ssr:anonymous".to_string());
            }
        }
        SmartIpKeyExtractor
            .extract(req)
            .map(|ip| format!("ip:{ip}"))
    }
}

/// In-memory cache of the slim listing responses (`GET
/// /v1/reference/{category}`), keyed by category.
///
/// Without it, every listing request re-reads EVERY row's full
/// `metadata` JSONB from Postgres just to build the slim `summary`
/// projection and then throws the metadata away — multi-MB reads per
/// request for `item` (12k rows) and `vehicle` (rich per-row metadata).
/// Reference data only changes on the daily reconcile, so the slim
/// response is cached as pre-serialized JSON bytes and served directly.
///
/// Freshness: a lazy TTL rebuilds on the request path when stale (so
/// admin edits surface within the TTL), and the reconcile cron calls
/// [`ReferenceListCache::rebuild`] after each successful category
/// refresh to keep the cache warm so users rarely hit a rebuild.
pub struct ReferenceListCache {
    ttl: Duration,
    inner: RwLock<HashMap<&'static str, CachedListing>>,
}

struct CachedListing {
    built_at: Instant,
    body: Bytes,
}

impl ReferenceListCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Serve the cached JSON body for `cat`, rebuilding from `store`
    /// when missing or older than the TTL.
    async fn serve<R: ReferenceStore>(
        &self,
        cat: ReferenceCategory,
        store: &R,
    ) -> Result<Bytes, ReferenceStoreError> {
        {
            let guard = self.inner.read().await;
            if let Some(c) = guard.get(cat.as_str()) {
                if c.built_at.elapsed() < self.ttl {
                    return Ok(c.body.clone());
                }
            }
        }
        self.rebuild(cat, store).await
    }

    /// Rebuild + store one category's cached body. Also called by the
    /// reconcile cron to prime the cache after a successful refresh.
    pub async fn rebuild<R: ReferenceStore>(
        &self,
        cat: ReferenceCategory,
        store: &R,
    ) -> Result<Bytes, ReferenceStoreError> {
        let entries = store.list_category(cat).await?;
        let slim: Vec<ReferenceListEntry> = entries.into_iter().map(entry_to_list_entry).collect();
        let json = serde_json::to_vec(&ReferenceListResponse { entries: slim })
            .map_err(|e| ReferenceStoreError::Backend(e.to_string()))?;
        let body = Bytes::from(json);
        let mut guard = self.inner.write().await;
        guard.insert(
            cat.as_str(),
            CachedListing {
                built_at: Instant::now(),
                body: body.clone(),
            },
        );
        Ok(body)
    }
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}/stats",
    tag = "reference",
    operation_id = "reference_category_stats",
    params(("category" = String, Path, description = "One of: vehicle, weapon, item, location")),
    responses(
        (status = 200, description = "Per-peer-group quantile stats for the category", body = crate::reference_stats::ReferenceStatsResponseSchema),
        (status = 404, description = "Unknown category", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn get_category_stats<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Extension(cache): Extension<Arc<ReferenceStatsCache>>,
    Path(category): Path<String>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    match cache.serve(cat, store.as_ref()).await {
        Ok(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, category = %category, "get_category_stats failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

/// Max slugs accepted in one compare request (anchor + 10 + slack).
const COMPARE_MAX_SLUGS: usize = 12;

#[derive(serde::Deserialize)]
pub struct CompareQuery {
    pub slugs: Option<String>,
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}/compare",
    tag = "reference",
    operation_id = "reference_compare",
    params(
        ("category" = String, Path, description = "One of: vehicle, weapon, item, location"),
        ("slugs" = String, Query, description = "Comma-separated entry slugs (max 12)"),
    ),
    responses(
        (status = 200, description = "Numeric vectors for the requested slugs (unknown slugs omitted)", body = crate::reference_vectors::CompareResponse),
        (status = 400, description = "Missing/empty slugs, too many, or malformed slug", body = ApiErrorBody),
        (status = 404, description = "Unknown category", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn get_compare<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Extension(cache): Extension<Arc<crate::reference_vectors::ReferenceVectorsCache>>,
    Path(category): Path<String>,
    axum::extract::Query(q): axum::extract::Query<CompareQuery>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    let raw = q.slugs.unwrap_or_default();
    let slugs: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if slugs.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "missing_slugs",
            Some("slugs query param is required".into()),
        );
    }
    if slugs.len() > COMPARE_MAX_SLUGS {
        return error(
            StatusCode::BAD_REQUEST,
            "too_many_slugs",
            Some(format!("at most {COMPARE_MAX_SLUGS} slugs per request")),
        );
    }
    if slugs
        .iter()
        .any(|s| !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    {
        return error(
            StatusCode::BAD_REQUEST,
            "bad_slug",
            Some("slugs must be ASCII alphanumeric + '-'".into()),
        );
    }
    let map = match cache.serve(cat, store.as_ref()).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, category = %category, "compare cache failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let entries: Vec<crate::reference_vectors::CompareEntry> = slugs
        .iter()
        .filter_map(|s| map.get(s).map(|r| r.entry.clone()))
        .collect();
    (
        StatusCode::OK,
        Json(crate::reference_vectors::CompareResponse { entries }),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct CohortQuery {
    pub key: Option<String>,
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}/cohort",
    tag = "reference",
    operation_id = "reference_cohort",
    params(
        ("category" = String, Path, description = "One of: vehicle, weapon, item, location"),
        ("key" = String, Query, description = "Cohort key, e.g. type:interceptor"),
    ),
    responses(
        (status = 200, description = "Member vectors for the cohort (bounded; empty for unknown key)", body = crate::reference_vectors::CompareResponse),
        (status = 400, description = "Missing/malformed key", body = ApiErrorBody),
        (status = 404, description = "Unknown category", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn get_cohort<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Extension(cache): Extension<Arc<crate::reference_vectors::ReferenceVectorsCache>>,
    Path(category): Path<String>,
    axum::extract::Query(q): axum::extract::Query<CohortQuery>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    let key = q.key.unwrap_or_default();
    let valid_kind = ["family:", "type:", "make:", "range:"]
        .iter()
        .any(|p| key.starts_with(p));
    if key.is_empty() || !valid_kind || key.len() > 80 || key.contains(|c: char| c.is_control()) {
        return error(
            StatusCode::BAD_REQUEST,
            "bad_cohort_key",
            Some("key must be <kind>:<value> with kind family|type|make|range".into()),
        );
    }
    let map = match cache.serve(cat, store.as_ref()).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, category = %category, "cohort cache failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let entries = crate::reference_vectors::members_for_cohort(&map, &key);
    (
        StatusCode::OK,
        Json(crate::reference_vectors::CompareResponse { entries }),
    )
        .into_response()
}

/// Build the `/v1/reference/*` sub-router. Public — no `BearerAuth`
/// layer is attached, but the per-IP rate limit is.
pub fn routes(
    store: Arc<PostgresReferenceStore>,
    list_cache: Arc<ReferenceListCache>,
    stats_cache: Arc<ReferenceStatsCache>,
    vectors_cache: Arc<crate::reference_vectors::ReferenceVectorsCache>,
) -> Router {
    // Per-IP rate limit. NOTE: the web frontend is server-rendered, so
    // every SSR reference read for EVERY end user arrives from the web
    // container's single IP — this one bucket fronts all of them. The
    // limit must therefore accommodate legitimate SSR bursts (a KB list
    // → detail navigation, dashboard/journey entity lookups) and not
    // just a single human's request rate. The original 10/s + burst 40
    // was sized for direct public clients and throttled the web's own
    // SSR (429 → KB detail pages crashed). 30/s + burst 150 gives real
    // headroom while still bounding scraping abuse.
    let public_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(30)
            .burst_size(150)
            .key_extractor(SsrAwareIpKeyExtractor::from_env())
            .finish()
            .expect("reference governor config builder produced no config"),
    );
    // Axum's matchit router prefers static segments over wildcards
    // when both could match the same path, so the legacy
    // `/v1/reference/vehicles` route wins over the generic
    // `/v1/reference/:category` route for the literal "vehicles"
    // segment. Registration order is informational only.
    //
    // The slug lookup route is 3-deep (`:category / slug / :slug`),
    // so it never conflicts with the 2-deep by-class route
    // (`:category / :class_name`). A request like
    // `/v1/reference/vehicle/slug` (no trailing slug) falls through
    // to by-class with class_name="slug" → 404, which is fine.
    Router::new()
        .route(
            "/v1/reference/vehicles",
            get(list_vehicles::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/vehicles/:class_name",
            get(get_vehicle::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category",
            get(list_entries::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category/compare",
            get(get_compare::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category/cohort",
            get(get_cohort::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category/slug/:slug",
            get(get_entry_by_slug::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category/stats",
            get(get_category_stats::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category/by-class/:class_name",
            get(get_entry_by_class_name::<PostgresReferenceStore>),
        )
        .route(
            "/v1/reference/:category/:class_name",
            get(get_entry::<PostgresReferenceStore>),
        )
        .with_state(store)
        .layer(Extension(list_cache))
        .layer(Extension(stats_cache))
        .layer(Extension(vectors_cache))
        .layer(GovernorLayer {
            config: public_governor,
        })
}

/// Wrapper for `GET /v1/reference/vehicles`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VehicleListResponse {
    pub vehicles: Vec<VehicleReference>,
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

#[utoipa::path(
    get,
    path = "/v1/reference/vehicles",
    tag = "reference",
    operation_id = "reference_list_vehicles",
    responses(
        (status = 200, description = "Full list of cached vehicles", body = VehicleListResponse),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn list_vehicles<R: ReferenceStore>(State(store): State<Arc<R>>) -> Response {
    match store.list_vehicles().await {
        Ok(vehicles) => (StatusCode::OK, Json(VehicleListResponse { vehicles })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list_vehicles failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/reference/vehicles/{class_name}",
    tag = "reference",
    operation_id = "reference_get_vehicle",
    params(("class_name" = String, Path, description = "Vehicle class_name (case-insensitive)")),
    responses(
        (status = 200, description = "Vehicle entry", body = VehicleReference),
        (status = 404, description = "No vehicle with that class_name", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn get_vehicle<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Path(class_name): Path<String>,
) -> Response {
    // The store contract (see `ReferenceStore::get_vehicle`) guarantees
    // case-insensitive lookup, matching the `lower(class_name)` index.
    // The route is a thin pass-through — no fallback scan needed.
    match store.get_vehicle(&class_name).await {
        Ok(Some(v)) => (StatusCode::OK, Json(v)).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "vehicle_not_found", None),
        Err(e) => {
            tracing::error!(error = %e, "get_vehicle failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

/// Slim per-entry shape served by the listing endpoint. Drops the
/// full `metadata` blob (which can run multi-MB per page on
/// vehicles) in favour of a curated `summary` projection.
/// Consumers that need the full metadata must hit the detail
/// endpoint (`/v1/reference/{category}/{class_name}` or
/// `/v1/reference/{category}/slug/{slug}`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReferenceListEntry {
    pub class_name: String,
    pub display_name: String,
    /// URL-safe canonical identifier. Null on rows persisted before
    /// the KB-v1 backfill; consumers should fall back to a
    /// `class_name` URL in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Per-category curated fields. Internally tagged enum — the
    /// JSON carries a `category` discriminator so TypeScript
    /// clients can narrow on it. See `build_summary` in
    /// `reference_data.rs` for the field set per category.
    pub summary: Summary,
}

/// Response wrapper for `GET /v1/reference/{category}`. Slim shape
/// — the consumer fetches detail on demand for the full metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReferenceListResponse {
    pub entries: Vec<ReferenceListEntry>,
}

fn entry_to_list_entry(e: ReferenceEntry) -> ReferenceListEntry {
    let summary = build_summary(e.category, &e.metadata);
    ReferenceListEntry {
        class_name: e.class_name,
        display_name: e.display_name,
        slug: e.slug,
        summary,
    }
}

/// Detail-endpoint response shape. Carries everything `ReferenceEntry`
/// has PLUS the curated `summary` projection — the detail page on
/// the web uses both (`summary` for the at-a-glance chip strip,
/// `metadata` for the full key/value table). Without `summary` on
/// the response, `Object.entries(entry.summary)` on the web's
/// `/kb/[category]/[slug]/page.tsx` would crash.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReferenceEntryDetail {
    pub category: ReferenceCategory,
    pub class_name: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Peer-group bucket this entry belongs to (e.g. vehicle role
    /// family). The web detail page reads the matching bucket from the
    /// `/stats` endpoint to render peer-relative context. Additive +
    /// defaulted so older clients ignore it.
    #[serde(default)]
    pub peer_group: String,
    /// The anchor's cohort memberships (family/type/make/range) — drives
    /// the web "Compared to" selector + cohort bulk-add.
    #[serde(default)]
    #[schema(value_type = Vec<CohortSchema>)]
    pub cohorts: Vec<starstats_core::cohort::Cohort>,
    /// Curated per-category fields as a typed discriminated union —
    /// see `build_summary` / [`Summary`] in `reference_data.rs`.
    /// `category` here on the top-level entry and `summary.category`
    /// are redundant but harmless; clients can read whichever is
    /// more convenient.
    pub summary: Summary,
    /// Full per-category extras as the wiki returned them. Kept
    /// here on the detail endpoint so the web detail page can
    /// surface the full key/value table; the listing endpoint
    /// strips this to keep the payload small.
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
}

fn entry_to_detail(e: ReferenceEntry) -> ReferenceEntryDetail {
    let summary = build_summary(e.category, &e.metadata);
    let peer_group = starstats_core::peer_group::peer_group(e.category.as_str(), &e.metadata);
    let cohorts = starstats_core::cohort::cohort_memberships(e.category.as_str(), &e.metadata);
    ReferenceEntryDetail {
        category: e.category,
        class_name: e.class_name,
        display_name: e.display_name,
        slug: e.slug,
        peer_group,
        cohorts,
        summary,
        metadata: e.metadata,
    }
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}",
    tag = "reference",
    operation_id = "reference_list_category",
    params(("category" = String, Path, description = "One of: vehicle, weapon, item, location")),
    responses(
        (status = 200, description = "Full list of cached entries for the category", body = ReferenceListResponse),
        (status = 404, description = "Unknown category", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn list_entries<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Extension(cache): Extension<Arc<ReferenceListCache>>,
    Path(category): Path<String>,
) -> Response {
    // Categories outside the allow-list 404 — the alternative is
    // letting the DB hit return an empty list, which would mask
    // typos like `/v1/reference/vehciles`.
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    // Served from the in-memory cache (pre-serialized JSON), which
    // avoids re-reading every row's full metadata per request.
    match cache.serve(cat, store.as_ref()).await {
        Ok(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, category = %category, "list_entries failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}/{class_name}",
    tag = "reference",
    operation_id = "reference_get_entry",
    params(
        ("category" = String, Path, description = "One of: vehicle, weapon, item, location"),
        ("class_name" = String, Path, description = "Entry class_name (case-insensitive)"),
    ),
    responses(
        (status = 200, description = "Reference entry", body = ReferenceEntry),
        (status = 404, description = "No entry with that (category, class_name)", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn get_entry<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Path((category, class_name)): Path<(String, String)>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    match store.get_entry(cat, &class_name).await {
        Ok(Some(entry)) => (StatusCode::OK, Json(entry)).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "entry_not_found", None),
        Err(e) => {
            tracing::error!(error = %e, category = %category, "get_entry failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}/slug/{slug}",
    tag = "reference",
    operation_id = "reference_get_entry_by_slug",
    params(
        ("category" = String, Path, description = "One of: vehicle, weapon, item, location"),
        ("slug" = String, Path, description = "URL-safe slug (case-insensitive)"),
    ),
    responses(
        (status = 200, description = "Reference entry with summary + full metadata", body = ReferenceEntryDetail),
        (status = 301, description = "Mixed-case slug → redirects to the canonical lowercase URL via `Location`"),
        (status = 404, description = "Unknown category, or no entry with that slug", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn get_entry_by_slug<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Path((category, slug)): Path<(String, String)>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    // Slug input contract: ASCII alphanumeric + hyphens (case
    // insensitive — store comparison lowercases both sides). Reject
    // anything else up-front rather than letting it travel through
    // to the DB — keeps audit logs clean and prevents wasted
    // catalog lookups on adversarial inputs.
    if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return error(StatusCode::NOT_FOUND, "entry_not_found", None);
    }
    // Canonical slug form is lowercase. If the caller hit us with a
    // mixed-case URL, redirect to the canonical lowercase form (301)
    // so bookmarks + crawlers converge on one URL. Done before the
    // store lookup — no DB round-trip wasted on a non-canonical URL.
    if slug.chars().any(|c| c.is_ascii_uppercase()) {
        let canonical = slug.to_ascii_lowercase();
        let location = format!("/v1/reference/{category}/slug/{canonical}");
        return (
            StatusCode::MOVED_PERMANENTLY,
            [(axum::http::header::LOCATION, location)],
        )
            .into_response();
    }
    match store.get_by_slug(cat, &slug).await {
        Ok(Some(entry)) => (
            StatusCode::OK,
            // CACHEABLE AT THE EDGE, which is the cheap half of fixing the
            // 429s. The rate limiter runs as a layer BEFORE this handler, so
            // an in-process cache would cut database load and not one 429 —
            // but a response a CDN can serve never reaches the limiter at all.
            // A crawler re-walking the catalogue then costs us nothing after
            // its first pass.
            //
            // Reference data changes on the daily reconcile, so five minutes
            // is conservative; `stale-while-revalidate` keeps the edge serving
            // through a refresh rather than stampeding back to us.
            [(
                header::CACHE_CONTROL,
                "public, max-age=300, stale-while-revalidate=3600",
            )],
            Json(entry_to_detail(entry)),
        )
            .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "entry_not_found", None),
        Err(e) => {
            tracing::error!(error = %e, category = %category, slug = %slug, "get_entry_by_slug failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/reference/{category}/by-class/{class_name}",
    tag = "reference",
    operation_id = "reference_get_entry_by_class",
    params(
        ("category" = String, Path, description = "One of: vehicle, weapon, item, location"),
        ("class_name" = String, Path, description = "Engine class identifier (case-insensitive)"),
    ),
    responses(
        (status = 308, description = "Resolved → permanent redirect to canonical `/slug/{slug}` URL via `Location`"),
        (status = 200, description = "Legacy row with null slug → returns the entry directly with summary + metadata", body = ReferenceEntryDetail),
        (status = 404, description = "Unknown category, or no entry with that class_name", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
/// Bookmark-survival endpoint. Resolves a `class_name` to the
/// entry's canonical `/slug/{slug}` URL and 308-redirects. Two
/// reasons this matters:
///
///   1. Tray / log surfaces can carry a raw `class_name` (the
///      payload's join key) without knowing the slug — linking
///      via this endpoint outsources slug resolution to the
///      server.
///   2. Wiki renames change `display_name` (and thus the derived
///      slug). A user's old bookmark of the previous slug 404s,
///      but a class-name link survives because class_name is the
///      stable join key.
///
/// 308 (not 301) preserves the request method on follow — irrelevant
/// for GET but explicit about intent. Falls through to a direct
/// `ReferenceEntryDetail` response when the entry has no slug yet
/// (legacy rows pre-dating the cron's first run); the caller still
/// gets the data, just without a canonical URL to redirect to.
pub async fn get_entry_by_class_name<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    Path((category, class_name)): Path<(String, String)>,
) -> Response {
    let Some(cat) = ReferenceCategory::parse(&category) else {
        return error(
            StatusCode::NOT_FOUND,
            "unknown_category",
            Some(format!(
                "category '{category}' is not recognised; expected one of: vehicle, weapon, item, location"
            )),
        );
    };
    match store.get_entry(cat, &class_name).await {
        Ok(Some(entry)) => match entry.slug.as_deref() {
            Some(slug) if !slug.is_empty() => {
                let location = format!("/v1/reference/{category}/slug/{slug}");
                (
                    StatusCode::PERMANENT_REDIRECT,
                    [(axum::http::header::LOCATION, location)],
                )
                    .into_response()
            }
            // Legacy row with no slug — just serve the detail
            // shape so the caller still gets a useful response.
            _ => (StatusCode::OK, Json(entry_to_detail(entry))).into_response(),
        },
        Ok(None) => error(StatusCode::NOT_FOUND, "entry_not_found", None),
        Err(e) => {
            tracing::error!(
                error = %e,
                category = %category,
                class_name = %class_name,
                "get_entry_by_class_name failed",
            );
            error(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_store::test_support::MemoryReferenceStore;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Build a minimal router with just the `/v1/reference/*` routes
    /// backed by an in-memory store. No governor — the rate limiter
    /// is irrelevant at this layer and pulling it in would add a
    /// hidden dependency on the per-IP key extractor.
    fn test_app(store: Arc<MemoryReferenceStore>) -> Router {
        Router::new()
            .route(
                "/v1/reference/:category",
                get(list_entries::<MemoryReferenceStore>),
            )
            .route(
                "/v1/reference/:category/compare",
                get(get_compare::<MemoryReferenceStore>),
            )
            .route(
                "/v1/reference/:category/cohort",
                get(get_cohort::<MemoryReferenceStore>),
            )
            .route(
                "/v1/reference/:category/stats",
                get(get_category_stats::<MemoryReferenceStore>),
            )
            .route(
                "/v1/reference/:category/slug/:slug",
                get(get_entry_by_slug::<MemoryReferenceStore>),
            )
            .route(
                "/v1/reference/:category/by-class/:class_name",
                get(get_entry_by_class_name::<MemoryReferenceStore>),
            )
            .route(
                "/v1/reference/:category/:class_name",
                get(get_entry::<MemoryReferenceStore>),
            )
            .with_state(store)
            .layer(Extension(Arc::new(ReferenceListCache::new(
                Duration::from_secs(300),
            ))))
            .layer(Extension(Arc::new(ReferenceStatsCache::new(
                Duration::from_secs(300),
            ))))
            .layer(Extension(Arc::new(
                crate::reference_vectors::ReferenceVectorsCache::new(Duration::from_secs(300)),
            )))
    }

    fn vehicle(class_name: &str, display_name: &str, slug: Option<&str>) -> ReferenceEntry {
        let mut meta = serde_json::Map::new();
        meta.insert(
            "manufacturer".into(),
            serde_json::Value::String("Aegis Dynamics".into()),
        );
        meta.insert("role".into(), serde_json::Value::String("Fighter".into()));
        meta.insert(
            "hull_size".into(),
            serde_json::Value::String("Small".into()),
        );
        ReferenceEntry {
            category: ReferenceCategory::Vehicle,
            class_name: class_name.into(),
            display_name: display_name.into(),
            slug: slug.map(str::to_owned),
            metadata: serde_json::Value::Object(meta),
        }
    }

    #[tokio::test]
    async fn list_entries_returns_slim_shape_with_slug_and_summary() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                Some("aegis-avenger-stalker"),
            )])
            .await
            .unwrap();
        let app = test_app(store);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let parsed: ReferenceListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        let e = &parsed.entries[0];
        assert_eq!(e.class_name, "AEGS_Avenger_Stalker");
        assert_eq!(e.display_name, "Aegis Avenger Stalker");
        assert_eq!(e.slug.as_deref(), Some("aegis-avenger-stalker"));
        let vehicle = match &e.summary {
            Summary::Vehicle(v) => v,
            other => panic!("expected Summary::Vehicle, got {other:?}"),
        };
        assert_eq!(vehicle.manufacturer.as_deref(), Some("Aegis Dynamics"));
        assert_eq!(vehicle.role.as_deref(), Some("Fighter"));
        // `metadata` MUST NOT appear in the listing response — the
        // whole point of the slim shape is to drop multi-MB blobs.
        let raw: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            raw["entries"][0].get("metadata").is_none(),
            "listing must not include the full metadata blob"
        );
    }

    #[tokio::test]
    async fn get_entry_by_slug_returns_full_entry_with_metadata() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                Some("aegis-avenger-stalker"),
            )])
            .await
            .unwrap();
        let app = test_app(store);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/slug/aegis-avenger-stalker")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let entry: ReferenceEntryDetail = serde_json::from_slice(&body).unwrap();
        assert_eq!(entry.class_name, "AEGS_Avenger_Stalker");
        assert_eq!(entry.slug.as_deref(), Some("aegis-avenger-stalker"));
        // Detail endpoint keeps the full metadata payload.
        assert_eq!(
            entry.metadata.get("manufacturer").and_then(|v| v.as_str()),
            Some("Aegis Dynamics")
        );
        // …AND the curated summary projection — without this, the
        // web detail page would crash trying to read summary fields
        // (TS contract is required-field).
        let vehicle = match &entry.summary {
            Summary::Vehicle(v) => v,
            other => panic!("expected Summary::Vehicle, got {other:?}"),
        };
        assert_eq!(vehicle.manufacturer.as_deref(), Some("Aegis Dynamics"));
        assert_eq!(vehicle.role.as_deref(), Some("Fighter"));
    }

    #[tokio::test]
    async fn get_entry_by_slug_301s_uppercase_to_canonical_lowercase() {
        // Slug allowlist accepts uppercase for ergonomics, but the
        // canonical URL is lowercase. The handler 301-redirects so
        // bookmarks + crawlers converge on one URL.
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                Some("aegis-avenger-stalker"),
            )])
            .await
            .unwrap();
        let app = test_app(store);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/slug/AEGIS-AVENGER-STALKER")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            resp.headers().get(axum::http::header::LOCATION).unwrap(),
            "/v1/reference/vehicle/slug/aegis-avenger-stalker",
        );
    }

    #[tokio::test]
    async fn get_entry_by_slug_preserves_lowercase_slug_with_200() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                Some("aegis-avenger-stalker"),
            )])
            .await
            .unwrap();
        let app = test_app(store);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/slug/aegis-avenger-stalker")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_entry_by_slug_404s_unknown_slug() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/slug/no-such-thing")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_entry_by_slug_rejects_malformed_input() {
        // Slug allowlist: ASCII alphanumeric + `-`. Other URI-safe
        // characters (`_`, `.`, `+`, `~`) are rejected at the handler
        // BEFORE touching the store. Uppercase is allowed because the
        // store comparison is case-insensitive — see the
        // `is_case_insensitive` test above.
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        for bad in ["foo_bar", "foo.bar", "foo+bar", "foo~bar"] {
            let uri = format!("/v1/reference/vehicle/slug/{bad}");
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&uri)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "expected 404 for slug {bad:?}"
            );
        }
    }

    #[tokio::test]
    async fn get_entry_by_slug_404s_unknown_category() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/npc/slug/foo")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_entry_by_class_name_308s_to_canonical_slug_url() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                Some("aegis-avenger-stalker"),
            )])
            .await
            .unwrap();
        let app = test_app(store);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/by-class/AEGS_Avenger_Stalker")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            resp.headers().get(axum::http::header::LOCATION).unwrap(),
            "/v1/reference/vehicle/slug/aegis-avenger-stalker",
        );
    }

    #[tokio::test]
    async fn get_entry_by_class_name_serves_detail_when_no_slug() {
        // Legacy row pre-dating the slug backfill — endpoint
        // falls through to a direct ReferenceEntryDetail response
        // so the caller still gets the data.
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                None,
            )])
            .await
            .unwrap();
        let app = test_app(store);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/by-class/AEGS_Avenger_Stalker")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let entry: ReferenceEntryDetail = serde_json::from_slice(&body).unwrap();
        assert_eq!(entry.class_name, "AEGS_Avenger_Stalker");
        assert!(entry.slug.is_none());
    }

    #[tokio::test]
    async fn get_entry_by_class_name_404s_unknown_class() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/by-class/NOT_A_REAL_CLASS")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_entries_404s_unknown_category() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/npc")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn detail_includes_peer_group() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger_Stalker",
                "Aegis Avenger Stalker",
                Some("aegis-avenger-stalker"),
            )])
            .await
            .unwrap();
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/slug/aegis-avenger-stalker")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let entry: ReferenceEntryDetail = serde_json::from_slice(&body).unwrap();
        assert_eq!(entry.peer_group, "combat"); // role "Fighter" → combat family
    }

    #[tokio::test]
    async fn stats_route_returns_groups() {
        let store = Arc::new(MemoryReferenceStore::new());
        // exactly MIN_SAMPLE entries — keep in sync with reference_stats::MIN_SAMPLE
        let entries: Vec<_> = (0..5)
            .map(|i| {
                let mut e = vehicle(
                    &format!("AEGS_F{i}"),
                    &format!("Fighter {i}"),
                    Some(&format!("fighter-{i}")),
                );
                if let serde_json::Value::Object(m) = &mut e.metadata {
                    m.insert("speed".into(), serde_json::json!({ "scm": 200 + i }));
                }
                e
            })
            .collect();
        store.upsert_entries(&entries).await.unwrap();
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/stats")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let parsed: crate::reference_stats::ReferenceStatsResponse =
            serde_json::from_slice(&body).unwrap();
        assert!(parsed.groups.contains_key("family:combat"));
        assert!(parsed.groups["family:combat"].contains_key("speed.scm"));
    }

    // ---- ReferenceListCache ----------------------------------------

    #[tokio::test]
    async fn cache_serves_stale_within_ttl_and_rebuild_refreshes() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger",
                "Aegis Avenger",
                Some("aegis-avenger"),
            )])
            .await
            .unwrap();
        let cache = ReferenceListCache::new(Duration::from_secs(300));

        // First serve reads the store and caches one entry.
        let body1 = cache
            .serve(ReferenceCategory::Vehicle, store.as_ref())
            .await
            .unwrap();
        let v1: ReferenceListResponse = serde_json::from_slice(&body1).unwrap();
        assert_eq!(v1.entries.len(), 1);

        // Mutate the store, then serve again WITHIN the TTL — must come
        // back from the cache (still 1 entry), proving no re-read.
        store
            .upsert_entries(&[vehicle("ORIG_300i", "Origin 300i", Some("origin-300i"))])
            .await
            .unwrap();
        let body2 = cache
            .serve(ReferenceCategory::Vehicle, store.as_ref())
            .await
            .unwrap();
        let v2: ReferenceListResponse = serde_json::from_slice(&body2).unwrap();
        assert_eq!(
            v2.entries.len(),
            1,
            "should serve the cached body, not re-read"
        );

        // An explicit rebuild (what the reconcile cron calls) refreshes.
        cache
            .rebuild(ReferenceCategory::Vehicle, store.as_ref())
            .await
            .unwrap();
        let body3 = cache
            .serve(ReferenceCategory::Vehicle, store.as_ref())
            .await
            .unwrap();
        let v3: ReferenceListResponse = serde_json::from_slice(&body3).unwrap();
        assert_eq!(v3.entries.len(), 2, "rebuild picks up the new row");
    }

    #[tokio::test]
    async fn compare_returns_requested_vectors() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[
                vehicle("AEGS_A", "Avenger", Some("avenger")),
                vehicle("ANVL_H", "Hornet", Some("hornet")),
            ])
            .await
            .unwrap();
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/compare?slugs=avenger,hornet,nope")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: crate::reference_vectors::CompareResponse =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.entries.len(), 2); // unknown slug "nope" omitted
        assert!(parsed.entries.iter().any(|e| e.slug == "avenger"));
    }

    #[tokio::test]
    async fn compare_rejects_over_cap_and_missing() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        let many = (0..13)
            .map(|i| format!("s{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/reference/vehicle/compare?slugs={many}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        // over-cap → too_many_slugs
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let err: ApiErrorBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.error, "too_many_slugs");

        let resp2 = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/compare")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::BAD_REQUEST);
        // missing → missing_slugs
        let body2 = to_bytes(resp2.into_body(), 1 << 20).await.unwrap();
        let err2: ApiErrorBody = serde_json::from_slice(&body2).unwrap();
        assert_eq!(err2.error, "missing_slugs");
    }

    #[tokio::test]
    async fn cache_scopes_by_category() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle(
                "AEGS_Avenger",
                "Aegis Avenger",
                Some("aegis-avenger"),
            )])
            .await
            .unwrap();
        let cache = ReferenceListCache::new(Duration::from_secs(300));

        let veh = cache
            .serve(ReferenceCategory::Vehicle, store.as_ref())
            .await
            .unwrap();
        let wpn = cache
            .serve(ReferenceCategory::Weapon, store.as_ref())
            .await
            .unwrap();
        let veh: ReferenceListResponse = serde_json::from_slice(&veh).unwrap();
        let wpn: ReferenceListResponse = serde_json::from_slice(&wpn).unwrap();
        assert_eq!(veh.entries.len(), 1);
        assert_eq!(
            wpn.entries.len(),
            0,
            "weapon cache is independent of vehicle"
        );
    }

    #[tokio::test]
    async fn detail_includes_cohorts() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[vehicle("AEGS_A", "Avenger", Some("avenger"))])
            .await
            .unwrap();
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/slug/avenger")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let entry: ReferenceEntryDetail = serde_json::from_slice(&body).unwrap();
        // vehicle() has role "Fighter" → family:combat
        assert!(
            entry.cohorts.iter().any(|c| c.key == "family:combat"),
            "expected family:combat cohort, got: {:?}",
            entry.cohorts
        );
        assert!(
            entry.cohorts.iter().any(|c| c.kind == "type"),
            "expected a type cohort, got: {:?}",
            entry.cohorts
        );
    }

    #[tokio::test]
    async fn cohort_route_returns_members() {
        let store = Arc::new(MemoryReferenceStore::new());
        let entries: Vec<_> = (0..3)
            .map(|i| {
                vehicle(
                    &format!("F{i}"),
                    &format!("Fighter {i}"),
                    Some(&format!("f{i}")),
                )
            })
            .collect();
        store.upsert_entries(&entries).await.unwrap();
        let app = test_app(store);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/cohort?key=family:combat")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let parsed: crate::reference_vectors::CompareResponse =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.entries.len(), 3);
    }

    #[tokio::test]
    async fn cohort_route_rejects_bad_key() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/cohort?key=nonsense")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp2 = app
            .oneshot(
                Request::builder()
                    .uri("/v1/reference/vehicle/cohort")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::BAD_REQUEST);
    }
}

#[cfg(test)]
mod ssr_key_tests {
    use super::*;
    use axum::http::Request;

    fn req(headers: &[(&str, &str)]) -> Request<()> {
        let mut b = Request::builder().uri("/v1/reference/vehicle/slug/x");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(()).unwrap()
    }

    /// True when the request was NOT granted an SSR bucket.
    ///
    /// The fallback is `SmartIpKeyExtractor`, which cannot produce a key for a
    /// synthetic request with no peer address and returns `Err`. Both outcomes
    /// mean the same thing here — the caller did not get SSR treatment — and
    /// conflating them keeps the assertion about the security property rather
    /// than about the test harness.
    fn denied_ssr(r: &Result<String, GovernorError>) -> bool {
        match r {
            Ok(key) => !key.starts_with("ssr:"),
            Err(_) => true,
        }
    }

    fn with_token(tok: &str) -> SsrAwareIpKeyExtractor {
        SsrAwareIpKeyExtractor {
            token: Some(Arc::from(tok)),
        }
    }

    #[test]
    fn forwarded_address_is_ignored_without_the_secret() {
        // The whole security property: an address a caller simply asserts must
        // buy them nothing. Otherwise anyone could mint a private bucket, or
        // evade the limit entirely by rotating the header.
        let k = with_token("s3cret");
        let key = k.extract(&req(&[(SSR_FOR_HEADER, "203.0.113.9")]));
        assert!(
            denied_ssr(&key),
            "keyed as SSR without presenting the secret: {key:?}"
        );
    }

    #[test]
    fn a_wrong_secret_is_ignored() {
        let k = with_token("s3cret");
        let key = k.extract(&req(&[
            (SSR_TOKEN_HEADER, "not-the-secret"),
            (SSR_FOR_HEADER, "203.0.113.9"),
        ]));
        assert!(denied_ssr(&key), "wrong secret was accepted: {key:?}");
    }

    #[test]
    fn the_mechanism_is_off_when_unconfigured() {
        // Fail closed: with no configured secret, a caller presenting ANY
        // token must be keyed by its own address.
        let k = SsrAwareIpKeyExtractor { token: None };
        let key = k.extract(&req(&[
            (SSR_TOKEN_HEADER, "anything"),
            (SSR_FOR_HEADER, "203.0.113.9"),
        ]));
        assert!(
            denied_ssr(&key),
            "SSR keying active while unconfigured: {key:?}"
        );
    }

    #[test]
    fn each_end_user_gets_their_own_bucket() {
        // The point of the change. Two readers rendered by the same web
        // container must not share a limit — that shared bucket is what let a
        // crawler 429 everybody else.
        let k = with_token("s3cret");
        let a = k
            .extract(&req(&[
                (SSR_TOKEN_HEADER, "s3cret"),
                (SSR_FOR_HEADER, "198.51.100.1"),
            ]))
            .unwrap();
        let b = k
            .extract(&req(&[
                (SSR_TOKEN_HEADER, "s3cret"),
                (SSR_FOR_HEADER, "198.51.100.2"),
            ]))
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(a, "ssr:198.51.100.1");
    }

    #[test]
    fn authenticated_ssr_naming_nobody_is_still_bounded() {
        // A warmup or a cron has no end user. It gets one bucket rather than
        // no bucket — bounded, just not attributed.
        let k = with_token("s3cret");
        let key = k.extract(&req(&[(SSR_TOKEN_HEADER, "s3cret")])).unwrap();
        assert_eq!(key, "ssr:anonymous");
    }

    #[test]
    fn a_forwarded_value_cannot_collide_with_a_direct_callers_key() {
        // Namespacing matters: without it a forged `ssr-for` of the web
        // container's own IP would land in the same bucket as direct traffic.
        let k = with_token("s3cret");
        let ssr = k
            .extract(&req(&[
                (SSR_TOKEN_HEADER, "s3cret"),
                (SSR_FOR_HEADER, "198.51.100.1"),
            ]))
            .unwrap();
        assert!(ssr.starts_with("ssr:"));
        assert!(!ssr.starts_with("ip:"));
    }

    #[test]
    fn secret_comparison_does_not_short_circuit_on_length() {
        assert!(SsrAwareIpKeyExtractor::token_matches("abc", "abc"));
        assert!(!SsrAwareIpKeyExtractor::token_matches("abc", "abcd"));
        assert!(!SsrAwareIpKeyExtractor::token_matches("abc", "ab"));
        assert!(!SsrAwareIpKeyExtractor::token_matches("abc", "abd"));
    }
}
