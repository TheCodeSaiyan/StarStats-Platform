//! Server-side sanity gates applied to events before they're persisted.
//!
//! Anyone can edit `Game.log` with Notepad and POST garbage at the API.
//! These validators catch the obvious cases. Statistical anomaly
//! detection lives further up the stack (in the API server's ingest
//! pipeline).

use crate::events::GameEvent;
use crate::metadata::{EventMetadata, EventSource};
use crate::wire::EventEnvelope;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("timestamp is empty")]
    EmptyTimestamp,
    #[error("timestamp does not look like ISO-8601")]
    BadTimestamp,
    #[error("required field {0} is empty")]
    EmptyField(&'static str),
    #[error("port {0} is out of plausible range")]
    BadPort(u16),
    #[error("invalid metadata: {reason}")]
    InvalidMetadata { reason: String },
}

/// Lightweight validity check. Cheap to call on every ingested event.
///
/// Validates the parsed `GameEvent` payload (when present) and any
/// attached `EventMetadata`. Envelopes with `event = None` (lines the
/// client recognised structurally but could not classify) pass the
/// payload check; metadata, when present, is still validated.
pub fn validate_event(envelope: &EventEnvelope) -> Result<(), ValidationError> {
    if let Some(event) = &envelope.event {
        validate_game_event(event)?;
    }
    if let Some(meta) = envelope.metadata.as_ref() {
        validate_metadata(meta)?;
    }
    Ok(())
}

fn validate_game_event(event: &GameEvent) -> Result<(), ValidationError> {
    let ts = match event {
        GameEvent::ProcessInit(e) => &e.timestamp,
        GameEvent::LegacyLogin(e) => &e.timestamp,
        GameEvent::JoinPu(e) => &e.timestamp,
        GameEvent::ChangeServer(e) => &e.timestamp,
        GameEvent::SeedSolarSystem(e) => &e.timestamp,
        GameEvent::ResolveSpawn(e) => &e.timestamp,
        GameEvent::ActorDeath(e) => &e.timestamp,
        GameEvent::PlayerDeath(e) => &e.timestamp,
        GameEvent::PlayerIncapacitated(e) => &e.timestamp,
        GameEvent::VehicleDestruction(e) => &e.timestamp,
        GameEvent::HudNotification(e) => &e.timestamp,
        GameEvent::LocationInventoryRequested(e) => &e.timestamp,
        GameEvent::PlanetTerrainLoad(e) => &e.timestamp,
        GameEvent::QuantumTargetSelected(e) => &e.timestamp,
        GameEvent::AttachmentReceived(e) => &e.timestamp,
        GameEvent::VehicleStowed(e) => &e.timestamp,
        GameEvent::GameCrash(e) => &e.timestamp,
        GameEvent::LauncherActivity(e) => &e.timestamp,
        GameEvent::MissionStart(e) => &e.timestamp,
        GameEvent::MissionEnd(e) => &e.timestamp,
        GameEvent::ShopBuyRequest(e) => &e.timestamp,
        GameEvent::ShopFlowResponse(e) => &e.timestamp,
        GameEvent::CommodityBuyRequest(e) => &e.timestamp,
        GameEvent::CommoditySellRequest(e) => &e.timestamp,
        GameEvent::SessionEnd(e) => &e.timestamp,
        GameEvent::RemoteMatch(e) => &e.timestamp,
        GameEvent::BurstSummary(e) => &e.timestamp,
        GameEvent::LocationChanged(e) => &e.timestamp,
        GameEvent::ShopRequestTimedOut(e) => &e.timestamp,
        GameEvent::MissionObjective(e) => &e.timestamp,
        GameEvent::MissionQuantumDestinationSelected(e) => &e.timestamp,
        GameEvent::TravelToContractLocation(e) => &e.timestamp,
        GameEvent::QuantumRoute(e) => &e.timestamp,
        GameEvent::QuantumArrived(e) => &e.timestamp,
        GameEvent::ItemEquipChange(e) => &e.timestamp,
    };
    check_timestamp(ts)?;

    match event {
        GameEvent::LegacyLogin(e) => {
            if e.handle.is_empty() {
                return Err(ValidationError::EmptyField("handle"));
            }
        }
        GameEvent::JoinPu(e) => {
            if e.shard.is_empty() {
                return Err(ValidationError::EmptyField("shard"));
            }
            if e.port == 0 {
                return Err(ValidationError::BadPort(e.port));
            }
        }
        GameEvent::ActorDeath(e) => {
            if e.victim.is_empty() {
                return Err(ValidationError::EmptyField("victim"));
            }
            if e.killer.is_empty() {
                return Err(ValidationError::EmptyField("killer"));
            }
        }
        GameEvent::VehicleDestruction(e) => {
            if e.vehicle_class.is_empty() {
                return Err(ValidationError::EmptyField("vehicle_class"));
            }
        }
        GameEvent::BurstSummary(e) => {
            // Rule id is the join key consumers will key off — empty
            // would break per-rule timeline aggregation. Size = 0
            // means the producer emitted a phantom summary; reject so
            // bad client logic doesn't silently corrupt the timeline.
            if e.rule_id.is_empty() {
                return Err(ValidationError::EmptyField("rule_id"));
            }
            if e.size == 0 {
                return Err(ValidationError::EmptyField("size"));
            }
            check_timestamp(&e.end_timestamp)?;
        }
        _ => {}
    }
    Ok(())
}

