//! Wire-format types shared by client and server. Anything that
//! crosses the network lives here.
//!
//! Stability rule: once a field is on the wire, **never remove or
//! repurpose it**. Add new optional fields. Bump `schema_version` on
//! breaking changes (none planned for v1).

use crate::events::GameEvent;
use crate::location_classifier::ResolvedLocation;
use crate::metadata::EventMetadata;
use serde::{Deserialize, Serialize};

/// Single event with the metadata the server needs for dedupe and
/// trust scoring.
///
/// `Eq` is dropped because `GameEvent::AttachmentReceived` carries an
/// `f64` for elapsed seconds, which only implements `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Stable event ID — derived by the client from `(line_offset, content)`
    /// so replays of the same line produce the same ID. UUIDv7 preferred.
    pub idempotency_key: String,

    /// Raw log line as it appeared in `Game.log`. Kept so the server
    /// can re-parse with newer rules without asking the client to
    /// re-upload.
    pub raw_line: String,

    /// Parsed event, if the client could parse it. May be `None` for
    /// lines the client recognised structurally but couldn't classify.
    pub event: Option<GameEvent>,

    /// Path of the source `Game.log` (relative to install root) — used
    /// to distinguish `LIVE/` from `PTU/` from `EPTU/` etc.
    pub source: LogSource,

    /// Byte offset within the source file. Lets the server reconstruct
    /// ordering even across out-of-order batch arrivals.
    pub source_offset: u64,

    /// Cross-cutting metadata stamped by the client (or by the server
    /// during the schema-v1 grace window). Optional on the wire so
    /// envelopes produced by pre-v2 clients still deserialise; the
    /// server back-fills a default observed metadata in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<EventMetadata>,

    /// Fuzzy-resolved location stamped by the tray's sync batcher from
    /// the shared classifier, so tray + web render identical
    /// resolution. `None` for placeless events and for envelopes from
    /// pre-resolution clients; optional + skip-on-None on the wire so
    /// those still deserialise. The server persists this verbatim to
    /// `events.resolved_location JSONB` (migration 0041) and echoes it
    /// back on the event-read endpoints. See
    /// [`crate::location_classifier::ResolvedLocation`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_location: Option<ResolvedLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    Live,
    Ptu,
    Eptu,
    Hotfix,
    Tech,
    Other,
}

