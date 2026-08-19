//! Runtime-loaded parser rules — the data + apply layer for the
//! dynamic-parser-definition feature.
//!
//! Wire format (`RemoteRule`) is what the server's
//! `GET /v1/parser-definitions` endpoint returns. `CompiledRemoteRule`
//! is what the client holds at runtime once the regex has been
//! pre-compiled. `apply_remote_rules` runs after the built-in
//! `classify` returns `None` — it never overrides a built-in match.
//!
//! Architectural rule: this crate stays I/O-free. Fetching, caching,
//! and signature verification live in the consuming crates
//! (`starstats-client` for the fetcher, `starstats-server` for the
//! manifest hosting). All this module does is parse + match.

use crate::events::{GameEvent, RemoteMatch};
use crate::inference_defs::RemoteInferenceRule;
use crate::parser::LogLine;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One rule on the wire. Mirrors the JSON shape documented in
/// `docs/PARSER_DEFINITION_UPDATES.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRule {
    /// Stable id assigned by the manifest publisher. Used to retract
    /// a bad rule without rebuilding clients (server publishes a
    /// fresh manifest with the rule absent or with a `disabled`
    /// flag — for v1 we just rely on absence).
    pub id: String,
    /// Either the `<EventName>` token to match against `LogLine.event_name`
    /// (when the line has a shell), OR a body-keyword to match against
    /// `LogLine.body` for function-call-style entries. The rule's
    /// `match_kind` disambiguates.
    pub event_name: String,
    /// `event_name` matches the `<EventName>` shell.
    /// `body_keyword` matches if `body.contains(event_name)`.
    #[serde(default = "default_match_kind")]
    pub match_kind: RuleMatchKind,
    /// Body regex with optional named captures. Captures listed in
    /// `fields` get extracted into `RemoteMatch.fields`. Anything else
    /// is ignored — extra captures don't error, missing captures don't
    /// fail the match (their fields just don't appear in the output).
    pub body_regex: String,
    /// Names of regex captures to surface as fields. Order is
    /// preserved in the BTreeMap keys for deterministic JSON output.
    #[serde(default)]
    pub fields: Vec<String>,
}

fn default_match_kind() -> RuleMatchKind {
    RuleMatchKind::EventName
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleMatchKind {
    /// Match `LogLine.event_name == rule.event_name`.
    EventName,
    /// Match `LogLine.body.contains(rule.event_name)`.
    BodyKeyword,
}

/// The full manifest shape returned by the server endpoint.
///
/// Adding inference rules is an additive wire change: the new
/// `inference_rules` field is `default` + `skip_serializing_if =
/// "Vec::is_empty"`, so v1 manifests without it deserialise fine and
/// v2 manifests with an empty list serialise without the field.
///
/// `Eq` is dropped from this derive because `RemoteInferenceRule`
/// carries an `f32` confidence, which only implements `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub schema_version: u32,
    pub issued_at: String,
    pub rules: Vec<RemoteRule>,
    /// Inference rules — declarative trigger+follow-up chains that
    /// emit synthesised events when matched against the post-classify
    /// stream. Compiled by
    /// [`crate::inference_defs::compile_inference_rules`].
    ///
    /// Defaults to the empty list for backwards compatibility with v1
    /// manifests; absent / empty values do not appear on the wire.
    ///
    /// NB: the v1 `signature` field is a passthrough `Option<String>`
    /// and no actual signing logic exists in-tree (see the doc
    /// comment on `signature` below). When trust comes online the
    /// signed payload must include `inference_rules` — until then,
    /// inference rules ride on the same TLS-trust assumption as the
    /// `rules` array.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_rules: Vec<RemoteInferenceRule>,
    /// Optional ed25519 signature over the canonicalised `rules`
    /// array. v1 ships unverified — clients trust TLS to the server.
    /// Verification is a follow-up.
    #[serde(default)]
    pub signature: Option<String>,
}

/// Serialisation view of a [`Manifest`] MINUS the `signature` field, in
/// declaration order — the canonical payload the server signs and the
/// client verifies (F10). Kept as an explicit struct (rather than
/// round-tripping the manifest with `signature: None`) so the signed byte
/// string is unambiguous and covers `inference_rules`, per the note on
/// [`Manifest::signature`].
#[derive(Serialize)]
struct ManifestSigningView<'a> {
    version: u32,
    schema_version: u32,
    issued_at: &'a str,
    rules: &'a [RemoteRule],
    inference_rules: &'a [RemoteInferenceRule],
}

