//! Contract ingest storage + wire models.
//!
//! Receiving side of the sp-ingest → StarStats push. sp-ingest
//! (the StarPlatform capture tool) POSTs a [`PublishBundleReq`] to
//! `/api/contracts/ingest`; we UPSERT one row per `canonical_id` and
//! serve the structured projection back on the public read endpoints.
//!
//! The wire models below are an owned Rust mirror of sp-ingest's
//! Pydantic `PublishBundle` / `AdminReviewPacket` / `Extraction` /
//! `ExtractedContract` / `ExtractedStep` / `Reward` / `Fee` /
//! `Timeframe` / `Attribute` / `UpdateSuggestion`. They are
//! deliberately lenient — every scalar is optional and the
//! open-vocabulary strings (`contract_type`, `step_type`,
//! `legal_status`, `risk`) are plain `String`, never hard enums — so a
//! sender that adds a field or ships an unknown enum value round-trips
//! instead of 4xx-ing. `#[serde(default)]` on each struct backfills
//! missing keys.
//!
//! Storage follows the project's Trait + Postgres impl + Memory impl
//! pattern (see `repo.rs`, `discover_routes.rs`): [`ContractStore`] is
//! the abstraction, [`PostgresContractStore`] is production, and
//! `test_support::MemoryContractStore` backs the route + store tests.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;

// ---------------------------------------------------------------------
// Wire models — owned mirror of sp-ingest's PublishBundle tree.
// ---------------------------------------------------------------------

/// A non-aUEC award, e.g. `14 MG Scrip`. Mirrors sp-ingest
/// `AdditionalReward`.
///
/// The sender reads these out of the DETAILS prose ("Completing this
/// contract awards 14 MG Scrip") rather than the Reward header, so they
/// are often the only record that a contract pays anything besides
/// aUEC. `unit` is deliberately free text — the in-game currencies are
/// an open set ("MG Scrip", "Council Scrip", ...), and an enum here
/// would silently drop the next one CIG adds.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct AdditionalRewardReq {
    /// Stated count. `None` when the prose names an award without a
    /// number — still a real award, so never treat it as absent.
    pub amount: Option<i64>,
    pub unit: Option<String>,
    /// The verbatim sentence the award was read from, for provenance.
    pub note: Option<String>,
}

/// Reward block. Mirrors sp-ingest `Reward`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct RewardReq {
    pub amount: Option<i64>,
    /// Defaults to `aUEC` on the sender; kept optional here so an
    /// omitted currency doesn't fail parsing.
    pub currency: Option<String>,
    pub bonus_amount: Option<i64>,
    /// Non-aUEC awards. Must be modelled here rather than left to ride
    /// along in the JSON: `record` is built by re-serializing this
    /// tree, so an unmodelled field is dropped on ingest while the push
    /// still answers 200.
    pub additional: Vec<AdditionalRewardReq>,
}

/// A fee attached to a contract. Mirrors sp-ingest `Fee`. `type` is an
/// open string on the wire ("deposit" | "entry" | "upfront" | ...).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct FeeReq {
    #[serde(rename = "type")]
    pub fee_type: Option<String>,
    pub amount: Option<i64>,
    pub currency: Option<String>,
    pub refundable: Option<bool>,
}

/// Time constraints. Mirrors sp-ingest `Timeframe`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct TimeframeReq {
    pub has_time_limit: Option<bool>,
    pub deadline_text: Option<String>,
    pub duration_minutes: Option<i64>,
    pub confidence: Option<f64>,
}

/// A labelled DETAILS key/value line from the contract UI. Mirrors
/// sp-ingest `Attribute` (e.g. "LAST KNOWN LOCATION" → "Glaciem Ring").
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct AttributeReq {
    pub label: Option<String>,
    pub value: Option<String>,
    pub confidence: Option<f64>,
}

/// The extracted contract. Mirrors sp-ingest `ExtractedContract`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ExtractedContractReq {
    pub canonical_name: Option<String>,
    pub display_name: Option<String>,
    pub contract_type: Option<String>,
    pub subcategory: Option<String>,
    pub gameplay_loop: Option<String>,
    pub issuer: Option<String>,
    pub faction: Option<String>,
    pub legal_status: Option<String>,
    pub required_reputation: Option<String>,
    pub reputation_rank: Option<String>,
    pub reward: RewardReq,
    pub fees: Vec<FeeReq>,
    pub failure_penalty: Option<String>,
    pub cargo_loss_penalty: Option<String>,
    pub rep_loss_warning: Option<String>,
    pub net_estimated_profit: Option<i64>,
    pub timeframe: TimeframeReq,
    pub attributes: Vec<AttributeReq>,
    pub primary_objectives: Vec<String>,
    /// What a player must have or do before accepting — "Tractor beam
    /// required", "Must supply own quantum fuel".
    ///
    /// Authored facts, not copied sentences: these are frequently stated
    /// only in the contract description, which is mission prose we must
    /// not republish. The sender writes the fact in its own words.
    ///
    /// Must be modelled here or serde drops it: `record` is built by
    /// re-serializing this tree, so an unmodelled field vanishes on
    /// ingest while the push still answers 200.
    pub requirements: Vec<String>,
    pub patch_version: Option<String>,
    pub confidence_score: Option<f64>,
}

/// A canonical Star Citizen entity a step refers to. Mirrors sp-ingest
/// `StepEntity`.
///
/// Distinct from `location` / `required_item`, which carry the
/// contract's descriptive phrasing ("Caterpillar wreck site near
/// microTech"). Those read well but name nothing the knowledge base can
/// match; this carries the proper names that phrasing refers to, so one
/// phrase can yield several entities of different kinds.
///
/// Must be modelled here rather than left to ride along in the JSON:
/// `record` is built by re-serializing this tree, so an unmodelled
/// field is dropped on ingest while the push still answers 200.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct StepEntityReq {
    /// `location` | `item` | `vehicle` | `weapon`. Open string.
    pub kind: Option<String>,
    /// Canonical in-game name, no descriptive qualifiers.
    pub name: Option<String>,
}

/// One execution step. Mirrors sp-ingest `ExtractedStep`. `step_type`
/// and `risk` are open strings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ExtractedStepReq {
    pub order: Option<i64>,
    pub step_type: Option<String>,
    pub summary: Option<String>,
    pub guidance: bool,
    pub tip: Option<String>,
    pub location: Option<String>,
    pub required_item: Option<String>,
    /// Canonical entity names this step refers to; see `StepEntityReq`.
    pub entities: Vec<StepEntityReq>,
    pub required_cargo: Option<String>,
    pub required_vehicle: Option<String>,
    pub required_equipment: Option<String>,
    pub risk: Option<String>,
    pub optional: bool,
    pub failure_condition: Option<String>,
    pub confidence: Option<f64>,
}

/// LLM extraction result. Mirrors sp-ingest `Extraction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ExtractionReq {
    pub contract: ExtractedContractReq,
    pub steps: Vec<ExtractedStepReq>,
}

/// Sender's update suggestion. Mirrors sp-ingest `UpdateSuggestion`.
/// `changed_fields` / `recommendations` are kept as raw JSON — they are
/// advisory-only, stored verbatim in `record`, and never read by the
/// server, so there's no value in fully typing them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct UpdateSuggestionReq {
    pub matched_existing_contract_id: Option<String>,
    pub match_confidence: Option<f64>,
    /// One of `create_new_contract | update_existing_contract |
    /// duplicate | patch_change | outdated_removed |
    /// partial_capture_review | low_confidence`. Advisory intent only —
    /// the server always upserts by `canonical_id` regardless.
    pub suggested_action: Option<String>,
    #[schema(value_type = Vec<Object>)]
    pub changed_fields: Vec<Value>,
    #[schema(value_type = Vec<Object>)]
    pub recommendations: Vec<Value>,
    pub editor_note: Option<String>,
}

/// The internal review packet — the whole record sp-ingest ships.
/// Carries the verbatim `raw_text`; the server stores it but NEVER
/// surfaces it on the public read DTOs. Mirrors sp-ingest
/// `AdminReviewPacket`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct AdminReviewPacketReq {
    pub source_capture_id: Option<String>,
    /// Verbatim OCR / pasted UI text. Internal-only — never returned by
    /// the public read endpoints.
    pub raw_text: Option<String>,
    pub extraction: ExtractionReq,
    pub suggestion: UpdateSuggestionReq,
    pub confidence_score: Option<f64>,
    pub flags: Vec<String>,
}

/// Root ingest payload. Mirrors sp-ingest `PublishBundle`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct PublishBundleReq {
    /// Payload schema version. sp-ingest ships `"1"`.
    pub schema_version: String,
    /// Upsert key — the sender's stable canonical id for this contract.
    /// Required; an empty value is rejected at the route layer.
    pub canonical_id: String,
    pub capture_id: Option<String>,
    pub internal: AdminReviewPacketReq,
}

impl Default for PublishBundleReq {
    fn default() -> Self {
        Self {
            schema_version: "1".to_string(),
            canonical_id: String::new(),
            capture_id: None,
            internal: AdminReviewPacketReq::default(),
        }
    }
}

// ---------------------------------------------------------------------
// Storage row types.
// ---------------------------------------------------------------------

/// Outcome of an upsert — lets the ingest handler report whether the
/// push created a new contract or folded into an existing canonical id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted,
    Updated,
}

impl UpsertOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            UpsertOutcome::Inserted => "inserted",
            UpsertOutcome::Updated => "updated",
        }
    }
}

