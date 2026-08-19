//! Contract ingest + public read routes.
//!
//! Receiving side of sp-ingest's "Publish" push (see `contracts.rs` for
//! the wire models + storage).
//!
//! ## Endpoints
//!   * `POST /api/contracts/ingest` — bearer-gated (shared static
//!     token). Validates the [`PublishBundleReq`] shape, upserts by
//!     `canonical_id`, returns `200 { canonical_id, outcome }`.
//!     Idempotent on repeat.
//!   * `GET  /api/contracts`            — list + filters + pagination.
//!   * `GET  /api/contracts/search`     — free-text / location search.
//!   * `GET  /api/contracts/{id}`       — one contract, or 404.
//!
//! Reads are fully public (no auth). Writes are gated by the shared
//! token in `STARSTATS_INGEST_TOKEN`; when that env var is unset the
//! ingest endpoint returns `503 not_configured` (same posture as the
//! Revolut donate routes) so the operator gets a clear "you forgot to
//! set the token" signal rather than a silent accept.
//!
//! Auth model matches the sender exactly (verified against
//! `sp_ingest/publish/starstats.py`): a single shared bearer token,
//! sent as `Authorization: Bearer <token>`, validated here. No OAuth,
//! no signing.

use crate::api_error::ApiErrorBody;
use crate::contracts::{
    AdminReviewPacketReq, ContractListFilter, ContractStore, ExtractedContractReq,
    ExtractedStepReq, IngestValidationError, NewContract, PublishBundleReq, StoredContract,
};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use starstats_core::contract_taxonomy::{
    classify, normalise_legal_status, normalise_risk, ContractCategory,
};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// The one schema major this server understands. sp-ingest ships
/// `schema_version = "1"`; a payload whose major is a *different*
/// known integer is rejected rather than mis-parsed.
const SUPPORTED_SCHEMA_MAJOR: u32 = 1;

/// Default list/search page size.
const DEFAULT_LIMIT: i64 = 50;
/// Hard upper bound on a single page.
const MAX_LIMIT: i64 = 200;
/// Body cap for the ingest POST. `raw_text` is verbatim OCR so a dense
/// capture can be sizeable, but 4 MB is generous — matches `/v1/ingest`.
const INGEST_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// Row ceiling for a `?category=`-filtered list.
///
/// Category is DERIVED (see `starstats_core::contract_taxonomy`), so it is not
/// a SQL predicate and cannot be pushed into the query without duplicating the
/// classifier in SQL — the exact divergence the derive-at-query-time design
/// exists to avoid. Instead the filtered path scans up to this many rows,
/// classifies in Rust and paginates the result.
///
/// The unfiltered path is untouched and still paginates in SQL, so this costs
/// nothing on the common request. The catalogue is 124 rows today; if it ever
/// approaches this cap, promote `category` to a column in an additive
/// migration and make it a real predicate.
const CATEGORY_SCAN_CAP: i64 = 5_000;

// ---------------------------------------------------------------------
// Router state.
// ---------------------------------------------------------------------

/// State shared by every contract route. `store` is the dyn-cast
/// [`ContractStore`]; `ingest_token` is the shared write secret
/// (`None` = ingest disabled → 503).
#[derive(Clone)]
pub struct ContractApiState {
    store: Arc<dyn ContractStore>,
    ingest_token: Arc<Option<String>>,
}

/// Build the `/api/contracts` sub-router. `ingest_token` comes from
/// `STARSTATS_INGEST_TOKEN` (see `config.rs`); pass `None` to leave
/// ingest disabled.
pub fn router(store: Arc<dyn ContractStore>, ingest_token: Option<String>) -> Router {
    let state = ContractApiState {
        store,
        ingest_token: Arc::new(ingest_token),
    };
    Router::new()
        .route(
            "/api/contracts/ingest",
            post(ingest).layer(DefaultBodyLimit::max(INGEST_BODY_LIMIT)),
        )
        .route(
            "/api/contracts",
            get(list_contracts).delete(delete_all_contracts),
        )
        .route("/api/contracts/search", get(search_contracts))
        // Registration order is not load-bearing here — axum's router
        // matches static segments ahead of dynamic ones, verified by
        // moving these below `:canonical_id` and watching the tests
        // still pass. Kept adjacent to `search` for readability.
        .route("/api/contracts/by-entity", get(contracts_by_entity))
        .route("/api/contracts/resolve", get(resolve_contract_names))
        .route(
            "/api/contracts/:canonical_id",
            get(get_contract).delete(delete_one_contract),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------
// Response DTOs.
// ---------------------------------------------------------------------

/// 200 body for a successful ingest.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestAccepted {
    pub canonical_id: String,
    /// `inserted` for a brand-new contract, `updated` when the push
    /// folded into an existing `canonical_id`.
    pub outcome: String,
}

/// 200 body for a delete. `deleted` is the number of rows removed
/// (0 or 1 for a single delete; the full count for a bulk delete).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteResult {
    pub deleted: u64,
}

/// Public list/search row. Deliberately omits `raw_text`, the sender's
/// `suggestion` internals, `capture_id`, and `flags`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractSummary {
    pub canonical_id: String,
    pub display_name: Option<String>,
    pub contract_type: Option<String>,
    /// Parent category, DERIVED from `contract_type` + `gameplay_loop` (see
    /// `starstats_core::contract_taxonomy`). Closed vocabulary, always present
    /// — an unrecognised contract classifies as `other`, never NULL.
    ///
    /// Prefer this over `contract_type` for grouping and filtering:
    /// `contract_type` is raw LLM output and splits one concept across several
    /// spellings (`Salvage` / `SALVAGE`, `Cargo Haul` / `Cargo Recovery`).
    pub category: String,
    pub subcategory: Option<String>,
    pub gameplay_loop: Option<String>,
    pub issuer: Option<String>,
    pub faction: Option<String>,
    pub legal_status: Option<String>,
    /// `legal_status` folded to the closed vocabulary (`Legal` and `Lawful`
    /// both become `legal`). NULL when absent or unrecognised — absent is NOT
    /// the same as legal, so this is deliberately not defaulted.
    pub legal_status_normalised: Option<String>,
    pub reward_amount: Option<i64>,
    pub reward_currency: Option<String>,
    pub confidence_score: Option<f64>,
    pub patch_version: Option<String>,
    /// First non-empty step location. Disambiguates same-named
    /// contracts whose steps play out in different places.
    pub first_step_location: Option<String>,
    /// First step's item requirement, quantity stripped (e.g. `"15
    /// Amioshi Plague"` -> `"Amioshi Plague"`). Singular by design —
    /// see `contracts::promote_step_fields`.
    pub required_item: Option<String>,
    /// Number of execution steps.
    pub step_count: Option<i32>,
    /// First ingest time, ISO 8601 UTC.
    pub first_seen_at: String,
    /// Most-recent ingest time, ISO 8601 UTC.
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractListResponse {
    pub contracts: Vec<ContractSummary>,
    /// Offset to pass back as `?offset=` for the next page; `None` when
    /// this page reached the end of the result set.
    pub next_offset: Option<i64>,
}