/// One client → server batch. Compressed (zstd) on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestBatch {
    /// Schema version — server rejects unknown versions with 400.
    pub schema_version: u16,

    /// Unique batch ID for tracing / dedupe.
    pub batch_id: String,

    /// Game build the events came from (from `<Init>` or `FileVersion`
    /// banner). Lets the server route to the correct parser revision
    /// when the spec drifts between patches.
    pub game_build: Option<String>,

    /// Version of the collector (tray) that produced this batch —
    /// `CARGO_PKG_VERSION`. Lets the server attribute ingested events to
    /// a collector release for parser-regression detection and future
    /// compatibility gating. `#[serde(default)]` so batches from
    /// pre-versioning clients (and any pinned test fixtures) still
    /// deserialise, with `None`.
    #[serde(default)]
    pub collector_version: Option<String>,

    /// Version of the remote parser-definition manifest the collector
    /// had adopted when it drained this batch (see
    /// [`crate::parser_defs::Manifest::version`]). Distinguishes two
    /// collectors on the same [`Self::collector_version`] but different
    /// adopted rule-sets — the axis that matters once the unknown-line
    /// loop starts publishing rules. `None` = the collector has fetched
    /// no manifest yet, or is a pre-versioning client. `#[serde(default)]`
    /// keeps it back-compat.
    #[serde(default)]
    pub parser_version: Option<u32>,

    /// Per-device monotonic batch counter — the ordinal of this upload in
    /// the sequence of batches this collector install has successfully
    /// sent. Lets the server detect **missing** (gap) or **out-of-order**
    /// uploads from a device — the axis F7 calls out that neither
    /// [`Self::collector_version`] nor [`Self::parser_version`] covers.
    /// Assigned optimistically and committed only on a 2xx, so a retried
    /// or poison-bisected send reuses its number rather than burning it
    /// (no false gaps); the only residual is an occasional duplicate when
    /// the priority and bulk lanes race, which is benign (distinct
    /// `batch_id`s, events dedupe). `None` = a pre-versioning client (or a
    /// pinned fixture). `#[serde(default)]` keeps it back-compat.
    #[serde(default)]
    pub batch_sequence: Option<u64>,

    /// Content-address of this batch's event set — a UUIDv5 over the
    /// sorted event idempotency keys (see the client's `build_batch`).
    /// Order-independent (a re-drain that reorders events hashes
    /// identically) and stable across machines/toolchains, giving the
    /// server a batch-level dedup / replay + integrity signal beyond
    /// per-event idempotency. `None` = pre-versioning client (or fixture).
    #[serde(default)]
    pub content_hash: Option<String>,

    /// Byte span of the single log source this batch's events cover, when
    /// the batch is single-source (the common live-tail case). `None` for
    /// a mixed-source batch — a drain batches by event *type*, so it can
    /// span the live tail and the launcher log, whose `source_offset`s
    /// reset per file and aren't comparable — and for pre-versioning
    /// clients. Byte-level coverage provenance, complementing the
    /// upload-level [`Self::batch_sequence`].
    #[serde(default)]
    pub source_range: Option<SourceRange>,

    /// Player handle — claimed by the client. Server cross-checks
    /// against the bearer token's identity claims; mismatch → reject.
    pub claimed_handle: String,

    pub events: Vec<EventEnvelope>,
}

impl IngestBatch {
    /// Bumped to 2 when `EventEnvelope.metadata` was added. The server
    /// accepts both v1 (no metadata, synthesised server-side) and v2
    /// during the grace window described in the design spec.
    pub const CURRENT_SCHEMA_VERSION: u16 = 2;
}

/// The contiguous byte span of ONE log source that a batch's events
/// cover — `start_offset..=end_offset` within `source`. Stamped by the
/// client only for single-source batches (mixed-source batches omit it,
/// since offsets aren't comparable across files). Read-only provenance
/// for byte-level coverage analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRange {
    pub source: LogSource,
    pub start_offset: u64,
    pub end_offset: u64,
}

/// One owned ship pulled from RSI's hangar / pledges page.
///
/// Fields are deliberately conservative: `name` is the only thing
/// guaranteed by RSI's HTML; manufacturer/kind/insurance are best-effort
/// and `None` when the upstream record is half-formed. The client
/// normalises whitespace and drops empty strings before serialising —
/// the server should never see `Some("")`.
///
/// `pledge_id` is RSI's internal record ID (the `data-pledge-id`
/// attribute on the pledge card). When present it lets dedupe key on a
/// stable identifier across snapshots; absence falls back to
/// `(name, manufacturer)` heuristic comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HangarShip {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pledge_id: Option<String>,
    /// "ship", "ground vehicle", "skin", "upgrade" etc. Free-form —
    /// we don't enumerate because RSI's classification drifts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Constituent items for a bundle/pack pledge, parsed from the RSI
    /// "Contains:" list. Empty for a plain single-item pledge. Additive +
    /// backward-compatible (old snapshots deserialize with an empty vec).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contains: Vec<String>,
}

/// Body of `POST /v1/me/hangar`. The tray client builds this after
/// scraping RSI; the server stamps `captured_at` server-side and
/// stores the snapshot keyed on the requesting user.
///
/// Empty `ships` is a valid (and important) signal: it can mean
/// "user has no hangar yet" OR "the parser found nothing on this
/// page" — distinguishing the two is the client's job (it shouldn't
/// POST a parser-failure as an empty hangar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HangarPushRequest {
    /// Schema version — bumped on breaking changes. Currently `1`.
    pub schema_version: u16,
    pub ships: Vec<HangarShip>,
}