/// A validated contract ready to persist. Promoted columns are derived
/// from the extraction; `record` is the full internal packet stored
/// verbatim (source of truth). Built via [`NewContract::from_bundle`].
#[derive(Debug, Clone)]
pub struct NewContract {
    pub canonical_id: String,
    pub schema_version: String,
    pub capture_id: Option<String>,
    pub display_name: Option<String>,
    pub contract_type: Option<String>,
    pub subcategory: Option<String>,
    pub gameplay_loop: Option<String>,
    pub issuer: Option<String>,
    pub faction: Option<String>,
    pub legal_status: Option<String>,
    pub reward_amount: Option<i64>,
    pub reward_currency: Option<String>,
    pub patch_version: Option<String>,
    pub confidence_score: Option<f64>,
    pub suggested_action: Option<String>,
    pub search_blob: String,
    /// Promoted from the first step whose `location` is non-empty
    /// (`extraction.steps`, in order). Must match migration 0061's
    /// backfill byte-for-byte — see `promote_step_fields`.
    pub first_step_location: Option<String>,
    /// Promoted from the first step whose quantity-stripped
    /// `required_item` is non-empty. Singular by design: real data
    /// repeats the same item with inconsistent formatting (quantity
    /// prefix/suffix, singular/plural — e.g. "15 Amioshi Plague" vs
    /// "Amioshi Plague", "Cave Kopion Horn" vs "Cave Kopion Horns"), so
    /// aggregating into a list produced near-duplicate garbage on a
    /// first draft (rejected — see migration 0061's comment).
    pub required_item: Option<String>,
    /// Count of `extraction.steps`. `Some(0)` for an empty array, never
    /// `None` — a contract with no steps still has a definite count.
    pub step_count: Option<i32>,
    /// KB entities this contract references (step locations, step items
    /// from *every* step, and `…LOCATION` attribute values), before
    /// resolution against `reference_registry`.
    ///
    /// Carried on `NewContract` rather than written by a separate call
    /// so a contract cannot be stored without its entity rows — the
    /// store writes both or neither. Resolution to a KB slug is the
    /// store's job, because it needs the registry.
    pub entities: Vec<crate::contract_entities::EntityRef>,
    /// Full internal packet (`PublishBundle.internal`) as JSON.
    pub record: Value,
}

/// Why a bundle couldn't be turned into a [`NewContract`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestValidationError {
    /// `canonical_id` was missing or blank — nothing to upsert on.
    MissingCanonicalId,
}

impl NewContract {
    /// Validate + promote a wire bundle into a persistable row. Fails
    /// only when the upsert key is absent; everything else is optional
    /// and flows through as-is.
    pub fn from_bundle(bundle: &PublishBundleReq) -> Result<Self, IngestValidationError> {
        let canonical_id = bundle.canonical_id.trim();
        if canonical_id.is_empty() {
            return Err(IngestValidationError::MissingCanonicalId);
        }
        let packet = &bundle.internal;
        let contract = &packet.extraction.contract;
        let record = serde_json::to_value(packet).unwrap_or(Value::Null);
        let (first_step_location, required_item, step_count) =
            promote_step_fields(&packet.extraction);
        let entities = crate::contract_entities::extract_entities(&packet.extraction);
        Ok(Self {
            canonical_id: canonical_id.to_string(),
            schema_version: bundle.schema_version.clone(),
            capture_id: bundle.capture_id.clone(),
            display_name: contract.display_name.clone(),
            contract_type: contract.contract_type.clone(),
            subcategory: contract.subcategory.clone(),
            gameplay_loop: contract.gameplay_loop.clone(),
            issuer: contract.issuer.clone(),
            faction: contract.faction.clone(),
            legal_status: contract.legal_status.clone(),
            reward_amount: contract.reward.amount,
            reward_currency: contract.reward.currency.clone(),
            patch_version: contract.patch_version.clone(),
            confidence_score: contract.confidence_score.or(packet.confidence_score),
            suggested_action: packet.suggestion.suggested_action.clone(),
            search_blob: build_search_blob(&packet.extraction),
            first_step_location,
            required_item,
            step_count,
            entities,
            record,
        })
    }
}

// ---------------------------------------------------------------------
// Step-derived disambiguation columns (migration 0061).
//
// `PostgresContractStore` promotes these at ingest; migration 0061
// backfills the same columns for rows that predate this. The two MUST
// agree byte-for-byte, or a contract's `required_item` /
// `first_step_location` silently depends on when it happened to be
// ingested. See the migration file for the full rationale (why a
// first draft that aggregated `required_item` into a joined list was
// rejected, and why the quantity-strip filters on the STRIPPED value).
// ---------------------------------------------------------------------

/// Matches a leading quantity: digits, an optional `x`/`X` unit marker,
/// surrounding whitespace. E.g. `"15 "`, `"15x "`.
static LEADING_QTY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*\d+\s*[xX]?\s*").expect("LEADING_QTY_RE compiles"));

/// Matches a trailing quantity: an `x`/`X` marker then digits, with
/// surrounding whitespace. E.g. `" x25"`. Unlike the leading pattern,
/// the `x`/`X` is mandatory here — a bare trailing number (no `x`)
/// is left alone, since it may just be part of the name (`"F7C Hornet
/// Mk I"` has no trailing digits at all, but this asymmetry is
/// intentional and mirrors the migration exactly).
static TRAILING_QTY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\s*[xX]\s*\d+\s*$").expect("TRAILING_QTY_RE compiles"));

/// Strip a leading and/or trailing quantity off a step's
/// `required_item`, then trim. Must match migration 0061's SQL
/// exactly: `btrim(regexp_replace(regexp_replace(v, LEADING, ''),
/// TRAILING, ''))`. Neither Postgres's `regexp_replace` (no `g` flag)
/// nor this call replace more than the first match, which is fine
/// since both patterns are anchored (`^`/`$`) and can only match once.
pub(crate) fn strip_item_qty(s: &str) -> String {
    let s = LEADING_QTY_RE.replace(s, "");
    let s = TRAILING_QTY_RE.replace(&s, "");
    s.trim().to_string()
}

/// Promote the three step-derived disambiguation columns from
/// `extraction.steps`, in step order.
fn promote_step_fields(
    extraction: &ExtractionReq,
) -> (Option<String>, Option<String>, Option<i32>) {
    // Raw non-empty check — NOT trimmed. Mirrors the migration's
    // `s ->> 'location' IS NOT NULL AND s ->> 'location' <> ''` filter,
    // which does not trim either, and the migration's SELECT returns
    // the location verbatim (also untrimmed).
    let first_step_location = extraction
        .steps
        .iter()
        .filter_map(|s| s.location.as_ref())
        .find(|l| !l.is_empty())
        .cloned();

    // Filter on the STRIPPED value, not the raw one: a step whose
    // `required_item` is a bare quantity ("25") passes a raw non-empty
    // test, strips to "", and would otherwise win over the real item
    // on a later step — rendering as a blank segment in the catalog
    // row. Verified against steps ["25", "Prota"]: '' before this
    // ordering, 'Prota' after.
    let required_item = extraction
        .steps
        .iter()
        .map(|s| strip_item_qty(s.required_item.as_deref().unwrap_or("")))
        .find(|stripped| !stripped.is_empty());

    (
        first_step_location,
        required_item,
        Some(extraction.steps.len() as i32),
    )
}

/// Build the lowercased, space-joined search bag. Folds in every field
/// a `?q=` / `?location=` search should hit: contract identity fields,
/// attribute values (which carry "LAST KNOWN LOCATION" etc.), step
/// locations, and the primary objectives.
fn build_search_blob(extraction: &ExtractionReq) -> String {
    let c = &extraction.contract;
    let mut parts: Vec<String> = Vec::new();
    let mut push = |s: &Option<String>| {
        if let Some(v) = s {
            if !v.trim().is_empty() {
                parts.push(v.clone());
            }
        }
    };
    push(&c.display_name);
    push(&c.canonical_name);
    push(&c.contract_type);
    push(&c.subcategory);
    push(&c.gameplay_loop);
    push(&c.issuer);
    push(&c.faction);
    for attr in &c.attributes {
        if let Some(v) = &attr.value {
            if !v.trim().is_empty() {
                parts.push(v.clone());
            }
        }
    }
    for obj in &c.primary_objectives {
        if !obj.trim().is_empty() {
            parts.push(obj.clone());
        }
    }
    // Requirements are exactly what a player searches for when deciding
    // whether they can take a contract ("tractor beam").
    for req in &c.requirements {
        if !req.trim().is_empty() {
            parts.push(req.clone());
        }
    }
    for step in &extraction.steps {
        if let Some(loc) = &step.location {
            if !loc.trim().is_empty() {
                parts.push(loc.clone());
            }
        }
    }
    parts.join(" ").to_lowercase()
}

