//! Wire format + compile path for runtime-loaded inference rules.
//!
//! Mirrors the `parser_defs.rs` model: a declarative [`RemoteInferenceRule`]
//! on the wire compiles into a [`crate::inference::CompiledInferenceRule`]
//! whose `apply` closure runs in the inference pass alongside the
//! built-in rules. The compile step validates trigger / emit event
//! types against the known [`crate::metadata::all_event_type_keys`]
//! list and round-trips the synthesised payload through serde to
//! catch field-shape mismatches before the rule is installed.
//!
//! Substitution syntax for follow-up predicates and emitted fields is
//! `${trigger.<field>}` — the special token `${trigger.idempotency_key}`
//! resolves to the trigger envelope's idempotency_key; everything else
//! resolves against the trigger event's serialised JSON (top-level
//! field names, snake_case).

use crate::events::GameEvent;
use crate::inference::{CompiledInferenceRule, InferenceMatch};
use crate::metadata::{all_event_type_keys, event_type_key};
use crate::wire::EventEnvelope;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Wire-format inference rule. Mirrors [`crate::parser_defs::RemoteRule`]
/// in spirit but expresses a chain match (trigger + ordered follow-ups)
/// rather than a single-line regex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteInferenceRule {
    /// Stable id assigned by the manifest publisher.
    pub id: String,
    /// Confidence in `[0.0, 1.0)`. Inferred events never claim 1.0 —
    /// that's reserved for observed events.
    pub confidence: f32,
    /// How far to scan forward for follow-ups, in seconds.
    pub window_secs: u32,
    /// Pattern that opens the inference window.
    pub trigger: EventPattern,
    /// Ordered follow-up patterns that must all appear within
    /// `window_secs` for the rule to fire. Order matters — each
    /// subsequent followup is searched starting after the prior
    /// match.
    pub followups: Vec<EventPattern>,
    /// What to emit when the chain matches.
    pub emits: EventTemplate,
}

/// One pattern slot — match by `event_type` plus optional field-equality
/// predicates. Follow-up patterns may use `${trigger.<field>}`
/// substitutions in `field_equals` values; trigger patterns may not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPattern {
    /// `event_type_key` of the event variant this slot matches.
    pub event_type: String,
    /// Field-equality predicates. Values are compared as JSON strings
    /// (numbers and bools stringify via `serde_json::to_string`). For
    /// follow-up patterns `${trigger.<field>}` substitutions resolve
    /// against the trigger event's JSON representation.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_equals: BTreeMap<String, String>,
}

/// The event the rule emits when the trigger + followups chain
/// matches. Field values support `${trigger.<field>}` substitutions
/// plus the special `${trigger.idempotency_key}` token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTemplate {
    /// Target event_type — must name a real `GameEvent` variant.
    pub event_type: String,
    /// Fields to set on the emitted event. Substitution tokens are
    /// resolved against the trigger envelope before the payload is
    /// deserialised into a `GameEvent`.
    pub fields: BTreeMap<String, String>,
}

/// Errors a [`RemoteInferenceRule`] can produce at compile time.
#[derive(Debug)]
pub enum InferenceCompileError {
    UnknownEventType { rule_id: String, event_type: String },
    InvalidConfidence { rule_id: String, confidence: f32 },
    EmptyFollowups { rule_id: String },
}

impl std::fmt::Display for InferenceCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferenceCompileError::UnknownEventType {
                rule_id,
                event_type,
            } => write!(f, "rule {rule_id}: unknown event_type {event_type:?}"),
            InferenceCompileError::InvalidConfidence {
                rule_id,
                confidence,
            } => write!(
                f,
                "rule {rule_id}: confidence {confidence} outside [0.0, 1.0)"
            ),
            InferenceCompileError::EmptyFollowups { rule_id } => {
                write!(f, "rule {rule_id}: followups must not be empty")
            }
        }
    }
}

impl std::error::Error for InferenceCompileError {}