/// Enforce the cross-field invariants documented on [`EventMetadata`].
///
/// Rules:
/// - `confidence` must lie in `[0.0, 1.0]`.
/// - `Observed` events anchor at `confidence = 1.0` with no
///   `inference_inputs`. Anything else is a producer bug.
/// - `Inferred` events must carry a sub-1.0 confidence and name at
///   least one `inference_inputs` ancestor so the provenance trail is
///   followable.
/// - `Synthesized` is unconstrained here — synthetic markers
///   (heartbeats, lifecycle events) describe themselves; no extra
///   invariant fits cleanly across that vocabulary.
pub fn validate_metadata(meta: &EventMetadata) -> Result<(), ValidationError> {
    if !(0.0..=1.0).contains(&meta.confidence) {
        return Err(ValidationError::InvalidMetadata {
            reason: "confidence out of range".into(),
        });
    }
    match meta.source {
        EventSource::Observed => {
            if (meta.confidence - 1.0).abs() > f32::EPSILON || !meta.inference_inputs.is_empty() {
                return Err(ValidationError::InvalidMetadata {
                    reason: "observed event must have confidence=1.0 and no inference_inputs"
                        .into(),
                });
            }
        }
        EventSource::Inferred => {
            if meta.confidence >= 1.0 || meta.inference_inputs.is_empty() {
                return Err(ValidationError::InvalidMetadata {
                    reason:
                        "inferred event must have confidence<1.0 and at least one inference input"
                            .into(),
                });
            }
        }
        EventSource::Synthesized => {}
    }
    Ok(())
}