/// A stored contract, full — includes the `record` JSONB so the detail
/// handler can project the structured extraction. `raw_text` inside
/// `record` is never surfaced.
#[derive(Debug, Clone)]
pub struct StoredContract {
    pub canonical_id: String,
    pub schema_version: String,
    pub suggested_action: Option<String>,
    pub record: Value,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A list/search row — promoted columns only (no JSONB read).
///
/// Derives `sqlx::FromRow` (matched by column name, not position) so
/// `PostgresContractStore::list` can select straight into it — a plain
/// positional tuple tops out at sqlx's 16-field `FromRow` impl, and
/// this row has 17 columns.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContractSummaryRow {
    pub canonical_id: String,
    pub display_name: Option<String>,
    pub contract_type: Option<String>,
    pub subcategory: Option<String>,
    pub gameplay_loop: Option<String>,
    pub issuer: Option<String>,
    pub faction: Option<String>,
    pub legal_status: Option<String>,
    pub reward_amount: Option<i64>,
    pub reward_currency: Option<String>,
    pub confidence_score: Option<f64>,
    pub patch_version: Option<String>,
    pub first_step_location: Option<String>,
    pub required_item: Option<String>,
    pub step_count: Option<i32>,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Filter + pagination spec for [`ContractStore::list`]. All predicates
/// are optional and compose with AND. `query` is the free-text term
/// (used by both `?q=` and `?location=`), matched case-insensitively
/// against the search blob.
#[derive(Debug, Clone)]
pub struct ContractListFilter {
    pub contract_type: Option<String>,
    pub issuer: Option<String>,
    pub legal_status: Option<String>,
    /// Measured as the joint-strongest discriminator between contracts
    /// sharing a display_name (varies in 56% of duplicate groups).
    pub faction: Option<String>,
    pub gameplay_loop: Option<String>,
    pub query: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

impl Default for ContractListFilter {
    fn default() -> Self {
        Self {
            contract_type: None,
            issuer: None,
            legal_status: None,
            faction: None,
            gameplay_loop: None,
            query: None,
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContractStoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait]
pub trait ContractStore: Send + Sync + 'static {
    /// Insert a new contract or fold into the existing `canonical_id`.
    /// Idempotent: a repeat push with the same id updates in place and
    /// preserves `first_seen_at`.
    async fn upsert(&self, contract: NewContract) -> Result<UpsertOutcome, ContractStoreError>;

    /// Fetch one contract by canonical id, or `None`.
    async fn get(&self, canonical_id: &str) -> Result<Option<StoredContract>, ContractStoreError>;

    /// List/search contracts, newest-updated first.
    async fn list(
        &self,
        filter: ContractListFilter,
    ) -> Result<Vec<ContractSummaryRow>, ContractStoreError>;

    /// Delete one contract by canonical id. Returns whether a row was
    /// removed (`false` = no such id).
    async fn delete(&self, canonical_id: &str) -> Result<bool, ContractStoreError>;

    /// Delete every contract. Returns the number of rows removed. Backs
    /// the token-gated bulk reset (`DELETE /api/contracts`).
    async fn delete_all(&self) -> Result<u64, ContractStoreError>;

    /// Contracts referencing a resolved KB entity, for the KB entity
    /// page. Exact join on `(ref_category, ref_slug)` — never a
    /// substring match against `search_blob`.
    async fn list_by_entity(
        &self,
        category: &str,
        slug: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ContractSummaryRow>, ContractStoreError>;

    /// How many catalogue rows carry each given name, and the id when
    /// exactly one does. One entry per requested name, including names
    /// with no match (`match_count == 0`) — the caller needs an answer
    /// for every name it asked about.
    async fn resolve_names(
        &self,
        names: &[String],
    ) -> Result<Vec<crate::contract_entities::NameResolution>, ContractStoreError>;

    /// The KB entities this contract references, for the detail view.
    async fn entities_for(
        &self,
        canonical_id: &str,
    ) -> Result<Vec<crate::contract_entities::EntityRow>, ContractStoreError>;
}

/// Collapse a `contract_type` to a comparison key: lowercase, with `_`,
/// `-` and whitespace runs folded to a single space.
///
/// `contract_type` is an open string (no enum), and sp-ingest has emitted
/// both `"Bounty Hunting"` and `"bounty_hunting"`. The list query already
/// compares `LOWER(a) = LOWER(b)`, which neutralises case but NOT the
/// separator split — so `?type=bounty_hunting` returned 2 of 32 rows.
/// This key closes that gap without rewriting stored values.
pub fn normalize_type_key(s: &str) -> String {
    let swapped: String = s
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    swapped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// ---------------------------------------------------------------------
// Postgres impl.
// ---------------------------------------------------------------------

pub struct PostgresContractStore {
    pool: PgPool,
}

impl PostgresContractStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Rewrite this contract's entity rows, resolving each against
    /// `reference_registry`.
    ///
    /// Delete-then-insert in one transaction: re-publishing a contract
    /// whose steps changed must not leave rows for entities it no
    /// longer references.
    async fn replace_entities(
        &self,
        canonical_id: &str,
        entities: &[crate::contract_entities::EntityRef],
    ) -> Result<(), ContractStoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM contract_entities WHERE canonical_id = $1")
            .bind(canonical_id)
            .execute(&mut *tx)
            .await?;

        for e in entities {
            // Resolve ONLY when the name identifies exactly one registry
            // row that has a slug. Several matches, no match, or a NULL
            // slug all store NULL — the row still carries `raw_value`
            // and the surface renders plain text. Never a guess.
            sqlx::query(
                r#"
                WITH m AS (
                    SELECT category, slug
                    FROM reference_registry
                    WHERE slug IS NOT NULL
                      AND (lower(display_name) = $4 OR lower(class_name) = $4)
                ), one AS (
                    SELECT category, slug FROM m WHERE (SELECT COUNT(*) FROM m) = 1
                )
                INSERT INTO contract_entities
                    (canonical_id, kind, raw_value, value_norm,
                     ref_slug, ref_category, ref_match_count)
                VALUES ($1, $2, $3, $4,
                        (SELECT slug FROM one), (SELECT category FROM one),
                        (SELECT COUNT(*) FROM m))
                ON CONFLICT (canonical_id, kind, value_norm) DO NOTHING
                "#,
            )
            .bind(canonical_id)
            .bind(&e.kind)
            .bind(&e.raw_value)
            .bind(&e.value_norm)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[async_trait]
impl ContractStore for PostgresContractStore {
    async fn upsert(&self, c: NewContract) -> Result<UpsertOutcome, ContractStoreError> {
        // `(xmax = 0)` is TRUE for a freshly inserted row and FALSE when
        // ON CONFLICT took the UPDATE branch — so one round-trip tells
        // us Inserted vs Updated. `first_seen_at` is only written on
        // insert (it is absent from the UPDATE SET list), so a repeat
        // push preserves the original first-seen timestamp while
        // bumping `updated_at`.
        let inserted: bool = sqlx::query_scalar(
            r#"
            INSERT INTO contracts (
                canonical_id, schema_version, capture_id, display_name,
                contract_type, subcategory, gameplay_loop, issuer, faction,
                legal_status, reward_amount, reward_currency, patch_version,
                confidence_score, suggested_action, search_blob, record,
                first_step_location, required_item, step_count,
                first_seen_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                $14, $15, $16, $17, $18, $19, $20, NOW(), NOW()
            )
            ON CONFLICT (canonical_id) DO UPDATE SET
                schema_version       = EXCLUDED.schema_version,
                capture_id           = EXCLUDED.capture_id,
                display_name         = EXCLUDED.display_name,
                contract_type        = EXCLUDED.contract_type,
                subcategory          = EXCLUDED.subcategory,
                gameplay_loop        = EXCLUDED.gameplay_loop,
                issuer               = EXCLUDED.issuer,
                faction              = EXCLUDED.faction,
                legal_status         = EXCLUDED.legal_status,
                reward_amount        = EXCLUDED.reward_amount,
                reward_currency      = EXCLUDED.reward_currency,
                patch_version        = EXCLUDED.patch_version,
                confidence_score     = EXCLUDED.confidence_score,
                suggested_action     = EXCLUDED.suggested_action,
                search_blob          = EXCLUDED.search_blob,
                record               = EXCLUDED.record,
                first_step_location  = EXCLUDED.first_step_location,
                required_item        = EXCLUDED.required_item,
                step_count           = EXCLUDED.step_count,
                updated_at           = NOW()
            RETURNING (xmax = 0) AS inserted
            "#,
        )
        .bind(&c.canonical_id)
        .bind(&c.schema_version)
        .bind(&c.capture_id)
        .bind(&c.display_name)
        .bind(&c.contract_type)
        .bind(&c.subcategory)
        .bind(&c.gameplay_loop)
        .bind(&c.issuer)
        .bind(&c.faction)
        .bind(&c.legal_status)
        .bind(c.reward_amount)
        .bind(&c.reward_currency)
        .bind(&c.patch_version)
        .bind(c.confidence_score)
        .bind(&c.suggested_action)
        .bind(&c.search_blob)
        .bind(&c.record)
        .bind(&c.first_step_location)
        .bind(&c.required_item)
        .bind(c.step_count)
        .fetch_one(&self.pool)
        .await?;

        self.replace_entities(&c.canonical_id, &c.entities).await?;

        Ok(if inserted {
            UpsertOutcome::Inserted
        } else {
            UpsertOutcome::Updated
        })
    }

    async fn get(&self, canonical_id: &str) -> Result<Option<StoredContract>, ContractStoreError> {
        let row: Option<(
            String,
            String,
            Option<String>,
            Value,
            DateTime<Utc>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT canonical_id, schema_version, suggested_action, record,
                        first_seen_at, updated_at
                 FROM contracts
                 WHERE canonical_id = $1",
        )
        .bind(canonical_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(
                canonical_id,
                schema_version,
                suggested_action,
                record,
                first_seen_at,
                updated_at,
            )| {
                StoredContract {
                    canonical_id,
                    schema_version,
                    suggested_action,
                    record,
                    first_seen_at,
                    updated_at,
                }
            },
        ))
    }

    async fn list(
        &self,
        filter: ContractListFilter,
    ) -> Result<Vec<ContractSummaryRow>, ContractStoreError> {
        // Every value binds as a parameter via QueryBuilder — no string
        // interpolation (SQL-injection safe, prepared-statement cache
        // friendly). Same idiom as `repo::PostgresStore::list_filtered`.
        let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
            "SELECT canonical_id, display_name, contract_type, subcategory,
                    gameplay_loop, issuer, faction, legal_status,
                    reward_amount, reward_currency, confidence_score,
                    patch_version, first_step_location, required_item,
                    step_count, first_seen_at, updated_at
             FROM contracts
             WHERE 1 = 1",
        );
        if let Some(t) = &filter.contract_type {
            // Fold separators on BOTH sides: the stored value may be
            // "bounty_hunting" while the query says "Bounty Hunting".
            // REPLACE/REGEXP_REPLACE are applied to the column rather
            // than a normalized column so no migration/backfill is
            // required for this filter. BTRIM is required, not
            // decorative: REGEXP_REPLACE(..., '\s+', ' ', 'g') collapses
            // *internal* whitespace runs but does not touch the edges,
            // so a stored "_bounty_hunting" becomes " bounty hunting"
            // (leading space intact) — without BTRIM that would never
            // match `normalize_type_key`'s output, which trims edges
            // via `split_whitespace().join(" ")`. The two sides must
            // stay byte-for-byte equivalent.
            //
            // Cost: wrapping the column in an expression means this
            // predicate can't use `contracts_type_idx` (0046_contracts.sql,
            // a plain btree on `contract_type`) — type-filtered lists
            // seq-scan. Accepted at current scale (266 rows); if the
            // catalog grows, add an expression index over this
            // normalized form (see `contracts_issuer_lower_idx` for the
            // precedent) rather than reverting this fix.
            qb.push(
                " AND BTRIM(LOWER(REGEXP_REPLACE(REPLACE(REPLACE(contract_type,'_',' '),'-',' '), '\\s+', ' ', 'g'))) = ",
            );
            qb.push_bind(normalize_type_key(t));
        }
        if let Some(i) = &filter.issuer {
            qb.push(" AND LOWER(issuer) = LOWER(");
            qb.push_bind(i.clone());
            qb.push(")");
        }
        if let Some(l) = &filter.legal_status {
            qb.push(" AND LOWER(legal_status) = LOWER(");
            qb.push_bind(l.clone());
            qb.push(")");
        }
        if let Some(f) = &filter.faction {
            qb.push(" AND LOWER(faction) = LOWER(");
            qb.push_bind(f.clone());
            qb.push(")");
        }
        if let Some(g) = &filter.gameplay_loop {
            qb.push(" AND LOWER(gameplay_loop) = LOWER(");
            qb.push_bind(g.clone());
            qb.push(")");
        }
        if let Some(q) = &filter.query {
            let term = q.trim();
            if !term.is_empty() {
                // search_blob is stored already-lowercased; match a
                // lowercased substring so ILIKE isn't needed.
                qb.push(" AND search_blob LIKE ");
                qb.push_bind(format!("%{}%", term.to_lowercase()));
            }
        }
        qb.push(" ORDER BY updated_at DESC, canonical_id ASC LIMIT ");
        qb.push_bind(filter.limit.max(0));
        qb.push(" OFFSET ");
        qb.push_bind(filter.offset.max(0));

        // `ContractSummaryRow` derives `FromRow` and selects straight
        // into it (matched by column name) — a positional tuple tops
        // out at sqlx's 16-field `FromRow` impl, and this row has 17
        // columns.
        let rows: Vec<ContractSummaryRow> = qb.build_query_as().fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn delete(&self, canonical_id: &str) -> Result<bool, ContractStoreError> {
        let result = sqlx::query("DELETE FROM contracts WHERE canonical_id = $1")
            .bind(canonical_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_all(&self) -> Result<u64, ContractStoreError> {
        let result = sqlx::query("DELETE FROM contracts")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    async fn list_by_entity(
        &self,
        category: &str,
        slug: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ContractSummaryRow>, ContractStoreError> {
        let rows = sqlx::query_as::<_, ContractSummaryRow>(
            r#"
            SELECT c.canonical_id, c.schema_version, c.display_name, c.contract_type,
                   c.subcategory, c.gameplay_loop, c.issuer, c.faction, c.legal_status,
                   c.reward_amount, c.reward_currency, c.confidence_score,
                   c.patch_version, c.first_step_location, c.required_item,
                   c.step_count, c.first_seen_at, c.updated_at
            FROM contracts c
            JOIN contract_entities ce ON ce.canonical_id = c.canonical_id
            WHERE ce.ref_category = $1 AND ce.ref_slug = $2
            ORDER BY c.display_name NULLS LAST, c.canonical_id
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(category)
        .bind(slug)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn resolve_names(
        &self,
        names: &[String],
    ) -> Result<Vec<crate::contract_entities::NameResolution>, ContractStoreError> {
        use crate::contract_entities::{normalize_value, NameResolution};
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: Vec<String> = names.iter().map(|n| normalize_value(n)).collect();

        // Group by the normalized display_name. `MIN(canonical_id)` is
        // only read when the group holds exactly one row, so it never
        // names a winner among same-named contracts.
        let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(
            r#"
            SELECT lower(btrim(regexp_replace(display_name, '\s+', ' ', 'g'))) AS name_norm,
                   COUNT(*)          AS match_count,
                   MIN(canonical_id) AS only_id
            FROM contracts
            WHERE display_name IS NOT NULL
              AND lower(btrim(regexp_replace(display_name, '\s+', ' ', 'g'))) = ANY($1)
            GROUP BY name_norm
            "#,
        )
        .bind(&wanted)
        .fetch_all(&self.pool)
        .await?;

        Ok(wanted
            .into_iter()
            .map(|name| {
                let hit = rows.iter().find(|(n, _, _)| *n == name);
                match hit {
                    Some((_, count, id)) => NameResolution {
                        name,
                        match_count: *count,
                        canonical_id: if *count == 1 { id.clone() } else { None },
                    },
                    None => NameResolution {
                        name,
                        match_count: 0,
                        canonical_id: None,
                    },
                }
            })
            .collect())
    }

    async fn entities_for(
        &self,
        canonical_id: &str,
    ) -> Result<Vec<crate::contract_entities::EntityRow>, ContractStoreError> {
        let rows: Vec<(String, String, Option<String>, Option<String>, i32)> = sqlx::query_as(
            "SELECT kind, raw_value, ref_slug, ref_category, ref_match_count
             FROM contract_entities WHERE canonical_id = $1
             ORDER BY kind, value_norm",
        )
        .bind(canonical_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(kind, raw_value, ref_slug, ref_category, ref_match_count)| {
                    crate::contract_entities::EntityRow {
                        kind,
                        raw_value,
                        ref_slug,
                        ref_category,
                        ref_match_count: ref_match_count as i64,
                    }
                },
            )
            .collect())
    }
}

// ---------------------------------------------------------------------
// Memory impl + store tests.
// ---------------------------------------------------------------------

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory [`ContractStore`] for store + route tests. Mirrors the
    /// Postgres semantics: upsert preserves `first_seen_at`, list is
    /// newest-updated-first with case-insensitive filters and a
    /// lowercased-substring search.
    #[derive(Default)]
    pub struct MemoryContractStore {
        rows: Mutex<HashMap<String, StoredRow>>,
        /// `(kind, value_norm)` -> `(category, slug)`. Stands in for
        /// `reference_registry`, which the Postgres store queries. A
        /// test declares what resolves; anything absent stays
        /// unresolved, matching the "never guess" rule.
        resolutions: Mutex<HashMap<(String, String), (String, String)>>,
        /// `(kind, value_norm)` -> how many registry rows matched.
        /// Lets a test model the third state — several matches, so we
        /// know entries exist but not which one.
        match_counts: Mutex<HashMap<(String, String), i64>>,
    }

    /// Internal storage cell — keeps the promoted columns alongside the
    /// record + timestamps so `list` doesn't have to re-parse JSON.
    #[derive(Clone)]
    struct StoredRow {
        new: NewContract,
        first_seen_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    }

    impl MemoryContractStore {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn len(&self) -> usize {
            self.rows.lock().unwrap().len()
        }

        // Present to satisfy clippy's `len_without_is_empty`; the tests
        // assert on `len()` directly.
        #[allow(dead_code)]
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// Resolve one entity against the declared map, or `None`.
        /// Stands in for the Postgres store's `reference_registry`
        /// lookup.
        fn resolved(&self, e: &crate::contract_entities::EntityRef) -> Option<(String, String)> {
            self.resolutions
                .lock()
                .unwrap()
                .get(&(e.kind.clone(), e.value_norm.clone()))
                .cloned()
        }

        /// Declare that `value` of `kind` resolves to a KB entry.
        /// Anything not declared resolves to nothing.
        pub fn resolve_to(&self, kind: &str, value: &str, category: &str, slug: &str) {
            let key = (
                kind.to_string(),
                crate::contract_entities::normalize_value(value),
            );
            self.resolutions
                .lock()
                .unwrap()
                .insert(key.clone(), (category.to_string(), slug.to_string()));
            self.match_counts.lock().unwrap().insert(key, 1);
        }

        /// Declare that `value` matches SEVERAL registry rows — the
        /// knowledge base holds entries but cannot say which is meant.
        pub fn resolve_ambiguously(&self, kind: &str, value: &str, matches: i64) {
            self.match_counts.lock().unwrap().insert(
                (
                    kind.to_string(),
                    crate::contract_entities::normalize_value(value),
                ),
                matches,
            );
        }
    }

    #[async_trait]
    impl ContractStore for MemoryContractStore {
        async fn upsert(&self, c: NewContract) -> Result<UpsertOutcome, ContractStoreError> {
            let now = Utc::now();
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&c.canonical_id) {
                Some(existing) => {
                    let first_seen = existing.first_seen_at;
                    *existing = StoredRow {
                        new: c,
                        first_seen_at: first_seen,
                        updated_at: now,
                    };
                    Ok(UpsertOutcome::Updated)
                }
                None => {
                    let id = c.canonical_id.clone();
                    rows.insert(
                        id,
                        StoredRow {
                            new: c,
                            first_seen_at: now,
                            updated_at: now,
                        },
                    );
                    Ok(UpsertOutcome::Inserted)
                }
            }
        }

        async fn get(
            &self,
            canonical_id: &str,
        ) -> Result<Option<StoredContract>, ContractStoreError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows.get(canonical_id).map(|r| StoredContract {
                canonical_id: r.new.canonical_id.clone(),
                schema_version: r.new.schema_version.clone(),
                suggested_action: r.new.suggested_action.clone(),
                record: r.new.record.clone(),
                first_seen_at: r.first_seen_at,
                updated_at: r.updated_at,
            }))
        }

        async fn list(
            &self,
            filter: ContractListFilter,
        ) -> Result<Vec<ContractSummaryRow>, ContractStoreError> {
            let rows = self.rows.lock().unwrap();
            let query = filter
                .query
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase);

            let mut out: Vec<(DateTime<Utc>, ContractSummaryRow)> = rows
                .values()
                .filter(|r| match &filter.contract_type {
                    Some(t) => r
                        .new
                        .contract_type
                        .as_deref()
                        .is_some_and(|v| v.eq_ignore_ascii_case(t)),
                    None => true,
                })
                .filter(|r| match &filter.issuer {
                    Some(i) => r
                        .new
                        .issuer
                        .as_deref()
                        .is_some_and(|v| v.eq_ignore_ascii_case(i)),
                    None => true,
                })
                .filter(|r| match &filter.legal_status {
                    Some(l) => r
                        .new
                        .legal_status
                        .as_deref()
                        .is_some_and(|v| v.eq_ignore_ascii_case(l)),
                    None => true,
                })
                .filter(|r| match &filter.faction {
                    Some(f) => r
                        .new
                        .faction
                        .as_deref()
                        .is_some_and(|v| v.eq_ignore_ascii_case(f)),
                    None => true,
                })
                .filter(|r| match &filter.gameplay_loop {
                    Some(g) => r
                        .new
                        .gameplay_loop
                        .as_deref()
                        .is_some_and(|v| v.eq_ignore_ascii_case(g)),
                    None => true,
                })
                .filter(|r| match &query {
                    Some(q) => r.new.search_blob.contains(q.as_str()),
                    None => true,
                })
                .map(|r| {
                    (
                        r.updated_at,
                        ContractSummaryRow {
                            canonical_id: r.new.canonical_id.clone(),
                            display_name: r.new.display_name.clone(),
                            contract_type: r.new.contract_type.clone(),
                            subcategory: r.new.subcategory.clone(),
                            gameplay_loop: r.new.gameplay_loop.clone(),
                            issuer: r.new.issuer.clone(),
                            faction: r.new.faction.clone(),
                            legal_status: r.new.legal_status.clone(),
                            reward_amount: r.new.reward_amount,
                            reward_currency: r.new.reward_currency.clone(),
                            confidence_score: r.new.confidence_score,
                            patch_version: r.new.patch_version.clone(),
                            first_step_location: r.new.first_step_location.clone(),
                            required_item: r.new.required_item.clone(),
                            step_count: r.new.step_count,
                            first_seen_at: r.first_seen_at,
                            updated_at: r.updated_at,
                        },
                    )
                })
                .collect();

            // Newest-updated first, canonical_id ASC as the stable
            // tiebreaker (matches the Postgres ORDER BY).
            out.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| a.1.canonical_id.cmp(&b.1.canonical_id))
            });

            let start = filter.offset.max(0) as usize;
            let take = filter.limit.max(0) as usize;
            Ok(out
                .into_iter()
                .skip(start)
                .take(take)
                .map(|(_, row)| row)
                .collect())
        }

        async fn delete(&self, canonical_id: &str) -> Result<bool, ContractStoreError> {
            Ok(self.rows.lock().unwrap().remove(canonical_id).is_some())
        }

        async fn delete_all(&self) -> Result<u64, ContractStoreError> {
            let mut rows = self.rows.lock().unwrap();
            let n = rows.len() as u64;
            rows.clear();
            Ok(n)
        }

        async fn list_by_entity(
            &self,
            category: &str,
            slug: &str,
            limit: i64,
            offset: i64,
        ) -> Result<Vec<ContractSummaryRow>, ContractStoreError> {
            let rows = self.rows.lock().unwrap();
            let mut hits: Vec<&StoredRow> =
                rows.values()
                    .filter(|r| {
                        r.new.entities.iter().any(|e| {
                            self.resolved(e) == Some((category.to_string(), slug.to_string()))
                        })
                    })
                    .collect();
            hits.sort_by(|a, b| {
                a.new
                    .display_name
                    .cmp(&b.new.display_name)
                    .then(a.new.canonical_id.cmp(&b.new.canonical_id))
            });
            Ok(hits
                .into_iter()
                .skip(offset.max(0) as usize)
                .take(limit.max(0) as usize)
                .map(|r| ContractSummaryRow {
                    canonical_id: r.new.canonical_id.clone(),
                    display_name: r.new.display_name.clone(),
                    contract_type: r.new.contract_type.clone(),
                    subcategory: r.new.subcategory.clone(),
                    gameplay_loop: r.new.gameplay_loop.clone(),
                    issuer: r.new.issuer.clone(),
                    faction: r.new.faction.clone(),
                    legal_status: r.new.legal_status.clone(),
                    reward_amount: r.new.reward_amount,
                    reward_currency: r.new.reward_currency.clone(),
                    confidence_score: r.new.confidence_score,
                    patch_version: r.new.patch_version.clone(),
                    first_step_location: r.new.first_step_location.clone(),
                    required_item: r.new.required_item.clone(),
                    step_count: r.new.step_count,
                    first_seen_at: r.first_seen_at,
                    updated_at: r.updated_at,
                })
                .collect())
        }

        async fn resolve_names(
            &self,
            names: &[String],
        ) -> Result<Vec<crate::contract_entities::NameResolution>, ContractStoreError> {
            use crate::contract_entities::{normalize_value, NameResolution};
            let rows = self.rows.lock().unwrap();
            Ok(names
                .iter()
                .map(|raw| {
                    let name = normalize_value(raw);
                    let matches: Vec<&StoredRow> = rows
                        .values()
                        .filter(|r| {
                            r.new
                                .display_name
                                .as_deref()
                                .map(|d| normalize_value(d) == name)
                                .unwrap_or(false)
                        })
                        .collect();
                    NameResolution {
                        match_count: matches.len() as i64,
                        // Exactly one, or nothing. Never a winner among
                        // same-named contracts.
                        canonical_id: if matches.len() == 1 {
                            Some(matches[0].new.canonical_id.clone())
                        } else {
                            None
                        },
                        name,
                    }
                })
                .collect())
        }

        async fn entities_for(
            &self,
            canonical_id: &str,
        ) -> Result<Vec<crate::contract_entities::EntityRow>, ContractStoreError> {
            let rows = self.rows.lock().unwrap();
            let Some(row) = rows.get(canonical_id) else {
                return Ok(Vec::new());
            };
            let mut out: Vec<crate::contract_entities::EntityRow> = row
                .new
                .entities
                .iter()
                .map(|e| {
                    let hit = self.resolved(e);
                    let count = self
                        .match_counts
                        .lock()
                        .unwrap()
                        .get(&(e.kind.clone(), e.value_norm.clone()))
                        .copied()
                        .unwrap_or(0);
                    crate::contract_entities::EntityRow {
                        kind: e.kind.clone(),
                        raw_value: e.raw_value.clone(),
                        ref_category: hit.as_ref().map(|(c, _)| c.clone()),
                        ref_slug: hit.as_ref().map(|(_, s)| s.clone()),
                        ref_match_count: count,
                    }
                })
                .collect();
            out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.raw_value.cmp(&b.raw_value)));
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryContractStore;
    use super::*;

