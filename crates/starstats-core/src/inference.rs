//! Post-classify pass that emits inferred events from surrounding context.
//!
//! Rules are pure-Rust structs in v1, mirroring the BurstRule / RemoteRule
//! style: declarative, append-only, no learned components. The inference
//! pass is pure — given the same input event slice it produces the same
//! output, which keeps tests deterministic and idempotent.

use crate::events::{
    GameEvent, LocationChanged, PlayerDeath, ShopRequestTimedOut, TravelToContractLocation,
};
use crate::metadata::{
    entity_kind_key, event_type_key, primary_entity_for, EntityRef, EventMetadata, EventSource,
};
use crate::wire::EventEnvelope;
use chrono::{DateTime, FixedOffset};
use std::collections::BTreeMap;

/// Tunables for the inference pass. The defaults match the design spec
/// (Phase 3); callers needing tighter bounds (e.g. unit tests probing
/// edge cases) can override.
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    /// Hard cap on the forward / backward scan distance in events. Acts
    /// as a defence-in-depth bound on top of the per-rule wall-clock
    /// limits so a malformed log can't make a rule walk the whole stream.
    pub window_size: usize,
    /// Reconciliation window in seconds. An inferred event is marked
    /// `superseded_by` the first observed event of matching
    /// `(event_type, primary_entity)` that lands within this window
    /// after the inferred timestamp.
    pub reconciliation_secs: i64,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            window_size: 200,
            reconciliation_secs: 5,
        }
    }
}

/// One result row from [`infer`]. Carries the synthesised event, its
/// metadata, and a back-reference to the observed event whose
/// classification triggered the rule.
#[derive(Debug, Clone, PartialEq)]
pub struct InferredEvent {
    pub event: GameEvent,
    pub metadata: EventMetadata,
    /// The observed event whose classification triggered this inference.
    /// Used for idempotency_key derivation and audit trail.
    pub trigger_idempotency_key: String,
    /// If this inferred event was later superseded by an actual observed
    /// event of the same `(event_type, primary_entity)` within the
    /// reconciliation window, this is the observed event's idempotency_key.
    /// Timeline consumers drop superseded rows but storage retains them.
    pub superseded_by: Option<String>,
}

/// One match produced by a compiled inference rule's `apply` closure.
/// Mirrors the shape of `InferredEvent` minus the metadata block,
/// which the outer loop in [`infer_with_rules`] composes from the
/// rule's static `id` / `confidence` and the trigger envelope.
#[derive(Debug, Clone)]
pub struct InferenceMatch {
    /// The synthesised event the rule emits.
    pub event: GameEvent,
    /// `idempotency_key`s of the observed envelopes that fed the
    /// derivation. The first id is conventionally the trigger; any
    /// follow-ups land after it. Used both for the metadata trail and
    /// for the [`InferredEvent::trigger_idempotency_key`] back-ref.
    pub source_event_ids: Vec<String>,
}

/// A runtime inference rule. The `apply` closure receives the trigger
/// envelope and the slice of envelopes that follow it within the
/// rule's window; returns `Some(InferenceMatch)` when the rule fires.
///
/// `infer_with_rules` calls `apply` once per envelope in the stream,
/// passing the post-trigger window each time. Rules that don't care
/// about a particular trigger return `None` cheaply (typically a
/// pattern-match on the event variant).
///
/// The `apply` callback lives behind an `Arc` so caches that hold
/// rule snapshots can `Clone` the rule without re-compiling the
/// closure. Closure invocation through `&Arc<dyn Fn>` is a single
/// indirection — the same cost as `&Box<dyn Fn>`.
#[derive(Clone)]
pub struct CompiledInferenceRule {
    pub id: String,
    pub confidence: f32,
    pub window_secs: i64,
    /// Callback invoked once per envelope. Returns the inferred event
    /// payload (if any) plus the source event IDs used as inputs.
    #[allow(clippy::type_complexity)]
    pub apply: std::sync::Arc<
        dyn Fn(&EventEnvelope, &[EventEnvelope]) -> Option<InferenceMatch> + Send + Sync,
    >,
}

impl std::fmt::Debug for CompiledInferenceRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledInferenceRule")
            .field("id", &self.id)
            .field("confidence", &self.confidence)
            .field("window_secs", &self.window_secs)
            .field("apply", &"<closure>")
            .finish()
    }
}

/// Run all inference rules over the event stream. Pure function:
/// given the same input it returns the same output.
pub fn infer(events: &[EventEnvelope], config: &InferenceConfig) -> Vec<InferredEvent> {
    let rules = built_in_inference_rules();
    infer_with_rules(events, config, &rules)
}

/// Same as [`infer`] but takes a runtime rule list. Lets callers mix
/// the built-in rules with manifest-loaded rules without rebuilding
/// the dispatch loop.
///
/// The outer loop walks the envelope slice once. For each envelope,
/// every rule's `apply` closure runs against the post-trigger window
/// bounded by the rule's `window_secs` (plus the global
/// [`InferenceConfig::window_size`] event cap). A rule that fires
/// produces one [`InferredEvent`]; the reconciliation pass then marks
/// inferred rows superseded by later observed rows of the same
/// `(event_type, primary_entity)`.
pub fn infer_with_rules(
    events: &[EventEnvelope],
    config: &InferenceConfig,
    rules: &[CompiledInferenceRule],
) -> Vec<InferredEvent> {
    let mut out = Vec::new();
    for (i, env) in events.iter().enumerate() {
        let scan_end = (i + 1 + config.window_size).min(events.len());
        let window_slice = &events[(i + 1)..scan_end];
        for rule in rules {
            let trimmed = window_within_secs(env, window_slice, rule.window_secs, &rule.id);
            let Some(m) = (rule.apply)(env, &trimmed) else {
                continue;
            };
            let primary_entity = primary_entity_for(&m.event, None);
            let metadata = build_inferred_metadata(
                primary_entity,
                &m.event,
                rule.confidence,
                &rule.id,
                m.source_event_ids,
            );
            out.push(InferredEvent {
                event: m.event,
                metadata,
                trigger_idempotency_key: env.idempotency_key.clone(),
                superseded_by: None,
            });
        }
    }
    reconcile_supersedes(events, config, &mut out);
    out
}

