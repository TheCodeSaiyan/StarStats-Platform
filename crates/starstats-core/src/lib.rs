//! `starstats-core` — wire types, log parser, validators shared by the
//! tray client and the API server.
//!
//! Design rule: this crate must compile on every platform we ship to
//! (Win, Linux, macOS) and depend on **no** runtime / framework crates.
//! It's pure types + functions. Anything async or I/O lives in the
//! consuming crates.

pub mod character_life;
pub mod cohort;
pub mod contract_life;
pub mod contract_taxonomy;
pub mod events;
pub mod inference;
pub mod inference_defs;
pub mod location_catalog;
pub mod location_classifier;
pub mod location_taxonomy;
pub mod metadata;
pub mod parser;
pub mod parser_defs;
pub mod peer_group;
pub mod stats;
pub mod templates;
pub mod transactions;
pub mod unknown_lines;
pub mod validators;
pub mod wire;

pub use character_life::{derive_lives, Life, LifeConfig, LifeEnd, LifeSummary};
pub use contract_life::{
    derive_contract_runs, ClosedBy, ContractConfig, ContractRun, ContractState, ContractStep,
    StepState,
};
pub use location_catalog::{LocationCatalog, LocationCatalogEntry};
pub use location_classifier::{
    classify as classify_location, ClassificationSource, LocationClassification,
};
pub use location_taxonomy::{
    parse_categories_to_taxonomy, slug_from_page_title, LocationTaxonomy, LocationTier, Placement,
};

pub use events::{
    ActorDeath, AttachmentReceived, BurstSummary, ChangeServer, CommodityBuyRequest,
    CommoditySellRequest, EquipAction, GameCrash, GameEvent, HudNotification, ItemEquipChange,
    JoinPu, LauncherActivity, LauncherCategory, LegacyLogin, LocationChanged,
    LocationInventoryRequested, MissionEnd, MissionMarkerKind, MissionObjective,
    MissionObjectiveState, MissionQuantumDestinationSelected, MissionStart, PlanetTerrainLoad,
    PlayerDeath, PlayerIncapacitated, ProcessInit, QuantumRoute, QuantumTargetPhase,
    QuantumTargetSelected, RemoteMatch, ResolveSpawn, SeedSolarSystem, ServerPhase, SessionEnd,
    SessionEndKind, ShopBuyRequest, ShopFlowResponse, ShopRequestTimedOut,
    TravelToContractLocation, VehicleDestruction, VehicleStowed,
};
pub use inference::{
    built_in_inference_rules, infer, infer_with_rules, CompiledInferenceRule, InferenceConfig,
    InferenceMatch, InferredEvent,
};
pub use inference_defs::{
    compile_inference_rules, EventPattern, EventTemplate, InferenceCompileError,
    RemoteInferenceRule,
};
pub use metadata::{
    all_event_type_keys, event_type_key, group_key_for, primary_entity_for,
    provenance_for_inferred_field, stamp, EntityKind, EntityRef, EventMetadata, EventSource,
    FieldProvenance,
};
pub use parser::{
    classify, classify_launcher_message, classify_or_capture, classify_with_metadata,
    parse_launcher_line, structural_parse, ClassifyOutcome, LauncherLogLine, LogLine, ParseStats,
};
pub use parser_defs::{
    apply_remote_rules, compile_rules, CompiledRemoteRule, Manifest, RemoteRule, RuleMatchKind,
};
pub use transactions::{pair_transactions, Transaction, TransactionKind, TransactionStatus};
pub use unknown_lines::{
    capture, coarse_shape_hash, coarse_shape_of, detect_pii, interest_score, is_garbage_line,
    shape_hash, shape_of, CaptureContextOwned, InterestContext, PiiKind, PiiToken, UnknownLine,
    GARBAGE_LINE_MARKERS,
};
pub use validators::{validate_event, validate_metadata, ValidationError};
pub use wire::{
    ContextExample, EventEnvelope, IngestBatch, LogSource, ParserSubmission, ParserSubmissionBatch,
    ParserSubmissionResponse,
};