    #[test]
    fn normalize_type_key_collapses_separators_and_case() {
        // The live catalog holds all of these as distinct contract_type
        // values; they must resolve to one key. Measured 2026-07-29:
        // "Bounty Hunting"=30 vs "bounty_hunting"=2 — filtering by the
        // latter returned 2 of 32 rows and implied the rest didn't exist.
        assert_eq!(normalize_type_key("Bounty Hunting"), "bounty hunting");
        assert_eq!(normalize_type_key("bounty_hunting"), "bounty hunting");
        assert_eq!(normalize_type_key("BOUNTY-HUNTING"), "bounty hunting");
        assert_eq!(normalize_type_key("  Bounty   Hunting  "), "bounty hunting");
        // Case-only pairs already worked via LOWER(); they must keep working.
        assert_eq!(
            normalize_type_key("Mercenary"),
            normalize_type_key("mercenary")
        );
        // Distinct types must NOT collapse.
        assert_ne!(
            normalize_type_key("Hauling"),
            normalize_type_key("Hauling - Stellar")
        );
    }

    /// The verbatim handoff example, as the sender would POST it.
    fn sample_bundle() -> PublishBundleReq {
        serde_json::from_value(serde_json::json!({
            "schema_version": "1",
            "canonical_id": "apprehend_zane_esteban",
            "capture_id": "cap_01H",
            "internal": {
                "source_capture_id": "cap_01H",
                "raw_text": "BOUNTY HUNTER\nApprehend Zane Esteban ...",
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
                        "reward": { "amount": 8500, "currency": "aUEC", "bonus_amount": 1500 },
                        "fees": [ { "type": "deposit", "amount": 0, "currency": "aUEC", "refundable": true } ],
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
                        { "order": 2, "step_type": "navigate", "summary": "Travel to Glaciem Ring.",
                          "location": "Glaciem Ring", "risk": "medium" }
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
        }))
        .expect("sample bundle deserializes")
    }

    /// Non-aUEC awards must survive the wire, because `record` is the
    /// RE-SERIALIZED packet rather than the raw body: any field missing
    /// from `RewardReq` is dropped at deserialize, stored nowhere, and
    /// can never be displayed — while the push still answers 200.
    ///
    /// sp-ingest reads these out of the DETAILS prose ("Completing this
    /// contract awards 14 MG Scrip"), so they are frequently the ONLY
    /// record that a contract pays anything besides aUEC.
    #[test]
    fn additional_rewards_survive_the_wire_into_the_stored_record() {
        let mut v = serde_json::to_value(sample_bundle()).expect("bundle serializes");
        v["internal"]["extraction"]["contract"]["reward"] = serde_json::json!({
            "amount": 23000,
            "currency": "aUEC",
            "additional": [
                { "amount": 14, "unit": "MG Scrip",
                  "note": "Completing this contract awards 14 MG Scrip." },
                { "amount": null, "unit": "Council Scrip", "note": null }
            ]
        });
        let bundle: PublishBundleReq =
            serde_json::from_value(v).expect("bundle with additional rewards deserializes");
        let c = NewContract::from_bundle(&bundle).expect("valid bundle");

        let addl = &c.record["extraction"]["contract"]["reward"]["additional"];
        assert_eq!(addl.as_array().map(Vec::len), Some(2), "both awards stored");
        assert_eq!(addl[0]["amount"].as_i64(), Some(14));
        assert_eq!(addl[0]["unit"].as_str(), Some("MG Scrip"));
        assert_eq!(
            addl[0]["note"].as_str(),
            Some("Completing this contract awards 14 MG Scrip.")
        );
        // An award with no stated count is still an award.
        assert!(addl[1]["amount"].is_null());
        assert_eq!(addl[1]["unit"].as_str(), Some("Council Scrip"));
    }

    #[test]
    fn requirements_survive_the_wire_and_reach_the_search_blob() {
        // Requirements live only in the description prose, so if the
        // wire model drops them they are gone entirely - and the push
        // still answers 200, which is how reward.additional was lost.
        let mut v = serde_json::to_value(sample_bundle()).unwrap();
        v["internal"]["extraction"]["contract"]["requirements"] =
            serde_json::json!(["Tractor beam required", "Must supply own quantum fuel"]);
        let bundle: PublishBundleReq = serde_json::from_value(v).expect("parses");
        let c = NewContract::from_bundle(&bundle).expect("valid");

        let stored = &c.record["extraction"]["contract"]["requirements"];
        assert_eq!(stored.as_array().map(Vec::len), Some(2));
        assert_eq!(stored[0].as_str(), Some("Tractor beam required"));
        // Findable: "can I take this contract" is a search, not a browse.
        assert!(c.search_blob.contains("tractor beam required"));
    }

    #[test]
    fn from_bundle_promotes_columns_and_builds_search_blob() {
        let c = NewContract::from_bundle(&sample_bundle()).expect("valid bundle");
        assert_eq!(c.canonical_id, "apprehend_zane_esteban");
        assert_eq!(c.display_name.as_deref(), Some("Apprehend Zane Esteban"));
        assert_eq!(c.contract_type.as_deref(), Some("bounty"));
        assert_eq!(c.issuer.as_deref(), Some("Crusader Security"));
        assert_eq!(c.reward_amount, Some(8500));
        assert_eq!(c.suggested_action.as_deref(), Some("create_new_contract"));
        // Search blob folds in the attribute location + step location.
        assert!(c.search_blob.contains("glaciem ring"));
        assert!(c.search_blob.contains("crusader security"));
        // raw_text is preserved in the record for internal storage.
        assert_eq!(
            c.record["raw_text"].as_str(),
            Some("BOUNTY HUNTER\nApprehend Zane Esteban ...")
        );
    }

    #[test]
    fn from_bundle_rejects_blank_canonical_id() {
        let mut b = sample_bundle();
        b.canonical_id = "   ".to_string();
        assert!(matches!(
            NewContract::from_bundle(&b),
            Err(IngestValidationError::MissingCanonicalId)
        ));
    }

    /// One step JSON value for the step-derived-column tests below.
    /// Everything but `location` / `required_item` defaults via
    /// `ExtractedStepReq`'s `#[serde(default)]`.
    fn step(location: Option<&str>, required_item: Option<&str>) -> serde_json::Value {
        serde_json::json!({ "location": location, "required_item": required_item })
    }

    /// A minimal bundle whose only points of interest are
    /// `canonical_id` and `extraction.steps` — every other field
    /// defaults per `#[serde(default)]` on the wire model structs.
    fn bundle_with_steps(canonical_id: &str, steps: Vec<serde_json::Value>) -> PublishBundleReq {
        serde_json::from_value(serde_json::json!({
            "schema_version": "1",
            "canonical_id": canonical_id,
            "internal": {
                "extraction": {
                    "contract": {},
                    "steps": steps
                }
            }
        }))
        .expect("bundle_with_steps deserializes")
    }

    /// Escape a prefix for use as a `LIKE 'prefix%'` pattern. Postgres's
    /// `_` is a single-character LIKE wildcard, so an unescaped prefix
    /// ending in `_` (every scoped-test prefix in this file does)
    /// matches a sibling prefix too — unescaped, `t1sep_%` also matches
    /// any `t1sepX_...` for a single character X standing in for the
    /// `_`. Escaping (Postgres's default LIKE escape character is
    /// backslash) makes the match exact. Shared by every
    /// `clear_scoped_rows` below.
    fn escape_like_prefix(prefix: &str) -> String {
        format!(
            "{}%",
            prefix
                .replace('\\', "\\\\")
                .replace('_', "\\_")
                .replace('%', "\\%")
        )
    }

    #[test]
    fn from_bundle_promotes_step_derived_columns() {
        let bundle = bundle_with_steps(
            "step_promo",
            vec![
                step(None, None), // no location, no item
                step(
                    Some("Rayari McGrath Research Outpost"),
                    Some("15 Amioshi Plague"),
                ),
                step(
                    Some("Rayari McGrath Research Outpost"),
                    Some("Amioshi Plague"),
                ),
            ],
        );
        let c = NewContract::from_bundle(&bundle).unwrap();
        assert_eq!(c.step_count, Some(3));
        // FIRST non-empty location, not the first step's (which is None).
        assert_eq!(
            c.first_step_location.as_deref(),
            Some("Rayari McGrath Research Outpost")
        );
        // FIRST item, quantity stripped. SINGULAR by design — see the
        // 0061 migration comment for why aggregating a list was
        // rejected.
        assert_eq!(c.required_item.as_deref(), Some("Amioshi Plague"));
    }

    #[test]
    fn from_bundle_step_count_is_zero_not_none_for_empty_steps() {
        let bundle = bundle_with_steps("no_steps", vec![]);
        let c = NewContract::from_bundle(&bundle).unwrap();
        assert_eq!(c.step_count, Some(0));
        assert_eq!(c.first_step_location, None);
        assert_eq!(c.required_item, None);
    }

    #[test]
    fn from_bundle_required_item_skips_a_bare_quantity_step() {
        // The trap verified against real data: a step whose
        // required_item is nothing but a quantity ("25") strips to an
        // empty string and must be skipped, not taken as the answer.
        let bundle = bundle_with_steps(
            "bare_qty",
            vec![step(None, Some("25")), step(None, Some("Prota"))],
        );
        let c = NewContract::from_bundle(&bundle).unwrap();
        assert_eq!(c.required_item.as_deref(), Some("Prota"));
    }

    #[test]
    fn strip_item_qty_matches_migration_0061_forms() {
        // Leading "<n> " and "<n>x ".
        assert_eq!(strip_item_qty("15 Sunset Berries"), "Sunset Berries");
        assert_eq!(strip_item_qty("15x Amioshi Plague"), "Amioshi Plague");
        // Trailing " x<n>".
        assert_eq!(
            strip_item_qty("Valakkar Fang (Juvenile) x25"),
            "Valakkar Fang (Juvenile)"
        );
        // Must not mangle an item that merely contains digits.
        assert_eq!(strip_item_qty("F7C Hornet Mk I"), "F7C Hornet Mk I");
        // A bare quantity strips to empty, not to itself.
        assert_eq!(strip_item_qty("25"), "");
    }

    /// Build a bundle whose steps carry the given locations/items.
    fn named_bundle_with_steps(id: &str, name: &str, steps: serde_json::Value) -> PublishBundleReq {
        serde_json::from_value(serde_json::json!({
            "schema_version": "1",
            "canonical_id": id,
            "capture_id": "cap",
            "internal": {
                "source_capture_id": "cap",
                "raw_text": "x",
                "extraction": {
                    "contract": { "display_name": name },
                    "steps": steps
                },
                "suggestion": { "suggested_action": "create_new_contract" },
                "confidence_score": 0.9,
                "flags": []
            }
        }))
        .expect("bundle parses")
    }

    #[tokio::test]
    async fn upsert_carries_entities_and_entities_for_resolves_them() {
        let store = MemoryContractStore::new();
        store.resolve_to("location", "Glaciem Ring", "location", "glaciem-ring");

        let b = named_bundle_with_steps(
            "c1",
            "Refuel Run",
            serde_json::json!([
                { "order": 1, "summary": "Go",
                  "entities": [{ "kind": "location", "name": "Glaciem Ring" }] },
                { "order": 2, "summary": "Go",
                  "entities": [{ "kind": "location", "name": "Somewhere Unmapped" }] }
            ]),
        );
        store
            .upsert(NewContract::from_bundle(&b).unwrap())
            .await
            .unwrap();

        let ents = store.entities_for("c1").await.unwrap();
        assert_eq!(ents.len(), 2, "upsert did not carry the entity rows");

        let resolved = ents.iter().find(|e| e.raw_value == "Glaciem Ring").unwrap();
        assert_eq!(resolved.ref_slug.as_deref(), Some("glaciem-ring"));
        assert_eq!(resolved.ref_category.as_deref(), Some("location"));

        // An unresolved entity keeps its raw value so the page can render
        // plain text instead of a link that 404s.
        let unresolved = ents
            .iter()
            .find(|e| e.raw_value == "Somewhere Unmapped")
            .unwrap();
        assert!(unresolved.ref_slug.is_none());
    }

    #[tokio::test]
    async fn list_by_entity_returns_only_contracts_referencing_that_slug() {
        let store = MemoryContractStore::new();
        store.resolve_to("location", "Glaciem Ring", "location", "glaciem-ring");
        store.resolve_to("location", "Area18", "location", "area18");

        for (id, loc) in [("hit", "Glaciem Ring"), ("miss", "Area18")] {
            let b = named_bundle_with_steps(
                id,
                "N",
                serde_json::json!([{ "order": 1, "summary": "Go",
                    "entities": [{ "kind": "location", "name": loc }] }]),
            );
            store
                .upsert(NewContract::from_bundle(&b).unwrap())
                .await
                .unwrap();
        }

        let rows = store
            .list_by_entity("location", "glaciem-ring", 50, 0)
            .await
            .unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.canonical_id.as_str()).collect();
        assert_eq!(ids, vec!["hit"]);
    }