/// Public detail view. Surfaces the structured extraction (contract +
/// steps) plus the advisory `suggested_action`. Never surfaces
/// `raw_text` or the sender's suggestion internals.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractDetail {
    pub canonical_id: String,
    pub schema_version: String,
    pub suggested_action: Option<String>,
    /// Parent category, DERIVED — see [`ContractSummary::category`]. Lives
    /// here rather than inside `contract` on purpose: `contract` is
    /// [`ExtractedContractReq`], the INGEST type, and `record` is built by
    /// re-serializing that tree. A derived field added there would be written
    /// into stored JSONB on every ingest, denormalising the one thing this
    /// design derives at query time.
    pub category: String,
    /// `contract.legal_status` folded to the closed vocabulary. NULL when
    /// absent or unrecognised.
    pub legal_status_normalised: Option<String>,
    pub contract: ExtractedContractReq,
    pub steps: Vec<ContractStepView>,
    /// KB entities this contract references, already resolved to a slug
    /// where resolution was unambiguous. Lets the page render KB links
    /// WITHOUT fetching a reference catalogue per render — the vehicles
    /// bundle alone is ~4 MB.
    pub entities: Vec<crate::contract_entities::EntityRow>,
    pub first_seen_at: String,
    pub updated_at: String,
}

/// One execution step as published: the stored step verbatim, plus the
/// derived `risk_normalised`.
///
/// A wrapper rather than extra fields on [`ExtractedStepReq`] because that type
/// is the ingest DTO and `record` is built by re-serializing it — a derived
/// field there would be persisted into stored JSONB. `#[serde(flatten)]` keeps
/// the wire shape byte-identical to before plus the one new key, and means a
/// future ingest field surfaces here automatically instead of being silently
/// dropped by a hand-maintained field list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractStepView {
    #[serde(flatten)]
    pub step: ExtractedStepReq,
    /// `risk` case-normalised to `low` | `medium` | `high`. The extractor emits
    /// both `Medium` and `medium`; the raw value is still published above.
    /// NULL when absent or unrecognised.
    pub risk_normalised: Option<String>,
}

impl ContractStepView {
    fn from_step(step: ExtractedStepReq) -> Self {
        let risk_normalised = normalise_risk(step.risk.as_deref()).map(|r| r.as_str().to_string());
        Self {
            step,
            risk_normalised,
        }
    }
}

impl ContractDetail {
    /// Project a stored row into the public detail DTO. The `record`
    /// JSONB is the full internal packet; we deserialize it and expose
    /// only the extraction + advisory action.
    fn from_stored(
        stored: StoredContract,
        entities: Vec<crate::contract_entities::EntityRow>,
    ) -> Self {
        let packet: AdminReviewPacketReq =
            serde_json::from_value(stored.record).unwrap_or_default();
        let mut contract = packet.extraction.contract;

        // `note` holds the VERBATIM sentence an award was read from
        // ("successful completion of the scenario will net the original
        // contract holder an award of 1 MG Scrip"). That is mission
        // prose, which `boundary.rs`'s counterpart in sp-ingest forbids
        // republishing: public output carries facts and authored
        // guidance, never source text.
        //
        // Stripped HERE rather than only in the page, because this
        // endpoint is public — hiding it in the UI would leave the prose
        // one request away. The note stays in `record` for provenance
        // and stays visible in the ingest tool, which is internal.
        for award in &mut contract.reward.additional {
            award.note = None;
        }

        // Derived on read from the SAME two fields the list path uses, so a
        // contract's category is necessarily identical in list and detail.
        let category = classify(
            contract.contract_type.as_deref(),
            contract.gameplay_loop.as_deref(),
        )
        .as_str()
        .to_string();
        let legal_status_normalised = normalise_legal_status(contract.legal_status.as_deref())
            .map(|l| l.as_str().to_string());

        Self {
            canonical_id: stored.canonical_id,
            schema_version: stored.schema_version,
            suggested_action: stored.suggested_action,
            category,
            legal_status_normalised,
            contract,
            steps: packet
                .extraction
                .steps
                .into_iter()
                .map(ContractStepView::from_step)
                .collect(),
            entities,
            first_seen_at: stored.first_seen_at.to_rfc3339(),
            updated_at: stored.updated_at.to_rfc3339(),
        }
    }
}

/// Derive a row's category. Split out so the `?category=` filter and the
/// response projection cannot drift apart.
fn category_of(r: &crate::contracts::ContractSummaryRow) -> ContractCategory {
    classify(r.contract_type.as_deref(), r.gameplay_loop.as_deref())
}

fn summary_from_row(r: crate::contracts::ContractSummaryRow) -> ContractSummary {
    let category = category_of(&r).as_str().to_string();
    let legal_status_normalised =
        normalise_legal_status(r.legal_status.as_deref()).map(|l| l.as_str().to_string());
    ContractSummary {
        canonical_id: r.canonical_id,
        display_name: r.display_name,
        contract_type: r.contract_type,
        category,
        subcategory: r.subcategory,
        gameplay_loop: r.gameplay_loop,
        issuer: r.issuer,
        faction: r.faction,
        legal_status: r.legal_status,
        legal_status_normalised,
        reward_amount: r.reward_amount,
        reward_currency: r.reward_currency,
        confidence_score: r.confidence_score,
        patch_version: r.patch_version,
        first_step_location: r.first_step_location,
        required_item: r.required_item,
        step_count: r.step_count,
        first_seen_at: r.first_seen_at.to_rfc3339(),
        updated_at: r.updated_at.to_rfc3339(),
    }
}