fn check_timestamp(ts: &str) -> Result<(), ValidationError> {
    if ts.is_empty() {
        return Err(ValidationError::EmptyTimestamp);
    }
    // We don't pull `chrono::DateTime::parse_from_rfc3339` here on
    // purpose — `chrono` lives in the consuming crates' deps; this
    // crate stays dependency-light. A shape check is enough.
    //
    // Accepted shapes (in order of how they reach us):
    //   - Game.log: `2026-05-02T21:14:23.189Z` (canonical ISO-8601)
    //   - GameCrash: `2026-05-04T21:10:12+00:00` (chrono `to_rfc3339`)
    //   - LauncherActivity: `2026-05-06 12:34:56.789` (Electron format,
    //     no offset, space separator).
    //
    // The relaxed check: must contain a date/time separator (`T` or
    // space) followed at some point by a colon. That catches the
    // everyday garbage cases (empty, "not-a-date", "abcdef") while
    // tolerating each upstream's preferred ISO dialect.
    let has_separator = ts.contains('T') || ts.contains(' ');
    let has_time_colon = ts.contains(':');
    if !has_separator || !has_time_colon {
        return Err(ValidationError::BadTimestamp);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::JoinPu;
    use crate::metadata::{EntityKind, EntityRef};
    use crate::wire::LogSource;
    use std::collections::BTreeMap;

    /// Build a minimal valid envelope wrapping a canonical JoinPu
    /// event. Callers patch fields (event, metadata) as needed for
    /// the case under test.
    fn make_valid_envelope() -> EventEnvelope {
        EventEnvelope {
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
        }
    }

    fn make_envelope_without_metadata() -> EventEnvelope {
        make_valid_envelope()
    }

    fn make_minimal_envelope_with_metadata(meta: EventMetadata) -> EventEnvelope {
        let mut env = make_valid_envelope();
        env.metadata = Some(meta);
        env
    }

    fn envelope_with_event(event: GameEvent) -> EventEnvelope {
        EventEnvelope {
            idempotency_key: "evt-1".into(),
            raw_line: "<...>".into(),
            event: Some(event),
            source: LogSource::Live,
            source_offset: 0,
            metadata: None,
            resolved_location: None,
        }
    }

    #[test]
    fn rejects_empty_shard() {
        let env = envelope_with_event(GameEvent::JoinPu(JoinPu {
            timestamp: "2026-05-02T21:14:23.189Z".into(),
            address: "1.2.3.4".into(),
            port: 64300,
            shard: String::new(),
            location_id: "1".into(),
        }));
        assert_eq!(
            validate_event(&env),
            Err(ValidationError::EmptyField("shard"))
        );
    }

    #[test]
    fn accepts_valid_join_pu() {
        let env = envelope_with_event(GameEvent::JoinPu(JoinPu {
            timestamp: "2026-05-02T21:14:23.189Z".into(),
            address: "1.2.3.4".into(),
            port: 64300,
            shard: "pub_euw1b".into(),
            location_id: "1".into(),
        }));
        assert_eq!(validate_event(&env), Ok(()));
    }

    #[test]
    fn rejects_bad_timestamp() {
        let env = envelope_with_event(GameEvent::JoinPu(JoinPu {
            timestamp: "not-a-date".into(),
            address: "1.2.3.4".into(),
            port: 64300,
            shard: "pub".into(),
            location_id: "1".into(),
        }));
        assert_eq!(validate_event(&env), Err(ValidationError::BadTimestamp));
    }

    #[test]
    fn accepts_chrono_rfc3339_offset_form() {
        // GameCrash events carry timestamps from chrono's `to_rfc3339()`,
        // which renders UTC as `+00:00` not `Z`. The validator must
        // not reject these.
        let env = envelope_with_event(GameEvent::JoinPu(JoinPu {
            timestamp: "2026-05-04T21:10:12+00:00".into(),
            address: "1.2.3.4".into(),
            port: 64300,
            shard: "pub".into(),
            location_id: "1".into(),
        }));
        assert_eq!(validate_event(&env), Ok(()));
    }

    #[test]
    fn accepts_launcher_space_separated_form() {
        // LauncherActivity carries `YYYY-MM-DD HH:MM:SS.mmm` — space
        // separator, no offset. Also acceptable.
        let env = envelope_with_event(GameEvent::JoinPu(JoinPu {
            timestamp: "2026-05-06 12:34:56.789".into(),
            address: "1.2.3.4".into(),
            port: 64300,
            shard: "pub".into(),
            location_id: "1".into(),
        }));
        assert_eq!(validate_event(&env), Ok(()));
    }

    #[test]
    fn validator_rejects_confidence_out_of_range() {
        let env = make_minimal_envelope_with_metadata(EventMetadata {
            primary_entity: EntityRef {
                kind: EntityKind::Player,
                id: "x".into(),
                display_name: "x".into(),
            },
            source: EventSource::Observed,
            confidence: 1.5,
            group_key: "k".into(),
            field_provenance: BTreeMap::new(),
            inference_inputs: vec![],
            rule_id: None,
        });
        let err = validate_event(&env).unwrap_err();
        let s = format!("{err:?}");
        assert!(s.contains("confidence"), "got: {s}");
    }

    #[test]
    fn validator_rejects_observed_with_confidence_below_one() {
        let env = make_minimal_envelope_with_metadata(EventMetadata {
            primary_entity: EntityRef {
                kind: EntityKind::Player,
                id: "x".into(),
                display_name: "x".into(),
            },
            source: EventSource::Observed,
            confidence: 0.8,
            group_key: "k".into(),
            field_provenance: BTreeMap::new(),
            inference_inputs: vec![],
            rule_id: None,
        });
        let err = validate_event(&env).unwrap_err();
        let s = format!("{err:?}");
        assert!(s.contains("observed"), "got: {s}");
    }

    #[test]
    fn validator_rejects_inferred_without_inputs() {
        let env = make_minimal_envelope_with_metadata(EventMetadata {
            primary_entity: EntityRef {
                kind: EntityKind::Player,
                id: "x".into(),
                display_name: "x".into(),
            },
            source: EventSource::Inferred,
            confidence: 0.85,
            group_key: "k".into(),
            field_provenance: BTreeMap::new(),
            inference_inputs: vec![],
            rule_id: None,
        });
        let err = validate_event(&env).unwrap_err();
        let s = format!("{err:?}");
        assert!(s.contains("inferred"), "got: {s}");
    }

    #[test]
    fn validator_accepts_inferred_with_inputs_and_sub_one_confidence() {
        let env = make_minimal_envelope_with_metadata(EventMetadata {
            primary_entity: EntityRef {
                kind: EntityKind::Player,
                id: "x".into(),
                display_name: "x".into(),
            },
            source: EventSource::Inferred,
            confidence: 0.85,
            group_key: "k".into(),
            field_provenance: BTreeMap::new(),
            inference_inputs: vec!["evt-source".into()],
            rule_id: Some("rule_a".into()),
        });
        assert_eq!(validate_event(&env), Ok(()));
    }

    #[test]
    fn validator_accepts_absent_metadata_for_legacy_clients() {
        let env = make_envelope_without_metadata();
        assert!(validate_event(&env).is_ok());
    }

    #[test]
    fn validator_accepts_observed_with_default_confidence() {
        let env = make_minimal_envelope_with_metadata(EventMetadata {
            primary_entity: EntityRef {
                kind: EntityKind::Player,
                id: "x".into(),
                display_name: "x".into(),
            },
            source: EventSource::Observed,
            confidence: 1.0,
            group_key: "k".into(),
            field_provenance: BTreeMap::new(),
            inference_inputs: vec![],
            rule_id: None,
        });
        assert_eq!(validate_event(&env), Ok(()));
    }
}