    #[tokio::test]
    async fn resolve_names_links_a_unique_name_and_refuses_an_ambiguous_one() {
        let store = MemoryContractStore::new();
        for (id, name) in [
            ("solo", "Patrol Dangerous Sector"),
            ("dup_a", "Combat Gauntlet - Scenario #1"),
            ("dup_b", "Combat Gauntlet - Scenario #1"),
        ] {
            let b = named_bundle_with_steps(id, name, serde_json::json!([]));
            store
                .upsert(NewContract::from_bundle(&b).unwrap())
                .await
                .unwrap();
        }

        let got = store
            .resolve_names(&[
                "  patrol   DANGEROUS sector ".to_string(),
                "Combat Gauntlet - Scenario #1".to_string(),
                "Never Published".to_string(),
            ])
            .await
            .unwrap();

        // Case and whitespace insensitive.
        assert_eq!(got[0].match_count, 1);
        assert_eq!(got[0].canonical_id.as_deref(), Some("solo"));

        // display_name is non-unique BY DESIGN — naming a winner here
        // would be the confident-wrong-answer failure (spec F6).
        assert_eq!(got[1].match_count, 2);
        assert!(
            got[1].canonical_id.is_none(),
            "must not pick among same-named contracts"
        );

        // Every requested name gets an answer, including misses.
        assert_eq!(got[2].match_count, 0);
        assert!(got[2].canonical_id.is_none());
    }