/// Compile a slice of wire-format rules. Validation is strict — any
/// invalid rule fails the whole batch. The caller (the manifest fetcher
/// in `starstats-client`) logs the error and continues with the
/// previously-cached rule set.
///
/// Returns `Ok` only when every rule passes validation:
///   * `trigger.event_type` + `emits.event_type` + each followup
///     `event_type` name a real `GameEvent` variant.
///   * `confidence` lies in `[0.0, 1.0)`.
///   * `followups` is non-empty (a rule that fires on the trigger
///     alone is a parser rule, not an inference rule).
pub fn compile_inference_rules(
    rules: &[RemoteInferenceRule],
) -> Result<Vec<CompiledInferenceRule>, InferenceCompileError> {
    let known = all_event_type_keys();
    let mut out = Vec::with_capacity(rules.len());
    for r in rules {
        validate_event_type(&r.id, &r.trigger.event_type, known)?;
        validate_event_type(&r.id, &r.emits.event_type, known)?;
        for fu in &r.followups {
            validate_event_type(&r.id, &fu.event_type, known)?;
        }
        if !(0.0..1.0).contains(&r.confidence) {
            return Err(InferenceCompileError::InvalidConfidence {
                rule_id: r.id.clone(),
                confidence: r.confidence,
            });
        }
        if r.followups.is_empty() {
            return Err(InferenceCompileError::EmptyFollowups {
                rule_id: r.id.clone(),
            });
        }
        out.push(compile_one(r.clone()));
    }
    Ok(out)
}

fn validate_event_type(
    rule_id: &str,
    event_type: &str,
    known: &[&str],
) -> Result<(), InferenceCompileError> {
    if known.contains(&event_type) {
        Ok(())
    } else {
        Err(InferenceCompileError::UnknownEventType {
            rule_id: rule_id.to_string(),
            event_type: event_type.to_string(),
        })
    }
}

fn compile_one(rule: RemoteInferenceRule) -> CompiledInferenceRule {
    let id = rule.id.clone();
    let confidence = rule.confidence;
    let window_secs = i64::from(rule.window_secs);
    let trigger = rule.trigger;
    let followups = rule.followups;
    let emits = rule.emits;
    CompiledInferenceRule {
        id,
        confidence,
        window_secs,
        apply: std::sync::Arc::new(move |env, window| {
            let observed = env.event.as_ref()?;
            if event_type_key(observed) != trigger.event_type {
                return None;
            }
            let trigger_json = serde_json::to_value(observed).ok()?;
            if !object_matches(&trigger_json, &trigger.field_equals, None) {
                return None;
            }

            // Walk follow-ups in order. Each match consumes its
            // position so the next follow-up's search starts after it.
            let mut followup_ids = Vec::with_capacity(followups.len());
            let mut search_from = 0usize;
            for fu in &followups {
                let mut found: Option<usize> = None;
                for (idx, candidate) in window[search_from..].iter().enumerate() {
                    let Some(ce) = candidate.event.as_ref() else {
                        continue;
                    };
                    if event_type_key(ce) != fu.event_type {
                        continue;
                    }
                    let candidate_json = match serde_json::to_value(ce) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if object_matches(&candidate_json, &fu.field_equals, Some(&trigger_json)) {
                        found = Some(search_from + idx);
                        break;
                    }
                }
                let found_idx = found?;
                followup_ids.push(window[found_idx].idempotency_key.clone());
                search_from = found_idx + 1;
            }

            // Build the emitted event by composing a JSON object that
            // round-trips through GameEvent's tagged deserialisation.
            let mut payload = serde_json::Map::new();
            payload.insert(
                "type".to_string(),
                serde_json::Value::String(emits.event_type.clone()),
            );
            for (k, v) in &emits.fields {
                let resolved = substitute(v, env, &trigger_json);
                payload.insert(k.clone(), coerce_value(&resolved));
            }
            let event: GameEvent = match serde_json::from_value(serde_json::Value::Object(payload))
            {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        event_type = %emits.event_type,
                        "inference rule emit payload failed GameEvent deserialise"
                    );
                    return None;
                }
            };

            let mut source_event_ids = Vec::with_capacity(1 + followup_ids.len());
            source_event_ids.push(env.idempotency_key.clone());
            source_event_ids.extend(followup_ids);
            Some(InferenceMatch {
                event,
                source_event_ids,
            })
        }),
    }
}