/// Collect the post-trigger envelopes whose timestamp lies in
/// `(trigger_ts, trigger_ts + window_secs]` — a STRICT lower bound so an
/// event at or before the trigger is never treated as a consequence of
/// it, plus the inclusive upper bound from the rule's window. Returns an
/// empty vec when the trigger's own timestamp doesn't parse (the caller
/// treats that as "rule cannot fire").
///
/// FILTERS rather than truncates. `window` is positional-after-trigger
/// and usually timestamp-monotonic, but timestamps can arrive OUT OF
/// ORDER (across a log rotation, or a backfill that interleaves files).
/// The old "stop at the first out-of-window envelope" admitted a
/// **negative-duration** pairing — an earlier event pulled in as a
/// follow-up — whenever a stale-timestamped row sat in the window (F8).
/// Scanning every envelope and gating on `0 < delta <= window_secs`
/// closes that; windows are tiny (bounded by `window_size`), so the
/// per-envelope scan + clone is negligible.
fn window_within_secs(
    trigger: &EventEnvelope,
    window: &[EventEnvelope],
    window_secs: i64,
    rule_id: &str,
) -> Vec<EventEnvelope> {
    let Some(trigger_ts) = envelope_timestamp(trigger).and_then(parse_ts) else {
        tracing::debug!(
            event = "inference.skip_pairing",
            reason = "timestamp_parse_failed",
            value = envelope_timestamp(trigger).unwrap_or(""),
            rule_id = rule_id,
        );
        return Vec::new();
    };
    let mut kept = Vec::new();
    for env in window {
        let Some(ts) = envelope_timestamp(env).and_then(parse_ts) else {
            tracing::debug!(
                event = "inference.skip_pairing",
                reason = "timestamp_parse_failed",
                value = envelope_timestamp(env).unwrap_or(""),
                rule_id = rule_id,
            );
            continue;
        };
        let delta = (ts - trigger_ts).num_seconds();
        // Strict lower bound (out-of-order / same-instant events are NOT
        // a consequence of the trigger) + inclusive upper bound.
        if delta > 0 && delta <= window_secs {
            kept.push(env.clone());
        }
    }
    kept
}

/// The three rules baked into v1. Returned as a fresh `Vec` so callers
/// can extend / filter / replace before feeding [`infer_with_rules`].
pub fn built_in_inference_rules() -> Vec<CompiledInferenceRule> {
    vec![
        built_in_implicit_death_after_vehicle_destruction(),
        built_in_implicit_location_change(),
        built_in_implicit_shop_request_timeout(),
        built_in_travel_to_contract_location(),
    ]
}

/// Mark inferred events as superseded when the observed stream later
/// produces an event of the same `(event_type, primary_entity)`
/// within `reconciliation_secs`. The observed event is the canonical
/// row the timeline shows; the inferred row is retained in storage
/// (with the back-reference set) so an audit pass can reconstruct
/// why we initially guessed.
///
/// Walks the stream once per inferred event — `inferred.len()` is
/// expected to be small (a handful per session, typically). The scan
/// stops at the first match (`break` after setting `superseded_by`);
/// in the no-match case it walks the full envelope slice. The
/// `reconciliation_secs` bound on `delta` filters out-of-window
/// candidates inline rather than terminating the loop early —
/// `window_size` is not consulted here.
fn reconcile_supersedes(
    events: &[EventEnvelope],
    config: &InferenceConfig,
    inferred: &mut [InferredEvent],
) {
    for row in inferred.iter_mut() {
        let inferred_key = event_type_key(&row.event);
        let inferred_entity = &row.metadata.primary_entity;
        let Some(inferred_ts) = parse_ts(timestamp_of(&row.event)) else {
            tracing::debug!(
                event = "inference.skip_pairing",
                reason = "timestamp_parse_failed",
                value = %timestamp_of(&row.event),
                rule_id = "reconcile",
            );
            continue;
        };

        for env in events {
            let Some(observed) = env.event.as_ref() else {
                continue;
            };
            // An inferred row can only be superseded by an observed
            // event — never by another inferred row. The wire-level
            // envelopes coming into this pass are all observed
            // (metadata.source defaults to None / Observed); inferred
            // outputs live only in the InferredEvent vec we're
            // mutating.
            if event_type_key(observed) != inferred_key {
                continue;
            }
            let observed_entity = primary_entity_for(observed, None);
            if observed_entity.kind != inferred_entity.kind
                || observed_entity.id != inferred_entity.id
            {
                continue;
            }
            let Some(observed_ts) = parse_ts(timestamp_of(observed)) else {
                tracing::debug!(
                    event = "inference.skip_pairing",
                    reason = "timestamp_parse_failed",
                    value = %timestamp_of(observed),
                    rule_id = "reconcile",
                );
                continue;
            };
            let delta = (observed_ts - inferred_ts).num_seconds();
            if delta < 0 || delta > config.reconciliation_secs {
                continue;
            }
            row.superseded_by = Some(env.idempotency_key.clone());
            break;
        }
    }
}