/// Canonical bytes over which the parser manifest is signed / verified.
/// Both the server (sign) and the client (verify) MUST derive the payload
/// from THIS function so the byte string is bit-identical — signing over
/// anything the client can't reproduce byte-for-byte would make every
/// signature fail. `serde_json` emits struct fields in declaration order
/// and `Vec`s in element order, and this shape has no maps, so the bytes
/// are deterministic without a canonical-JSON dependency. The `signature`
/// field is excluded (it can't sign itself).
pub fn manifest_signing_bytes(manifest: &Manifest) -> Vec<u8> {
    let view = ManifestSigningView {
        version: manifest.version,
        schema_version: manifest.schema_version,
        issued_at: &manifest.issued_at,
        rules: &manifest.rules,
        inference_rules: &manifest.inference_rules,
    };
    serde_json::to_vec(&view).expect("manifest signing view always serialises")
}

impl Manifest {
    pub fn empty() -> Self {
        Self {
            version: 0,
            schema_version: 1,
            issued_at: String::new(),
            rules: Vec::new(),
            inference_rules: Vec::new(),
            signature: None,
        }
    }
}

/// Runtime-ready rule with a pre-compiled regex. Constructed via
/// [`compile_rules`]; rules whose regex fails to compile are silently
/// dropped (the caller logs the failure during fetch).
#[derive(Debug, Clone)]
pub struct CompiledRemoteRule {
    pub id: String,
    pub event_name: String,
    pub match_kind: RuleMatchKind,
    pub regex: Regex,
    pub fields: Vec<String>,
}

/// Compile a slice of wire-format rules. Returns the compiled subset
/// + a Vec of `(rule_id, error_message)` pairs for any rules whose
/// regex failed to compile. The caller logs the errors; bad rules
/// are not fatal.
pub fn compile_rules(rules: &[RemoteRule]) -> (Vec<CompiledRemoteRule>, Vec<(String, String)>) {
    let mut ok = Vec::with_capacity(rules.len());
    let mut bad = Vec::new();
    for r in rules {
        match Regex::new(&r.body_regex) {
            Ok(rx) => ok.push(CompiledRemoteRule {
                id: r.id.clone(),
                event_name: r.event_name.clone(),
                match_kind: r.match_kind,
                regex: rx,
                fields: r.fields.clone(),
            }),
            Err(e) => bad.push((r.id.clone(), e.to_string())),
        }
    }
    (ok, bad)
}