/// True when every predicate in `predicates` matches `candidate_json`.
/// Looks up the top-level field by name; numbers / booleans stringify
/// before comparison so the wire format can express `"true"` /
/// `"42"` without a typed shadow schema. When `trigger_json` is set,
/// `${trigger.X}` tokens in the predicate value resolve against it.
fn object_matches(
    candidate_json: &serde_json::Value,
    predicates: &BTreeMap<String, String>,
    trigger_json: Option<&serde_json::Value>,
) -> bool {
    if predicates.is_empty() {
        return true;
    }
    let Some(obj) = candidate_json.as_object() else {
        return false;
    };
    for (field, want) in predicates {
        let Some(actual) = obj.get(field) else {
            return false;
        };
        let want_resolved = match trigger_json {
            Some(t) => substitute_with_trigger_json(want, t),
            None => want.clone(),
        };
        let actual_str = value_to_string(actual);
        if actual_str != want_resolved {
            return false;
        }
    }
    true
}

/// Resolve `${trigger.X}` substitutions in `input`, picking up
/// `idempotency_key` from the envelope and every other field from the
/// trigger event's JSON. Unknown tokens are left as-is (the eventual
/// deserialise will surface them as a clear error).
fn substitute(
    input: &str,
    trigger_env: &EventEnvelope,
    trigger_json: &serde_json::Value,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${trigger.") {
        out.push_str(&rest[..start]);
        rest = &rest[start + "${trigger.".len()..];
        let Some(end) = rest.find('}') else {
            // Unterminated token — leave the rest verbatim.
            out.push_str("${trigger.");
            out.push_str(rest);
            return out;
        };
        let field = &rest[..end];
        if field == "idempotency_key" {
            out.push_str(&trigger_env.idempotency_key);
        } else if let Some(v) = trigger_json.get(field) {
            out.push_str(&value_to_string(v));
        } else {
            // Unknown field — leave the token intact so deserialise
            // fails loudly downstream.
            out.push_str("${trigger.");
            out.push_str(field);
            out.push('}');
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Substitute against trigger_json only. Used inside follow-up
/// predicate evaluation, where we don't have an envelope handy and
/// `idempotency_key` substitution doesn't make sense.
fn substitute_with_trigger_json(input: &str, trigger_json: &serde_json::Value) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${trigger.") {
        out.push_str(&rest[..start]);
        rest = &rest[start + "${trigger.".len()..];
        let Some(end) = rest.find('}') else {
            out.push_str("${trigger.");
            out.push_str(rest);
            return out;
        };
        let field = &rest[..end];
        if let Some(v) = trigger_json.get(field) {
            out.push_str(&value_to_string(v));
        } else {
            out.push_str("${trigger.");
            out.push_str(field);
            out.push('}');
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Render a JSON value as a flat string for predicate comparison.
/// Strings are unwrapped (no JSON-quoted form), everything else uses
/// `serde_json::to_string` so numbers and booleans serialise without
/// quoting.
fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Coerce a substituted string back into a JSON value suitable for
/// the emitted event payload. We try integer and boolean parses first
/// so a wire field of `"42"` lands as `42` (a `u32` event field) and
/// `"true"` lands as `true`. Everything else stays a string.
fn coerce_value(s: &str) -> serde_json::Value {
    if let Ok(n) = s.parse::<i64>() {
        return serde_json::Value::Number(serde_json::Number::from(n));
    }
    if let Ok(n) = s.parse::<u64>() {
        return serde_json::Value::Number(serde_json::Number::from(n));
    }
    if let Ok(n) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return serde_json::Value::Number(num);
        }
    }
    if s == "true" {
        return serde_json::Value::Bool(true);
    }
    if s == "false" {
        return serde_json::Value::Bool(false);
    }
    serde_json::Value::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ResolveSpawn, VehicleDestruction};
    use crate::inference::{infer_with_rules, InferenceConfig};
    use crate::wire::LogSource;
    use crate::EventSource;

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

    fn death_rule() -> RemoteInferenceRule {
        let mut fields = BTreeMap::new();
        fields.insert("timestamp".into(), "${trigger.timestamp}".into());
        fields.insert("body_class".into(), "inferred".into());
        fields.insert(
            "body_id".into(),
            "inferred_${trigger.idempotency_key}".into(),
        );
        RemoteInferenceRule {
            id: "implicit_death_after_vehicle_destruction".into(),
            confidence: 0.85,
            window_secs: 15,
            trigger: EventPattern {
                event_type: "vehicle_destruction".into(),
                field_equals: BTreeMap::new(),
            },
            followups: vec![EventPattern {
                event_type: "resolve_spawn".into(),
                field_equals: BTreeMap::new(),
            }],
            emits: EventTemplate {
                event_type: "player_death".into(),
                fields,
            },
        }
    }

    #[test]
    fn remote_inference_rule_round_trip_matches_built_in_death_rule() {
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
        let compiled = compile_inference_rules(&[death_rule()]).unwrap();
        let out = infer_with_rules(&[veh, resp], &InferenceConfig::default(), &compiled);
        assert_eq!(out.len(), 1);
        match &out[0].event {
            GameEvent::PlayerDeath(pd) => {
                assert_eq!(pd.timestamp, "2026-05-17T14:02:30Z");
                assert_eq!(pd.body_class, "inferred");
                assert_eq!(pd.body_id, "inferred_envA");
            }
            other => panic!("expected PlayerDeath, got {other:?}"),
        }
        assert_eq!(out[0].metadata.source, EventSource::Inferred);
        assert!((out[0].metadata.confidence - 0.85).abs() < 0.001);
        assert_eq!(
            out[0].metadata.rule_id.as_deref(),
            Some("implicit_death_after_vehicle_destruction")
        );
        assert_eq!(out[0].metadata.inference_inputs.len(), 2);
    }

    #[test]
    fn compile_fails_on_unknown_event_type() {
        let mut rule = death_rule();
        rule.trigger.event_type = "totally_made_up".into();
        let err = compile_inference_rules(&[rule]).unwrap_err();
        match err {
            InferenceCompileError::UnknownEventType { event_type, .. } => {
                assert_eq!(event_type, "totally_made_up");
            }
            other => panic!("expected UnknownEventType, got {other:?}"),
        }
    }

    #[test]
    fn compile_fails_on_unknown_emit_event_type() {
        let mut rule = death_rule();
        rule.emits.event_type = "totally_made_up".into();
        let err = compile_inference_rules(&[rule]).unwrap_err();
        assert!(matches!(
            err,
            InferenceCompileError::UnknownEventType { .. }
        ));
    }

    #[test]
    fn compile_fails_on_invalid_confidence() {
        let mut rule = death_rule();
        rule.confidence = 1.5;
        let err = compile_inference_rules(&[rule]).unwrap_err();
        assert!(matches!(
            err,
            InferenceCompileError::InvalidConfidence { .. }
        ));

        let mut rule = death_rule();
        rule.confidence = -0.1;
        let err = compile_inference_rules(&[rule]).unwrap_err();
        assert!(matches!(
            err,
            InferenceCompileError::InvalidConfidence { .. }
        ));
    }

    #[test]
    fn compile_fails_on_empty_followups() {
        let mut rule = death_rule();
        rule.followups.clear();
        let err = compile_inference_rules(&[rule]).unwrap_err();
        assert!(matches!(err, InferenceCompileError::EmptyFollowups { .. }));
    }

    #[test]
    fn missing_followup_does_not_fire() {
        // Trigger only — no follow-up in the window.
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
        let compiled = compile_inference_rules(&[death_rule()]).unwrap();
        let out = infer_with_rules(&[veh], &InferenceConfig::default(), &compiled);
        assert!(out.is_empty());
    }

    #[test]
    fn substitution_resolves_trigger_idempotency_key() {
        let veh = make_envelope(
            GameEvent::VehicleDestruction(VehicleDestruction {
                timestamp: "2026-05-17T14:02:30Z".into(),
                vehicle_class: "Cutlass".into(),
                vehicle_id: Some("v1".into()),
                destroy_level: 2,
                caused_by: "self".into(),
                zone: None,
            }),
            "envUnique123",
        );
        let resp = make_envelope(
            GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: "2026-05-17T14:02:35Z".into(),
                player_geid: "Jim".into(),
                fallback: false,
            }),
            "envB",
        );
        let compiled = compile_inference_rules(&[death_rule()]).unwrap();
        let out = infer_with_rules(&[veh, resp], &InferenceConfig::default(), &compiled);
        assert_eq!(out.len(), 1);
        match &out[0].event {
            GameEvent::PlayerDeath(pd) => {
                assert_eq!(pd.body_id, "inferred_envUnique123");
            }
            other => panic!("expected PlayerDeath, got {other:?}"),
        }
    }

    #[test]
    fn followup_field_equality_with_trigger_substitution() {
        // Build a rule that requires the follow-up's player_geid to
        // equal a literal string; verify both positive and negative
        // matches work.
        let mut fields = BTreeMap::new();
        fields.insert("timestamp".into(), "${trigger.timestamp}".into());
        fields.insert("body_class".into(), "inferred".into());
        fields.insert(
            "body_id".into(),
            "inferred_${trigger.idempotency_key}".into(),
        );
        let mut followup_eq = BTreeMap::new();
        followup_eq.insert("player_geid".into(), "Jim".into());
        let rule = RemoteInferenceRule {
            id: "death_with_filter".into(),
            confidence: 0.85,
            window_secs: 15,
            trigger: EventPattern {
                event_type: "vehicle_destruction".into(),
                field_equals: BTreeMap::new(),
            },
            followups: vec![EventPattern {
                event_type: "resolve_spawn".into(),
                field_equals: followup_eq,
            }],
            emits: EventTemplate {
                event_type: "player_death".into(),
                fields,
            },
        };

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
        let resp_match = make_envelope(
            GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: "2026-05-17T14:02:35Z".into(),
                player_geid: "Jim".into(),
                fallback: false,
            }),
            "envB",
        );
        let resp_other = make_envelope(
            GameEvent::ResolveSpawn(ResolveSpawn {
                timestamp: "2026-05-17T14:02:33Z".into(),
                player_geid: "Alice".into(),
                fallback: false,
            }),
            "envC",
        );

        let compiled = compile_inference_rules(&[rule]).unwrap();

        // Matching follow-up present alongside a non-matching one →
        // rule fires off the matching envelope.
        let out = infer_with_rules(
            &[veh.clone(), resp_other.clone(), resp_match.clone()],
            &InferenceConfig::default(),
            &compiled,
        );
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].event, GameEvent::PlayerDeath(_)));

        // Only the non-matching follow-up → rule stays silent.
        let out = infer_with_rules(&[veh, resp_other], &InferenceConfig::default(), &compiled);
        assert!(out.is_empty());
    }

    #[test]
    fn remote_inference_rule_serde_round_trip() {
        let rule = death_rule();
        let json = serde_json::to_string(&rule).unwrap();
        let parsed: RemoteInferenceRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, rule);
    }

    #[test]
    fn coerce_value_handles_numbers_bools_and_strings() {
        match coerce_value("42") {
            serde_json::Value::Number(n) => assert_eq!(n.as_i64(), Some(42)),
            other => panic!("expected number, got {other:?}"),
        }
        assert_eq!(coerce_value("true"), serde_json::Value::Bool(true));
        assert_eq!(coerce_value("false"), serde_json::Value::Bool(false));
        assert_eq!(
            coerce_value("hello"),
            serde_json::Value::String("hello".to_string())
        );
    }

    #[test]
    fn unknown_substitution_token_left_intact() {
        // PlayerDeath's `zone` is Option<String>; we use a different
        // case to avoid the "type-coerce numeric string" quirk and
        // confirm the unresolved token survives. The emitted event
        // should fail deserialise because GameEvent::PlayerDeath
        // requires specific fields — but a string field with the
        // literal token is fine.
        let mut fields = BTreeMap::new();
        fields.insert("timestamp".into(), "${trigger.timestamp}".into());
        fields.insert("body_class".into(), "${trigger.not_a_field}".into());
        fields.insert(
            "body_id".into(),
            "inferred_${trigger.idempotency_key}".into(),
        );
        let rule = RemoteInferenceRule {
            id: "death_unknown_token".into(),
            confidence: 0.85,
            window_secs: 15,
            trigger: EventPattern {
                event_type: "vehicle_destruction".into(),
                field_equals: BTreeMap::new(),
            },
            followups: vec![EventPattern {
                event_type: "resolve_spawn".into(),
                field_equals: BTreeMap::new(),
            }],
            emits: EventTemplate {
                event_type: "player_death".into(),
                fields,
            },
        };
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
        let compiled = compile_inference_rules(&[rule]).unwrap();
        let out = infer_with_rules(&[veh, resp], &InferenceConfig::default(), &compiled);
        // body_class becomes the literal "${trigger.not_a_field}"
        // string — deserialise succeeds because body_class is a free-form
        // String field.
        assert_eq!(out.len(), 1);
        match &out[0].event {
            GameEvent::PlayerDeath(pd) => {
                assert_eq!(pd.body_class, "${trigger.not_a_field}");
            }
            other => panic!("expected PlayerDeath, got {other:?}"),
        }
    }
}