    #[tokio::test]
    async fn upsert_inserts_then_get_returns_record() {
        let store = MemoryContractStore::new();
        let c = NewContract::from_bundle(&sample_bundle()).unwrap();
        assert_eq!(store.upsert(c).await.unwrap(), UpsertOutcome::Inserted);

        let got = store.get("apprehend_zane_esteban").await.unwrap().unwrap();
        assert_eq!(got.schema_version, "1");
        assert_eq!(got.suggested_action.as_deref(), Some("create_new_contract"));
        // Full internal packet round-trips through storage.
        assert_eq!(
            got.record["extraction"]["contract"]["display_name"].as_str(),
            Some("Apprehend Zane Esteban")
        );
    }

    #[tokio::test]
    async fn repeat_upsert_same_id_updates_not_duplicates() {
        let store = MemoryContractStore::new();
        let c1 = NewContract::from_bundle(&sample_bundle()).unwrap();
        assert_eq!(store.upsert(c1).await.unwrap(), UpsertOutcome::Inserted);
        let first_seen = store
            .get("apprehend_zane_esteban")
            .await
            .unwrap()
            .unwrap()
            .first_seen_at;

        // Second push of the same canonical_id with a changed reward.
        let mut b2 = sample_bundle();
        b2.internal.extraction.contract.reward.amount = Some(9999);
        let c2 = NewContract::from_bundle(&b2).unwrap();
        assert_eq!(store.upsert(c2).await.unwrap(), UpsertOutcome::Updated);

        assert_eq!(store.len(), 1, "same canonical_id must not duplicate");
        let got = store.get("apprehend_zane_esteban").await.unwrap().unwrap();
        assert_eq!(
            got.record["extraction"]["contract"]["reward"]["amount"].as_i64(),
            Some(9999)
        );
        assert_eq!(
            got.first_seen_at, first_seen,
            "first_seen_at must be preserved across upserts"
        );
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let store = MemoryContractStore::new();
        assert!(store.get("nope").await.unwrap().is_none());
    }