/// Extract a timestamp out of an owned `GameEvent`. Mirrors
/// [`envelope_timestamp`] for the inferred-event path where we hold
/// the event directly (not wrapped in an envelope).
fn timestamp_of(event: &GameEvent) -> &str {
    match event {
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
        GameEvent::QuantumRoute(e) => &e.timestamp,
        GameEvent::QuantumArrived(e) => &e.timestamp,
        GameEvent::ItemEquipChange(e) => &e.timestamp,
        GameEvent::MissionQuantumDestinationSelected(e) => &e.timestamp,
        GameEvent::TravelToContractLocation(e) => &e.timestamp,
    }
}

/// Parse the wire timestamps as fixed-offset RFC3339. Returns `None`
/// when the field doesn't parse — rules treat that as "skip this
/// pairing" rather than failing the whole pass.
pub(crate) fn parse_ts(ts: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(ts).ok()
}

/// Extract the parsed game event's timestamp, when the envelope
/// actually carries a classified event.
pub(crate) fn envelope_timestamp(env: &EventEnvelope) -> Option<&str> {
    env.event.as_ref().map(timestamp_of)
}

/// Build the inferred-event metadata block. Centralised so every rule
/// produces a consistent envelope (snake_case group_key shape, Inferred
/// source, sub-1.0 confidence) without duplicating the format string.
fn build_inferred_metadata(
    primary_entity: EntityRef,
    event: &GameEvent,
    confidence: f32,
    rule_id: &str,
    inference_inputs: Vec<String>,
) -> EventMetadata {
    let group_key = format!(
        "{}:{}:{}",
        event_type_key(event),
        entity_kind_key(primary_entity.kind),
        primary_entity.id,
    );
    EventMetadata {
        primary_entity,
        source: EventSource::Inferred,
        confidence,
        group_key,
        field_provenance: BTreeMap::new(),
        inference_inputs,
        rule_id: Some(rule_id.to_string()),
    }
}

/// Rule: `implicit_death_after_vehicle_destruction`.
///
/// Pattern: a `VehicleDestruction` immediately followed (within 15s
/// and `window_size` events) by a `ResolveSpawn` is the engine's tell
/// that the local player died with their ship. The line classifier
/// doesn't get a `<Actor Death>` for the pilot in modern builds (CIG
/// removed that branch), so we synthesise the death here at the
/// VehicleDestruction's timestamp.
///
/// The rule emits a `PlayerDeath` with `body_class = "inferred"` and a
/// derived `body_id` so consumers can distinguish a real corpse-cleanup
/// from this synthesised marker. Confidence is 0.85 — high but not
/// 1.0, because the engine occasionally writes spawn-resolve lines
/// for non-death scenarios (e.g. an explicit respawn beacon).
const RULE_ID_IMPLICIT_DEATH: &str = "implicit_death_after_vehicle_destruction";
const IMPLICIT_DEATH_WINDOW_SECS: i64 = 15;
const IMPLICIT_DEATH_CONFIDENCE: f32 = 0.85;

fn built_in_implicit_death_after_vehicle_destruction() -> CompiledInferenceRule {
    CompiledInferenceRule {
        id: RULE_ID_IMPLICIT_DEATH.to_string(),
        confidence: IMPLICIT_DEATH_CONFIDENCE,
        window_secs: IMPLICIT_DEATH_WINDOW_SECS,
        apply: std::sync::Arc::new(|env, window| {
            let veh = match env.event.as_ref()? {
                GameEvent::VehicleDestruction(v) => v,
                _ => return None,
            };
            let spawn = window
                .iter()
                .find(|f| matches!(f.event.as_ref(), Some(GameEvent::ResolveSpawn(_))))?;
            let event = GameEvent::PlayerDeath(PlayerDeath {
                timestamp: veh.timestamp.clone(),
                body_class: "inferred".into(),
                body_id: format!("inferred_{}", env.idempotency_key),
                zone: veh.zone.clone(),
            });
            Some(InferenceMatch {
                event,
                source_event_ids: vec![env.idempotency_key.clone(), spawn.idempotency_key.clone()],
            })
        }),
    }
}

/// Rule: `implicit_location_change`.
///
/// Pattern: two `PlanetTerrainLoad` events naming different planets,
/// with no `LocationInventoryRequested` separating them, indicates the
/// player moved between celestial bodies without explicitly opening
/// an inventory (which would otherwise anchor a high-confidence
/// location). The engine doesn't write a "you changed location" line,
/// so we synthesise one from the planet-terrain pair.
///
/// Confidence is 0.70 — lower than the death rule because the engine
/// fires `<InvalidateAllTerrainCells>` on both load AND unload, which
/// occasionally produces back-to-back terrain loads while the player
/// is still in the same orbital neighbourhood.
const RULE_ID_IMPLICIT_LOCATION_CHANGE: &str = "implicit_location_change";
const IMPLICIT_LOCATION_CHANGE_CONFIDENCE: f32 = 0.70;

/// Internal state for the stateful location-change rule. Tracked
/// inside a `Mutex` so the `apply` closure can mutate it across
/// successive trigger envelopes while keeping `Fn` (not `FnMut`)
/// for the trait-object signature.
#[derive(Default)]
struct LocationChangeState {
    prev_planet: Option<(String, String)>, // (idempotency_key, planet_name)
    inventory_seen_since_prev: bool,
}

