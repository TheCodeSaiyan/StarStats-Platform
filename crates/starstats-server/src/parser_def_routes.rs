//! Public, read-only endpoint that hosts the runtime parser-definition
//! manifest fetched by the tray client.
//!
//! `GET /v1/parser-definitions` returns a [`Manifest`] from
//! `starstats-core`, served from the DB-backed `parser_rules` table
//! (migration 0048) via [`crate::parser_rules::ParserRulesStore`] — the
//! enabled rows become the manifest's `rules`. This replaced the former
//! hardcoded-empty stub, which was the unknown-line loop's "physical
//! open end" (nothing an approved rule could ever be published through).
//!
//! Rate-limited per-IP to discourage scraping. The response is
//! freshness-tolerant: clients cache for hours, so a 429 here is a
//! non-event.

use crate::inference_rules::InferenceRulesStore;
use crate::parser_rules::ParserRulesStore;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use starstats_core::Manifest;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use utoipa::ToSchema;

/// Build the `/v1/parser-definitions` sub-router. Unauthenticated;
/// IP-rate-limited. The manifest is served from `store` (the DB-backed
/// `parser_rules` table) plus `inference_store` (the DB-backed
/// `parser_inference_rules` table).
pub fn routes(
    store: Arc<dyn ParserRulesStore>,
    inference_store: Arc<dyn InferenceRulesStore>,
) -> Router {
    let governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(2)
            .burst_size(10)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("parser-defs governor config builder produced no config"),
    );
    Router::new()
        .route("/v1/parser-definitions", get(get_manifest))
        .layer(GovernorLayer { config: governor })
        .with_state((store, inference_store))
}

/// OpenAPI-friendly wrapper. utoipa can't derive `ToSchema` for the
/// `starstats_core::Manifest` directly because it lives in another
/// crate; this transparent wrapper restates the shape minimally.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ManifestResponse {
    pub version: u32,
    pub schema_version: u32,
    pub issued_at: String,
    pub rules: Vec<RemoteRuleDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_rules: Vec<RemoteInferenceRuleDoc>,
    pub signature: Option<String>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RemoteRuleDoc {
    pub id: String,
    pub event_name: String,
    pub match_kind: String,
    pub body_regex: String,
    pub fields: Vec<String>,
}

/// Doc-only mirror of `starstats_core::RemoteInferenceRule`. Re-stating
/// it here keeps utoipa happy without forcing a `ToSchema` derive on
/// the core crate (which would pull utoipa into a crate that must stay
/// I/O-free).
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RemoteInferenceRuleDoc {
    pub id: String,
    pub confidence: f32,
    pub window_secs: u32,
    pub trigger: EventPatternDoc,
    pub followups: Vec<EventPatternDoc>,
    pub emits: EventTemplateDoc,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct EventPatternDoc {
    pub event_type: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub field_equals: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct EventTemplateDoc {
    pub event_type: String,
    pub fields: std::collections::BTreeMap<String, String>,
}

#[utoipa::path(
    get,
    path = "/v1/parser-definitions",
    tag = "parser-definitions",
    operation_id = "parser_definitions_get_manifest",
    responses(
        (status = 200, description = "Active parser-definition manifest", body = ManifestResponse),
    ),
)]
pub async fn get_manifest(
    State((store, inference_store)): State<(
        Arc<dyn ParserRulesStore>,
        Arc<dyn InferenceRulesStore>,
    )>,
) -> Response {
    let manifest = current_manifest(store.as_ref(), inference_store.as_ref()).await;
    (StatusCode::OK, Json(manifest)).into_response()
}

/// Source-of-truth for the active manifest: the enabled rows of the
/// `parser_rules` table, projected to `RemoteRule`s, plus the enabled
/// rows of the `parser_inference_rules` table (migration 0050) served
/// via `inference_store`. A DB error on either store degrades to an
/// empty list rather than 500-ing this public, cache-tolerant endpoint
/// — collectors keep their last-known-good rule set.
///
/// `version`/`issued_at` freshness signalling is intentionally minimal
/// here; a richer generation counter lands with the client-adoption
/// slice that will actually consume it (until then no shipping client
/// fetches this endpoint).
async fn current_manifest(
    store: &dyn ParserRulesStore,
    inference_store: &dyn InferenceRulesStore,
) -> Manifest {
    let rules = match store.active_rules().await {
        Ok(rules) => rules,
        Err(e) => {
            tracing::error!(error = %e, "failed to load parser rules; serving empty manifest");
            Vec::new()
        }
    };
    let inference_rules = inference_store.active_rules().await.unwrap_or_else(|e| {
        tracing::error!(error=%e, "failed to load inference rules; serving none");
        Vec::new()
    });
    let mut manifest = Manifest {
        version: 1,
        schema_version: 1,
        issued_at: "2026-05-07T00:00:00Z".to_string(),
        rules,
        inference_rules,
        signature: None,
    };
    // F10: sign the canonical payload when a signing key is configured.
    // Unset (the default) → the manifest ships unsigned, no behaviour change;
    // the client verifies against a pinned pubkey only once one is provisioned.
    if let Some(key) = parser_signing_key() {
        manifest.signature = Some(sign_manifest(&manifest, key));
    }
    manifest
}