/// Context lines that bracketed an unknown line at capture time —
/// up to five lines from before and after in source order. The tray
/// builds these from its rolling buffer; the server stores them
/// verbatim so a reviewer can see how the line sat in its surrounding
/// log context without needing the original `Game.log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextExample {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// One unknown-line submission promoted from the tray to the server's
/// rule-author moderation queue. Mirrors the spec at Phase 4 §4.
///
/// Identity is `(shape_hash, client_anon_id)` — repeated submissions
/// from the same install fold into a single row with bumped occurrence
/// totals; distinct installs each get their own row so the server can
/// count *how many distinct users* surfaced the same shape (a stronger
/// signal than raw occurrence count from one user).
///
/// `client_anon_id` is a stable per-install hash — it groups submissions
/// without identifying the user. The bearer token, not this field,
/// authoritatively identifies the submitter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserSubmission {
    pub shape_hash: String,
    pub raw_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub partial_structured: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_event_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_field_names: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_examples: Vec<ContextExample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_build: Option<String>,
    pub channel: LogSource,
    pub occurrence_count: u32,
    pub client_anon_id: String,
    /// Tray user's opt-in attribution choice, captured at submit time
    /// (forced explicit in the tray UI — no default there). The wire
    /// carries only this boolean intent; the SERVER resolves the actual
    /// identity from the authenticated device token when true. Default
    /// false keeps older payloads / tray builds parseable (anonymous is
    /// the safe direction).
    #[serde(default)]
    pub attributed: bool,
}

/// Body of `POST /v1/parser-submissions`. A batch wrapper so the tray
/// can flush multiple promoted shapes in one round-trip; per-element
/// dedupe still applies row-by-row on the server side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserSubmissionBatch {
    pub submissions: Vec<ParserSubmission>,
}

/// Server response to a submission batch. `accepted` counts new rows;
/// `deduped` counts updates to an existing `(shape_hash, client_anon_id)`
/// row (occurrence bump, payload refresh). `ids` is the row id (as a
/// string for forward-compat with non-int keys) for each submission in
/// the batch, in the same order as the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserSubmissionResponse {
    pub accepted: u32,
    pub deduped: u32,
    pub ids: Vec<String>,
}

/// Per-user UI preferences. Stored as JSONB on `users.preferences`
/// and surfaced through `GET/PUT /v1/me/preferences`. Forward-extensible:
/// every field is optional + skip-on-None so adding new fields
/// (notifications, accent intensity, name plate, etc.) does not break
/// older clients that round-trip the value.
///
/// All optional + `skip_serializing_if = "Option::is_none"` so the
/// wire form stays minimal AND so the server's sparse-merge PUT
/// semantics work cleanly (absent → leave stored value alone;
/// explicit null → clear). See `preferences_routes.rs::put`.
///
/// `theme` and `release_channel` are intentionally `Option<String>`
/// (not enums) so unknown values round-trip cleanly when the wire
/// crate is older than the server. The route layer enforces the
/// allowlists at write time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_logging: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_update_check: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_sync: Option<RemoteSyncPrefs>,
    /// KB detail view mode. One of `visual`, `compact`. Validated at the
    /// route layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_view: Option<String>,
    /// IANA timezone name, e.g. `Europe/London`. Absent → the server makes
    /// no clock-time claims about this player at all.
    ///
    /// An IANA name rather than a fixed offset on purpose: an offset is
    /// wrong by an hour for half the year wherever DST applies, and the
    /// facts this feeds ("what hour do you fly") are exactly where that
    /// error shows. Validated against the tz database at the route layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// KB units preference. One of `metric`, `imperial`. Validated at the
    /// route layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kb_units: Option<String>,
    /// Theme-switch wave animation speed. One of `off`, `slow`, `normal`,
    /// `fast`. Absent → fall back to the sitewide `appearance_config`
    /// default. Validated at the route layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_wave_speed: Option<String>,
}