fn built_in_implicit_location_change() -> CompiledInferenceRule {
    // Window is unbounded by wall-clock — the rule fires on planet
    // adjacency rather than a time budget. We still need a non-zero
    // window_secs so `trim_window_by_secs` doesn't clip the window
    // away; the apply closure doesn't consult `window` at all.
    let state = std::sync::Mutex::new(LocationChangeState::default());
    CompiledInferenceRule {
        id: RULE_ID_IMPLICIT_LOCATION_CHANGE.to_string(),
        confidence: IMPLICIT_LOCATION_CHANGE_CONFIDENCE,
        window_secs: i64::MAX,
        apply: std::sync::Arc::new(move |env, _window| {
            let mut st = state.lock().expect("location-change state mutex poisoned");
            match env.event.as_ref() {
                Some(GameEvent::PlanetTerrainLoad(planet_load)) => {
                    let emit = if let Some((prev_key, prev_planet_name)) = st.prev_planet.as_ref() {
                        if !st.inventory_seen_since_prev && prev_planet_name != &planet_load.planet
                        {
                            Some((prev_key.clone(), prev_planet_name.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let result = emit.map(|(prev_key, prev_planet_name)| {
                        let event = GameEvent::LocationChanged(LocationChanged {
                            timestamp: planet_load.timestamp.clone(),
                            from: Some(prev_planet_name),
                            to: planet_load.planet.clone(),
                        });
                        InferenceMatch {
                            event,
                            source_event_ids: vec![prev_key, env.idempotency_key.clone()],
                        }
                    });
                    st.prev_planet =
                        Some((env.idempotency_key.clone(), planet_load.planet.clone()));
                    st.inventory_seen_since_prev = false;
                    result
                }
                Some(GameEvent::LocationInventoryRequested(_)) => {
                    st.inventory_seen_since_prev = true;
                    None
                }
                _ => None,
            }
        }),
    }
}

/// Rule: `implicit_shop_request_timeout`.
///
/// Pattern: a `ShopBuyRequest` with no matching `ShopFlowResponse`
/// landing within the 30-second SLO budget. The engine optimistically
/// surfaces the buy in the kiosk UI before the backend confirms; when
/// the backend never replies we synthesise a timeout event so the
/// timeline shows the failed purchase instead of leaving the pending
/// row dangling.
///
/// "Matching" here is by `shop_id` when both halves carry one,
/// otherwise by the unkeyed pair (any in-flight response satisfies an
/// in-flight request). Confidence is 0.90 — the highest of the three
/// rules because the rule fires on the absence of a signal rather
/// than a sequence: there's no benign alternative explanation for a
/// missing response within 30s.
const RULE_ID_IMPLICIT_SHOP_REQUEST_TIMEOUT: &str = "implicit_shop_request_timeout";
const IMPLICIT_SHOP_REQUEST_TIMEOUT_SECS: i64 = 30;
const IMPLICIT_SHOP_REQUEST_TIMEOUT_CONFIDENCE: f32 = 0.90;

fn built_in_implicit_shop_request_timeout() -> CompiledInferenceRule {
    CompiledInferenceRule {
        id: RULE_ID_IMPLICIT_SHOP_REQUEST_TIMEOUT.to_string(),
        confidence: IMPLICIT_SHOP_REQUEST_TIMEOUT_CONFIDENCE,
        window_secs: IMPLICIT_SHOP_REQUEST_TIMEOUT_SECS,
        apply: std::sync::Arc::new(|env, window| {
            let req = match env.event.as_ref()? {
                GameEvent::ShopBuyRequest(r) => r,
                _ => return None,
            };
            // Look for any in-window response whose shop_id matches
            // (or any response at all when the request had no id —
            // the engine doesn't keep multiple in-flight requests
            // against the same kiosk).
            let matched = window.iter().any(|f| {
                let Some(GameEvent::ShopFlowResponse(resp)) = f.event.as_ref() else {
                    return false;
                };
                match (&req.shop_id, &resp.shop_id) {
                    (Some(a), Some(b)) => a == b,
                    (None, _) | (_, None) => true,
                }
            });
            if matched {
                return None;
            }
            let timed_out_after_secs: u32 = IMPLICIT_SHOP_REQUEST_TIMEOUT_SECS
                .try_into()
                .unwrap_or(u32::MAX);
            let event = GameEvent::ShopRequestTimedOut(ShopRequestTimedOut {
                timestamp: req.timestamp.clone(),
                shop_id: req.shop_id.clone(),
                item_class: req.item_class.clone(),
                timed_out_after_secs,
            });
            Some(InferenceMatch {
                event,
                source_event_ids: vec![env.idempotency_key.clone()],
            })
        }),
    }
}

/// Rule: `travel_to_contract_location`.
///
/// Pattern: a `MissionQuantumDestinationSelected` observation — the
/// player picked a mission/contract quantum beacon as their nav
/// destination. That is direct, explicit evidence of *intent to travel
/// to a contract location*, so we surface a `TravelToContractLocation`
/// inference off the single trigger (no follow-up required, like the
/// shop-timeout rule).
///
/// Confidence is 0.90 — high, because the trigger line names a mission
/// QT beacon explicitly; there's no benign alternative reading of
/// "selected point MISSION_QT_Quantum_Beacon_… as their destination".
///
/// CRITICAL — intent, NOT progress. The emitted event carries
/// `travel_started = false` and `travel_completed = false`. This line
/// proves only that a destination was *selected* and locally routed; it
/// is NOT a quantum-travel-start signal. A travel-START inference must
/// be a SEPARATE rule keyed on a quantum-drive activation / travel-state
/// transition — this rule must never emit one. Do not conflate the two.
const RULE_ID_TRAVEL_TO_CONTRACT_LOCATION: &str = "travel_to_contract_location";
const TRAVEL_TO_CONTRACT_LOCATION_CONFIDENCE: f32 = 0.90;
/// The rule fires on the trigger alone and does not consult the
/// follow-up window; the value only needs to be a positive, non-`MAX`
/// bound so `window_within_secs` doesn't reject the (unused) window.
const TRAVEL_TO_CONTRACT_LOCATION_WINDOW_SECS: i64 = 1;

fn built_in_travel_to_contract_location() -> CompiledInferenceRule {
    CompiledInferenceRule {
        id: RULE_ID_TRAVEL_TO_CONTRACT_LOCATION.to_string(),
        confidence: TRAVEL_TO_CONTRACT_LOCATION_CONFIDENCE,
        window_secs: TRAVEL_TO_CONTRACT_LOCATION_WINDOW_SECS,
        apply: std::sync::Arc::new(|env, _window| {
            let selected = match env.event.as_ref()? {
                GameEvent::MissionQuantumDestinationSelected(s) => s,
                _ => return None,
            };
            // Intent only. Never mark travel started/completed here —
            // that is a separate rule's job (see the doc comment).
            let event = GameEvent::TravelToContractLocation(TravelToContractLocation {
                timestamp: selected.timestamp.clone(),
                beacon_id: selected.beacon_id.clone(),
                travel_started: false,
                travel_completed: false,
            });
            Some(InferenceMatch {
                event,
                source_event_ids: vec![env.idempotency_key.clone()],
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{
        LocationInventoryRequested, MissionQuantumDestinationSelected, PlanetTerrainLoad,
        PlayerDeath, QuantumTargetPhase, QuantumTargetSelected, ResolveSpawn, ShopBuyRequest,
        ShopFlowResponse, VehicleDestruction,
    };
    use crate::wire::LogSource;

    fn make_envelope(event: GameEvent, idk: &str) -> EventEnvelope {
        EventEnvelope {
            idempotency_key: idk.into(),
            raw_line: format!("synthetic_for_{idk}"),
            event: Some(event),
            source: LogSource::Live,
            source_offset: 0,
            metadata: None,
            resolved_location: None,
        }
    }

    #[test]
    fn infer_on_empty_stream_returns_no_inferences() {
        assert!(infer(&[], &InferenceConfig::default()).is_empty());
    }

    #[test]
    fn implicit_death_emitted_when_vehicle_destruction_followed_by_resolve_spawn() {
        let veh = make_envelope(
            GameEvent::VehicleDestruction(VehicleDestruction {
                timestamp: "2026-05-17T14:02:30Z".into(),
                vehicle_class: "Cutlass".into(),
                vehicle_id: Some("v1".into()),
                destroy_level: 2,
                caused_by: "self".into(),
                zone: None,
            }),
            "envA",
        );
        let resp = make_envelope(
            GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: "2026-05-17T14:02:35Z".into(),
                player_geid: "Jim".into(),
                fallback: false,
            }),
            "envB",
        );
        let out = infer(&[veh, resp], &InferenceConfig::default());
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].event, GameEvent::PlayerDeath(_)));
        assert_eq!(out[0].metadata.source, EventSource::Inferred);
        assert!((out[0].metadata.confidence - 0.85).abs() < 0.001);
        assert_eq!(
            out[0].metadata.rule_id.as_deref(),
            Some("implicit_death_after_vehicle_destruction")
        );
        assert_eq!(out[0].metadata.inference_inputs.len(), 2);
        assert_eq!(out[0].trigger_idempotency_key, "envA");
    }

    #[test]
    fn implicit_death_not_emitted_when_resolve_spawn_too_late() {
        let veh = make_envelope(
            GameEvent::VehicleDestruction(VehicleDestruction {
                timestamp: "2026-05-17T14:02:30Z".into(),
                vehicle_class: "Cutlass".into(),
                vehicle_id: Some("v1".into()),
                destroy_level: 2,
                caused_by: "self".into(),
                zone: None,
            }),
            "envA",
        );
        let resp = make_envelope(
            GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: "2026-05-17T14:02:50Z".into(),
                player_geid: "Jim".into(),
                fallback: false,
            }),
            "envB",
        );
        assert!(infer(&[veh, resp], &InferenceConfig::default()).is_empty());
    }

    #[test]
    fn implicit_death_not_emitted_when_no_resolve_spawn() {
        let veh = make_envelope(
            GameEvent::VehicleDestruction(VehicleDestruction {
                timestamp: "2026-05-17T14:02:30Z".into(),
                vehicle_class: "Cutlass".into(),
                vehicle_id: Some("v1".into()),
                destroy_level: 2,
                caused_by: "self".into(),
                zone: None,
            }),
            "envA",
        );
        assert!(infer(&[veh], &InferenceConfig::default()).is_empty());
    }

    fn planet_load(ts: &str, planet: &str, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::PlanetTerrainLoad(PlanetTerrainLoad {
                timestamp: ts.into(),
                planet: planet.into(),
            }),
            idk,
        )
    }

    #[test]
    fn window_within_secs_excludes_out_of_order_and_out_of_window() {
        // Trigger at T+10s. The window (positional-after-trigger) holds a
        // stale earlier row + a same-instant row + an in-window row + a
        // past-window row. Only the strictly-after, in-window row survives,
        // guarding the negative-duration pairing an out-of-order timestamp
        // would otherwise cause (F8).
        let trigger = planet_load("2026-01-01T00:00:10Z", "Trigger", "trig");
        let window = vec![
            planet_load("2026-01-01T00:00:05Z", "A", "before"), // delta -5 -> excluded
            planet_load("2026-01-01T00:00:10Z", "B", "same"),   // delta  0 -> excluded (strict)
            planet_load("2026-01-01T00:00:12Z", "C", "within"), // delta +2 -> kept
            planet_load("2026-01-01T00:01:20Z", "D", "past"),   // delta +70 -> excluded (win 30)
        ];
        let kept = window_within_secs(&trigger, &window, 30, "test_rule");
        let ids: Vec<&str> = kept.iter().map(|e| e.idempotency_key.as_str()).collect();
        assert_eq!(
            ids,
            vec!["within"],
            "only strictly-after, in-window envelopes survive"
        );
    }

    #[test]
    fn implicit_location_change_emitted_for_new_planet() {
        let a = planet_load("2026-05-17T14:00:00Z", "OOC_Stanton_1_Hurston", "envA");
        let b = planet_load("2026-05-17T14:05:00Z", "OOC_Stanton_2b_Daymar", "envB");
        let out = infer(&[a, b], &InferenceConfig::default());
        assert_eq!(out.len(), 1);
        match &out[0].event {
            GameEvent::LocationChanged(lc) => {
                assert_eq!(lc.from.as_deref(), Some("OOC_Stanton_1_Hurston"));
                assert_eq!(lc.to, "OOC_Stanton_2b_Daymar");
            }
            other => panic!("expected LocationChanged, got {:?}", other),
        }
        assert_eq!(
            out[0].metadata.rule_id.as_deref(),
            Some("implicit_location_change")
        );
        assert!((out[0].metadata.confidence - 0.70).abs() < 0.001);
        assert_eq!(out[0].metadata.inference_inputs.len(), 2);
    }

    #[test]
    fn implicit_location_change_not_emitted_when_inventory_in_between() {
        let a = planet_load("2026-05-17T14:00:00Z", "OOC_Stanton_1_Hurston", "envA");
        let inv = make_envelope(
            GameEvent::LocationInventoryRequested(LocationInventoryRequested {
                timestamp: "2026-05-17T14:02:00Z".into(),
                player: "alice".into(),
                location: "Stanton1_Lorville".into(),
            }),
            "envInv",
        );
        let b = planet_load("2026-05-17T14:05:00Z", "OOC_Stanton_2b_Daymar", "envB");
        let out = infer(&[a, inv, b], &InferenceConfig::default());
        // Inventory between planets breaks the chain — no LocationChanged
        // surfaces because the observed inventory request already anchors
        // the transition.
        let lcs: Vec<_> = out
            .iter()
            .filter(|i| matches!(i.event, GameEvent::LocationChanged(_)))
            .collect();
        assert!(lcs.is_empty(), "expected no LocationChanged, got {:?}", lcs);
    }

    #[test]
    fn implicit_location_change_not_emitted_for_same_planet() {
        let a = planet_load("2026-05-17T14:00:00Z", "OOC_Stanton_2b_Daymar", "envA");
        let b = planet_load("2026-05-17T14:05:00Z", "OOC_Stanton_2b_Daymar", "envB");
        let out = infer(&[a, b], &InferenceConfig::default());
        assert!(out.is_empty());
    }

    fn shop_buy(ts: &str, shop_id: Option<&str>, item: Option<&str>, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::ShopBuyRequest(ShopBuyRequest {
                timestamp: ts.into(),
                shop_id: shop_id.map(str::to_string),
                item_class: item.map(str::to_string),
                quantity: None,
                raw: "r".into(),
                price: None,
                shop_name: None,
            }),
            idk,
        )
    }

    fn shop_response(ts: &str, shop_id: Option<&str>, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::ShopFlowResponse(ShopFlowResponse {
                timestamp: ts.into(),
                shop_id: shop_id.map(str::to_string),
                success: Some(true),
                raw: "r".into(),
            }),
            idk,
        )
    }

    #[test]
    fn implicit_shop_request_timeout_emitted_when_no_response() {
        let req = shop_buy(
            "2026-05-17T14:00:00Z",
            Some("kiosk_1"),
            Some("rsi_rifle"),
            "envReq",
        );
        let out = infer(&[req], &InferenceConfig::default());
        assert_eq!(out.len(), 1);
        match &out[0].event {
            GameEvent::ShopRequestTimedOut(s) => {
                assert_eq!(s.shop_id.as_deref(), Some("kiosk_1"));
                assert_eq!(s.item_class.as_deref(), Some("rsi_rifle"));
                assert_eq!(s.timed_out_after_secs, 30);
            }
            other => panic!("expected ShopRequestTimedOut, got {:?}", other),
        }
        assert_eq!(
            out[0].metadata.rule_id.as_deref(),
            Some("implicit_shop_request_timeout")
        );
        assert!((out[0].metadata.confidence - 0.90).abs() < 0.001);
        assert_eq!(out[0].metadata.inference_inputs, vec!["envReq"]);
    }

    #[test]
    fn implicit_shop_request_timeout_not_emitted_when_response_in_window() {
        let req = shop_buy(
            "2026-05-17T14:00:00Z",
            Some("kiosk_1"),
            Some("rsi_rifle"),
            "envReq",
        );
        let resp = shop_response("2026-05-17T14:00:15Z", Some("kiosk_1"), "envResp");
        let out = infer(&[req, resp], &InferenceConfig::default());
        let timeouts: Vec<_> = out
            .iter()
            .filter(|i| matches!(i.event, GameEvent::ShopRequestTimedOut(_)))
            .collect();
        assert!(timeouts.is_empty());
    }

    fn vehicle_destruction(ts: &str, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::VehicleDestruction(VehicleDestruction {
                timestamp: ts.into(),
                vehicle_class: "Cutlass".into(),
                vehicle_id: Some("v1".into()),
                destroy_level: 2,
                caused_by: "self".into(),
                zone: None,
            }),
            idk,
        )
    }

    fn resolve_spawn(ts: &str, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: ts.into(),
                player_geid: "Jim".into(),
                fallback: false,
            }),
            idk,
        )
    }

    fn observed_player_death(ts: &str, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::PlayerDeath(PlayerDeath {
                timestamp: ts.into(),
                body_class: "body_01_noMagicPocket".into(),
                body_id: "id1".into(),
                zone: None,
            }),
            idk,
        )
    }

    #[test]
    fn inferred_death_superseded_by_observed_player_death() {
        // Vehicle blows up → spawn resolves → real PlayerDeath lands 6s
        // after the synthesised one. Default reconciliation window is 5s,
        // so we widen it for this case.
        let veh = vehicle_destruction("2026-05-17T14:02:30Z", "envA");
        let resp = resolve_spawn("2026-05-17T14:02:35Z", "envB");
        let observed = observed_player_death("2026-05-17T14:02:33Z", "envObsDeath");
        let cfg = InferenceConfig {
            reconciliation_secs: 10,
            ..InferenceConfig::default()
        };
        let out = infer(&[veh, resp, observed], &cfg);
        let inferred_deaths: Vec<_> = out
            .iter()
            .filter(|i| matches!(i.event, GameEvent::PlayerDeath(_)))
            .collect();
        assert_eq!(inferred_deaths.len(), 1);
        assert_eq!(
            inferred_deaths[0].superseded_by.as_deref(),
            Some("envObsDeath")
        );
    }

    #[test]
    fn inferred_death_not_superseded_when_observed_too_late() {
        let veh = vehicle_destruction("2026-05-17T14:02:30Z", "envA");
        let resp = resolve_spawn("2026-05-17T14:02:35Z", "envB");
        // Observed death lands 20s after the inferred timestamp —
        // outside the default 5s window.
        let observed = observed_player_death("2026-05-17T14:02:50Z", "envObsDeath");
        let out = infer(&[veh, resp, observed], &InferenceConfig::default());
        let inferred_deaths: Vec<_> = out
            .iter()
            .filter(|i| matches!(i.event, GameEvent::PlayerDeath(_)))
            .collect();
        assert_eq!(inferred_deaths.len(), 1);
        assert!(inferred_deaths[0].superseded_by.is_none());
    }

    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;

    /// Strategy: monotonic-timestamped envelopes drawn from the event
    /// kinds the inference pass actually inspects. Keeps the search
    /// space small so the test stays fast — exhaustive coverage isn't
    /// the goal, varied combinations are.
    fn arb_event_stream() -> impl Strategy<Value = Vec<EventEnvelope>> {
        // 0..6 → pick which event kind. Each picks reuses the same
        // index for the idempotency_key so successive shrinks produce
        // stable, distinct keys.
        prop_vec(0u8..6, 1..20usize).prop_map(|kinds| {
            kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| {
                    // Monotonic seconds-offset timestamps. The same
                    // index drives both the timestamp and the
                    // idempotency_key so the input is deterministic
                    // for any sampled kind sequence.
                    let ts = format!("2026-05-17T14:00:{:02}Z", (i % 60) as u32);
                    let idk = format!("env{i}");
                    let event = match kind {
                        0 => GameEvent::VehicleDestruction(VehicleDestruction {
                            timestamp: ts,
                            vehicle_class: "Cutlass".into(),
                            vehicle_id: Some("v1".into()),
                            destroy_level: 2,
                            caused_by: "self".into(),
                            zone: None,
                        }),
                        1 => GameEvent::ResolveSpawn(ResolveSpawn {
                            timestamp: ts,
                            player_geid: "Jim".into(),
                            fallback: false,
                        }),
                        2 => GameEvent::PlanetTerrainLoad(PlanetTerrainLoad {
                            timestamp: ts,
                            planet: format!("planet_{}", i % 3),
                        }),
                        3 => GameEvent::ShopBuyRequest(ShopBuyRequest {
                            timestamp: ts,
                            shop_id: Some("kiosk_1".into()),
                            item_class: Some("rsi_rifle".into()),
                            quantity: None,
                            raw: "r".into(),
                            price: None,
                            shop_name: None,
                        }),
                        4 => GameEvent::ShopFlowResponse(ShopFlowResponse {
                            timestamp: ts,
                            shop_id: Some("kiosk_1".into()),
                            success: Some(true),
                            raw: "r".into(),
                        }),
                        _ => GameEvent::PlayerDeath(PlayerDeath {
                            timestamp: ts,
                            body_class: "body_01_noMagicPocket".into(),
                            body_id: format!("body_{i}"),
                            zone: None,
                        }),
                    };
                    make_envelope(event, &idk)
                })
                .collect()
        })
    }

    proptest! {
        #[test]
        fn infer_is_idempotent_on_same_input(events in arb_event_stream()) {
            let cfg = InferenceConfig::default();
            let r1 = infer(&events, &cfg);
            let r2 = infer(&events, &cfg);
            prop_assert_eq!(r1, r2);
        }
    }

    #[test]
    fn implicit_shop_request_timeout_emitted_when_response_too_late() {
        let req = shop_buy(
            "2026-05-17T14:00:00Z",
            Some("kiosk_1"),
            Some("rsi_rifle"),
            "envReq",
        );
        // Response lands 45s later — well past the 30s SLO.
        let resp = shop_response("2026-05-17T14:00:45Z", Some("kiosk_1"), "envResp");
        let out = infer(&[req, resp], &InferenceConfig::default());
        let timeouts: Vec<_> = out
            .iter()
            .filter(|i| matches!(i.event, GameEvent::ShopRequestTimedOut(_)))
            .collect();
        assert_eq!(timeouts.len(), 1);
    }

    // --- travel_to_contract_location inference (M9) ---

    fn mission_selected(ts: &str, beacon: &str, idk: &str) -> EventEnvelope {
        make_envelope(
            GameEvent::MissionQuantumDestinationSelected(MissionQuantumDestinationSelected {
                timestamp: ts.into(),
                beacon_id: beacon.into(),
                is_mission_destination: true,
                travel_confirmed: false,
            }),
            idk,
        )
    }

    /// Test 10 (+ evidence + confidence): a mission-beacon selection
    /// yields a `TravelToContractLocation` inference off the single
    /// trigger, with HIGH confidence and the trigger as its evidence.
    #[test]
    fn travel_to_contract_location_inferred_from_mission_selection() {
        let sel = mission_selected("2026-07-20T17:45:35.741Z", "720518925602", "envSel");
        let out = infer(&[sel], &InferenceConfig::default());
        let travels: Vec<_> = out
            .iter()
            .filter(|i| matches!(i.event, GameEvent::TravelToContractLocation(_)))
            .collect();
        assert_eq!(travels.len(), 1, "exactly one travel-intent inference");
        let row = travels[0];
        match &row.event {
            GameEvent::TravelToContractLocation(t) => {
                assert_eq!(t.beacon_id, "720518925602");
            }
            other => panic!("expected TravelToContractLocation, got {other:?}"),
        }
        assert_eq!(row.metadata.source, EventSource::Inferred);
        assert!(
            (row.metadata.confidence - 0.90).abs() < 0.001,
            "HIGH confidence"
        );
        // Evidence == the observed mission-selection event.
        assert_eq!(row.metadata.inference_inputs, vec!["envSel".to_string()]);
        assert_eq!(
            row.metadata.rule_id.as_deref(),
            Some("travel_to_contract_location")
        );
        // No observed TravelToContractLocation exists, so it is not
        // superseded — the intent inference stands.
        assert!(row.superseded_by.is_none());
    }

    /// Test 11: the inference does NOT mark travel as started.
    #[test]
    fn travel_intent_does_not_mark_started() {
        let sel = mission_selected("2026-07-20T17:45:35.741Z", "720518925602", "envSel");
        let out = infer(&[sel], &InferenceConfig::default());
        match &out
            .iter()
            .find(|i| matches!(i.event, GameEvent::TravelToContractLocation(_)))
            .expect("travel intent inferred")
            .event
        {
            GameEvent::TravelToContractLocation(t) => {
                assert!(
                    !t.travel_started,
                    "selection proves intent, not travel start"
                );
            }
            other => panic!("expected TravelToContractLocation, got {other:?}"),
        }
    }

    /// Test 12: the inference does NOT mark travel as completed.
    #[test]
    fn travel_intent_does_not_mark_completed() {
        let sel = mission_selected("2026-07-20T17:45:35.741Z", "720518925602", "envSel");
        let out = infer(&[sel], &InferenceConfig::default());
        match &out
            .iter()
            .find(|i| matches!(i.event, GameEvent::TravelToContractLocation(_)))
            .expect("travel intent inferred")
            .event
        {
            GameEvent::TravelToContractLocation(t) => {
                assert!(!t.travel_completed, "arrival is a separate, later signal");
            }
            other => panic!("expected TravelToContractLocation, got {other:?}"),
        }
    }

    /// Test 14: the existing quantum-nav path is unaffected — a generic
    /// `QuantumTargetSelected` never triggers the mission travel-intent
    /// rule (no conflation of a plain destination with a contract), and
    /// the mission rule is registered alongside the pre-existing built-ins.
    #[test]
    fn generic_quantum_target_does_not_infer_travel_intent() {
        let generic = make_envelope(
            GameEvent::QuantumTargetSelected(QuantumTargetSelected {
                timestamp: "2026-05-03T18:03:22.115Z".into(),
                phase: QuantumTargetPhase::Selected,
                vehicle_class: "CRUS_Starfighter_Ion".into(),
                vehicle_id: "10012175043656".into(),
                destination: "OOC_Stanton_2_Crusader".into(),
            }),
            "envGeneric",
        );
        let out = infer(&[generic], &InferenceConfig::default());
        assert!(
            !out.iter()
                .any(|i| matches!(i.event, GameEvent::TravelToContractLocation(_))),
            "a plain quantum destination must not infer contract travel intent"
        );
        // The new rule is registered without dropping the originals.
        assert_eq!(built_in_inference_rules().len(), 4);
    }
}
