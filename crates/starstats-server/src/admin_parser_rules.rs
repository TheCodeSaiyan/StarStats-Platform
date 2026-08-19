//! Moderator endpoint to publish a parser rule into the served manifest.
//!
//! `POST /v1/admin/parser-rules` — upserts a [`ParserRule`] into the
//! `parser_rules` table (migration 0048), which
//! [`crate::parser_def_routes`] serves at `GET /v1/parser-definitions`.
//! This is the write half that makes an approved unknown-line submission
//! actually reachable by collectors (audit §3 repair point).
//!
//! Design: a dedicated rule-authoring endpoint rather than overloading
//! the submission-triage PATCH — rule authoring and triage are separate
//! concerns, and the moderator links a submission to its rule by setting
//! the submission's `rule_id` to the published rule's id.
//!
//! `GET /v1/admin/parser-rules` lists every rule (enabled + disabled)
//! for the admin rule-authoring UI — the read half backing that same
//! table. Both endpoints carry `#[utoipa::path]` annotations; OpenAPI
//! spec registration happens where `openapi.rs` wires up the other
//! admin routes.

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::parser_rules::{AdminParserRuleRow, ParserRule, ParserRulesStore};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use starstats_core::{compile_rules, RemoteRule, RuleMatchKind};
use std::sync::Arc;
use utoipa::ToSchema;