    async fn seed(store: &MemoryContractStore, id: &str, ty: &str, issuer: &str, loc: &str) {
        let bundle: PublishBundleReq = serde_json::from_value(serde_json::json!({
            "canonical_id": id,
            "internal": { "extraction": { "contract": {
                "display_name": id,
                "contract_type": ty,
                "issuer": issuer,
                "legal_status": "legal",
                "attributes": [ { "label": "LAST KNOWN LOCATION", "value": loc } ]
            }, "steps": [] } }
        }))
        .unwrap();
        store
            .upsert(NewContract::from_bundle(&bundle).unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_filters_by_type_and_issuer_case_insensitive() {
        let store = MemoryContractStore::new();
        seed(&store, "b1", "bounty", "Crusader Security", "Glaciem Ring").await;
        seed(&store, "d1", "delivery", "Hurston", "Lorville").await;

        let bounties = store
            .list(ContractListFilter {
                contract_type: Some("BOUNTY".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(bounties.len(), 1);
        assert_eq!(bounties[0].canonical_id, "b1");

        let hurston = store
            .list(ContractListFilter {
                issuer: Some("hurston".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(hurston.len(), 1);
        assert_eq!(hurston[0].canonical_id, "d1");
    }

    #[tokio::test]
    async fn search_matches_location_in_blob() {
        let store = MemoryContractStore::new();
        seed(&store, "b1", "bounty", "Crusader Security", "Glaciem Ring").await;
        seed(&store, "d1", "delivery", "Hurston", "Lorville").await;

        let hits = store
            .list(ContractListFilter {
                query: Some("glaciem".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].canonical_id, "b1");
    }

    #[tokio::test]
    async fn list_paginates_by_offset_limit() {
        let store = MemoryContractStore::new();
        for i in 0..5 {
            seed(
                &store,
                &format!("c{i}"),
                "bounty",
                "Crusader Security",
                "Glaciem Ring",
            )
            .await;
        }
        let page = store
            .list(ContractListFilter {
                limit: 2,
                offset: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.len(), 2);
        let page2 = store
            .list(ContractListFilter {
                limit: 2,
                offset: 2,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
        // No overlap between pages.
        assert_ne!(page[0].canonical_id, page2[0].canonical_id);
    }

    // -- contract_type separator/boundary matching against a REAL
    // Postgres (env-gated, parallel-safe) ------------------------------
    //
    // `list_filters_by_type_and_issuer_case_insensitive` above only
    // exercises `MemoryContractStore`, which matches via
    // `eq_ignore_ascii_case` — a different code path that never runs
    // the `REGEXP_REPLACE`/`BTRIM` predicate in
    // `PostgresContractStore::list`. This test is that missing
    // exercise: it seeds the SAME logical type under four spellings,
    // including the boundary cases (leading `_`, surrounding
    // whitespace) that whitespace-collapse alone does not handle, and
    // asserts every spelling returns all four rows.
    //
    // Parallel-safe: every row this test seeds carries a unique
    // `t1sep_` prefix, a scoped `DELETE ... WHERE canonical_id LIKE
    // 't1sep_%'` replaces a table-wide TRUNCATE (run on entry, so a
    // previous crashed run can't poison this one, and again on exit),
    // and assertions filter the returned rows down to that prefix
    // before comparing — so this test does NOT require
    // `--test-threads=1` and cannot be disturbed by, or disturb, other
    // tests that seed their own `contracts` rows (e.g. Task 2's filter
    // fixtures, Task 3's backfill fixture). Runs ONLY when
    // `STARSTATS_TEST_DATABASE_URL` points at a real Postgres; offline
    // `cargo test` skips it (early return).
    #[tokio::test]
    async fn list_filters_contract_type_across_separator_and_boundary_variants_on_real_postgres() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!(
                "STARSTATS_TEST_DATABASE_URL unset — skipping contract_type separator PG test"
            );
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        async fn clear_scoped_rows(pool: &PgPool, prefix: &str) {
            sqlx::query("DELETE FROM contracts WHERE canonical_id LIKE $1")
                .bind(escape_like_prefix(prefix))
                .execute(pool)
                .await
                .expect("delete this test's scoped rows");
        }

        let prefix = "t1sep_";
        // Self-heal: a previous crashed run may have left its rows
        // behind under this prefix.
        clear_scoped_rows(&pool, prefix).await;

        let store = PostgresContractStore::new(pool.clone());

        fn seed_row(id: &str, contract_type: &str) -> NewContract {
            NewContract {
                canonical_id: id.to_string(),
                schema_version: "1".to_string(),
                capture_id: None,
                display_name: Some(id.to_string()),
                contract_type: Some(contract_type.to_string()),
                subcategory: None,
                gameplay_loop: None,
                issuer: None,
                faction: None,
                legal_status: None,
                reward_amount: None,
                reward_currency: None,
                patch_version: None,
                confidence_score: None,
                suggested_action: None,
                search_blob: String::new(),
                first_step_location: None,
                required_item: None,
                step_count: None,
                entities: Vec::new(),
                record: serde_json::json!({}),
            }
        }

        // Four spellings the live catalog holds (or could hold) for the
        // SAME type, all under the unique `t1sep_` prefix so this
        // test's rows can never be confused with a sibling test's
        // fixtures. "_bounty_hunting" and " Bounty Hunting " are the
        // boundary cases from the leading/trailing separator/whitespace
        // bug: REGEXP_REPLACE collapses whitespace runs but does not
        // trim edges, so an un-trimmed predicate leaves a stray
        // leading/trailing space on some of these and matches none of
        // the others.
        let seeded: [(String, &str); 4] = [
            (format!("{prefix}b1"), "Bounty Hunting"),
            (format!("{prefix}b2"), "bounty_hunting"),
            (format!("{prefix}b3"), "_bounty_hunting"),
            (format!("{prefix}b4"), " Bounty Hunting "),
        ];
        let mut expected_ids: Vec<String> = seeded.iter().map(|(id, _)| id.clone()).collect();
        expected_ids.sort();

        for (id, ty) in seeded {
            store
                .upsert(seed_row(&id, ty))
                .await
                .unwrap_or_else(|e| panic!("seed {id}: {e}"));
        }

        for query_spelling in [
            "Bounty Hunting",
            "bounty_hunting",
            "_bounty_hunting",
            " Bounty Hunting ",
        ] {
            let rows = store
                .list(ContractListFilter {
                    contract_type: Some(query_spelling.to_string()),
                    limit: 1000,
                    ..Default::default()
                })
                .await
                .unwrap_or_else(|e| panic!("list by {query_spelling:?}: {e}"));
            // Scope to rows THIS test seeded — a concurrent sibling
            // test (Task 2's filter fixtures, Task 3's backfill
            // fixture) may share a matching contract_type in flight;
            // only the `t1sep_` prefix is ours to assert on.
            let mut ids: Vec<String> = rows
                .into_iter()
                .map(|r| r.canonical_id)
                .filter(|id| id.starts_with(prefix))
                .collect();
            ids.sort();
            assert_eq!(
                ids, expected_ids,
                "filtering by {query_spelling:?} must return all 4 separator/boundary variants"
            );
        }

        clear_scoped_rows(&pool, prefix).await;
    }

    // -- faction / legal_status / gameplay_loop filters against a REAL
    // Postgres (env-gated, parallel-safe) ------------------------------
    //
    // Two rows share a `display_name` — the disambiguation case these
    // filters exist for (faction and legal_status each measured to vary
    // in 56% of duplicate-name groups; `issuer`, already filterable,
    // measured 0%). `legal_status` already had a query-builder predicate
    // (proven by the test above's sibling coverage of `contract_type`);
    // what was missing was `SearchQuery` never populating it, which is a
    // `contract_routes.rs`-level wiring gap covered separately by
    // `contract_routes::tests::search_filters_by_faction_legal_status_and_gameplay_loop`.
    // This test exercises the store/query-builder side: that all three
    // predicates actually narrow results against real Postgres.
    //
    // Parallel-safe per the Task 1 convention: unique `t2filt_` prefix,
    // scoped `DELETE ... WHERE canonical_id LIKE 't2filt_%'` on entry and
    // exit (no table-wide TRUNCATE), assertions filtered to the prefix.
    #[tokio::test]
    async fn list_filters_by_faction_legal_status_and_gameplay_loop_on_real_postgres() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!(
                "STARSTATS_TEST_DATABASE_URL unset — skipping faction/legal_status/gameplay_loop filter PG test"
            );
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        async fn clear_scoped_rows(pool: &PgPool, prefix: &str) {
            sqlx::query("DELETE FROM contracts WHERE canonical_id LIKE $1")
                .bind(escape_like_prefix(prefix))
                .execute(pool)
                .await
                .expect("delete this test's scoped rows");
        }

        let prefix = "t2filt_";
        // Self-heal: a previous crashed run may have left its rows behind.
        clear_scoped_rows(&pool, prefix).await;

        let store = PostgresContractStore::new(pool.clone());

        #[allow(clippy::too_many_arguments)]
        fn seed_row(
            id: &str,
            display_name: &str,
            faction: Option<&str>,
            legal_status: Option<&str>,
            gameplay_loop: Option<&str>,
        ) -> NewContract {
            NewContract {
                canonical_id: id.to_string(),
                schema_version: "1".to_string(),
                capture_id: None,
                display_name: Some(display_name.to_string()),
                contract_type: None,
                subcategory: None,
                gameplay_loop: gameplay_loop.map(str::to_string),
                issuer: None,
                faction: faction.map(str::to_string),
                legal_status: legal_status.map(str::to_string),
                reward_amount: None,
                reward_currency: None,
                patch_version: None,
                confidence_score: None,
                suggested_action: None,
                search_blob: String::new(),
                first_step_location: None,
                required_item: None,
                step_count: None,
                entities: Vec::new(),
                record: serde_json::json!({}),
            }
        }

        // Two rows sharing a display_name — the case this exists for.
        let c1 = format!("{prefix}c1");
        let c2 = format!("{prefix}c2");
        store
            .upsert(seed_row(
                &c1,
                "Same Name",
                Some("UEE"),
                Some("legal"),
                Some("bounty"),
            ))
            .await
            .unwrap_or_else(|e| panic!("seed {c1}: {e}"));
        store
            .upsert(seed_row(
                &c2,
                "Same Name",
                Some("Nine Tails"),
                Some("illegal"),
                Some("smuggling"),
            ))
            .await
            .unwrap_or_else(|e| panic!("seed {c2}: {e}"));

        async fn scoped_ids(
            store: &PostgresContractStore,
            prefix: &str,
            filter: ContractListFilter,
        ) -> Vec<String> {
            let mut ids: Vec<String> = store
                .list(ContractListFilter {
                    limit: 1000,
                    ..filter
                })
                .await
                .unwrap_or_else(|e| panic!("list: {e}"))
                .into_iter()
                .map(|r| r.canonical_id)
                .filter(|id| id.starts_with(prefix))
                .collect();
            ids.sort();
            ids
        }

        let by_faction = scoped_ids(
            &store,
            prefix,
            ContractListFilter {
                faction: Some("nine tails".into()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(by_faction, vec![c2.clone()]);

        let by_legal = scoped_ids(
            &store,
            prefix,
            ContractListFilter {
                legal_status: Some("LEGAL".into()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(by_legal, vec![c1.clone()]);

        let by_loop = scoped_ids(
            &store,
            prefix,
            ContractListFilter {
                gameplay_loop: Some("Smuggling".into()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(by_loop, vec![c2.clone()]);

        clear_scoped_rows(&pool, prefix).await;
    }

    /// Seed one contract via `NewContract::from_bundle` (capturing its
    /// promoted values before upsert), wipe just the three promoted
    /// columns, recompute them by running migration 0061's backfill
    /// expression verbatim (copy-pasted from
    /// `migrations/0061_contract_disambiguation_cols.sql`) scoped to
    /// that one row over the SAME stored `record`, and assert the two
    /// agree. Shared by both rows in
    /// `from_bundle_promotion_matches_migration_0061_backfill_on_real_postgres`
    /// so each quantity form gets its own row instead of one starving
    /// the other out (see that test's doc for why).
    /// `rust_*` are captured by the caller from `NewContract::from_bundle`
    /// BEFORE upsert (upsert consumes the `NewContract`); `id`'s row
    /// must already be seeded in `contracts`.
    async fn assert_backfill_parity(
        pool: &PgPool,
        id: &str,
        rust_step_count: Option<i32>,
        rust_first_loc: Option<String>,
        rust_required_item: Option<String>,
    ) {
        // Wipe the promoted columns, then recompute them via migration
        // 0061's exact backfill expression, scoped to this one row.
        sqlx::query(
            "UPDATE contracts SET first_step_location = NULL, required_item = NULL, \
             step_count = NULL WHERE canonical_id = $1",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("reset promoted columns");

        sqlx::query(
            r#"
            UPDATE contracts SET
                step_count = (
                    SELECT COUNT(*)::INT
                    FROM jsonb_array_elements(record #> '{extraction,steps}') AS s
                ),
                first_step_location = (
                    SELECT s ->> 'location'
                    FROM jsonb_array_elements(record #> '{extraction,steps}') WITH ORDINALITY AS t(s, ord)
                    WHERE s ->> 'location' IS NOT NULL AND s ->> 'location' <> ''
                    ORDER BY ord
                    LIMIT 1
                ),
                required_item = (
                    SELECT btrim(
                        regexp_replace(
                            regexp_replace(s ->> 'required_item', '^\s*\d+\s*[xX]?\s*', ''),
                            '\s*[xX]\s*\d+\s*$', ''
                        ),
                        E' \t\n\r'
                    )
                    FROM jsonb_array_elements(record #> '{extraction,steps}') WITH ORDINALITY AS t(s, ord)
                    WHERE btrim(
                              regexp_replace(
                                  regexp_replace(coalesce(s ->> 'required_item', ''), '^\s*\d+\s*[xX]?\s*', ''),
                                  '\s*[xX]\s*\d+\s*$', ''
                              ),
                              E' \t\n\r'
                          ) <> ''
                    ORDER BY ord
                    LIMIT 1
                )
            WHERE canonical_id = $1
              AND jsonb_typeof(record #> '{extraction,steps}') = 'array'
            "#,
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("run migration 0061's backfill expression");

        #[allow(clippy::type_complexity)]
        let (sql_first_loc, sql_required_item, sql_step_count): (
            Option<String>,
            Option<String>,
            Option<i32>,
        ) = sqlx::query_as(
            "SELECT first_step_location, required_item, step_count \
             FROM contracts WHERE canonical_id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("read back SQL-computed columns");

        assert_eq!(
            rust_step_count, sql_step_count,
            "step_count must match migration 0061's backfill for {id}"
        );
        assert_eq!(
            rust_first_loc, sql_first_loc,
            "first_step_location must match migration 0061's backfill for {id}"
        );
        assert_eq!(
            rust_required_item, sql_required_item,
            "required_item must match migration 0061's backfill for {id}"
        );
    }

    // -- `NewContract::from_bundle`'s step-derived promotion vs.
    // migration 0061's backfill, against a REAL Postgres
    // (env-gated, parallel-safe) --------------------------------------
    //
    // The defect this task is most exposed to: two independently-coded
    // implementations of the same rule (Rust promotes at ingest, SQL
    // backfills rows that predate it) silently drifting apart, so the
    // same contract's `first_step_location` / `required_item` /
    // `step_count` depend on when it happened to be ingested.
    // `assert_backfill_parity` seeds one row via
    // `NewContract::from_bundle`, wipes the promoted columns, recomputes
    // them by running migration 0061's backfill verbatim over the same
    // stored `record`, and asserts the two agree.
    //
    // TWO rows, not one: `required_item` takes the FIRST non-empty
    // stripped step value, so whichever trap step comes first "wins"
    // and every later trap is never evaluated by either side. A single
    // row ending in `[..., "15x Amioshi Plague", "Valakkar Fang
    // (Juvenile) x25"]` only ever exercises the leading form (it wins
    // at that earlier step) — the trailing form's own regexp_replace
    // pass was silently never compared. Row `a` below makes the
    // LEADING form the winner; row `b` makes the TRAILING form the
    // winner, so each quantity-stripping regex is independently
    // load-bearing. (Verified: deleting `TRAILING_QTY_RE`'s
    // `regexp_replace` — or its SQL mirror — now fails row `b`.)
    //
    // Both rows also carry a null-location step (must be skipped), a
    // bare-quantity `required_item` ("25", strips to empty, must not
    // win), and a whitespace-only `required_item` (a bare tab) — a trap
    // for the SAME reason as the bare quantity: `btrim(x)` alone only
    // strips spaces, so an un-scoped `btrim` would treat the
    // stripped-to-tab value as non-empty and pick it, while Rust's
    // `str::trim()` reduces it to "" and skips it. (Verified: reverting
    // the migration's two-argument `btrim(x, E' \t\n\r')` back to
    // single-argument `btrim(x)` now fails both rows.)
    //
    // Parallel-safe per the Task 1/2 convention: unique `t4prom_`
    // prefix, scoped `DELETE ... WHERE canonical_id LIKE 't4prom_%'`
    // (properly escaped — see `escape_like_prefix`) on entry and exit
    // (no table-wide TRUNCATE).
    #[tokio::test]
    async fn from_bundle_promotion_matches_migration_0061_backfill_on_real_postgres() {
        let Ok(url) = std::env::var("STARSTATS_TEST_DATABASE_URL") else {
            eprintln!("STARSTATS_TEST_DATABASE_URL unset — skipping Task 4 SQL-parity PG test");
            return;
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect STARSTATS_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations on the test DB");

        async fn clear_scoped_rows(pool: &PgPool, prefix: &str) {
            sqlx::query("DELETE FROM contracts WHERE canonical_id LIKE $1")
                .bind(escape_like_prefix(prefix))
                .execute(pool)
                .await
                .expect("delete this test's scoped rows");
        }

        let prefix = "t4prom_";
        clear_scoped_rows(&pool, prefix).await;

        let store = PostgresContractStore::new(pool.clone());

        // Row `a`: the LEADING form ("15x Amioshi Plague") is the first
        // non-empty stripped candidate, so it wins.
        let id_a = format!("{prefix}a");
        let bundle_a = bundle_with_steps(
            &id_a,
            vec![
                step(None, None),
                step(Some("Rayari McGrath Research Outpost"), Some("25")),
                step(Some("Rayari McGrath Research Outpost"), Some("\t")),
                step(
                    Some("Rayari McGrath Research Outpost"),
                    Some("15x Amioshi Plague"),
                ),
            ],
        );
        let contract_a = NewContract::from_bundle(&bundle_a).expect("valid bundle");
        let (a_step_count, a_first_loc, a_required_item) = (
            contract_a.step_count,
            contract_a.first_step_location.clone(),
            contract_a.required_item.clone(),
        );
        store
            .upsert(contract_a)
            .await
            .unwrap_or_else(|e| panic!("seed {id_a}: {e}"));

        // Row `b`: no leading-form step at all, so the TRAILING form
        // ("Valakkar Fang (Juvenile) x25") is the first non-empty
        // stripped candidate instead.
        let id_b = format!("{prefix}b");
        let bundle_b = bundle_with_steps(
            &id_b,
            vec![
                step(None, None),
                step(Some("Rayari McGrath Research Outpost"), Some("25")),
                step(Some("Rayari McGrath Research Outpost"), Some("\t")),
                step(None, Some("Valakkar Fang (Juvenile) x25")),
            ],
        );
        let contract_b = NewContract::from_bundle(&bundle_b).expect("valid bundle");
        let (b_step_count, b_first_loc, b_required_item) = (
            contract_b.step_count,
            contract_b.first_step_location.clone(),
            contract_b.required_item.clone(),
        );
        store
            .upsert(contract_b)
            .await
            .unwrap_or_else(|e| panic!("seed {id_b}: {e}"));

        assert_backfill_parity(&pool, &id_a, a_step_count, a_first_loc, a_required_item).await;
        assert_backfill_parity(&pool, &id_b, b_step_count, b_first_loc, b_required_item).await;

        clear_scoped_rows(&pool, prefix).await;
    }
}
