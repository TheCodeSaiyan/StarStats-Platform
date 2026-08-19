//! `POST /v1/ingest` — accepts a [`IngestBatch`] from the desktop
//! client. Validates schema, normalises, and writes via [`EventStore`].
//!
//! Authentication: extracts an [`AuthenticatedUser`] from the bearer
//! token. The batch's `claimed_handle` must match the token's
//! `preferred_username` (case-insensitive) — clients can't push
//! events under another user's handle.

use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::{AuthenticatedUser, TokenType};
use crate::devices::DeviceStore;
use crate::repo::{from_envelope, EventStore, InsertOutcome, QuarantinedEvent};
use crate::restriction_guard::{Ingest, RequireUnrestricted};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    Extension,
};
use metrics::{counter, histogram};
use serde::{Deserialize, Serialize};
use serde_json::json;
use starstats_core::metadata::stamp;
use starstats_core::validators::validate_event;
use starstats_core::wire::IngestBatch;
use std::sync::Arc;
use std::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

/// What a device's newly-arrived `batch_sequence` implies relative to the
/// highest ordinal previously accepted from that device (F7). Pure so the
/// gap logic is unit-testable without a store or HTTP round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchSequenceObservation {
    /// First batch ever seen from this device — nothing to compare.
    FirstSeen,
    /// Exactly one past the last — the contiguous happy path.
    InOrder,
    /// A forward jump: `missing` ordinals never arrived (uploads lost or
    /// dropped, or the device drained against another server).
    Gap { missing: u64 },
    /// `seq <= prev` — an out-of-order / retried arrival or a client whose
    /// counter reset. Not data loss on its own, but worth surfacing.
    Regression { prev: i64 },
}

/// Classify an incoming ordinal `seq` against a device's prior high-water
/// mark (`None` = first batch seen). Pure and total.
fn classify_batch_sequence(prev: Option<i64>, seq: i64) -> BatchSequenceObservation {
    match prev {
        None => BatchSequenceObservation::FirstSeen,
        Some(prev) if seq == prev + 1 => BatchSequenceObservation::InOrder,
        Some(prev) if seq > prev + 1 => BatchSequenceObservation::Gap {
            missing: (seq - prev - 1) as u64,
        },
        Some(prev) => BatchSequenceObservation::Regression { prev },
    }
}

/// Parse a bare `major.minor.patch` version (the collector's
/// `CARGO_PKG_VERSION`) into a comparable tuple. Tolerates a pre-release
/// suffix on the patch (`"37-alpha.1"` → 37) and returns `None` for
/// anything it can't confidently read.
fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

/// Is a batch's `collector_version` older than the configured minimum?
/// Absent or unparseable versions are NOT flagged — we only flag a
/// collector we can confidently compare (legacy unversioned clients are
/// left to auto-update rather than spammed as "outdated").
fn collector_below_min(min: (u32, u32, u32), collector_version: Option<&str>) -> bool {
    matches!(collector_version.and_then(parse_semver), Some(v) if v < min)
}

/// Minimum supported collector version, read once from
/// `STARSTATS_MIN_COLLECTOR_VERSION` (bare semver). Unset → `None` = the
/// gate is disabled (the default; zero behaviour change). Cached so the
/// env read happens once, not per request.
fn min_collector_version() -> Option<(u32, u32, u32)> {
    static MIN: std::sync::OnceLock<Option<(u32, u32, u32)>> = std::sync::OnceLock::new();
    *MIN.get_or_init(|| {
        std::env::var("STARSTATS_MIN_COLLECTOR_VERSION")
            .ok()
            .and_then(|s| parse_semver(&s))
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IngestResponse {
    pub batch_id: String,
    pub accepted: u32,
    pub duplicate: u32,
    pub rejected: u32,
}

/// OpenAPI-only mirror of `starstats_core::wire::IngestBatch`. The
/// real type lives in `starstats-core` and we deliberately don't
/// touch that crate (it's also used by the desktop client and we
/// don't want a `utoipa` dep leaking down). The shapes match field
/// for field; the `GameEvent` tagged enum is mirrored variant-by-variant
/// in `GameEventSchema` below so the generated client sees the full
/// discriminated union.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct IngestBatchSchema {
    pub schema_version: u16,
    pub batch_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_build: Option<String>,
    pub claimed_handle: String,
    pub events: Vec<EventEnvelopeSchema>,
}

/// Schema-only mirror of `starstats_core::wire::EventEnvelope`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct EventEnvelopeSchema {
    pub idempotency_key: String,
    pub raw_line: String,
    /// Discriminated union mirroring `starstats_core::events::GameEvent`.
    /// Internally tagged on `type` (snake_case discriminant); every
    /// variant carries at least `timestamp`.
    pub event: Option<GameEventSchema>,
    /// One of: `live`, `ptu`, `eptu`, `hotfix`, `tech`, `other`.
    pub source: String,
    pub source_offset: u64,
    /// Cross-cutting metadata stamped by the client (or by the server
    /// during the schema-v1 grace window). Optional on the wire so
    /// envelopes produced by pre-v2 clients still deserialise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<EventMetadataSchema>,
    /// Fuzzy-resolved location stamped by the tray's sync batcher.
    /// Optional; absent for placeless events and pre-resolution
    /// clients. Mirrors `starstats_core::location_classifier::ResolvedLocation`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_location: Option<ResolvedLocationSchema>,
}

/// Schema-only mirror of
/// `starstats_core::location_classifier::ResolvedLocation`. Kept in
/// sync by hand — if you touch the core type, mirror it here.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ResolvedLocationSchema {
    /// Player-friendly name (catalog display or title-cased fallback).
    pub display_name: String,
    /// Catalog slug — present only on a confident catalog/fuzzy hit.
    /// When present, the web links to `/kb/location/{slug}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Canonical system display, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Location tier — always present.
    pub tier: LocationTierSchema,
    /// Where the binding came from.
    pub source: ClassificationSourceSchema,
}

/// Schema-only mirror of `starstats_core::location_taxonomy::LocationTier`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum LocationTierSchema {
    System,
    AstronomicalObject,
    LandingZone,
    SpaceStation,
    Landmark,
    Flotilla,
    NavalBase,
    AnonymousPoi,
}

/// Schema-only mirror of
/// `starstats_core::location_classifier::ClassificationSource`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ClassificationSourceSchema {
    Catalog,
    Fuzzy,
    Synthetic,
    Heuristic,
    Fallback,
}

