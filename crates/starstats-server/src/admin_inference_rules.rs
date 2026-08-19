//! Moderator endpoints to publish/list inference rules and to enumerate
//! the known event-type keys inference rules may reference.
//!
//! `POST /v1/admin/parser-inference-rules` — upserts a
//! [`RemoteInferenceRule`] into the `parser_inference_rules` table
//! (migration 0051), validating it exactly as the client will compile
//! it (`starstats_core::compile_inference_rules`). This is the
//! inference-chain counterpart to [`crate::admin_parser_rules`]'s
//! single-line rule publish endpoint.
//!
//! Retraction is by re-publishing the same rule with `enabled: false`
//! in the request body (see [`PublishInferenceRuleRequest`]) — the same
//! "publish by absence" posture as the parser-rule manifest.
//!
//! `GET /v1/admin/parser-inference-rules` lists every rule (enabled +
//! disabled) for the admin management page. `GET /v1/admin/event-types`
//! exposes the core's known event-type keys so the rule-authoring UI
//! can populate trigger/followup/emit dropdowns without hardcoding the
//! list client-side.
//!
//! DTOs (`EventPatternDto`/`EventTemplateDto`/`InferenceRuleDto`) are
//! field-identical mirrors of the core wire types — kept server-side
//! so `utoipa` never becomes a `starstats-core` dependency.

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::inference_rules::InferenceRulesStore;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use starstats_core::{
    all_event_type_keys, built_in_inference_rules, compile_inference_rules, EventPattern,
    EventTemplate, InferenceCompileError, RemoteInferenceRule,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use utoipa::ToSchema;

/// Mirror of [`starstats_core::EventPattern`] with `ToSchema` derived
/// server-side.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventPatternDto {
    pub event_type: String,
    #[serde(default)]
    pub field_equals: BTreeMap<String, String>,
}

impl From<EventPatternDto> for EventPattern {
    fn from(dto: EventPatternDto) -> Self {
        EventPattern {
            event_type: dto.event_type,
            field_equals: dto.field_equals,
        }
    }
}

impl From<EventPattern> for EventPatternDto {
    fn from(p: EventPattern) -> Self {
        EventPatternDto {
            event_type: p.event_type,
            field_equals: p.field_equals,
        }
    }
}

/// Mirror of [`starstats_core::EventTemplate`] with `ToSchema` derived
/// server-side.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventTemplateDto {
    pub event_type: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

impl From<EventTemplateDto> for EventTemplate {
    fn from(dto: EventTemplateDto) -> Self {
        EventTemplate {
            event_type: dto.event_type,
            fields: dto.fields,
        }
    }
}

impl From<EventTemplate> for EventTemplateDto {
    fn from(t: EventTemplate) -> Self {
        EventTemplateDto {
            event_type: t.event_type,
            fields: t.fields,
        }
    }
}

/// Mirror of [`starstats_core::RemoteInferenceRule`] with `ToSchema`
/// derived server-side. Deliberately has no `enabled` field — that
/// flag lives on [`PublishInferenceRuleRequest`] so the same DTO
/// serves both the publish request body and the admin list response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InferenceRuleDto {
    pub id: String,
    pub confidence: f32,
    pub window_secs: u32,
    pub trigger: EventPatternDto,
    pub followups: Vec<EventPatternDto>,
    pub emits: EventTemplateDto,
}

impl From<InferenceRuleDto> for RemoteInferenceRule {
    fn from(dto: InferenceRuleDto) -> Self {
        RemoteInferenceRule {
            id: dto.id,
            confidence: dto.confidence,
            window_secs: dto.window_secs,
            trigger: dto.trigger.into(),
            followups: dto.followups.into_iter().map(Into::into).collect(),
            emits: dto.emits.into(),
        }
    }
}