/// Cadence + transport prefs for the tray's remote sync lane. Nested
/// to match the tray's `Config.remote_sync` shape; depth in JSONB is
/// free. All optional so sparse-merge can touch individual fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSyncPrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_interval_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{GameEvent, JoinPu, PlayerDeath};
    use crate::metadata::{stamp, EntityKind};

    #[test]
    fn round_trips_through_json() {
        let batch = IngestBatch {
            schema_version: 1,
            batch_id: "01934f5a-3b2a-7000-a000-000000000000".into(),
            game_build: Some("4.7.178.50402".into()),
            collector_version: Some("1.8.60".into()),
            parser_version: Some(7),
            batch_sequence: Some(3),
            content_hash: Some("00000000-0000-0000-0000-000000000abc".into()),
            source_range: Some(SourceRange {
                source: LogSource::Live,
                start_offset: 100,
                end_offset: 500,
            }),
            claimed_handle: "TheCodeSaiyan".into(),
            events: vec![EventEnvelope {
                idempotency_key: "evt-1".into(),
                raw_line: "<...>".into(),
                event: Some(GameEvent::JoinPu(JoinPu {
                    timestamp: "2026-05-02T21:14:23.189Z".into(),
                    address: "1.2.3.4".into(),
                    port: 64300,
                    shard: "pub_euw1b".into(),
                    location_id: "1".into(),
                })),
                source: LogSource::Live,
                source_offset: 0,
                metadata: None,
                resolved_location: None,
            }],
        };
        let s = serde_json::to_string(&batch).unwrap();
        let parsed: IngestBatch = serde_json::from_str(&s).unwrap();
        assert_eq!(batch, parsed);
    }

    #[test]
    fn envelope_with_metadata_round_trips() {
        let ev = GameEvent::PlayerDeath(PlayerDeath {
            timestamp: "2026-05-17T00:00:00.000Z".into(),
            body_class: "body_01_noMagicPocket".into(),
            body_id: "1".into(),
            zone: None,
        });
        let env = EventEnvelope {
            idempotency_key: "evt-1".into(),
            raw_line: "<...>".into(),
            event: Some(ev.clone()),
            source: LogSource::Live,
            source_offset: 0,
            metadata: Some(stamp(&ev, Some("alice"))),
            resolved_location: None,
        };
        let s = serde_json::to_string(&env).unwrap();
        let parsed: EventEnvelope = serde_json::from_str(&s).unwrap();
        assert_eq!(env, parsed);
        let metadata = parsed.metadata.expect("metadata must survive round-trip");
        assert_eq!(metadata.primary_entity.kind, EntityKind::Player);
    }

    #[test]
    fn envelope_with_resolved_location_round_trips() {
        use crate::location_classifier::{ClassificationSource, ResolvedLocation};
        use crate::location_taxonomy::LocationTier;
        let env = EventEnvelope {
            idempotency_key: "evt-1".into(),
            raw_line: "<...>".into(),
            event: None,
            source: LogSource::Live,
            source_offset: 0,
            metadata: None,
            resolved_location: Some(ResolvedLocation {
                display_name: "Lorville".into(),
                slug: Some("lorville".into()),
                system: Some("Stanton".into()),
                tier: LocationTier::LandingZone,
                source: ClassificationSource::Catalog,
            }),
        };
        let s = serde_json::to_string(&env).unwrap();
        let parsed: EventEnvelope = serde_json::from_str(&s).unwrap();
        assert_eq!(env, parsed);
        let loc = parsed
            .resolved_location
            .expect("resolved_location must survive round-trip");
        assert_eq!(loc.slug.as_deref(), Some("lorville"));
    }

    #[test]
    fn envelope_without_resolved_location_still_deserialises() {
        // Wire form from a pre-resolution client: no `resolved_location`
        // key at all (the field rode in after `metadata`).
        let legacy = r#"{
            "idempotency_key": "evt-1",
            "raw_line": "<...>",
            "event": null,
            "source": "live",
            "source_offset": 0
        }"#;
        let parsed: EventEnvelope = serde_json::from_str(legacy).unwrap();
        assert!(parsed.resolved_location.is_none());
    }

    #[test]
    fn envelope_without_metadata_still_deserialises() {
        // Wire form produced by a pre-v2 client: no `metadata` key.
        let legacy = r#"{
            "idempotency_key": "evt-1",
            "raw_line": "<...>",
            "event": null,
            "source": "live",
            "source_offset": 0
        }"#;
        let parsed: EventEnvelope = serde_json::from_str(legacy).unwrap();
        assert!(parsed.metadata.is_none());
    }

    #[test]
    fn schema_version_bumped_to_two() {
        assert_eq!(IngestBatch::CURRENT_SCHEMA_VERSION, 2);
    }

    #[test]
    fn batch_without_collector_version_still_deserialises() {
        // Wire form from a pre-versioning collector: no `collector_version`
        // key at all. `#[serde(default)]` must fill it with `None` rather
        // than reject the whole batch — the server still ingests events
        // from old trays, it just can't attribute them to a release.
        let legacy = r#"{
            "schema_version": 1,
            "batch_id": "01934f5a-3b2a-7000-a000-000000000000",
            "game_build": null,
            "claimed_handle": "TheCodeSaiyan",
            "events": []
        }"#;
        let parsed: IngestBatch = serde_json::from_str(legacy).unwrap();
        // Every provenance field defaults to None on a pre-versioning batch.
        assert!(parsed.collector_version.is_none());
        assert!(parsed.parser_version.is_none());
        assert!(parsed.batch_sequence.is_none());
        assert!(parsed.content_hash.is_none());
        assert!(parsed.source_range.is_none());
    }

    #[test]
    fn batch_provenance_versions_survive_round_trip() {
        let batch = IngestBatch {
            schema_version: IngestBatch::CURRENT_SCHEMA_VERSION,
            batch_id: "01934f5a-3b2a-7000-a000-000000000001".into(),
            game_build: None,
            collector_version: Some("1.8.60".into()),
            parser_version: Some(42),
            batch_sequence: Some(9),
            content_hash: Some("00000000-0000-0000-0000-000000000def".into()),
            source_range: Some(SourceRange {
                source: LogSource::Ptu,
                start_offset: 0,
                end_offset: 42,
            }),
            claimed_handle: "TheCodeSaiyan".into(),
            events: vec![],
        };
        let s = serde_json::to_string(&batch).unwrap();
        let parsed: IngestBatch = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.collector_version.as_deref(), Some("1.8.60"));
        assert_eq!(parsed.parser_version, Some(42));
        assert_eq!(parsed.batch_sequence, Some(9));
        assert_eq!(
            parsed.content_hash.as_deref(),
            Some("00000000-0000-0000-0000-000000000def")
        );
        assert_eq!(
            parsed.source_range,
            Some(SourceRange {
                source: LogSource::Ptu,
                start_offset: 0,
                end_offset: 42,
            })
        );
    }

    #[test]
    fn hangar_push_request_round_trips_through_json() {
        let req = HangarPushRequest {
            schema_version: 1,
            ships: vec![
                HangarShip {
                    name: "Aegis Avenger Titan".into(),
                    manufacturer: Some("Aegis Dynamics".into()),
                    pledge_id: Some("12345678".into()),
                    kind: Some("ship".into()),
                    contains: vec![],
                },
                HangarShip {
                    name: "Greycat PTV".into(),
                    manufacturer: None,
                    pledge_id: None,
                    kind: None,
                    contains: vec![],
                },
            ],
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: HangarPushRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(req, parsed);

        // Optional fields with `None` should be omitted from the wire
        // form (skip_serializing_if), keeping the payload lean and
        // distinguishing absent from `null`. Each optional key appears
        // exactly once across the two ships (only the first carries it).
        assert_eq!(s.matches("\"manufacturer\"").count(), 1);
        assert_eq!(s.matches("\"pledge_id\"").count(), 1);
        assert_eq!(s.matches("\"kind\"").count(), 1);
        // Both ships have empty `contains`; skip_serializing_if keeps it
        // off the wire entirely.
        assert!(!s.contains("\"contains\""));
    }

    #[test]
    fn hangar_ship_contains_round_trips() {
        let ship = HangarShip {
            name: "Gear - HighSec - Bundle".into(),
            manufacturer: Some("HighSec".into()),
            pledge_id: Some("105938296".into()),
            kind: Some("Gear".into()),
            contains: vec![
                "Aegis Avenger Titan".into(),
                "Alpha Skin".into(),
                "Extra Widget".into(),
            ],
        };
        let s = serde_json::to_string(&ship).unwrap();
        assert!(s.contains("\"contains\""));
        let parsed: HangarShip = serde_json::from_str(&s).unwrap();
        assert_eq!(ship, parsed);
        assert_eq!(parsed.contains.len(), 3);
    }

    #[test]
    fn hangar_ship_without_contains_still_deserialises() {
        // Wire form from a pre-bundle tray build: no `contains` key at
        // all. `#[serde(default)]` must fill it with an empty vec so old
        // snapshots still round-trip.
        let legacy = r#"{"name":"Aegis Avenger Titan"}"#;
        let parsed: HangarShip = serde_json::from_str(legacy).unwrap();
        assert!(parsed.contains.is_empty());
    }

    #[test]
    fn parser_submission_round_trips() {
        let s = ParserSubmission {
            shape_hash: "sh_abc".into(),
            raw_examples: vec!["raw1".into()],
            partial_structured: Default::default(),
            shell_tag: Some("Foo".into()),
            suggested_event_name: None,
            suggested_field_names: None,
            notes: None,
            context_examples: vec![],
            game_build: Some("4.0".into()),
            channel: LogSource::Live,
            occurrence_count: 3,
            client_anon_id: "anon_xyz".into(),
            attributed: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ParserSubmission = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn parser_submission_batch_round_trips() {
        let mut partial = std::collections::BTreeMap::new();
        partial.insert("ts".to_string(), "2026-05-17T12:34:56Z".to_string());
        let batch = ParserSubmissionBatch {
            submissions: vec![ParserSubmission {
                shape_hash: "sh_a".into(),
                raw_examples: vec!["<X> hello".into(), "<X> world".into()],
                partial_structured: partial,
                shell_tag: Some("Actor Death".into()),
                suggested_event_name: Some("actor_death".into()),
                suggested_field_names: None,
                notes: Some("looks combat-related".into()),
                context_examples: vec![ContextExample {
                    before: vec!["pre-1".into(), "pre-2".into()],
                    after: vec!["post-1".into()],
                }],
                game_build: None,
                channel: LogSource::Ptu,
                occurrence_count: 7,
                client_anon_id: "anon_42".into(),
                attributed: false,
            }],
        };
        let json = serde_json::to_string(&batch).unwrap();
        let back: ParserSubmissionBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(batch, back);
    }

    #[test]
    fn parser_submission_omits_empty_optional_fields() {
        let s = ParserSubmission {
            shape_hash: "sh_min".into(),
            raw_examples: vec!["only".into()],
            partial_structured: Default::default(),
            shell_tag: None,
            suggested_event_name: None,
            suggested_field_names: None,
            notes: None,
            context_examples: vec![],
            game_build: None,
            channel: LogSource::Live,
            occurrence_count: 1,
            client_anon_id: "anon_min".into(),
            attributed: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        // skip_serializing_if must keep the wire form clean.
        assert!(!json.contains("partial_structured"));
        assert!(!json.contains("shell_tag"));
        assert!(!json.contains("suggested_event_name"));
        assert!(!json.contains("suggested_field_names"));
        assert!(!json.contains("notes"));
        assert!(!json.contains("context_examples"));
        assert!(!json.contains("game_build"));
    }

    #[test]
    fn parser_submission_attributed_defaults_false_when_absent() {
        // A payload from an older tray build has no `attributed` key.
        let json = r#"{
            "shape_hash":"sh_a","raw_examples":["x"],
            "channel":"live","occurrence_count":1,"client_anon_id":"anon_x"
        }"#;
        let sub: ParserSubmission = serde_json::from_str(json).unwrap();
        assert!(!sub.attributed);
    }

    #[test]
    fn parser_submission_attributed_roundtrips_true() {
        let sub = ParserSubmission {
            shape_hash: "sh_a".into(),
            raw_examples: vec!["x".into()],
            partial_structured: Default::default(),
            shell_tag: None,
            suggested_event_name: None,
            suggested_field_names: None,
            notes: None,
            context_examples: vec![],
            game_build: None,
            channel: LogSource::Live,
            occurrence_count: 1,
            client_anon_id: "anon_x".into(),
            attributed: true,
        };
        let s = serde_json::to_string(&sub).unwrap();
        let back: ParserSubmission = serde_json::from_str(&s).unwrap();
        assert!(back.attributed);
    }

    #[test]
    fn parser_submission_response_round_trips() {
        let r = ParserSubmissionResponse {
            accepted: 2,
            deduped: 1,
            ids: vec!["1".into(), "2".into(), "3".into()],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ParserSubmissionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn user_preferences_grow_round_trips_all_fields() {
        let prefs = UserPreferences {
            theme: Some("pyro".into()),
            debug_logging: Some(true),
            auto_update_check: Some(false),
            release_channel: Some("beta".into()),
            api_url: Some("https://example.invalid".into()),
            remote_sync: Some(RemoteSyncPrefs {
                enabled: Some(true),
                priority_interval_secs: Some(5),
                interval_secs: Some(60),
                batch_size: Some(200),
            }),
            kb_view: Some("compact".into()),
            kb_units: Some("metric".into()),
            timezone: Some("Europe/London".into()),
            theme_wave_speed: Some("fast".into()),
        };
        let s = serde_json::to_string(&prefs).unwrap();
        let parsed: UserPreferences = serde_json::from_str(&s).unwrap();
        assert_eq!(prefs, parsed);
    }

    #[test]
    fn user_preferences_omits_none_fields_on_wire() {
        let prefs = UserPreferences {
            theme: Some("stanton".into()),
            ..UserPreferences::default()
        };
        let s = serde_json::to_string(&prefs).unwrap();
        // Only `theme` should appear; everything else is None → skipped.
        assert!(s.contains("\"theme\""));
        assert!(!s.contains("\"debug_logging\""));
        assert!(!s.contains("\"auto_update_check\""));
        assert!(!s.contains("\"release_channel\""));
        assert!(!s.contains("\"api_url\""));
        assert!(!s.contains("\"remote_sync\""));
    }

    #[test]
    fn user_preferences_kb_fields_round_trip() {
        let prefs = UserPreferences {
            kb_view: Some("compact".into()),
            kb_units: Some("imperial".into()),
            ..UserPreferences::default()
        };
        let s = serde_json::to_string(&prefs).unwrap();
        let parsed: UserPreferences = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.kb_view.as_deref(), Some("compact"));
        assert_eq!(parsed.kb_units.as_deref(), Some("imperial"));
    }

    #[test]
    fn user_preferences_legacy_payload_with_only_theme_deserialises() {
        // Existing wire form from clients that only know about `theme`.
        let legacy = r#"{"theme":"stanton"}"#;
        let parsed: UserPreferences = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.theme.as_deref(), Some("stanton"));
        assert!(parsed.debug_logging.is_none());
        assert!(parsed.remote_sync.is_none());
    }
}