/// Try the cached remote rules against a log line. Returns the first
/// match or `None`. Order in `rules` matters — rule authors who care
/// about specificity should put narrower rules first.
///
/// This function does not run if the built-in `classify` already
/// produced a `Some` — see the gamelog ingest path. That guarantees
/// remote rules can only *add* recognition, never override built-ins.
pub fn apply_remote_rules(line: &LogLine<'_>, rules: &[CompiledRemoteRule]) -> Option<GameEvent> {
    for rule in rules {
        let matches_anchor = match rule.match_kind {
            RuleMatchKind::EventName => line.event_name == Some(rule.event_name.as_str()),
            RuleMatchKind::BodyKeyword => line.body.contains(rule.event_name.as_str()),
        };
        if !matches_anchor {
            continue;
        }

        let caps = rule.regex.captures(line.body)?;
        let mut fields = BTreeMap::new();
        for f in &rule.fields {
            if let Some(m) = caps.name(f) {
                fields.insert(f.clone(), m.as_str().to_string());
            }
        }
        return Some(GameEvent::RemoteMatch(RemoteMatch {
            timestamp: line.timestamp.to_string(),
            rule_id: rule.id.clone(),
            event_name: rule.event_name.clone(),
            fields,
        }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::structural_parse;

    fn rule(id: &str, event: &str, kind: RuleMatchKind, rx: &str, fields: &[&str]) -> RemoteRule {
        RemoteRule {
            id: id.to_string(),
            event_name: event.to_string(),
            match_kind: kind,
            body_regex: rx.to_string(),
            fields: fields.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn manifest_signing_bytes_exclude_signature_and_track_signed_fields() {
        let mut m = Manifest::empty();
        m.version = 7;
        m.rules = vec![rule(
            "r1",
            "E",
            RuleMatchKind::EventName,
            r"x=(?P<x>\w+)",
            &["x"],
        )];
        let base = manifest_signing_bytes(&m);

        // The signature field is NOT part of the signed payload — setting it
        // must not change the bytes (a signature can't sign itself).
        m.signature = Some("anything".to_string());
        assert_eq!(
            manifest_signing_bytes(&m),
            base,
            "signature must be excluded from the signed bytes"
        );

        // A change to any SIGNED field changes the bytes.
        m.version = 8;
        assert_ne!(manifest_signing_bytes(&m), base);
    }

    #[test]
    fn matches_event_name_anchor() {
        let rules = vec![rule(
            "r1",
            "PlayerDance",
            RuleMatchKind::EventName,
            r"emote=(?P<emote>\w+)",
            &["emote"],
        )];
        let (compiled, bad) = compile_rules(&rules);
        assert!(bad.is_empty());
        let line = "<2026-05-07T15:00:00.000Z> [Notice] <PlayerDance> emote=salute [Team_X]";
        let parsed = structural_parse(line).unwrap();
        let ev = apply_remote_rules(&parsed, &compiled).unwrap();
        match ev {
            GameEvent::RemoteMatch(m) => {
                assert_eq!(m.event_name, "PlayerDance");
                assert_eq!(m.rule_id, "r1");
                assert_eq!(m.fields.get("emote").map(|s| s.as_str()), Some("salute"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn matches_body_keyword_anchor() {
        let rules = vec![rule(
            "r2",
            "SendCustomThing",
            RuleMatchKind::BodyKeyword,
            r"shopId=(?P<shop>\w+)",
            &["shop"],
        )];
        let (compiled, _) = compile_rules(&rules);
        // No <EventName> shell — function-call-style line.
        let line = "<2026-05-07T15:00:00.000Z> [Notice] SendCustomThing(shopId=area18, qty=1)";
        let parsed = structural_parse(line).unwrap();
        let ev = apply_remote_rules(&parsed, &compiled).unwrap();
        match ev {
            GameEvent::RemoteMatch(m) => {
                assert_eq!(m.fields.get("shop").map(|s| s.as_str()), Some("area18"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn returns_none_when_no_rule_matches() {
        let rules = vec![rule(
            "r1",
            "SomethingElse",
            RuleMatchKind::EventName,
            r"x=(?P<x>\d+)",
            &["x"],
        )];
        let (compiled, _) = compile_rules(&rules);
        let line = "<2026-05-07T15:00:00.000Z> [Notice] <PlayerDance> emote=salute";
        let parsed = structural_parse(line).unwrap();
        assert!(apply_remote_rules(&parsed, &compiled).is_none());
    }

    #[test]
    fn bad_regex_lands_in_error_list_not_compiled() {
        let rules = vec![rule(
            "r1",
            "X",
            RuleMatchKind::EventName,
            "[unclosed",
            &["x"],
        )];
        let (ok, bad) = compile_rules(&rules);
        assert!(ok.is_empty());
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, "r1");
    }

    #[test]
    fn manifest_round_trips_without_inference_rules_for_backcompat() {
        // A v1 manifest payload (no inference_rules field). It must
        // deserialise cleanly and produce an empty inference_rules
        // vec — old clients on the network speak this shape.
        let v1_payload = r#"{
            "version": 1,
            "schema_version": 1,
            "issued_at": "2026-05-07T00:00:00Z",
            "rules": [],
            "signature": null
        }"#;
        let parsed: Manifest = serde_json::from_str(v1_payload).unwrap();
        assert_eq!(parsed.version, 1);
        assert!(parsed.inference_rules.is_empty());
        assert!(parsed.rules.is_empty());

        // Re-serialising the empty manifest must NOT add an
        // `inference_rules` key — `skip_serializing_if = "Vec::is_empty"`
        // guarantees the field is absent unless populated, so a v2
        // manifest with no inference rules is byte-identical to a v1
        // manifest on the wire (modulo whitespace).
        let serialised = serde_json::to_value(&parsed).unwrap();
        assert!(!serialised
            .as_object()
            .unwrap()
            .contains_key("inference_rules"));
    }

    #[test]
    fn manifest_serialises_inference_rules_when_present() {
        use crate::inference_defs::{EventPattern, EventTemplate, RemoteInferenceRule};
        use std::collections::BTreeMap;
        let manifest = Manifest {
            version: 2,
            schema_version: 1,
            issued_at: "2026-05-18T00:00:00Z".into(),
            rules: Vec::new(),
            inference_rules: vec![RemoteInferenceRule {
                id: "demo".into(),
                confidence: 0.5,
                window_secs: 10,
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
                    fields: BTreeMap::new(),
                },
            }],
            signature: None,
        };
        let value = serde_json::to_value(&manifest).unwrap();
        let inference_rules = value.get("inference_rules").unwrap();
        assert_eq!(inference_rules.as_array().unwrap().len(), 1);
        let round_trip: Manifest = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, manifest);
    }

    #[test]
    fn missing_capture_field_is_silently_omitted() {
        // Regex matches but doesn't capture `expected_field` — the
        // output map should just lack that key, not error.
        let rules = vec![rule(
            "r1",
            "X",
            RuleMatchKind::EventName,
            r"present=(?P<present>\w+)",
            &["present", "expected_field"],
        )];
        let (compiled, _) = compile_rules(&rules);
        let line = "<2026-05-07T15:00:00.000Z> [Notice] <X> present=hi";
        let parsed = structural_parse(line).unwrap();
        let ev = apply_remote_rules(&parsed, &compiled).unwrap();
        match ev {
            GameEvent::RemoteMatch(m) => {
                assert!(m.fields.contains_key("present"));
                assert!(!m.fields.contains_key("expected_field"));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