impl From<RemoteInferenceRule> for InferenceRuleDto {
    fn from(r: RemoteInferenceRule) -> Self {
        InferenceRuleDto {
            id: r.id,
            confidence: r.confidence,
            window_secs: r.window_secs,
            trigger: r.trigger.into(),
            followups: r.followups.into_iter().map(Into::into).collect(),
            emits: r.emits.into(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Publish request body. `enabled` defaults to `true` for a normal
/// author flow; the admin management page (Task 7) retracts a rule by
/// re-posting it with `enabled: false` — `InferenceRuleDto` itself has
/// no `enabled` field, so this wrapper is the only place that flag
/// lives on the wire.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishInferenceRuleRequest {
    #[serde(flatten)]
    pub rule: InferenceRuleDto,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublishInferenceRuleResponse {
    pub rule_id: String,
    pub enabled: bool,
}

/// One row in the admin listing — every stored rule (enabled or not).
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminInferenceRuleRow {
    pub rule_id: String,
    pub enabled: bool,
    pub definition: InferenceRuleDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminInferenceRulesListResponse {
    pub rules: Vec<AdminInferenceRuleRow>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventTypesResponse {
    pub event_types: Vec<String>,
}

fn err(status: StatusCode, code: &str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/parser-inference-rules",
    tag = "admin",
    operation_id = "admin_inference_rules_publish",
    request_body = PublishInferenceRuleRequest,
    responses(
        (status = 200, description = "Inference rule published", body = PublishInferenceRuleResponse),
        (status = 400, description = "Validation error", body = ApiErrorBody),
        (status = 403, description = "Not a moderator"),
    ),
    security(("BearerAuth" = [])),
)]
pub async fn publish_rule(
    moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn InferenceRulesStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(req): Json<PublishInferenceRuleRequest>,
) -> Response {
    let rule: RemoteInferenceRule = req.rule.into();
    // Validate exactly as the client will compile it.
    if let Err(e) = compile_inference_rules(std::slice::from_ref(&rule)) {
        let code = match e {
            InferenceCompileError::UnknownEventType { .. } => "unknown_event_type",
            InferenceCompileError::InvalidConfidence { .. } => "invalid_confidence",
            InferenceCompileError::EmptyFollowups { .. } => "empty_followups",
        };
        return err(StatusCode::BAD_REQUEST, code);
    }
    // A window of 0 means a followup would need the trigger's exact
    // timestamp to ever match `window_within_secs`'s strict `delta > 0`
    // lower bound — the rule can never fire. Reject rather than publish
    // a silently-dead rule.
    if rule.window_secs == 0 {
        return err(StatusCode::BAD_REQUEST, "invalid_window");
    }
    // `combined_inference_rules()` on the client concatenates built-in
    // rules with remote ones and does not de-dup by id, so a published
    // rule whose id equals a built-in's id makes BOTH fire on every
    // matching trigger — a silent double-emission of inferred events.
    if built_in_inference_rules().iter().any(|b| b.id == rule.id) {
        return err(StatusCode::BAD_REQUEST, "id_shadows_builtin");
    }
    let rule_id = rule.id.clone();
    if let Err(e) = store.upsert(&rule_id, &rule, req.enabled).await {
        tracing::error!(error = %e, %rule_id, "inference rule upsert failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
    }

    // Best-effort audit — a published inference rule runs on every
    // collector, so the action must be traceable. A chain hiccup never
    // fails the write.
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(moderator.0.sub.clone()),
            actor_handle: Some(moderator.0.preferred_username.clone()),
            action: "admin.inference_rule.published".to_string(),
            payload: serde_json::json!({ "rule_id": rule_id, "enabled": req.enabled }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed (admin.inference_rule.published)");
    }

    (
        StatusCode::OK,
        Json(PublishInferenceRuleResponse {
            rule_id,
            enabled: req.enabled,
        }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/parser-inference-rules",
    tag = "admin",
    operation_id = "admin_inference_rules_list",
    responses(
        (status = 200, description = "All published inference rules", body = AdminInferenceRulesListResponse),
        (status = 403, description = "Not a moderator"),
    ),
    security(("BearerAuth" = [])),
)]
pub async fn list_rules(
    _moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn InferenceRulesStore>>,
) -> Response {
    match store.all_rules().await {
        Ok(rows) => {
            let rules = rows
                .into_iter()
                .map(|r| AdminInferenceRuleRow {
                    rule_id: r.rule_id,
                    enabled: r.enabled,
                    definition: r.definition.into(),
                })
                .collect();
            (
                StatusCode::OK,
                Json(AdminInferenceRulesListResponse { rules }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "inference rules list failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/event-types",
    tag = "admin",
    operation_id = "admin_inference_rules_list_event_types",
    responses(
        (status = 200, description = "Known event-type keys", body = EventTypesResponse),
        (status = 403, description = "Not a moderator"),
    ),
    security(("BearerAuth" = [])),
)]
pub async fn list_event_types(_moderator: RequireModerator) -> Response {
    (
        StatusCode::OK,
        Json(EventTypesResponse {
            event_types: all_event_type_keys()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }),
    )
        .into_response()
}

/// Build the admin inference-rules sub-router. Parameterless: the
/// inference rules store, audit log, auth verifier, and staff role
/// store are installed as Extension layers on the outer router by
/// `main.rs`.
pub fn router() -> Router {
    Router::new()
        .route(
            "/v1/admin/parser-inference-rules",
            post(publish_rule).get(list_rules),
        )
        .route("/v1/admin/event-types", get(list_event_types))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::inference_rules::test_support::MemoryInferenceRulesStore;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    fn sample_rule_json(id: &str, trigger_event_type: &str, confidence: f64) -> Value {
        json!({
            "id": id,
            "confidence": confidence,
            "window_secs": 15,
            "trigger": { "event_type": trigger_event_type, "field_equals": {} },
            "followups": [
                { "event_type": "resolve_spawn", "field_equals": {} }
            ],
            "emits": {
                "event_type": "player_death",
                "fields": { "timestamp": "${trigger.timestamp}" }
            }
        })
    }

    fn build_app(
        rules: Arc<MemoryInferenceRulesStore>,
        audit: Arc<MemoryAuditLog>,
        staff: Arc<MemoryStaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let rules_dyn: Arc<dyn InferenceRulesStore> = rules;
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        router()
            .layer(Extension(verifier))
            .layer(Extension(rules_dyn))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_dyn))
    }

    async fn moderator_token(
        staff: &MemoryStaffRoleStore,
        issuer: &TokenIssuer,
        handle: &str,
    ) -> String {
        let user_id = Uuid::now_v7();
        staff
            .grant(user_id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign moderator token")
    }

    fn post_req(token: &str, body: Value) -> Request<axum::body::Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/admin/parser-inference-rules")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn get_req(token: &str, uri: &str) -> Request<axum::body::Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn publish_moderator_200_and_active() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(post_req(
                &token,
                sample_rule_json("implicit_death.v1", "vehicle_destruction", 0.85),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let active = rules.active_rules().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "implicit_death.v1");
    }

    #[tokio::test]
    async fn publish_non_moderator_403() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        // A valid token but no moderator grant.
        let token = issuer
            .sign_user(&Uuid::now_v7().to_string(), "plainuser")
            .unwrap();
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(post_req(
                &token,
                sample_rule_json("x.v1", "vehicle_destruction", 0.85),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(
            rules.active_rules().await.unwrap().is_empty(),
            "a forbidden request must not publish a rule"
        );
    }

    #[tokio::test]
    async fn publish_unknown_event_type_400() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(post_req(&token, sample_rule_json("x.v1", "nope", 0.85)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "unknown_event_type");
        assert!(rules.active_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_invalid_confidence_400() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(post_req(
                &token,
                sample_rule_json("x.v1", "vehicle_destruction", 1.5),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "invalid_confidence");
        assert!(rules.active_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_empty_followups_400() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let mut body = sample_rule_json("x.v1", "vehicle_destruction", 0.85);
        body["followups"] = json!([]);

        let resp = app.oneshot(post_req(&token, body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "empty_followups");
        assert!(rules.active_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_window_zero_returns_400() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let mut body = sample_rule_json("custom.test.v1", "vehicle_destruction", 0.85);
        body["window_secs"] = json!(0);

        let resp = app.oneshot(post_req(&token, body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "invalid_window");
        assert!(rules.active_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_id_shadowing_builtin_returns_400() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        // "implicit_death_after_vehicle_destruction" is a real built-in
        // rule id (see starstats_core::inference::RULE_ID_IMPLICIT_DEATH).
        // Publishing a remote rule under the same id would make both
        // fire on the client, double-emitting an inferred event.
        let body = sample_rule_json(
            "implicit_death_after_vehicle_destruction",
            "vehicle_destruction",
            0.85,
        );

        let resp = app.oneshot(post_req(&token, body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "id_shadows_builtin");
        assert!(rules.active_rules().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn enabled_false_round_trips_disabled() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let mut body = sample_rule_json("retract.v1", "vehicle_destruction", 0.85);
        body["enabled"] = json!(false);

        let resp = app.oneshot(post_req(&token, body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let resp_body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp_body["enabled"], false);

        // Disabled: excluded from active_rules, present (disabled) in
        // all_rules.
        assert!(rules.active_rules().await.unwrap().is_empty());
        let all = rules.all_rules().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].rule_id, "retract.v1");
        assert!(!all[0].enabled);
    }

    #[tokio::test]
    async fn list_moderator_sees_all() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        // Publish one enabled and one disabled rule directly through
        // the store to seed the listing.
        let dto: InferenceRuleDto =
            serde_json::from_value(sample_rule_json("enabled.v1", "vehicle_destruction", 0.85))
                .unwrap();
        let enabled_rule: RemoteInferenceRule = dto.into();
        rules
            .upsert("enabled.v1", &enabled_rule, true)
            .await
            .unwrap();
        rules
            .upsert("disabled.v1", &enabled_rule, false)
            .await
            .unwrap();

        let resp = app
            .oneshot(get_req(&token, "/v1/admin/parser-inference-rules"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let rules_arr = body["rules"].as_array().expect("rules array");
        assert_eq!(rules_arr.len(), 2);
        let ids: Vec<&str> = rules_arr
            .iter()
            .map(|r| r["rule_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"enabled.v1"));
        assert!(ids.contains(&"disabled.v1"));
    }

    #[tokio::test]
    async fn event_types_returns_core_list() {
        let rules = Arc::new(MemoryInferenceRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(get_req(&token, "/v1/admin/event-types"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let event_types = body["event_types"].as_array().expect("event_types array");
        let keys: Vec<&str> = event_types.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(keys.contains(&"player_death"));
    }
}