/// The ed25519 key that signs the parser manifest (F10), loaded once from
/// `STARSTATS_PARSER_SIGNING_KEY` (inline base64 of the 32-byte secret seed)
/// or the docker-secret mount at `STARSTATS_PARSER_SIGNING_KEY_FILE` — the
/// same `read_env_or_file` convention the roadmap / revolut / ingest secrets
/// use. `None` (the default, neither set) → manifests ship unsigned, no
/// behaviour change. A malformed value logs and yields `None` (fail-open on
/// the SERVER is safe: an unsigned manifest is only rejected by clients that
/// require signing).
fn parser_signing_key() -> Option<&'static ed25519_dalek::SigningKey> {
    use base64::Engine as _;
    static KEY: std::sync::OnceLock<Option<ed25519_dalek::SigningKey>> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let b64 = crate::config::read_env_or_file(
            "STARSTATS_PARSER_SIGNING_KEY",
            "STARSTATS_PARSER_SIGNING_KEY_FILE",
        )
        .ok()
        .flatten()?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(
                |e| tracing::error!(error = %e, "STARSTATS_PARSER_SIGNING_KEY is not valid base64"),
            )
            .ok()?;
        let seed: [u8; 32] = bytes
            .try_into()
            .map_err(|_| tracing::error!("STARSTATS_PARSER_SIGNING_KEY must decode to 32 bytes"))
            .ok()?;
        Some(ed25519_dalek::SigningKey::from_bytes(&seed))
    })
    .as_ref()
}

/// Sign a manifest's canonical payload (`manifest_signing_bytes`) and
/// return the base64 ed25519 signature to stamp on `Manifest::signature`.
fn sign_manifest(manifest: &Manifest, key: &ed25519_dalek::SigningKey) -> String {
    use base64::Engine as _;
    use ed25519_dalek::Signer as _;
    let sig = key.sign(&starstats_core::parser_defs::manifest_signing_bytes(
        manifest,
    ));
    base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference_rules::test_support::MemoryInferenceRulesStore;
    use crate::parser_rules::test_support::MemoryParserRulesStore;
    use crate::parser_rules::ParserRule;
    use starstats_core::{EventPattern, EventTemplate, RemoteInferenceRule, RuleMatchKind};
    use std::collections::BTreeMap;

    #[test]
    fn sign_manifest_produces_a_signature_its_pubkey_verifies() {
        use base64::Engine as _;
        use ed25519_dalek::Verifier as _;

        let key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let mut manifest = starstats_core::parser_defs::Manifest::empty();
        manifest.version = 3;

        let sig_b64 = sign_manifest(&manifest, &key);
        let sig_bytes: [u8; 64] = base64::engine::general_purpose::STANDARD
            .decode(&sig_b64)
            .unwrap()
            .try_into()
            .unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);

        // Verifies over the SAME canonical bytes the client will use.
        let bytes = starstats_core::parser_defs::manifest_signing_bytes(&manifest);
        assert!(key.verifying_key().verify(&bytes, &sig).is_ok());

        // A tampered manifest fails verification against the same signature.
        manifest.version = 4;
        let tampered = starstats_core::parser_defs::manifest_signing_bytes(&manifest);
        assert!(key.verifying_key().verify(&tampered, &sig).is_err());
    }

    #[tokio::test]
    async fn manifest_serves_enabled_rules_from_store() {
        let store = MemoryParserRulesStore::new();
        store
            .upsert(ParserRule {
                rule_id: "new_death_variant".into(),
                event_name: "SomeNewDeath".into(),
                match_kind: RuleMatchKind::EventName,
                body_regex: r"(?P<victim>\w+)".into(),
                fields: vec!["victim".into()],
                enabled: true,
            })
            .await
            .unwrap();

        let inference_store = MemoryInferenceRulesStore::new();
        let manifest = current_manifest(&store, &inference_store).await;
        assert_eq!(manifest.rules.len(), 1, "the enabled rule must be served");
        assert_eq!(manifest.rules[0].id, "new_death_variant");
        assert_eq!(manifest.rules[0].event_name, "SomeNewDeath");
        assert_eq!(manifest.rules[0].fields, vec!["victim".to_string()]);
    }

    #[tokio::test]
    async fn manifest_is_empty_when_no_rules_published() {
        let store = MemoryParserRulesStore::new();
        let inference_store = MemoryInferenceRulesStore::new();
        let manifest = current_manifest(&store, &inference_store).await;
        assert!(
            manifest.rules.is_empty(),
            "no published rules → empty manifest (former stub behaviour, now DB-driven)"
        );
    }

    fn sample_inference_rule() -> RemoteInferenceRule {
        let mut fields = BTreeMap::new();
        fields.insert("timestamp".into(), "${trigger.timestamp}".into());
        RemoteInferenceRule {
            id: "implicit_death.v1".into(),
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

    #[tokio::test]
    async fn current_manifest_includes_active_inference_rules() {
        let store = MemoryParserRulesStore::new();
        let inference_store = MemoryInferenceRulesStore::new();
        inference_store
            .upsert("implicit_death.v1", &sample_inference_rule(), true)
            .await
            .unwrap();

        let manifest = current_manifest(&store, &inference_store).await;
        assert_eq!(
            manifest.inference_rules.len(),
            1,
            "the enabled inference rule must be served"
        );
        assert_eq!(manifest.inference_rules[0].id, "implicit_death.v1");
    }
}