/// Same identifier charset the submission PATCH accepts for `rule_id`
/// (alphanumeric + `_ - .`), so a manifest id like `combat.kill.v1`
/// round-trips.
const RULE_ID_MAX_LEN: usize = 256;
const EVENT_NAME_MAX_LEN: usize = 256;
const MAX_FIELDS: usize = 32;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishRuleRequest {
    pub rule_id: String,
    pub event_name: String,
    /// `"event_name"` (default) or `"body_keyword"`.
    #[serde(default)]
    pub match_kind: String,
    #[serde(default)]
    pub body_regex: String,
    #[serde(default)]
    pub fields: Vec<String>,
    /// Publish enabled (default) or stage disabled. Setting `false`
    /// retracts a previously-published rule from the manifest.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublishRuleResponse {
    pub rule_id: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminParserRulesListResponse {
    pub rules: Vec<AdminParserRuleRow>,
}

/// Validate a publish request into a storable [`ParserRule`]. Pure — the
/// unit tests exercise every rejection path without the HTTP/auth layer.
/// The `body_regex` is validated by compiling it exactly as the client
/// will (core's `compile_rules`; the `regex` crate is linear-time so a
/// published rule can't introduce catastrophic backtracking).
fn build_parser_rule(body: &PublishRuleRequest) -> Result<ParserRule, &'static str> {
    let rule_id = body.rule_id.trim();
    if rule_id.is_empty() {
        return Err("rule_id_required");
    }
    if rule_id.chars().count() > RULE_ID_MAX_LEN {
        return Err("rule_id_too_long");
    }
    if !rule_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err("invalid_rule_id");
    }

    let event_name = body.event_name.trim();
    if event_name.is_empty() {
        return Err("event_name_required");
    }
    if event_name.chars().count() > EVENT_NAME_MAX_LEN {
        return Err("event_name_too_long");
    }

    let match_kind = match body.match_kind.trim() {
        "" | "event_name" => RuleMatchKind::EventName,
        "body_keyword" => RuleMatchKind::BodyKeyword,
        _ => return Err("invalid_match_kind"),
    };

    if body.fields.len() > MAX_FIELDS {
        return Err("too_many_fields");
    }
    let fields: Vec<String> = body
        .fields
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();

    let rule = ParserRule {
        rule_id: rule_id.to_string(),
        event_name: event_name.to_string(),
        match_kind,
        body_regex: body.body_regex.clone(),
        fields,
        enabled: body.enabled,
    };

    let candidate: RemoteRule = rule.clone().into_remote_rule();
    let (_ok, bad) = compile_rules(&[candidate]);
    if !bad.is_empty() {
        return Err("invalid_body_regex");
    }
    Ok(rule)
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
    path = "/v1/admin/parser-rules",
    tag = "admin",
    request_body = PublishRuleRequest,
    responses(
        (status = 200, description = "Rule published", body = PublishRuleResponse),
        (status = 400, description = "Validation error", body = ApiErrorBody),
        (status = 403, description = "Not a moderator"),
    ),
    security(("BearerAuth" = [])),
)]
pub async fn publish_rule(
    moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn ParserRulesStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(body): Json<PublishRuleRequest>,
) -> Response {
    let rule = match build_parser_rule(&body) {
        Ok(r) => r,
        Err(code) => return err(StatusCode::BAD_REQUEST, code),
    };
    let rule_id = rule.rule_id.clone();
    let enabled = rule.enabled;

    if let Err(e) = store.upsert(rule).await {
        tracing::error!(error = %e, rule_id = %rule_id, "parser rule upsert failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error");
    }

    // Best-effort audit — a published rule runs on every collector, so
    // the action must be traceable. A chain hiccup never fails the write.
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(moderator.0.sub.clone()),
            actor_handle: Some(moderator.0.preferred_username.clone()),
            action: "admin.parser_rule.published".to_string(),
            payload: serde_json::json!({ "rule_id": rule_id, "enabled": enabled }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit log append failed (admin.parser_rule.published)");
    }

    (
        StatusCode::OK,
        Json(PublishRuleResponse { rule_id, enabled }),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/parser-rules",
    tag = "admin",
    responses(
        (status = 200, description = "All published rules", body = AdminParserRulesListResponse),
        (status = 403, description = "Not a moderator"),
    ),
    security(("BearerAuth" = [])),
)]
pub async fn list_rules(
    _moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn ParserRulesStore>>,
) -> Response {
    match store.all_rules().await {
        Ok(rules) => (StatusCode::OK, Json(AdminParserRulesListResponse { rules })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "parser rules list failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
    }
}

/// Build the admin parser-rules sub-router. Parameterless: the rules
/// store, audit log, auth verifier, and staff role store are installed as
/// Extension layers on the outer router by `main.rs`.
pub fn router() -> Router {
    Router::new().route("/v1/admin/parser-rules", post(publish_rule).get(list_rules))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::parser_rules::test_support::MemoryParserRulesStore;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn req(rule_id: &str, event_name: &str, body_regex: &str) -> PublishRuleRequest {
        PublishRuleRequest {
            rule_id: rule_id.into(),
            event_name: event_name.into(),
            match_kind: String::new(),
            body_regex: body_regex.into(),
            fields: vec![],
            enabled: true,
        }
    }

    #[test]
    fn build_accepts_a_valid_rule() {
        let r = build_parser_rule(&req("combat.kill.v1", "NewKill", r"(?P<who>\w+)")).unwrap();
        assert_eq!(r.rule_id, "combat.kill.v1");
        assert_eq!(r.match_kind, RuleMatchKind::EventName);
    }

    #[test]
    fn build_rejects_bad_inputs() {
        assert_eq!(
            build_parser_rule(&req("", "E", "")).unwrap_err(),
            "rule_id_required"
        );
        assert_eq!(
            build_parser_rule(&req("bad id!", "E", "")).unwrap_err(),
            "invalid_rule_id"
        );
        assert_eq!(
            build_parser_rule(&req("ok", "", "")).unwrap_err(),
            "event_name_required"
        );
        // Unbalanced group → regex won't compile.
        assert_eq!(
            build_parser_rule(&req("ok", "E", "(?P<x>")).unwrap_err(),
            "invalid_body_regex"
        );
        let mut bad_kind = req("ok", "E", "");
        bad_kind.match_kind = "nonsense".into();
        assert_eq!(
            build_parser_rule(&bad_kind).unwrap_err(),
            "invalid_match_kind"
        );
    }

    fn build_app(
        rules: Arc<MemoryParserRulesStore>,
        audit: Arc<MemoryAuditLog>,
        staff: Arc<MemoryStaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let rules_dyn: Arc<dyn ParserRulesStore> = rules;
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

    fn post_req(token: &str, body: serde_json::Value) -> Request<axum::body::Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/admin/parser-rules")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn moderator_publishes_rule_and_it_becomes_active() {
        let rules = Arc::new(MemoryParserRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(post_req(
                &token,
                json!({
                    "rule_id": "combat.kill.v1",
                    "event_name": "NewKill",
                    "body_regex": r"(?P<victim>\w+)",
                    "fields": ["victim"],
                }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let active = rules.active_rules().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "combat.kill.v1");
        assert_eq!(active[0].event_name, "NewKill");
    }

    #[tokio::test]
    async fn non_moderator_is_forbidden() {
        let rules = Arc::new(MemoryParserRulesStore::new());
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
                json!({"rule_id": "x.v1", "event_name": "E", "body_regex": ""}),
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
    async fn invalid_regex_is_rejected() {
        let rules = Arc::new(MemoryParserRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(post_req(
                &token,
                json!({"rule_id": "x.v1", "event_name": "E", "body_regex": "(?P<x>"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(err["error"], "invalid_body_regex");
        assert!(rules.active_rules().await.unwrap().is_empty());
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
    async fn list_rules_moderator_sees_all_rows() {
        let rules = Arc::new(MemoryParserRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "modhandle").await;

        rules
            .upsert(ParserRule {
                rule_id: "enabled.rule.v1".to_string(),
                event_name: "EnabledEvent".to_string(),
                match_kind: RuleMatchKind::EventName,
                body_regex: String::new(),
                fields: vec![],
                enabled: true,
            })
            .await
            .unwrap();
        rules
            .upsert(ParserRule {
                rule_id: "disabled.rule.v1".to_string(),
                event_name: "DisabledEvent".to_string(),
                match_kind: RuleMatchKind::EventName,
                body_regex: String::new(),
                fields: vec![],
                enabled: false,
            })
            .await
            .unwrap();

        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(get_req(&token, "/v1/admin/parser-rules"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let rules = body["rules"].as_array().expect("rules array");
        assert_eq!(rules.len(), 2);
        let ids: Vec<&str> = rules
            .iter()
            .map(|r| r["rule_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"enabled.rule.v1"));
        assert!(ids.contains(&"disabled.rule.v1"));
    }

    #[tokio::test]
    async fn list_rules_requires_moderator() {
        let rules = Arc::new(MemoryParserRulesStore::new());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        // A valid token but no moderator grant.
        let token = issuer
            .sign_user(&Uuid::now_v7().to_string(), "plainuser")
            .unwrap();
        let app = build_app(rules.clone(), audit, staff, Arc::new(verifier));

        let resp = app
            .oneshot(get_req(&token, "/v1/admin/parser-rules"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