// ---------------------------------------------------------------------
// GameEvent schema mirror
//
// `starstats_core::events::GameEvent` is a 29-variant tagged enum that
// doesn't (and shouldn't) carry a `utoipa` dep. We mirror it here in
// the server crate so the OpenAPI spec — and through it the generated
// TS client — exposes the full discriminated union instead of an
// opaque `Object`. Field shapes match the core types exactly; if you
// touch a core variant, mirror the change here too.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ServerPhaseSchema {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum QuantumTargetPhaseSchema {
    FuelRequested,
    Selected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum SessionEndKindSchema {
    SystemQuit,
    FastShutdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum MissionMarkerKindSchema {
    Phase,
    Objective,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum LauncherCategorySchema {
    Auth,
    Install,
    Patch,
    Update,
    Error,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ProcessInitSchema {
    pub timestamp: String,
    pub local_session: String,
    pub env_session: String,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct LegacyLoginSchema {
    pub timestamp: String,
    pub handle: String,
    pub server_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct JoinPuSchema {
    pub timestamp: String,
    pub address: String,
    pub port: u16,
    pub shard: String,
    pub location_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ChangeServerSchema {
    pub timestamp: String,
    pub phase: ServerPhaseSchema,
    pub is_shard_persisted: bool,
    pub is_server: bool,
    pub is_multiplayer: bool,
    pub is_online: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SeedSolarSystemSchema {
    pub timestamp: String,
    pub solar_system: String,
    pub shard: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ResolveSpawnSchema {
    pub timestamp: String,
    pub player_geid: String,
    pub fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ActorDeathSchema {
    pub timestamp: String,
    pub victim: String,
    pub victim_geid: Option<String>,
    pub zone: String,
    pub killer: String,
    pub killer_geid: Option<String>,
    pub weapon: String,
    pub damage_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct PlayerDeathSchema {
    pub timestamp: String,
    pub body_class: String,
    pub body_id: String,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct PlayerIncapacitatedSchema {
    pub timestamp: String,
    pub queue_id: u64,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct VehicleDestructionSchema {
    pub timestamp: String,
    pub vehicle_class: String,
    pub vehicle_id: Option<String>,
    pub destroy_level: u8,
    pub caused_by: String,
    pub zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct HudNotificationSchema {
    pub timestamp: String,
    pub text: String,
    pub notification_id: u64,
    pub mission_id: Option<String>,
    /// Trays predating this release don't send this field; default to
    /// `None` so their posts keep ingesting instead of failing.
    #[serde(default)]
    pub objective_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct LocationInventoryRequestedSchema {
    pub timestamp: String,
    pub player: String,
    pub location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct PlanetTerrainLoadSchema {
    pub timestamp: String,
    pub planet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct QuantumTargetSelectedSchema {
    pub timestamp: String,
    pub phase: QuantumTargetPhaseSchema,
    pub vehicle_class: String,
    pub vehicle_id: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct AttachmentReceivedSchema {
    pub timestamp: String,
    pub player: String,
    pub item_class: String,
    pub item_id: String,
    pub status: String,
    pub port: String,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct VehicleStowedSchema {
    pub timestamp: String,
    pub vehicle_id: String,
    pub landing_area: String,
    pub landing_area_id: String,
    pub zone_host_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct GameCrashSchema {
    pub timestamp: String,
    pub channel: String,
    pub crash_dir_name: String,
    pub primary_log_name: Option<String>,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct LauncherActivitySchema {
    pub timestamp: String,
    pub level: String,
    pub message: String,
    pub category: LauncherCategorySchema,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct MissionStartSchema {
    pub timestamp: String,
    pub mission_id: String,
    pub marker_kind: MissionMarkerKindSchema,
    pub mission_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct MissionEndSchema {
    pub timestamp: String,
    pub mission_id: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ShopBuyRequestSchema {
    pub timestamp: String,
    pub shop_id: Option<String>,
    pub item_class: Option<String>,
    pub quantity: Option<u32>,
    pub raw: String,
    pub price: Option<i64>,
    pub shop_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ShopFlowResponseSchema {
    pub timestamp: String,
    pub shop_id: Option<String>,
    pub success: Option<bool>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct CommodityBuyRequestSchema {
    pub timestamp: String,
    pub commodity: Option<String>,
    pub quantity: Option<f64>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct CommoditySellRequestSchema {
    pub timestamp: String,
    pub commodity: Option<String>,
    pub quantity: Option<f64>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct SessionEndSchema {
    pub timestamp: String,
    pub kind: SessionEndKindSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct RemoteMatchSchema {
    pub timestamp: String,
    pub rule_id: String,
    pub event_name: String,
    pub fields: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct BurstSummarySchema {
    pub timestamp: String,
    pub rule_id: String,
    pub size: u32,
    pub end_timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_body_sample: Option<String>,
    /// Semantic kind of the burst. `"loadout_restore"` for
    /// loadout-restore bursts; absent for other burst types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Per-category item counts for `loadout_restore` bursts.
    /// Keys: `weapons`, `armor`, `attachments`, `consumables`, `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<std::collections::HashMap<String, u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct LocationChangedSchema {
    pub timestamp: String,
    pub from: Option<String>,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ShopRequestTimedOutSchema {
    pub timestamp: String,
    pub shop_id: Option<String>,
    pub item_class: Option<String>,
    pub timed_out_after_secs: u32,
}

/// Discriminated union mirror of `starstats_core::events::GameEvent`.
/// Internally tagged on `type` (snake_case), so every variant
/// serialises as `{ "type": "...", ...variant_fields }` on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum GameEventSchema {
    ProcessInit(ProcessInitSchema),
    LegacyLogin(LegacyLoginSchema),
    JoinPu(JoinPuSchema),
    ChangeServer(ChangeServerSchema),
    SeedSolarSystem(SeedSolarSystemSchema),
    ResolveSpawn(ResolveSpawnSchema),
    ActorDeath(ActorDeathSchema),
    PlayerDeath(PlayerDeathSchema),
    PlayerIncapacitated(PlayerIncapacitatedSchema),
    VehicleDestruction(VehicleDestructionSchema),
    HudNotification(HudNotificationSchema),
    LocationInventoryRequested(LocationInventoryRequestedSchema),
    PlanetTerrainLoad(PlanetTerrainLoadSchema),
    QuantumTargetSelected(QuantumTargetSelectedSchema),
    AttachmentReceived(AttachmentReceivedSchema),
    VehicleStowed(VehicleStowedSchema),
    GameCrash(GameCrashSchema),
    LauncherActivity(LauncherActivitySchema),
    MissionStart(MissionStartSchema),
    MissionEnd(MissionEndSchema),
    ShopBuyRequest(ShopBuyRequestSchema),
    ShopFlowResponse(ShopFlowResponseSchema),
    CommodityBuyRequest(CommodityBuyRequestSchema),
    CommoditySellRequest(CommoditySellRequestSchema),
    SessionEnd(SessionEndSchema),
    RemoteMatch(RemoteMatchSchema),
    BurstSummary(BurstSummarySchema),
    LocationChanged(LocationChangedSchema),
    ShopRequestTimedOut(ShopRequestTimedOutSchema),
}

/// Categorical kind of the primary entity an event is about. Mirrors
/// `starstats_core::metadata::EntityKind` (closed-vocabulary,
/// snake_case on the wire).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum EntityKindSchema {
    Player,
    Vehicle,
    Item,
    Location,
    Shop,
    Mission,
    Session,
    System,
}

/// Where an event came from. Mirrors
/// `starstats_core::metadata::EventSource`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum EventSourceSchema {
    Observed,
    Inferred,
    Synthesized,
}

/// Reference to the primary entity an event is about. Mirrors
/// `starstats_core::metadata::EntityRef`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct EntityRefSchema {
    pub kind: EntityKindSchema,
    /// Stable identifier the timeline can dedupe / group on. The
    /// sentinel value `"unknown"` is used when the source line did
    /// not name one.
    pub id: String,
    /// Human-readable label. May differ from `id` (e.g. mission
    /// title vs UUID).
    pub display_name: String,
}

/// Per-field provenance. Mirrors
/// `starstats_core::metadata::FieldProvenance`. Externally-tagged on
/// `type` with `observed` / `inferred_from` discriminators.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub enum FieldProvenanceSchema {
    Observed,
    InferredFrom {
        source_event_ids: Vec<String>,
        rule_id: String,
    },
}

/// Cross-cutting event metadata. Mirrors
/// `starstats_core::metadata::EventMetadata`. See the core module
/// docs for the design rationale.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct EventMetadataSchema {
    pub primary_entity: EntityRefSchema,
    pub source: EventSourceSchema,
    /// Confidence in `[0.0, 1.0]`. Observed events anchor at `1.0`.
    pub confidence: f32,
    /// Precomputed key used by the timeline to collapse near-duplicate
    /// rows: `"{event_type}:{entity_kind}:{entity_id}"`.
    pub group_key: String,
    /// Per-field provenance map. Omitted from the wire when empty.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub field_provenance: std::collections::BTreeMap<String, FieldProvenanceSchema>,
    /// Idempotency keys of the source events that triggered an
    /// inferred event. Empty for observed / synthesized events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_inputs: Vec<String>,
    /// Rule that produced an inferred event. None for observed /
    /// synthesized events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
}

/// Build the canonical `403 device_sync_disabled` response.
fn reject_sync(detail: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ApiErrorBody {
            error: "device_sync_disabled".into(),
            detail: Some(detail.into()),
        }),
    )
        .into_response()
}

/// Server side of the two-gate cloud-sync model (docs/ENGINEERING.md "two-gate
/// model for cross-device toggles"). A device token may only ingest
/// when its own `devices.sync_enabled` row is `true` — the same gate
/// `preferences_routes::enforce_device_sync_gate` applies to prefs.
/// User tokens fall through unchecked. Returns `Some(response)` when
/// the request must be short-circuited. Without this, a paired device
/// whose sync the user has disabled still silently uploads events,
/// violating the consent model (the local tray gate alone is not
/// authoritative).
async fn enforce_device_sync_gate(
    user: &AuthenticatedUser,
    devices: &Arc<dyn DeviceStore>,
) -> Option<Response> {
    if !matches!(user.token_type, TokenType::Device) {
        return None;
    }
    let Some(device_id) = user.device_id else {
        // Defensive: the auth extractor refuses a device token without
        // the claim, but never 500 silently on this path.
        return Some(reject_sync("device token without device_id claim"));
    };
    let user_id = match Uuid::parse_str(&user.sub) {
        Ok(id) => id,
        Err(_) => {
            return Some(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorBody {
                        error: "bad_subject".into(),
                        detail: None,
                    }),
                )
                    .into_response(),
            );
        }
    };
    match devices.sync_enabled_for(user_id, device_id).await {
        Ok(Some(true)) => None,
        Ok(Some(false)) | Ok(None) => {
            counter!("starstats_ingest_batches_rejected", "reason" => "sync_disabled").increment(1);
            Some(reject_sync(
                "this uplink's sync is disabled — re-enable from the \
                 Connected Uplinks page or from the tray's Cloud sync toggle",
            ))
        }
        Err(e) => {
            tracing::error!(error = ?e, device_id = %device_id, "ingest sync_enabled lookup failed");
            Some(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorBody {
                        error: "internal".into(),
                        detail: None,
                    }),
                )
                    .into_response(),
            )
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/ingest",
    tag = "ingest",
    request_body = IngestBatchSchema,
    responses(
        (status = 200, description = "Batch accepted (may include duplicates)", body = IngestResponse),
        (status = 400, description = "Schema-level rejection", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Handle mismatch, or device sync disabled", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn handle<S: EventStore>(
    State(store): State<Arc<S>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(devices): Extension<Arc<dyn DeviceStore>>,
    guard: RequireUnrestricted<Ingest>,
    Json(mut batch): Json<IngestBatch>,
) -> impl IntoResponse {
    // The guard already extracted and verified the JWT; taking a second
    // `AuthenticatedUser` parameter would verify it twice per request.
    let user = guard.into_user();
    let started = Instant::now();

    // Accept any version in `[1, CURRENT]`. v1 envelopes predate the
    // `metadata` field on `EventEnvelope`; we synthesise observed
    // metadata server-side below so downstream consumers see a
    // uniform shape regardless of client age.
    if batch.schema_version < 1 || batch.schema_version > IngestBatch::CURRENT_SCHEMA_VERSION {
        counter!("starstats_ingest_batches_rejected", "reason" => "bad_schema").increment(1);
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorBody {
                error: "unsupported_schema_version".into(),
                detail: Some(format!(
                    "got {}, server speaks 1..={}",
                    batch.schema_version,
                    IngestBatch::CURRENT_SCHEMA_VERSION
                )),
            }),
        )
            .into_response();
    }

    if batch.claimed_handle.trim().is_empty() {
        counter!("starstats_ingest_batches_rejected", "reason" => "empty_handle").increment(1);
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorBody {
                error: "missing_claimed_handle".into(),
                detail: None,
            }),
        )
            .into_response();
    }

    // Cross-check: the bearer token's preferred_username must match
    // the batch's claimed_handle. Prevents user A from pushing events
    // under user B's handle.
    if !batch
        .claimed_handle
        .eq_ignore_ascii_case(&user.preferred_username)
    {
        counter!("starstats_ingest_batches_rejected", "reason" => "handle_mismatch").increment(1);
        tracing::warn!(
            sub = %user.sub,
            token_handle = %user.preferred_username,
            claimed = %batch.claimed_handle,
            "ingest rejected — handle mismatch"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(ApiErrorBody {
                error: "handle_mismatch".into(),
                detail: Some("claimed_handle does not match authenticated identity".into()),
            }),
        )
            .into_response();
    }

    // Two-gate model: even with a valid token + matching handle, a
    // device whose server-side `sync_enabled` is false must not ingest
    // — the user has withdrawn cloud-sync consent for this uplink.
    if let Some(resp) = enforce_device_sync_gate(&user, &devices).await {
        return resp;
    }

    // Canonicalize the handle to lowercase so events are always stored
    // under a single case regardless of what the client sent.  The
    // cross-check above already ran on the originals (case-insensitive)
    // so it is safe to normalize from here on.
    let canonical_handle = batch.claimed_handle.to_lowercase();

    // Backfill default Observed metadata for any envelope a legacy
    // (v1) client uploaded without it. Newer clients stamp on the
    // wire; the server only synthesises when the field is absent so
    // we never overwrite explicit producer-supplied metadata.
    for env in batch.events.iter_mut() {
        if env.metadata.is_none() {
            if let Some(ev) = &env.event {
                env.metadata = Some(stamp(ev, Some(&canonical_handle)));
            }
        }
    }

    let mut accepted = 0u32;
    let mut duplicate = 0u32;
    let mut rejected_validation = 0u32;
    let mut rejected_insert = 0u32;

    // Validate every envelope first (quarantining rejects), collecting the
    // valid ones for a single set-based insert. Validation is per-envelope and
    // CPU-only; only the surviving events reach the one bulk DB round-trip.
    let mut to_insert = Vec::with_capacity(batch.events.len());
    for envelope in &batch.events {
        // Independent server-side validation. The device is
        // authenticated but its content is untrusted: reject events
        // whose payload or client-supplied metadata violate the
        // documented invariants instead of persisting garbage that
        // would skew downstream metrics. The idempotency_key is logged
        // (not the raw line) so a rejection is diagnosable without
        // spilling potentially sensitive log content into server logs.
        if let Err(e) = validate_event(envelope) {
            tracing::warn!(
                idempotency_key = %envelope.idempotency_key,
                error = %e,
                "event rejected — failed server-side validation"
            );
            // Quarantine the rejected event for out-of-band diagnosis
            // rather than dropping it silently (F5). Best-effort: a
            // quarantine-write failure is logged but never fails the
            // batch — the valid events in it still land.
            let quarantined = QuarantinedEvent {
                id: Uuid::now_v7(),
                idempotency_key: envelope.idempotency_key.clone(),
                claimed_handle: canonical_handle.clone(),
                reason: "validation".to_string(),
                detail: Some(e.to_string()),
                log_source: envelope.source,
                source_offset: envelope.source_offset as i64,
                raw_line: envelope.raw_line.clone(),
                payload: serde_json::to_value(&envelope.event).unwrap_or(serde_json::Value::Null),
            };
            if let Err(qe) = store.quarantine(quarantined).await {
                tracing::warn!(
                    error = %qe,
                    idempotency_key = %envelope.idempotency_key,
                    "quarantine write failed"
                );
            }
            rejected_validation += 1;
            continue;
        }
        to_insert.push(from_envelope(envelope, &canonical_handle));
    }

    // Single-transaction bulk insert of the validated events: one round-trip,
    // one commit, and one in-transaction stat_event_counts rollup upsert,
    // instead of N of each. Outcomes come back in input order.
    let batch_len = to_insert.len();
    match store.insert_batch(to_insert).await {
        Ok(outcomes) => {
            for outcome in outcomes {
                match outcome {
                    InsertOutcome::Inserted => accepted += 1,
                    InsertOutcome::Duplicate => duplicate += 1,
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, count = batch_len, "batch insert failed");
            rejected_insert += batch_len as u32;
        }
    }

    let rejected = rejected_validation + rejected_insert;
    counter!("starstats_events_ingested").increment(accepted as u64);
    counter!("starstats_events_duplicate").increment(duplicate as u64);
    if rejected_validation > 0 {
        counter!("starstats_events_rejected", "reason" => "validation")
            .increment(rejected_validation as u64);
    }
    if rejected_insert > 0 {
        counter!("starstats_events_rejected", "reason" => "insert_error")
            .increment(rejected_insert as u64);
    }
    histogram!("starstats_ingest_batch_duration_seconds").record(started.elapsed().as_secs_f64());

    // F7 compatibility gate (flag-only). When a minimum supported collector
    // version is configured, flag batches from older collectors so an
    // outdated fleet is visible (metric + log) — the `collector_version`
    // stamp (slice 1) makes this comparison possible. Default: no minimum →
    // no-op, zero behaviour change. Rejection is a deliberate follow-up (a
    // product call); this slice observes only.
    if let Some(min) = min_collector_version() {
        if collector_below_min(min, batch.collector_version.as_deref()) {
            counter!("starstats_ingest_collector_outdated").increment(1);
            tracing::warn!(
                collector_version = ?batch.collector_version,
                min = ?min,
                claimed_handle = %batch.claimed_handle,
                "ingest: collector below configured minimum supported version (flag-only)"
            );
        }
    }

    // F7 online gap detection. When a device-scoped batch carries a
    // per-device ordinal, diff it against the device's high-water mark to
    // surface lost (gap) or out-of-order (regression) uploads. Pure
    // observability: a store error is logged and never fails ingest, and
    // user-scoped tokens (no device_id) are skipped entirely.
    if let (Some(device_id), Some(seq)) = (user.device_id.as_ref(), batch.batch_sequence) {
        let seq = seq as i64;
        // Key on the token's device_id claim rendered as a string — the
        // same form the audit payload and `device_batch_progress` use.
        let device_id = device_id.to_string();
        match store.observe_batch_sequence(&device_id, seq).await {
            Ok(prev) => match classify_batch_sequence(prev, seq) {
                BatchSequenceObservation::FirstSeen | BatchSequenceObservation::InOrder => {}
                BatchSequenceObservation::Gap { missing } => {
                    counter!("starstats_ingest_batch_sequence_anomaly", "kind" => "gap")
                        .increment(1);
                    tracing::warn!(
                        device_id = %device_id,
                        batch_sequence = seq,
                        missing,
                        "ingest: batch_sequence gap — upload(s) never arrived from this device"
                    );
                }
                BatchSequenceObservation::Regression { prev } => {
                    counter!("starstats_ingest_batch_sequence_anomaly", "kind" => "regression")
                        .increment(1);
                    tracing::warn!(
                        device_id = %device_id,
                        batch_sequence = seq,
                        prev,
                        "ingest: batch_sequence regression — ordinal <= last seen \
                         (out-of-order, retry, or counter reset)"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    device_id = %device_id,
                    "observe_batch_sequence failed; skipping gap check"
                );
            }
        }
    }

    tracing::info!(
        sub = %user.sub,
        batch_id = %batch.batch_id,
        accepted,
        duplicate,
        rejected,
        total = batch.events.len(),
        "ingest batch processed"
    );

    // Best-effort audit. A failure here is logged but doesn't fail
    // the request — the events themselves already landed. We trade
    // strict atomicity for availability; audit drift is detectable
    // out-of-band (and rare in practice).
    //
    // device_id is server-determined: read off the bearer token's
    // device claim (populated for device-paired tokens by the auth
    // extractor). User-tokens have None here — those rows show up in
    // the account-wide stream and never match a `?device_id=` filter.
    // Storing it inside the audit payload (rather than as a separate
    // column) keeps the hash chain canonical — see migration 0026.
    let audit_entry = AuditEntry {
        actor_sub: Some(user.sub.clone()),
        actor_handle: Some(user.preferred_username.to_lowercase()),
        action: "ingest.batch_processed".to_string(),
        payload: json!({
            "batch_id": batch.batch_id,
            "claimed_handle": batch.claimed_handle,
            "game_build": batch.game_build,
            "collector_version": batch.collector_version,
            "parser_version": batch.parser_version,
            "batch_sequence": batch.batch_sequence,
            "content_hash": batch.content_hash,
            "source_range": batch.source_range,
            "device_id": user.device_id,
            "total": batch.events.len(),
            "accepted": accepted,
            "duplicate": duplicate,
            "rejected": rejected,
        }),
    };
    if let Err(e) = audit.append(audit_entry).await {
        tracing::error!(error = %e, "audit append failed");
    }

    (
        StatusCode::OK,
        Json(IngestResponse {
            batch_id: batch.batch_id,
            accepted,
            duplicate,
            rejected,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use starstats_core::wire::SourceRange;

    #[test]
    fn classify_batch_sequence_covers_first_inorder_gap_regression() {
        use BatchSequenceObservation as O;
        // First batch from a device: nothing to compare.
        assert_eq!(classify_batch_sequence(None, 1), O::FirstSeen);
        // Contiguous: exactly one past the last.
        assert_eq!(classify_batch_sequence(Some(5), 6), O::InOrder);
        // Forward jump: two ordinals (6, 7) never arrived.
        assert_eq!(classify_batch_sequence(Some(5), 8), O::Gap { missing: 2 });
        // Duplicate (seq == prev) and a lower arrival both read as regression.
        assert_eq!(
            classify_batch_sequence(Some(5), 5),
            O::Regression { prev: 5 }
        );
        assert_eq!(
            classify_batch_sequence(Some(5), 3),
            O::Regression { prev: 5 }
        );
    }

    #[test]
    fn collector_compat_parse_and_below_min_gate() {
        // parse_semver: bare + pre-release-suffixed patch + rejects junk.
        assert_eq!(parse_semver("1.8.37"), Some((1, 8, 37)));
        assert_eq!(parse_semver("1.8.37-alpha.2"), Some((1, 8, 37)));
        assert_eq!(parse_semver("garbage"), None);
        assert_eq!(parse_semver("1.8"), None);

        // collector_below_min: strictly-older is flagged; equal / newer /
        // absent / unparseable are not.
        let min = (1, 8, 40);
        assert!(collector_below_min(min, Some("1.8.39")), "older patch");
        assert!(collector_below_min(min, Some("1.7.99")), "older minor");
        assert!(collector_below_min(min, Some("0.9.0")), "older major");
        assert!(
            !collector_below_min(min, Some("1.8.40")),
            "equal is not below"
        );
        assert!(
            !collector_below_min(min, Some("1.9.0")),
            "newer is not below"
        );
        assert!(
            !collector_below_min(min, None),
            "unversioned is not flagged"
        );
        assert!(
            !collector_below_min(min, Some("nonsense")),
            "unparseable is not flagged"
        );
    }
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::devices::test_support::MemoryDeviceStore;
    use crate::repo::MemoryStore;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::routing::post;
    use axum::Router;
    use starstats_core::events::{GameEvent, JoinPu};
    use starstats_core::wire::{EventEnvelope, LogSource};
    use tower::ServiceExt;

    const HANDLE: &str = "TheCodeSaiyan";

    struct TestEnv {
        issuer: TokenIssuer,
        verifier: Arc<AuthVerifier>,
    }

    fn test_env() -> TestEnv {
        let (issuer, verifier) = fresh_pair();
        TestEnv {
            issuer,
            verifier: Arc::new(verifier),
        }
    }

    fn sign_token(issuer: &TokenIssuer, username: &str) -> String {
        // `sub` MUST be a UUID: that is what the issuer mints in
        // production (`TokenIssuer::sign_user`), and both the admin
        // role gate and the restriction guard parse it as one, denying
        // when they cannot. This helper previously signed
        // `user-{username}`, which no real token ever looks like — the
        // restriction guard is what surfaced it. Derived from the
        // username so a given test user keeps a stable id across calls.
        let sub = Uuid::new_v5(&Uuid::NAMESPACE_OID, username.as_bytes());
        issuer
            .sign_user(&sub.to_string(), username)
            .expect("sign user token")
    }

    fn router(
        store: Arc<MemoryStore>,
        verifier: Arc<AuthVerifier>,
        audit: Arc<MemoryAuditLog>,
    ) -> Router {
        // Existing tests use user tokens, which fall through the sync
        // gate — but the `Extension<Arc<dyn DeviceStore>>` extractor on
        // `handle` still needs *a* device store installed, so seed an
        // empty one.
        router_with_devices(store, verifier, audit, Arc::new(MemoryDeviceStore::new()))
    }

    fn router_with_devices(
        store: Arc<MemoryStore>,
        verifier: Arc<AuthVerifier>,
        audit: Arc<MemoryAuditLog>,
        devices: Arc<MemoryDeviceStore>,
    ) -> Router {
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let devices_dyn: Arc<dyn DeviceStore> = devices;
        Router::new()
            .route("/v1/ingest", post(handle::<MemoryStore>))
            .layer(Extension(verifier))
            .layer(Extension({
                // Restriction-gated routes deny when this extension is
                // absent, so every test router that mounts one needs an
                // (unrestricted) store. That is the guard being loud
                // about a wiring gap rather than silently permitting.
                let s: std::sync::Arc<dyn crate::account_restrictions::AccountRestrictionStore> =
                    std::sync::Arc::new(
                        crate::account_restrictions::test_support::MemoryAccountRestrictionStore::new(),
                    );
                s
            }))
            .layer(Extension(audit_dyn))
            .layer(Extension(devices_dyn))
            .with_state(store)
    }

    fn sample_envelope(key: &str) -> EventEnvelope {
        EventEnvelope {
            idempotency_key: key.into(),
            raw_line: "<2026-05-02T21:14:23.189Z> ...".into(),
            event: Some(GameEvent::JoinPu(JoinPu {
                timestamp: "2026-05-02T21:14:23.189Z".into(),
                address: "1.2.3.4".into(),
                port: 64300,
                shard: "pub_euw1b".into(),
                location_id: "562954248454145".into(),
            })),
            source: LogSource::Live,
            source_offset: 1234,
            metadata: None,
            resolved_location: None,
        }
    }

    fn batch(events: Vec<EventEnvelope>) -> IngestBatch {
        IngestBatch {
            schema_version: IngestBatch::CURRENT_SCHEMA_VERSION,
            batch_id: "01934f5a-3b2a-7000-a000-000000000000".into(),
            game_build: Some("4.7.178".into()),
            collector_version: Some("1.8.60".into()),
            parser_version: Some(7),
            batch_sequence: Some(5),
            content_hash: Some("00000000-0000-0000-0000-00000000c0de".into()),
            source_range: Some(SourceRange {
                source: LogSource::Live,
                start_offset: 10,
                end_offset: 20,
            }),
            claimed_handle: HANDLE.into(),
            events,
        }
    }

    async fn post_batch_with(
        router: &Router,
        token: Option<&str>,
        body: &IngestBatch,
    ) -> (StatusCode, axum::body::Bytes) {
        let mut req = Request::builder()
            .method("POST")
            .uri("/v1/ingest")
            .header("content-type", "application/json");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let req = req
            .body(axum::body::Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, bytes)
    }

    /// Router whose restriction store blocks `username` from ingesting.
    ///
    /// Deliberately mirrors `router()` rather than reusing it, because
    /// the whole point is to vary the ONE extension under test.
    fn router_with_restriction(
        store: Arc<MemoryStore>,
        verifier: Arc<AuthVerifier>,
        audit: Arc<MemoryAuditLog>,
        username: &str,
        restriction: crate::account_restrictions::Restriction,
    ) -> Router {
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let devices_dyn: Arc<dyn DeviceStore> = Arc::new(MemoryDeviceStore::new());
        let user_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, username.as_bytes());
        let restrictions: Arc<dyn crate::account_restrictions::AccountRestrictionStore> = Arc::new(
            crate::account_restrictions::test_support::MemoryAccountRestrictionStore::new()
                .with_restriction(user_id, restriction),
        );
        Router::new()
            .route("/v1/ingest", post(handle::<MemoryStore>))
            .layer(Extension(verifier))
            .layer(Extension(restrictions))
            .layer(Extension(audit_dyn))
            .layer(Extension(devices_dyn))
            .with_state(store)
    }

    fn blocking(ingest: bool, sharing: bool) -> crate::account_restrictions::Restriction {
        crate::account_restrictions::Restriction {
            ingest_blocked: ingest,
            sharing_blocked: sharing,
            public_profile_blocked: false,
            submissions_blocked: false,
            reason: "uploading junk".into(),
            restricted_by: "modhandle".into(),
            restricted_at: chrono::Utc::now(),
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn ingest_blocked_user_is_refused_at_the_route() {
        // Asserts the 403 AT THE ROUTE, not that a flag was written.
        // The bug this whole feature replaces wrote its flag perfectly.
        let store = Arc::new(MemoryStore::default());
        let (issuer, verifier) = crate::auth::test_support::fresh_pair();
        let app = router_with_restriction(
            store.clone(),
            Arc::new(verifier),
            Arc::new(MemoryAuditLog::default()),
            HANDLE,
            blocking(true, false),
        );
        let token = sign_token(&issuer, HANDLE);
        let (status, bytes) =
            post_batch_with(&app, Some(&token), &batch(vec![sample_envelope("evt-r1")])).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "account_restricted");
        assert_eq!(body["capability"], "ingest");
        // And nothing was written: a guard that 403s but still persists
        // would be worse than no guard.
        assert_eq!(store.snapshot().len(), 0);
    }

    #[tokio::test]
    async fn a_sharing_only_restriction_does_not_block_ingest() {
        // This is what makes "limit" different from "suspend". If the
        // guard ignored its capability, every targeted limit would
        // silently become a full suspension.
        let store = Arc::new(MemoryStore::default());
        let (issuer, verifier) = crate::auth::test_support::fresh_pair();
        let app = router_with_restriction(
            store.clone(),
            Arc::new(verifier),
            Arc::new(MemoryAuditLog::default()),
            HANDLE,
            blocking(false, true),
        );
        let token = sign_token(&issuer, HANDLE);
        let (status, _) =
            post_batch_with(&app, Some(&token), &batch(vec![sample_envelope("evt-r2")])).await;
        assert_eq!(status, StatusCode::OK);
    }

    fn parse_response(bytes: &[u8]) -> IngestResponse {
        serde_json::from_slice(bytes).unwrap()
    }

    /// Mint a device JWT for `user_id` whose `preferred_username`
    /// matches the batch handle (so the handle check passes and we
    /// actually exercise the sync gate).
    fn sign_device_token(issuer: &TokenIssuer, user_id: Uuid, device_id: Uuid) -> String {
        issuer
            .sign_device(&user_id.to_string(), HANDLE, device_id)
            .expect("sign device token")
    }

    #[tokio::test]
    async fn rejects_device_token_when_sync_disabled() {
        // Two-gate model: a paired device defaults to sync_enabled=false
        // (migration 0036). Ingest from such a device must be refused —
        // without the gate this returned 200 and silently persisted.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let audit = Arc::new(MemoryAuditLog::default());
        let devices = Arc::new(MemoryDeviceStore::new());
        let user_id = Uuid::new_v4();
        let device_id = devices
            .seed_paired_device(user_id, "Test PC")
            .await
            .unwrap();
        // sync_enabled left at its false default.
        let token = sign_device_token(&env.issuer, user_id, device_id);
        let app = router_with_devices(store.clone(), env.verifier, audit, devices);

        let body = batch(vec![sample_envelope("evt-1")]);
        let (status, bytes) = post_batch_with(&app, Some(&token), &body).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err["error"], "device_sync_disabled");
        assert_eq!(
            store.snapshot().len(),
            0,
            "no events must be persisted when the sync gate rejects"
        );
    }

    #[tokio::test]
    async fn accepts_device_token_when_sync_enabled() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let audit = Arc::new(MemoryAuditLog::default());
        let devices = Arc::new(MemoryDeviceStore::new());
        let user_id = Uuid::new_v4();
        let device_id = devices
            .seed_paired_device(user_id, "Test PC")
            .await
            .unwrap();
        devices
            .set_sync_enabled(user_id, device_id, true)
            .await
            .unwrap();
        let token = sign_device_token(&env.issuer, user_id, device_id);
        let app = router_with_devices(store.clone(), env.verifier, audit, devices);

        let body = batch(vec![sample_envelope("evt-1")]);
        let (status, bytes) = post_batch_with(&app, Some(&token), &body).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(parse_response(&bytes).accepted, 1);
        assert_eq!(store.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn rejects_invalid_events_and_persists_only_valid_ones() {
        // The server independently validates collector content — the
        // device is authenticated but its payload is untrusted. A batch
        // mixing a valid event with an invalid one (empty shard) must
        // accept the valid event and reject the invalid one WITHOUT
        // persisting it, rather than trusting the collector field-for-field.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit);

        let invalid = EventEnvelope {
            idempotency_key: "bad-1".into(),
            raw_line: "<2026-05-02T21:14:23.189Z> ...".into(),
            event: Some(GameEvent::JoinPu(JoinPu {
                timestamp: "2026-05-02T21:14:23.189Z".into(),
                address: "1.2.3.4".into(),
                port: 64300,
                shard: String::new(), // empty shard → invalid
                location_id: "1".into(),
            })),
            source: LogSource::Live,
            source_offset: 0,
            metadata: None,
            resolved_location: None,
        };
        let body = batch(vec![sample_envelope("evt-1"), invalid]);
        let (status, bytes) = post_batch_with(&app, Some(&token), &body).await;

        assert_eq!(status, StatusCode::OK);
        let resp = parse_response(&bytes);
        assert_eq!(resp.accepted, 1, "the valid event is persisted");
        assert_eq!(resp.rejected, 1, "the invalid event is rejected");
        assert_eq!(
            store.snapshot().len(),
            1,
            "the invalid event must NOT be persisted"
        );
        // ...but it must be quarantined for diagnosis, not dropped (F5).
        let quarantined = store.quarantined_snapshot();
        assert_eq!(
            quarantined.len(),
            1,
            "the invalid event must be quarantined"
        );
        assert_eq!(quarantined[0].idempotency_key, "bad-1");
        assert_eq!(quarantined[0].reason, "validation");
        assert!(
            quarantined[0].detail.is_some(),
            "quarantine must record why the event was rejected"
        );
    }

    #[tokio::test]
    async fn accepts_valid_batch_and_persists_events() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit.clone());

        let body = batch(vec![sample_envelope("evt-1"), sample_envelope("evt-2")]);
        let (status, bytes) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);
        let resp = parse_response(&bytes);
        assert_eq!(resp.accepted, 2);
        assert_eq!(resp.duplicate, 0);
        assert_eq!(store.snapshot().len(), 2);

        // Audit row written for the batch.
        let audited = audit.snapshot();
        assert_eq!(audited.len(), 1);
        assert_eq!(audited[0].action, "ingest.batch_processed");
        // actor_handle is lowercased at ingest (normalization, migration 0045).
        assert_eq!(
            audited[0].actor_handle.as_deref(),
            Some(HANDLE.to_lowercase().as_str())
        );
        assert_eq!(audited[0].payload["accepted"], 2);
        // Collector release + adopted rule-set are attributed on the
        // audit row so ingested events can be traced back to a tray
        // version AND the parser manifest it was running (F7 — parser
        // -regression triage).
        assert_eq!(audited[0].payload["collector_version"], "1.8.60");
        assert_eq!(audited[0].payload["parser_version"], 7);
        // Per-device batch ordinal is captured too, so gaps / out-of-order
        // uploads from a device are observable in the audit chain (F7).
        assert_eq!(audited[0].payload["batch_sequence"], 5);
        // Batch content hash + byte-coverage range are captured for
        // dedup/replay + coverage forensics (F7).
        assert_eq!(
            audited[0].payload["content_hash"],
            "00000000-0000-0000-0000-00000000c0de"
        );
        assert_eq!(audited[0].payload["source_range"]["start_offset"], 10);
        assert_eq!(audited[0].payload["source_range"]["end_offset"], 20);
    }

    #[tokio::test]
    async fn dedupes_by_idempotency_key_per_handle() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit.clone());

        let body = batch(vec![sample_envelope("evt-1")]);
        let (_, first) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(parse_response(&first).accepted, 1);

        let (_, second) = post_batch_with(&app, Some(&token), &body).await;
        let resp = parse_response(&second);
        assert_eq!(resp.accepted, 0);
        assert_eq!(resp.duplicate, 1);
        assert_eq!(store.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn rejects_unknown_schema_version() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store, env.verifier, audit);

        let mut bad = batch(vec![sample_envelope("evt-1")]);
        bad.schema_version = 999;
        let (status, _) = post_batch_with(&app, Some(&token), &bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_above_window_schema_version() {
        // 99 is outside [1, CURRENT]; must reject.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store, env.verifier, audit);

        let mut bad = batch(vec![sample_envelope("evt-1")]);
        bad.schema_version = 99;
        let (status, _) = post_batch_with(&app, Some(&token), &bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn accepts_legacy_v1_schema_version() {
        // schema_version=1 predates the metadata field; the server
        // must still accept it (within the [1, CURRENT] window) and
        // synthesise metadata server-side. Verified separately below.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit);

        let mut body = batch(vec![sample_envelope("evt-v1")]);
        body.schema_version = 1;
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(store.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn ingest_synthesises_metadata_for_v1_envelopes_missing_it() {
        // A v1 client uploads an envelope with `metadata = None`. The
        // server must backfill default Observed metadata so downstream
        // consumers see a uniform shape.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit);

        let envelope = sample_envelope("evt-synth");
        // Sanity-check the test fixture: it must start without metadata
        // so the synthesis path is exercised.
        assert!(envelope.metadata.is_none());
        let mut body = batch(vec![envelope]);
        body.schema_version = 1;
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);

        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        let meta = rows[0]
            .metadata
            .as_ref()
            .expect("server must synthesise metadata for v1 envelopes");
        assert_eq!(meta.source, starstats_core::metadata::EventSource::Observed);
        assert!((meta.confidence - 1.0).abs() < f32::EPSILON);
        // JoinPu's primary entity is its shard string (see
        // `primary_entity_for` in starstats-core).
        assert_eq!(
            meta.primary_entity.kind,
            starstats_core::metadata::EntityKind::Session
        );
        assert_eq!(meta.primary_entity.id, "pub_euw1b");
    }

    #[tokio::test]
    async fn ingest_preserves_explicit_metadata_when_present() {
        // A v2 client uploads an envelope with metadata already
        // attached. The server must not overwrite it.
        use starstats_core::metadata::{stamp, EntityKind};
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit);

        let mut envelope = sample_envelope("evt-explicit");
        let preset = stamp(
            envelope.event.as_ref().unwrap(),
            Some("ExplicitlyDifferentHandle"),
        );
        let expected_id = preset.primary_entity.id.clone();
        envelope.metadata = Some(preset);
        let body = batch(vec![envelope]);
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);

        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        let meta = rows[0].metadata.as_ref().expect("metadata must round-trip");
        // The preset entity id wins over the claimed_handle-derived one
        // — JoinPu maps to its shard not its handle, but the point is
        // that the server respected the supplied metadata.
        assert_eq!(meta.primary_entity.id, expected_id);
        assert_eq!(meta.primary_entity.kind, EntityKind::Session);
    }

    #[tokio::test]
    async fn rejects_missing_bearer_token() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store, env.verifier, audit);

        let body = batch(vec![sample_envelope("evt-1")]);
        let (status, _) = post_batch_with(&app, None, &body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_handle_mismatch() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        // Token says "OtherUser", batch claims "TheCodeSaiyan"
        let token = sign_token(&env.issuer, "OtherUser");
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store, env.verifier, audit);

        let body = batch(vec![sample_envelope("evt-1")]);
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_invalid_token_signature() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        // Sign with a foreign issuer the server's verifier doesn't trust.
        let (rogue_issuer, _) = fresh_pair();
        let token = sign_token(&rogue_issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store, env.verifier, audit);

        let body = batch(vec![sample_envelope("evt-1")]);
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn extracts_event_type_from_payload() {
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit.clone());

        let body = batch(vec![sample_envelope("evt-1")]);
        let _ = post_batch_with(&app, Some(&token), &body).await;

        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_type, "join_pu");
        assert!(rows[0].event_timestamp.is_some());
    }

    #[tokio::test]
    async fn user_scoped_token_writes_audit_with_null_device_id() {
        // User-scoped bearer tokens have no device claim; the audit
        // payload's device_id must be JSON null so the per-device
        // filter on /v1/me/ingest-history correctly excludes the row.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE);
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store, env.verifier, audit.clone());

        let body = batch(vec![sample_envelope("evt-u1")]);
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);

        let audited = audit.snapshot();
        assert_eq!(audited.len(), 1);
        // Payload contains the key, set to null.
        assert!(
            audited[0]
                .payload
                .as_object()
                .unwrap()
                .contains_key("device_id"),
            "device_id key must always be present in the audit payload"
        );
        assert!(audited[0].payload["device_id"].is_null());
    }

    #[tokio::test]
    async fn device_scoped_token_writes_audit_with_device_id_string() {
        // Device JWTs carry a `device_id` claim. The ingest handler
        // copies it into the audit payload so the per-device Activity
        // tab can filter on it later.
        use crate::devices::test_support::MemoryDeviceStore;
        use crate::devices::DeviceStore;
        use chrono::Duration as ChronoDuration;

        let store = Arc::new(MemoryStore::new());
        let env = test_env();

        // The auth extractor consults the DeviceStore for device
        // tokens to enforce revocation, so we have to seed a real,
        // active device row before signing the device JWT.
        let device_store = Arc::new(MemoryDeviceStore::new());
        let user_id = uuid::Uuid::new_v4();
        let pairing = device_store
            .create_pairing(user_id, HANDLE, ChronoDuration::minutes(5))
            .await
            .expect("create pairing");
        let redeemed = device_store
            .redeem(&pairing.code)
            .await
            .expect("redeem pairing");
        let device_id = redeemed.device_id;
        // This uplink must clear the server-side sync gate to ingest;
        // the test's focus is the audited device_id, not the gate.
        device_store
            .set_sync_enabled(user_id, device_id, true)
            .await
            .expect("enable sync");

        // Production signs device tokens with sub = user UUID and
        // preferred_username = claimed_handle (device_routes.rs redeem).
        let token = env
            .issuer
            .sign_device(&user_id.to_string(), HANDLE, device_id)
            .expect("sign device token");

        let audit = Arc::new(MemoryAuditLog::default());
        let audit_dyn: Arc<dyn AuditLog> = audit.clone();
        let device_dyn: Arc<dyn DeviceStore> = device_store;
        let app: Router = Router::new()
            .route("/v1/ingest", post(handle::<MemoryStore>))
            .layer(Extension(env.verifier))
            .layer(Extension({
                // Restriction-gated routes deny when this extension is
                // absent, so every test router that mounts one needs an
                // (unrestricted) store. That is the guard being loud
                // about a wiring gap rather than silently permitting.
                let s: std::sync::Arc<dyn crate::account_restrictions::AccountRestrictionStore> =
                    std::sync::Arc::new(
                        crate::account_restrictions::test_support::MemoryAccountRestrictionStore::new(),
                    );
                s
            }))
            .layer(Extension(audit_dyn))
            .layer(Extension(device_dyn))
            .with_state(store);

        let body = batch(vec![sample_envelope("evt-d1")]);
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);

        let audited = audit.snapshot();
        assert_eq!(audited.len(), 1);
        assert_eq!(audited[0].action, "ingest.batch_processed");
        assert_eq!(
            audited[0].payload["device_id"].as_str(),
            Some(device_id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn claimed_handle_is_lowercased_at_ingest() {
        // PART 1 normalization: the stored event's claimed_handle must
        // be lowercase even when the batch sends mixed-case.
        // token's preferred_username also mixed-case so the cross-check
        // passes (it's case-insensitive), but the stored value must be
        // canonical lowercase.
        let store = Arc::new(MemoryStore::new());
        let env = test_env();
        let token = sign_token(&env.issuer, HANDLE); // "TheCodeSaiyan"
        let audit = Arc::new(MemoryAuditLog::default());
        let app = router(store.clone(), env.verifier, audit.clone());

        // batch() uses HANDLE = "TheCodeSaiyan" as claimed_handle.
        let body = batch(vec![sample_envelope("evt-lower-1")]);
        let (status, _) = post_batch_with(&app, Some(&token), &body).await;
        assert_eq!(status, StatusCode::OK);

        let rows = store.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].claimed_handle,
            HANDLE.to_lowercase(),
            "stored claimed_handle must be lowercase regardless of what the client sent"
        );

        // Audit actor_handle must also be lowercase.
        let audited = audit.snapshot();
        assert_eq!(audited.len(), 1);
        assert_eq!(
            audited[0].actor_handle.as_deref(),
            Some(HANDLE.to_lowercase().as_str()),
            "audit actor_handle must be lowercase"
        );
    }
}