// ---------------------------------------------------------------------
// Query params.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct ListQuery {
    /// Filter by contract type (case-insensitive), e.g. `bounty`. Matches the
    /// RAW extracted value; prefer `category` for grouping.
    #[serde(rename = "type")]
    pub contract_type: Option<String>,
    /// Filter by derived parent category: `bounty_hunter` | `mercenary` |
    /// `hauling` | `delivery` | `salvage` | `mining` | `collection` |
    /// `investigation` | `maintenance` | `training` | `other`.
    /// An unrecognised value is a 400 rather than a silent empty page.
    pub category: Option<String>,
    /// Filter by issuer (case-insensitive).
    pub issuer: Option<String>,
    /// Filter by legal status (case-insensitive), e.g. `legal`.
    pub legal_status: Option<String>,
    /// Filter by faction (case-insensitive). The joint-strongest
    /// discriminator between same-named contracts.
    pub faction: Option<String>,
    /// Filter by gameplay loop (case-insensitive).
    pub gameplay_loop: Option<String>,
    /// Page size, clamped to `[1, 200]`; defaults to 50.
    pub limit: Option<i64>,
    /// Row offset for pagination; defaults to 0.
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct SearchQuery {
    /// Free-text term matched against name, issuer, type, subcategory,
    /// faction, attribute values, step locations, and objectives.
    pub q: Option<String>,
    /// Convenience alias — searches the same blob for a location.
    /// Ignored when `q` is also present.
    pub location: Option<String>,
    /// Filter by contract type (case-insensitive). Matches the RAW extracted
    /// value; prefer `category`.
    #[serde(rename = "type")]
    pub contract_type: Option<String>,
    /// Filter by derived parent category — see [`ListQuery::category`].
    pub category: Option<String>,
    /// Filter by issuer (case-insensitive).
    pub issuer: Option<String>,
    /// Filter by legal status (case-insensitive).
    pub legal_status: Option<String>,
    /// Filter by faction (case-insensitive). The joint-strongest
    /// discriminator between same-named contracts.
    pub faction: Option<String>,
    /// Filter by gameplay loop (case-insensitive).
    pub gameplay_loop: Option<String>,
    /// Page size, clamped to `[1, 200]`; defaults to 50.
    pub limit: Option<i64>,
    /// Row offset for pagination; defaults to 0.
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------
// Ingest handler.
// ---------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/contracts/ingest",
    tag = "contracts",
    request_body = PublishBundleReq,
    responses(
        (status = 200, description = "Contract upserted", body = IngestAccepted),
        (status = 400, description = "Missing canonical_id or unsupported schema version", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 503, description = "Ingest token not configured on the server", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn ingest(
    State(state): State<ContractApiState>,
    headers: HeaderMap,
    Json(bundle): Json<PublishBundleReq>,
) -> Response {
    // 1. Auth (shared write token). 503 when unconfigured, 401 when the
    // caller's bearer is missing/wrong.
    if let Err(resp) = check_token(&state, &headers) {
        return resp;
    }

    // 2. Schema version. Reject a known-different major; stay lenient on
    // an unparseable version (store as-is) so a sender quirk doesn't
    // drop otherwise-good data.
    if let Some(major) = schema_major(&bundle.schema_version) {
        if major != SUPPORTED_SCHEMA_MAJOR {
            return err(
                StatusCode::BAD_REQUEST,
                "unsupported_schema_version",
                Some(&format!(
                    "got major {major}, server speaks {SUPPORTED_SCHEMA_MAJOR}"
                )),
            );
        }
    }

    // 3. Validate + promote.
    let new_contract = match NewContract::from_bundle(&bundle) {
        Ok(c) => c,
        Err(IngestValidationError::MissingCanonicalId) => {
            return err(
                StatusCode::BAD_REQUEST,
                "missing_canonical_id",
                Some("canonical_id is required"),
            );
        }
    };
    let canonical_id = new_contract.canonical_id.clone();

    // 4. Upsert.
    match state.store.upsert(new_contract).await {
        Ok(outcome) => {
            tracing::info!(
                canonical_id = %canonical_id,
                outcome = outcome.as_str(),
                "contract ingested"
            );
            (
                StatusCode::OK,
                Json(IngestAccepted {
                    canonical_id,
                    outcome: outcome.as_str().to_string(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, canonical_id = %canonical_id, "contract upsert failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

// ---------------------------------------------------------------------
// Read handlers.
// ---------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/contracts",
    tag = "contracts",
    params(ListQuery),
    responses((status = 200, description = "Contract listing slice", body = ContractListResponse))
)]
pub async fn list_contracts(
    State(state): State<ContractApiState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let category = match parse_category(q.category) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let filter = ContractListFilter {
        contract_type: clean(q.contract_type),
        issuer: clean(q.issuer),
        legal_status: clean(q.legal_status),
        faction: clean(q.faction),
        gameplay_loop: clean(q.gameplay_loop),
        query: None,
        limit: clamp_limit(q.limit),
        offset: q.offset.unwrap_or(0).max(0),
    };
    run_list(&state, filter, category).await
}

#[utoipa::path(
    get,
    path = "/api/contracts/search",
    tag = "contracts",
    params(SearchQuery),
    responses((status = 200, description = "Contract search slice", body = ContractListResponse))
)]
pub async fn search_contracts(
    State(state): State<ContractApiState>,
    Query(q): Query<SearchQuery>,
) -> Response {
    let category = match parse_category(q.category) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    // `q` takes precedence; `location` is a convenience alias for the
    // same blob search.
    let query = clean(q.q).or_else(|| clean(q.location));
    let filter = ContractListFilter {
        contract_type: clean(q.contract_type),
        issuer: clean(q.issuer),
        legal_status: clean(q.legal_status),
        faction: clean(q.faction),
        gameplay_loop: clean(q.gameplay_loop),
        query,
        limit: clamp_limit(q.limit),
        offset: q.offset.unwrap_or(0).max(0),
    };
    run_list(&state, filter, category).await
}

/// Parse `?category=`. Absent/blank is `Ok(None)`; an unrecognised value is a
/// 400 so a typo cannot masquerade as "no contracts in that category".
fn parse_category(raw: Option<String>) -> Result<Option<ContractCategory>, Response> {
    let Some(raw) = clean(raw) else {
        return Ok(None);
    };
    match ContractCategory::parse(&raw.to_ascii_lowercase()) {
        Some(c) => Ok(Some(c)),
        None => Err(err(
            StatusCode::BAD_REQUEST,
            "invalid_category",
            Some(&format!(
                "unknown category {raw:?}; expected one of: {}",
                ContractCategory::ALL
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        )),
    }
}

/// Shared list/search execution: fetch `limit + 1` to detect a next
/// page without a count query, then shape the response.
///
/// When `category` is set the SQL pagination is bypassed — category is derived,
/// not stored, so it cannot be a predicate. See [`CATEGORY_SCAN_CAP`].
async fn run_list(
    state: &ContractApiState,
    mut filter: ContractListFilter,
    category: Option<ContractCategory>,
) -> Response {
    if let Some(want) = category {
        return run_list_by_category(state, filter, want).await;
    }

    let page_limit = filter.limit;
    let offset = filter.offset;
    filter.limit = page_limit.saturating_add(1);

    match state.store.list(filter).await {
        Ok(mut rows) => {
            let next_offset = if rows.len() as i64 > page_limit {
                rows.truncate(page_limit as usize);
                Some(offset + page_limit)
            } else {
                None
            };
            (
                StatusCode::OK,
                Json(ContractListResponse {
                    contracts: rows.into_iter().map(summary_from_row).collect(),
                    next_offset,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "contract list query failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

/// `?category=` path: scan up to [`CATEGORY_SCAN_CAP`] rows matching the other
/// (real, SQL) predicates, classify each in Rust, then paginate the survivors.
///
/// Pagination is applied AFTER filtering, so `limit` and `next_offset` mean the
/// same thing they do on the unfiltered path — a page is `limit` matching
/// contracts, not "whatever survived from a SQL page of `limit`".
async fn run_list_by_category(
    state: &ContractApiState,
    mut filter: ContractListFilter,
    want: ContractCategory,
) -> Response {
    let page_limit = filter.limit;
    let offset = filter.offset;
    filter.limit = CATEGORY_SCAN_CAP;
    filter.offset = 0;

    let rows = match state.store.list(filter).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "contract list query failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None);
        }
    };
    if rows.len() as i64 == CATEGORY_SCAN_CAP {
        tracing::warn!(
            cap = CATEGORY_SCAN_CAP,
            "category filter hit the scan cap; results may be incomplete"
        );
    }

    let matching: Vec<_> = rows
        .into_iter()
        .filter(|r| category_of(r) == want)
        .collect();

    let start = offset.clamp(0, matching.len() as i64) as usize;
    let end = (start as i64)
        .saturating_add(page_limit)
        .clamp(0, matching.len() as i64) as usize;
    let next_offset = if (end as i64) < matching.len() as i64 {
        Some(end as i64)
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(ContractListResponse {
            contracts: matching[start..end]
                .iter()
                .cloned()
                .map(summary_from_row)
                .collect(),
            next_offset,
        }),
    )
        .into_response()
}

/// Query for `GET /api/contracts/by-entity`.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
pub struct ByEntityQuery {
    /// KB category: `vehicle` | `weapon` | `item` | `location`.
    pub category: Option<String>,
    /// KB slug, as it appears in `/kb/{category}/{slug}`.
    pub slug: Option<String>,
    /// Page size, clamped to `[1, 200]`; defaults to 50.
    pub limit: Option<i64>,
    /// Row offset for pagination; defaults to 0.
    pub offset: Option<i64>,
}

/// Contracts referencing a knowledge-base entity.
///
/// Backs the Contracts section on a KB entity page. Joins the resolved
/// `(category, slug)` exactly — never a substring match against
/// `search_blob`, which would pair "Yela" with "Yela Ring".
#[utoipa::path(
    get,
    path = "/api/contracts/by-entity",
    tag = "contracts",
    params(ByEntityQuery),
    responses((status = 200, description = "Contracts referencing the entity", body = ContractListResponse))
)]
pub async fn contracts_by_entity(
    State(state): State<ContractApiState>,
    Query(q): Query<ByEntityQuery>,
) -> Response {
    let (Some(category), Some(slug)) = (clean(q.category), clean(q.slug)) else {
        // Without both halves the query is meaningless; an empty page is
        // a friendlier answer than a 400 for a page that renders this
        // section optionally.
        return (
            StatusCode::OK,
            Json(ContractListResponse {
                contracts: Vec::new(),
                next_offset: None,
            }),
        )
            .into_response();
    };
    let page_limit = clamp_limit(q.limit);
    let offset = q.offset.unwrap_or(0).max(0);

    // limit + 1 detects a next page without a COUNT, as `run_list` does.
    match state
        .store
        .list_by_entity(&category, &slug, page_limit.saturating_add(1), offset)
        .await
    {
        Ok(mut rows) => {
            let next_offset = if rows.len() as i64 > page_limit {
                rows.truncate(page_limit as usize);
                Some(offset + page_limit)
            } else {
                None
            };
            (
                StatusCode::OK,
                Json(ContractListResponse {
                    contracts: rows.into_iter().map(summary_from_row).collect(),
                    next_offset,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "contracts by-entity query failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

/// Response for `GET /api/contracts/resolve`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResolveNamesResponse {
    pub resolved: Vec<crate::contract_entities::NameResolution>,
}

/// Most names a caller resolves at once. `/me/contracts` renders many
/// run cards, and one request per card would burst the per-IP governor.
const MAX_RESOLVE_NAMES: usize = 100;

/// Resolve contract names to catalogue entries.
///
/// Repeat `name` to resolve several at once. `canonical_id` is non-null
/// ONLY when exactly one catalogue row carries the name; `display_name`
/// is deliberately non-unique, so any other count leaves it null and the
/// caller links to the filtered candidate list instead of guessing.
#[utoipa::path(
    get,
    path = "/api/contracts/resolve",
    tag = "contracts",
    params(("name" = Vec<String>, Query, description = "Contract name; repeat for several")),
    responses((status = 200, description = "One entry per requested name", body = ResolveNamesResponse))
)]
pub async fn resolve_contract_names(
    State(state): State<ContractApiState>,
    Query(raw): Query<Vec<(String, String)>>,
) -> Response {
    // axum 0.7 does not fold repeated params into a Vec, so read the
    // raw pairs and keep the ones keyed `name`.
    let mut names: Vec<String> = Vec::new();
    for (k, v) in raw {
        if k == "name" {
            if let Some(v) = clean(Some(v)) {
                if !names.iter().any(|n| n == &v) {
                    names.push(v);
                }
            }
        }
        if names.len() >= MAX_RESOLVE_NAMES {
            break;
        }
    }

    match state.store.resolve_names(&names).await {
        Ok(resolved) => (StatusCode::OK, Json(ResolveNamesResponse { resolved })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "contract name resolution failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/contracts/{canonical_id}",
    tag = "contracts",
    params(("canonical_id" = String, Path, description = "Canonical contract id")),
    responses(
        (status = 200, description = "One contract", body = ContractDetail),
        (status = 404, description = "No such contract", body = ApiErrorBody),
    )
)]
pub async fn get_contract(
    State(state): State<ContractApiState>,
    Path(canonical_id): Path<String>,
) -> Response {
    match state.store.get(&canonical_id).await {
        Ok(Some(stored)) => {
            // Entity rows are decoration: if the lookup fails the page
            // should still render, with plain text instead of KB links.
            let entities = state
                .store
                .entities_for(&canonical_id)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, canonical_id = %canonical_id,
                                   "entity lookup failed; detail rendered without KB links");
                    Vec::new()
                });
            (
                StatusCode::OK,
                Json(ContractDetail::from_stored(stored, entities)),
            )
                .into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "not_found", None),
        Err(e) => {
            tracing::error!(error = %e, canonical_id = %canonical_id, "contract fetch failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

#[utoipa::path(
    delete,
    path = "/api/contracts",
    tag = "contracts",
    responses(
        (status = 200, description = "All contracts deleted", body = DeleteResult),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 503, description = "Writes not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn delete_all_contracts(
    State(state): State<ContractApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&state, &headers) {
        return resp;
    }
    match state.store.delete_all().await {
        Ok(deleted) => {
            tracing::info!(deleted, "contracts bulk-deleted");
            (StatusCode::OK, Json(DeleteResult { deleted })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "contracts delete_all failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

#[utoipa::path(
    delete,
    path = "/api/contracts/{canonical_id}",
    tag = "contracts",
    params(("canonical_id" = String, Path, description = "Canonical contract id")),
    responses(
        (status = 200, description = "Contract deleted", body = DeleteResult),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorBody),
        (status = 404, description = "No such contract", body = ApiErrorBody),
        (status = 503, description = "Writes not configured", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn delete_one_contract(
    State(state): State<ContractApiState>,
    headers: HeaderMap,
    Path(canonical_id): Path<String>,
) -> Response {
    if let Err(resp) = check_token(&state, &headers) {
        return resp;
    }
    match state.store.delete(&canonical_id).await {
        Ok(true) => (StatusCode::OK, Json(DeleteResult { deleted: 1 })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "not_found", None),
        Err(e) => {
            tracing::error!(error = %e, canonical_id = %canonical_id, "contract delete failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "database_error", None)
        }
    }
}

// ---------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------

/// Build a JSON `ApiErrorBody` response.
fn err(status: StatusCode, code: &str, detail: Option<&str>) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: detail.map(str::to_string),
        }),
    )
        .into_response()
}

/// Validate the shared write token (gates ingest + delete). Returns
/// `Err(response)` to short-circuit: `503 not_configured` when the
/// server has no token set, `401 unauthorized` when the caller's
/// bearer is missing or wrong. `Ok(())` when it matches.
fn check_token(state: &ContractApiState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = state.ingest_token.as_ref().as_deref() else {
        return Err(err(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_configured",
            Some("contract writes are not enabled on this server (STARSTATS_INGEST_TOKEN unset)"),
        ));
    };
    match bearer_token(headers) {
        Some(tok) if ct_eq(tok.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => Err(err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            Some("missing or invalid bearer token"),
        )),
    }
}

/// Trim + drop empty query-string values so `?type=` behaves like an
/// absent filter rather than "match the empty string".
fn clean(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Clamp a requested page size to `[1, MAX_LIMIT]`, defaulting to
/// [`DEFAULT_LIMIT`]. `?limit=0` collapses to 1 so a page is never
/// silently empty.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Extract the bearer token from an `Authorization: Bearer <token>`
/// header. Case-insensitive on the scheme keyword.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let t = token.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    } else {
        None
    }
}

/// Constant-time byte compare — avoids leaking token length/prefix via
/// response timing. Returns false immediately on a length mismatch
/// (length isn't secret) and otherwise ORs every byte difference.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse the major version from a `schema_version` string ("1",
/// "1.2", ...). Returns `None` for a version whose leading component
/// isn't an integer — the caller treats that as "can't tell, be
/// lenient" rather than a hard reject.
fn schema_major(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.parse::<u32>().ok()
}

// ---------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::test_support::MemoryContractStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    const TOKEN: &str = "shared-secret-token";

    fn app_with_token(token: Option<&str>) -> (Router, Arc<MemoryContractStore>) {
        let store = Arc::new(MemoryContractStore::new());
        let store_dyn: Arc<dyn ContractStore> = store.clone();
        let app = router(store_dyn, token.map(str::to_string));
        (app, store)
    }

    fn app() -> (Router, Arc<MemoryContractStore>) {
        app_with_token(Some(TOKEN))
    }

    /// Publish a contract whose single step names one canonical entity.
    async fn seed_entity_contract(app: &Router, id: &str, name: &str, kind: &str, entity: &str) {
        let body = json!({
            "schema_version": "1",
            "canonical_id": id,
            "capture_id": "cap",
            "internal": {
                "source_capture_id": "cap",
                "raw_text": "x",
                "extraction": {
                    "contract": { "display_name": name },
                    "steps": [ { "order": 1, "step_type": "navigate", "summary": "Go",
                                 "entities": [ { "kind": kind, "name": entity } ] } ]
                },
                "suggestion": { "suggested_action": "create_new_contract" },
                "confidence_score": 0.9,
                "flags": []
            }
        });
        let resp = post_ingest(app, Some(TOKEN), &body).await;
        assert_eq!(resp.status(), StatusCode::OK, "seed ingest failed");
    }

    #[tokio::test]
    async fn detail_never_serves_the_verbatim_award_sentence() {
        // The award itself is a fact and must survive; the sentence it
        // was read from is mission prose and must not leave the server.
        let (app, _store) = app();
        let mut body = example_payload();
        body["internal"]["extraction"]["contract"]["reward"] = serde_json::json!({
            "amount": 23000,
            "currency": "aUEC",
            "additional": [ { "amount": 1, "unit": "MG Scrip",
                "note": "successful completion will net the holder an award of 1 MG Scrip" } ]
        });
        let resp = post_ingest(&app, Some(TOKEN), &body).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let (status, got) = get_json(&app, "/api/contracts/apprehend_zane_esteban").await;
        assert_eq!(status, StatusCode::OK);
        let award = &got["contract"]["reward"]["additional"][0];
        assert_eq!(award["amount"], 1, "the award must survive");
        assert_eq!(award["unit"], "MG Scrip");
        assert!(
            award["note"].is_null(),
            "verbatim mission prose must not reach a public response"
        );
        // And it must not appear anywhere else in the payload either.
        assert!(
            !serde_json::to_string(&got)
                .unwrap()
                .contains("net the holder"),
            "award sentence leaked into the detail response"
        );
    }

    #[tokio::test]
    async fn detail_surfaces_resolved_and_unresolved_entities() {
        let (app, store) = app();
        store.resolve_to("location", "Glaciem Ring", "location", "glaciem-ring");
        seed_entity_contract(&app, "c1", "A", "location", "Glaciem Ring").await;
        seed_entity_contract(&app, "c2", "B", "location", "Somewhere Unmapped").await;

        let (status, body) = get_json(&app, "/api/contracts/c1").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["entities"][0]["ref_slug"], "glaciem-ring");
        assert_eq!(body["entities"][0]["ref_category"], "location");
        assert_eq!(body["entities"][0]["raw_value"], "Glaciem Ring");

        // Unresolved entities still surface, carrying the raw value the
        // page renders as plain text instead of a link that would 404.
        let (_, body2) = get_json(&app, "/api/contracts/c2").await;
        assert_eq!(body2["entities"][0]["raw_value"], "Somewhere Unmapped");
        assert!(body2["entities"][0]["ref_slug"].is_null());
    }

    #[tokio::test]
    async fn detail_distinguishes_ambiguous_entities_from_unknown_ones() {
        // Both leave ref_slug empty, but they are NOT the same: several
        // matches means the knowledge base holds entries and we cannot
        // say which — that links to a search. No match means we know
        // nothing, and a search link would be a dead end.
        //
        // Measured cause: "Sunset Berries" exists three times in the
        // registry as sunset-berries, -2 and -3.
        let (app, store) = app();
        store.resolve_ambiguously("item", "Sunset Berries", 3);
        seed_entity_contract(&app, "amb", "A", "item", "Sunset Berries").await;
        seed_entity_contract(&app, "unk", "B", "item", "Nothing Named This").await;

        let (_, amb) = get_json(&app, "/api/contracts/amb").await;
        assert!(
            amb["entities"][0]["ref_slug"].is_null(),
            "must not pick one"
        );
        assert_eq!(amb["entities"][0]["ref_match_count"], 3);

        let (_, unk) = get_json(&app, "/api/contracts/unk").await;
        assert!(unk["entities"][0]["ref_slug"].is_null());
        assert_eq!(unk["entities"][0]["ref_match_count"], 0);
    }

    #[tokio::test]
    async fn by_entity_returns_only_contracts_referencing_that_slug() {
        let (app, store) = app();
        store.resolve_to("location", "Glaciem Ring", "location", "glaciem-ring");
        store.resolve_to("location", "Area18", "location", "area18");
        seed_entity_contract(&app, "hit", "A", "location", "Glaciem Ring").await;
        seed_entity_contract(&app, "miss", "B", "location", "Area18").await;

        let (status, body) = get_json(
            &app,
            "/api/contracts/by-entity?category=location&slug=glaciem-ring",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<&str> = body["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["canonical_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["hit"]);
    }

    #[tokio::test]
    async fn by_entity_is_not_captured_by_the_canonical_id_route() {
        // Pins that `by-entity` reaches its own handler rather than the
        // detail route, which would answer 404 for the id "by-entity".
        //
        // NOTE: this does NOT depend on registration order. Moving the
        // static routes below `:canonical_id` was tried and the test
        // still passed — axum matches static segments first. The guard
        // that matters is that the path resolves to a handler returning
        // a contracts page at all.
        let (app, _store) = app();
        let (status, body) = get_json(
            &app,
            "/api/contracts/by-entity?category=location&slug=nowhere",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "by-entity hit the detail route");
        assert!(body["contracts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolve_links_a_unique_name_and_refuses_an_ambiguous_one() {
        let (app, _store) = app();
        seed_entity_contract(&app, "solo", "Patrol Dangerous Sector", "location", "X").await;
        seed_entity_contract(
            &app,
            "dup_a",
            "Combat Gauntlet - Scenario #1",
            "location",
            "X",
        )
        .await;
        seed_entity_contract(
            &app,
            "dup_b",
            "Combat Gauntlet - Scenario #1",
            "location",
            "X",
        )
        .await;

        let uri = concat!(
            "/api/contracts/resolve",
            "?name=Patrol%20Dangerous%20Sector",
            "&name=Combat%20Gauntlet%20-%20Scenario%20%231",
            "&name=Never%20Published"
        );
        let (status, body) = get_json(&app, uri).await;
        assert_eq!(status, StatusCode::OK);
        let r = body["resolved"].as_array().unwrap();
        assert_eq!(r.len(), 3, "one entry per requested name");

        assert_eq!(r[0]["match_count"], 1);
        assert_eq!(r[0]["canonical_id"], "solo");

        // display_name is non-unique BY DESIGN — naming a winner here is
        // the confident-wrong-answer failure the design forbids.
        assert_eq!(r[1]["match_count"], 2);
        assert!(r[1]["canonical_id"].is_null());

        assert_eq!(r[2]["match_count"], 0);
        assert!(r[2]["canonical_id"].is_null());
    }

    #[tokio::test]
    async fn resolve_with_no_names_answers_empty_not_an_error() {
        let (app, _store) = app();
        let (status, body) = get_json(&app, "/api/contracts/resolve").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["resolved"].as_array().unwrap().is_empty());
    }

    /// The verbatim handoff example JSON.
    fn example_payload() -> serde_json::Value {
        json!({
            "schema_version": "1",
            "canonical_id": "apprehend_zane_esteban",
            "capture_id": "cap_01H",
            "internal": {
                "source_capture_id": "cap_01H",
                "raw_text": "BOUNTY HUNTER\nApprehend Zane Esteban ... (verbatim OCR)",
                "extraction": {
                    "contract": {
                        "canonical_name": "apprehend_zane_esteban",
                        "display_name": "Apprehend Zane Esteban",
                        "contract_type": "bounty",
                        "subcategory": "Apprehension",
                        "gameplay_loop": "bounty_hunting",
                        "issuer": "Crusader Security",
                        "faction": "Crusader",
                        "legal_status": "legal",
                        "required_reputation": null,
                        "reputation_rank": null,
                        "reward": { "amount": 8500, "currency": "aUEC", "bonus_amount": 1500 },
                        "fees": [ { "type": "deposit", "amount": 0, "currency": "aUEC", "refundable": true } ],
                        "failure_penalty": null,
                        "cargo_loss_penalty": null,
                        "rep_loss_warning": null,
                        "net_estimated_profit": null,
                        "timeframe": { "has_time_limit": true, "deadline_text": "2h", "duration_minutes": 120, "confidence": 0.7 },
                        "attributes": [
                            { "label": "LAST KNOWN LOCATION", "value": "Glaciem Ring", "confidence": 0.8 },
                            { "label": "RISK ASSESSMENT", "value": "Medium", "confidence": 0.9 }
                        ],
                        "primary_objectives": [ "Travel to Glaciem Ring", "Apprehend or eliminate the target" ],
                        "patch_version": "3.23",
                        "confidence_score": 0.86
                    },
                    "steps": [
                        { "order": 1, "step_type": "accept_contract", "summary": "Accept the bounty from the contract manager.",
                          "guidance": true, "tip": "Fit a component-damage loadout.", "location": null, "required_item": null,
                          "required_cargo": null, "required_vehicle": null, "required_equipment": null, "risk": null,
                          "optional": false, "failure_condition": null, "confidence": 0.0 },
                        { "order": 2, "step_type": "navigate", "summary": "Travel to Glaciem Ring.", "guidance": false, "tip": null,
                          "location": "Glaciem Ring", "required_item": null, "required_cargo": null, "required_vehicle": null,
                          "required_equipment": null, "risk": "medium", "optional": false, "failure_condition": null, "confidence": 0.0 },
                        { "order": 3, "step_type": "engage", "summary": "Apprehend or eliminate the target.", "guidance": false,
                          "tip": null, "location": null, "required_item": null, "required_cargo": null, "required_vehicle": null,
                          "required_equipment": null, "risk": "high", "optional": false, "failure_condition": "Target escapes the area",
                          "confidence": 0.0 }
                    ]
                },
                "suggestion": {
                    "matched_existing_contract_id": null,
                    "match_confidence": 0.0,
                    "suggested_action": "create_new_contract",
                    "changed_fields": [],
                    "recommendations": [],
                    "editor_note": null
                },
                "confidence_score": 0.86,
                "flags": []
            }
        })
    }

    async fn post_ingest(app: &Router, token: Option<&str>, body: &serde_json::Value) -> Response {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/contracts/ingest")
            .header("content-type", "application/json");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let req = req
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    async fn get_json(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, v)
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The handoff-required round-trip: ingest the EXACT example JSON,
    /// then read it back and assert the structured projection survives
    /// AND that raw_text never leaks to the public DTO.
    #[tokio::test]
    async fn ingest_then_read_back_roundtrip() {
        let (app, _store) = app();

        let resp = post_ingest(&app, Some(TOKEN), &example_payload()).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let accepted = body_json(resp).await;
        assert_eq!(accepted["canonical_id"], "apprehend_zane_esteban");
        assert_eq!(accepted["outcome"], "inserted");

        let (status, detail) = get_json(&app, "/api/contracts/apprehend_zane_esteban").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["canonical_id"], "apprehend_zane_esteban");
        assert_eq!(detail["schema_version"], "1");
        assert_eq!(detail["suggested_action"], "create_new_contract");
        let c = &detail["contract"];
        assert_eq!(c["display_name"], "Apprehend Zane Esteban");
        assert_eq!(c["contract_type"], "bounty");
        assert_eq!(c["issuer"], "Crusader Security");
        assert_eq!(c["reward"]["amount"], 8500);
        assert_eq!(c["reward"]["bonus_amount"], 1500);
        assert_eq!(c["timeframe"]["duration_minutes"], 120);
        assert_eq!(c["attributes"][0]["value"], "Glaciem Ring");
        assert_eq!(detail["steps"].as_array().unwrap().len(), 3);
        assert_eq!(
            detail["steps"][2]["failure_condition"],
            "Target escapes the area"
        );

        // raw_text must NOT appear anywhere in the public payload.
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(
            !serialized.contains("verbatim OCR"),
            "raw_text must never leak into the public detail DTO"
        );
        assert!(detail.get("raw_text").is_none());
    }

    #[tokio::test]
    async fn ingest_is_idempotent_on_repeat() {
        let (app, store) = app();
        let first = post_ingest(&app, Some(TOKEN), &example_payload()).await;
        assert_eq!(body_json(first).await["outcome"], "inserted");
        let second = post_ingest(&app, Some(TOKEN), &example_payload()).await;
        assert_eq!(body_json(second).await["outcome"], "updated");
        assert_eq!(store.len(), 1, "repeat push must not duplicate");
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let (app, _) = app();
        let resp = post_ingest(&app, None, &example_payload()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_wrong_token() {
        let (app, _) = app();
        let resp = post_ingest(&app, Some("not-the-token"), &example_payload()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn returns_503_when_token_unconfigured() {
        let (app, _) = app_with_token(None);
        let resp = post_ingest(&app, Some(TOKEN), &example_payload()).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let v = body_json(resp).await;
        assert_eq!(v["error"], "not_configured");
    }

    #[tokio::test]
    async fn rejects_blank_canonical_id() {
        let (app, _) = app();
        let mut payload = example_payload();
        payload["canonical_id"] = json!("");
        let resp = post_ingest(&app, Some(TOKEN), &payload).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(resp).await["error"], "missing_canonical_id");
    }

    #[tokio::test]
    async fn rejects_unsupported_schema_major() {
        let (app, _) = app();
        let mut payload = example_payload();
        payload["schema_version"] = json!("2");
        let resp = post_ingest(&app, Some(TOKEN), &payload).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(resp).await["error"], "unsupported_schema_version");
    }

    #[tokio::test]
    async fn get_unknown_id_is_404() {
        let (app, _) = app();
        let (status, v) = get_json(&app, "/api/contracts/does_not_exist").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "not_found");
    }

    #[tokio::test]
    async fn list_and_search_are_public_and_return_ingested() {
        let (app, _) = app();
        post_ingest(&app, Some(TOKEN), &example_payload()).await;

        // List — no auth header at all.
        let (status, list) = get_json(&app, "/api/contracts").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["contracts"].as_array().unwrap().len(), 1);
        assert_eq!(
            list["contracts"][0]["canonical_id"],
            "apprehend_zane_esteban"
        );
        assert_eq!(list["contracts"][0]["reward_amount"], 8500);

        // Filter by type.
        let (_, bounties) = get_json(&app, "/api/contracts?type=bounty").await;
        assert_eq!(bounties["contracts"].as_array().unwrap().len(), 1);
        let (_, empty) = get_json(&app, "/api/contracts?type=mining").await;
        assert_eq!(empty["contracts"].as_array().unwrap().len(), 0);

        // Search by location.
        let (_, hits) = get_json(&app, "/api/contracts/search?location=glaciem").await;
        assert_eq!(hits["contracts"].as_array().unwrap().len(), 1);
        let (_, miss) = get_json(&app, "/api/contracts/search?q=nonexistent").await;
        assert_eq!(miss["contracts"].as_array().unwrap().len(), 0);
    }

    /// `/api/contracts/search` specifically — proves `SearchQuery` now
    /// carries `legal_status`/`faction`/`gameplay_loop` through to the
    /// store filter. Before this fix, `legal_status` was silently
    /// dropped (hardcoded `None` in the handler) and `faction`/
    /// `gameplay_loop` didn't exist on `SearchQuery` at all, so an
    /// unrecognized query key is ignored by serde rather than erroring —
    /// each of these assertions would have seen BOTH rows (the filter
    /// never applied) instead of narrowing to one.
    #[tokio::test]
    async fn search_filters_by_faction_legal_status_and_gameplay_loop() {
        let (app, _) = app();

        let mut a = example_payload();
        a["canonical_id"] = json!("cA");
        a["internal"]["extraction"]["contract"]["faction"] = json!("UEE");
        a["internal"]["extraction"]["contract"]["legal_status"] = json!("legal");
        a["internal"]["extraction"]["contract"]["gameplay_loop"] = json!("bounty_hunting");
        post_ingest(&app, Some(TOKEN), &a).await;

        let mut b = example_payload();
        b["canonical_id"] = json!("cB");
        b["internal"]["extraction"]["contract"]["faction"] = json!("Nine Tails");
        b["internal"]["extraction"]["contract"]["legal_status"] = json!("illegal");
        b["internal"]["extraction"]["contract"]["gameplay_loop"] = json!("smuggling");
        post_ingest(&app, Some(TOKEN), &b).await;

        fn ids(v: &serde_json::Value) -> Vec<&str> {
            v["contracts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["canonical_id"].as_str().unwrap())
                .collect()
        }

        let (_, by_faction) = get_json(&app, "/api/contracts/search?faction=nine%20tails").await;
        assert_eq!(ids(&by_faction), vec!["cB"]);

        let (_, by_legal) = get_json(&app, "/api/contracts/search?legal_status=LEGAL").await;
        assert_eq!(
            ids(&by_legal),
            vec!["cA"],
            "legal_status must be wired from SearchQuery through to the store filter"
        );

        let (_, by_loop) = get_json(&app, "/api/contracts/search?gameplay_loop=Smuggling").await;
        assert_eq!(ids(&by_loop), vec!["cB"]);
    }

    /// `/api/contracts` (the LIST endpoint) specifically — not `/search`.
    ///
    /// The client only ever calls `/api/contracts/search` when a
    /// free-text `q`/`location` term is present; a bare facet filter
    /// (faction alone, say) goes to `/api/contracts`. `ListQuery` had no
    /// `faction`/`gameplay_loop` fields at all until this fix, so
    /// filtering the catalog by either of the two strongest same-name
    /// discriminators was unreachable without also typing a search term.
    /// This test would pass against `/search` regardless of whether
    /// `ListQuery` is wired — it must hit `/api/contracts` to mean
    /// anything.
    #[tokio::test]
    async fn list_filters_by_faction_legal_status_and_gameplay_loop() {
        let (app, _) = app();

        let mut a = example_payload();
        a["canonical_id"] = json!("cA");
        a["internal"]["extraction"]["contract"]["faction"] = json!("UEE");
        a["internal"]["extraction"]["contract"]["legal_status"] = json!("legal");
        a["internal"]["extraction"]["contract"]["gameplay_loop"] = json!("bounty_hunting");
        post_ingest(&app, Some(TOKEN), &a).await;

        let mut b = example_payload();
        b["canonical_id"] = json!("cB");
        b["internal"]["extraction"]["contract"]["faction"] = json!("Nine Tails");
        b["internal"]["extraction"]["contract"]["legal_status"] = json!("illegal");
        b["internal"]["extraction"]["contract"]["gameplay_loop"] = json!("smuggling");
        post_ingest(&app, Some(TOKEN), &b).await;

        fn ids(v: &serde_json::Value) -> Vec<&str> {
            v["contracts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["canonical_id"].as_str().unwrap())
                .collect()
        }

        // No `q`/`location` anywhere in these requests — pure facet
        // filters against the plain list endpoint.
        let (_, by_faction) = get_json(&app, "/api/contracts?faction=nine%20tails").await;
        assert_eq!(ids(&by_faction), vec!["cB"]);

        let (_, by_legal) = get_json(&app, "/api/contracts?legal_status=LEGAL").await;
        assert_eq!(ids(&by_legal), vec!["cA"]);

        let (_, by_loop) = get_json(&app, "/api/contracts?gameplay_loop=Smuggling").await;
        assert_eq!(
            ids(&by_loop),
            vec!["cB"],
            "faction/gameplay_loop must be wired from ListQuery through to the store filter"
        );
    }

    #[tokio::test]
    async fn accepts_unknown_enum_values_leniently() {
        // contract_type / step_type are open strings — an unknown value
        // must be stored, not rejected.
        let (app, _) = app();
        let mut payload = example_payload();
        payload["internal"]["extraction"]["contract"]["contract_type"] = json!("time_trial_race");
        let resp = post_ingest(&app, Some(TOKEN), &payload).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let (_, detail) = get_json(&app, "/api/contracts/apprehend_zane_esteban").await;
        assert_eq!(detail["contract"]["contract_type"], "time_trial_race");
    }

    async fn delete_req(
        app: &Router,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut b = Request::builder().method("DELETE").uri(uri);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let resp = app
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, v)
    }

    #[tokio::test]
    async fn delete_all_requires_token_and_clears() {
        let (app, store) = app();
        post_ingest(&app, Some(TOKEN), &example_payload()).await;
        assert_eq!(store.len(), 1);

        // No token → 401, nothing deleted.
        let (s401, _) = delete_req(&app, "/api/contracts", None).await;
        assert_eq!(s401, StatusCode::UNAUTHORIZED);
        assert_eq!(store.len(), 1);

        // Wrong token → 401.
        let (sbad, _) = delete_req(&app, "/api/contracts", Some("nope")).await;
        assert_eq!(sbad, StatusCode::UNAUTHORIZED);
        assert_eq!(store.len(), 1);

        // Correct token → 200 with count, store cleared.
        let (s200, body) = delete_req(&app, "/api/contracts", Some(TOKEN)).await;
        assert_eq!(s200, StatusCode::OK);
        assert_eq!(body["deleted"], 1);
        assert_eq!(store.len(), 0);
    }

    #[tokio::test]
    async fn delete_all_503_when_unconfigured() {
        let (app, _) = app_with_token(None);
        let (s, v) = delete_req(&app, "/api/contracts", Some(TOKEN)).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "not_configured");
    }

    #[tokio::test]
    async fn delete_one_by_id() {
        let (app, store) = app();
        post_ingest(&app, Some(TOKEN), &example_payload()).await;

        // Unknown id → 404.
        let (s404, _) = delete_req(&app, "/api/contracts/nope", Some(TOKEN)).await;
        assert_eq!(s404, StatusCode::NOT_FOUND);
        assert_eq!(store.len(), 1);

        // Known id → 200, removed.
        let (s200, body) =
            delete_req(&app, "/api/contracts/apprehend_zane_esteban", Some(TOKEN)).await;
        assert_eq!(s200, StatusCode::OK);
        assert_eq!(body["deleted"], 1);
        assert_eq!(store.len(), 0);
    }

    // -----------------------------------------------------------------
    // Derived taxonomy (category / legal_status_normalised /
    // risk_normalised) and the `?category=` filter.
    // -----------------------------------------------------------------

    /// A second contract with a deliberately messy `contract_type`, so the
    /// filter tests exercise real normalisation rather than exact matching.
    fn salvage_payload() -> serde_json::Value {
        let mut v = example_payload();
        v["canonical_id"] = json!("strip_the_caterpillar");
        let c = &mut v["internal"]["extraction"]["contract"];
        c["canonical_name"] = json!("strip_the_caterpillar");
        c["display_name"] = json!("Strip the Caterpillar");
        c["contract_type"] = json!("SALVAGE"); // shouty, must still classify
        c["gameplay_loop"] = json!("Salvage");
        c["legal_status"] = json!("Lawful"); // must fold to `legal`
        v
    }

    #[tokio::test]
    async fn list_carries_derived_category_on_every_row() {
        let (app, _) = app();
        post_ingest(&app, Some(TOKEN), &example_payload()).await;
        post_ingest(&app, Some(TOKEN), &salvage_payload()).await;

        let (status, body) = get_json(&app, "/api/contracts").await;
        assert_eq!(status, StatusCode::OK);
        let rows = body["contracts"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        for r in rows {
            assert!(
                r["category"].is_string(),
                "category must always be present, got {r:?}"
            );
        }
        let by_id: std::collections::HashMap<&str, &serde_json::Value> = rows
            .iter()
            .map(|r| (r["canonical_id"].as_str().unwrap(), r))
            .collect();
        assert_eq!(by_id["apprehend_zane_esteban"]["category"], "bounty_hunter");
        assert_eq!(by_id["strip_the_caterpillar"]["category"], "salvage");
        // Raw values are retained untouched alongside the derived one.
        assert_eq!(by_id["strip_the_caterpillar"]["contract_type"], "SALVAGE");
        // `Lawful` folds to `legal`, raw kept.
        assert_eq!(
            by_id["strip_the_caterpillar"]["legal_status_normalised"],
            "legal"
        );
        assert_eq!(by_id["strip_the_caterpillar"]["legal_status"], "Lawful");
    }

    #[tokio::test]
    async fn detail_category_matches_the_list_row() {
        let (app, _) = app();
        post_ingest(&app, Some(TOKEN), &salvage_payload()).await;

        let (_, list) = get_json(&app, "/api/contracts").await;
        let list_cat = list["contracts"][0]["category"]
            .as_str()
            .unwrap()
            .to_string();

        let (status, detail) = get_json(&app, "/api/contracts/strip_the_caterpillar").await;
        assert_eq!(status, StatusCode::OK);
        // The whole point of classifying from promoted columns only: the two
        // read paths cannot disagree.
        assert_eq!(detail["category"], list_cat.as_str());
        assert_eq!(detail["legal_status_normalised"], "legal");
    }

    #[tokio::test]
    async fn steps_carry_risk_normalised_and_keep_the_raw_value() {
        let (app, _) = app();
        let mut payload = example_payload();
        // Shouty risk on the middle step; the extractor really does this.
        payload["internal"]["extraction"]["steps"][1]["risk"] = json!("Medium");
        post_ingest(&app, Some(TOKEN), &payload).await;

        let (status, detail) = get_json(&app, "/api/contracts/apprehend_zane_esteban").await;
        assert_eq!(status, StatusCode::OK);
        let steps = detail["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 3);
        // Step 1 has no risk at all.
        assert!(steps[0]["risk"].is_null());
        assert!(steps[0]["risk_normalised"].is_null());
        // Step 2: raw preserved verbatim, normalised alongside.
        assert_eq!(steps[1]["risk"], "Medium");
        assert_eq!(steps[1]["risk_normalised"], "medium");
        assert_eq!(steps[2]["risk_normalised"], "high");
        // The flattened wrapper must not change the step's wire shape.
        assert_eq!(steps[1]["step_type"], "navigate");
        assert_eq!(steps[1]["location"], "Glaciem Ring");
    }

    #[tokio::test]
    async fn category_filter_selects_only_matching_contracts() {
        let (app, _) = app();
        post_ingest(&app, Some(TOKEN), &example_payload()).await;
        post_ingest(&app, Some(TOKEN), &salvage_payload()).await;

        let (status, body) = get_json(&app, "/api/contracts?category=salvage").await;
        assert_eq!(status, StatusCode::OK);
        let rows = body["contracts"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["canonical_id"], "strip_the_caterpillar");
        assert!(body["next_offset"].is_null());

        // The search route takes the same filter.
        let (_, searched) = get_json(&app, "/api/contracts/search?category=bounty_hunter").await;
        let rows = searched["contracts"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["canonical_id"], "apprehend_zane_esteban");

        // A category nobody matches is an empty page, not an error.
        let (s, empty) = get_json(&app, "/api/contracts?category=mining").await;
        assert_eq!(s, StatusCode::OK);
        assert!(empty["contracts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn category_filter_paginates_over_the_filtered_set() {
        let (app, _) = app();
        // Three salvage contracts; page size 2 must yield 2 then 1.
        for i in 0..3 {
            let mut v = salvage_payload();
            v["canonical_id"] = json!(format!("salv_{i}"));
            post_ingest(&app, Some(TOKEN), &v).await;
        }
        post_ingest(&app, Some(TOKEN), &example_payload()).await; // a non-match

        let (_, p1) = get_json(&app, "/api/contracts?category=salvage&limit=2").await;
        assert_eq!(p1["contracts"].as_array().unwrap().len(), 2);
        assert_eq!(p1["next_offset"], 2);

        let (_, p2) = get_json(&app, "/api/contracts?category=salvage&limit=2&offset=2").await;
        assert_eq!(p2["contracts"].as_array().unwrap().len(), 1);
        assert!(
            p2["next_offset"].is_null(),
            "last page must not advertise another"
        );
    }

    #[tokio::test]
    async fn unknown_category_is_a_400_not_an_empty_page() {
        let (app, _) = app();
        post_ingest(&app, Some(TOKEN), &example_payload()).await;

        let (status, body) = get_json(&app, "/api/contracts?category=hauling_typo").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_category");

        // Blank is treated as absent, not as invalid.
        let (ok, _) = get_json(&app, "/api/contracts?category=").await;
        assert_eq!(ok, StatusCode::OK);
    }
}
