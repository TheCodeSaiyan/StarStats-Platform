//! Moderator endpoints for parser-health findings.
//!
//! `GET  /v1/admin/parser-health` — last run + every finding.
//! `POST /v1/admin/parser-health/{event_type}/acknowledge` — silence a known
//! dead type (CIG removed `Actor Death`) without deleting the record.
//! `POST /v1/admin/parser-health/{event_type}/resolve` — close it manually.
//!
//! The response leads with `last_run` rather than the findings list, on
//! purpose: a stale or missing run is a louder signal than an empty findings
//! array, and the two must never be confused. See [`crate::parser_health`].

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::parser_health_store::{HealthRun, ParserHealthStore, StoredFinding};
use crate::unknown_tags::{candidate_window, TagCandidate, UnknownTagStore};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// Event types are parser-emitted snake_case identifiers. Validate before
/// touching the store so a hostile path segment can't reach a query.
const EVENT_TYPE_MAX_LEN: usize = 128;
const NOTE_MAX_LEN: usize = 2_000;

fn valid_event_type(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= EVENT_TYPE_MAX_LEN
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Most candidate tags offered per finding. A shortlist is the useful
/// artefact — a long tail of unrelated tags that happen to share the window
/// would bury the real one.
const MAX_CANDIDATES: i64 = 5;

/// A finding plus the unknown shell tags that first appeared around the
/// moment it went dark. Empty until at least one tray opts into reporting
/// tags, which is why the finding itself must stand on its own evidence.
#[derive(Debug, Serialize, ToSchema)]
/// Nested rather than `#[serde(flatten)]`: utoipa's schema derive does not
/// understand flatten and emits an EMPTY property set, so the generated TS
/// type would compile against nothing while the runtime payload was fine.
pub struct FindingView {
    pub finding: StoredFinding,
    pub candidates: Vec<TagCandidate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParserHealthResponse {
    /// The most recent detector pass, or `null` before the first one
    /// completes. A `finished_at` far in the past means the detector itself
    /// has stopped — treat that as more urgent than any single finding.
    pub last_run: Option<HealthRun>,
    pub findings: Vec<FindingView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AcknowledgeRequest {
    /// Why this type is expected to be dead, e.g. "CIG removed this line
    /// from the default log in 4.2".
    #[serde(default)]
    pub note: Option<String>,
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
    get,
    path = "/v1/admin/parser-health",
    tag = "admin",
    responses(
        (status = 200, description = "Detector state", body = ParserHealthResponse),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_health(
    _moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn ParserHealthStore>>,
    Extension(tags): Extension<Arc<dyn UnknownTagStore>>,
) -> Response {
    let last_run = match store.latest_run().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "parser-health: latest_run failed");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "parser_health_unavailable",
            );
        }
    };
    let findings = match store.list_findings().await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(error = %e, "parser-health: list_findings failed");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "parser_health_unavailable",
            );
        }
    };
    // Correlate each finding against tags first sighted around its collapse
    // moment. Computed at read time rather than stored: candidates improve as
    // more trays opt in, and a stale snapshot would hide a tag reported after
    // the detector last ran. Findings are few, so the per-finding query is
    // cheap; a lookup failure degrades to no candidates rather than failing
    // the page, because the finding is the load-bearing part.
    let mut views = Vec::with_capacity(findings.len());
    for finding in findings {
        let candidates = match finding.last_event_at {
            None => Vec::new(),
            Some(at) => {
                let (from, to) = candidate_window(at);
                match tags.candidates(from, to, MAX_CANDIDATES).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            event_type = %finding.event_type,
                            "parser-health: candidate lookup failed"
                        );
                        Vec::new()
                    }
                }
            }
        };
        views.push(FindingView {
            finding,
            candidates,
        });
    }

    (
        StatusCode::OK,
        Json(ParserHealthResponse {
            last_run,
            findings: views,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/parser-health/{event_type}/acknowledge",
    tag = "admin",
    params(("event_type" = String, Path, description = "Event type to acknowledge")),
    request_body = AcknowledgeRequest,
    responses(
        (status = 200, description = "Acknowledged"),
        (status = 400, description = "Malformed event type", body = ApiErrorBody),
        (status = 404, description = "No finding for that event type", body = ApiErrorBody),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
    ),
    security(("bearer_auth" = []))
)]
pub async fn acknowledge(
    moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn ParserHealthStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(event_type): Path<String>,
    Json(body): Json<AcknowledgeRequest>,
) -> Response {
    if !valid_event_type(&event_type) {
        return err(StatusCode::BAD_REQUEST, "invalid_event_type");
    }
    let note = body.note.as_deref().map(|n| {
        if n.len() > NOTE_MAX_LEN {
            &n[..NOTE_MAX_LEN]
        } else {
            n
        }
    });

    match store
        .acknowledge(&event_type, &moderator.0.preferred_username, note)
        .await
    {
        Ok(false) => err(StatusCode::NOT_FOUND, "finding_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "parser-health: acknowledge failed");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "parser_health_unavailable",
            )
        }
        Ok(true) => {
            emit_audit(
                audit.as_ref(),
                &moderator,
                "admin.parser_health.acknowledged",
                &event_type,
            )
            .await;
            StatusCode::OK.into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/parser-health/{event_type}/resolve",
    tag = "admin",
    params(("event_type" = String, Path, description = "Event type to resolve")),
    responses(
        (status = 200, description = "Resolved"),
        (status = 400, description = "Malformed event type", body = ApiErrorBody),
        (status = 404, description = "No finding for that event type", body = ApiErrorBody),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
    ),
    security(("bearer_auth" = []))
)]
pub async fn resolve(
    moderator: RequireModerator,
    Extension(store): Extension<Arc<dyn ParserHealthStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(event_type): Path<String>,
) -> Response {
    if !valid_event_type(&event_type) {
        return err(StatusCode::BAD_REQUEST, "invalid_event_type");
    }
    match store
        .resolve(&event_type, &moderator.0.preferred_username)
        .await
    {
        Ok(false) => err(StatusCode::NOT_FOUND, "finding_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "parser-health: resolve failed");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "parser_health_unavailable",
            )
        }
        Ok(true) => {
            emit_audit(
                audit.as_ref(),
                &moderator,
                "admin.parser_health.resolved",
                &event_type,
            )
            .await;
            StatusCode::OK.into_response()
        }
    }
}

/// Best-effort audit per docs/ENGINEERING.md: a hiccup warns but never poisons the
/// response the moderator already earned.
async fn emit_audit(
    audit: &dyn AuditLog,
    moderator: &RequireModerator,
    action: &str,
    event_type: &str,
) {
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(moderator.0.sub.clone()),
            actor_handle: Some(moderator.0.preferred_username.clone()),
            action: action.to_string(),
            payload: serde_json::json!({ "event_type": event_type }),
        })
        .await
    {
        tracing::warn!(error = %e, action, "audit log append failed");
    }
}

pub fn router() -> Router {
    Router::new()
        .route("/v1/admin/parser-health", get(get_health))
        .route(
            "/v1/admin/parser-health/:event_type/acknowledge",
            post(acknowledge),
        )
        .route("/v1/admin/parser-health/:event_type/resolve", post(resolve))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::parser_health::{Finding, Severity};
    use crate::parser_health_store::test_support::MemoryParserHealthStore;
    use crate::parser_health_store::FindingStatus;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use crate::unknown_tags::test_support::MemoryUnknownTagStore;
    use crate::unknown_tags::TagSighting;
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    struct Harness {
        app: Router,
        store: Arc<MemoryParserHealthStore>,
        tags: Arc<MemoryUnknownTagStore>,
        token: String,
    }

    async fn harness() -> Harness {
        let (issuer, verifier) = fresh_pair();
        let store = Arc::new(MemoryParserHealthStore::new());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let audit = Arc::new(MemoryAuditLog::default());

        let user_id = Uuid::now_v7();
        staff
            .grant(user_id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        let token = issuer
            .sign_user(&user_id.to_string(), "nigel")
            .expect("sign moderator token");

        let tags = Arc::new(MemoryUnknownTagStore::new());
        let store_dyn: Arc<dyn ParserHealthStore> = store.clone();
        let tags_dyn: Arc<dyn UnknownTagStore> = tags.clone();
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;

        let app = router()
            .layer(Extension(Arc::new(verifier)))
            .layer(Extension(store_dyn))
            .layer(Extension(tags_dyn))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_dyn));

        Harness {
            app,
            store,
            tags,
            token,
        }
    }

    fn finding(event_type: &str) -> Finding {
        Finding {
            event_type: event_type.to_string(),
            severity: Severity::Dark,
            baseline_events: 1_900,
            recent_events: 0,
            share_baseline: 0.1,
            share_recent: 0.0,
            baseline_handles: 3,
            carried_handles: 3,
            affected_handles: 3,
            last_event_at: None,
        }
    }

    async fn send(app: &Router, req: Request<axum::body::Body>) -> (StatusCode, serde_json::Value) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    #[tokio::test]
    async fn get_returns_last_run_and_findings() {
        let h = harness().await;
        h.store
            .upsert_finding(&finding("vehicle_stowed"))
            .await
            .unwrap();
        let id = h.store.start_run().await.unwrap();
        let now = chrono::Utc::now();
        h.store.finish_run(id, now, now, 27, 1, None).await.unwrap();

        let (status, body) = send(
            &h.app,
            Request::builder()
                .uri("/v1/admin/parser-health")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["findings"][0]["finding"]["event_type"],
            "vehicle_stowed"
        );
        assert_eq!(body["findings"][0]["finding"]["severity"], "dark");
        assert_eq!(body["last_run"]["types_examined"], 27);
    }

    #[tokio::test]
    async fn a_finding_names_the_tag_that_replaced_it() {
        // The end-to-end point of tag correlation, replayed on the real
        // incident: vehicle_stowed last fired 2026-07-14T21:13:51Z, and
        // `LandingArea_UnregisterFromExternalSystems_StowingVehicle` first
        // appeared 2026-07-16. The finding must surface that tag by name —
        // turning a multi-hour investigation into a glance.
        let h = harness().await;
        let t = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let mut f = finding("vehicle_stowed");
        f.last_event_at = Some(t("2026-07-14T21:13:51Z"));
        h.store.upsert_finding(&f).await.unwrap();
        h.tags
            .record(
                "nigel",
                &[TagSighting {
                    shell_tag: "LandingArea_UnregisterFromExternalSystems_StowingVehicle".into(),
                    first_seen: t("2026-07-16T00:00:00Z"),
                    last_seen: t("2026-07-30T00:00:00Z"),
                    occurrences: 3307,
                    game_build: None,
                }],
            )
            .await
            .unwrap();

        let (status, body) = send(
            &h.app,
            Request::builder()
                .uri("/v1/admin/parser-health")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let c = &body["findings"][0]["candidates"][0];
        assert_eq!(
            c["shell_tag"],
            "LandingArea_UnregisterFromExternalSystems_StowingVehicle"
        );
        assert_eq!(c["occurrences"], 3307);
        // Nested under `finding` — see the FindingView doc comment.
        assert_eq!(
            body["findings"][0]["finding"]["event_type"],
            "vehicle_stowed"
        );
    }

    #[tokio::test]
    async fn an_unrelated_tag_is_not_offered_as_a_candidate() {
        // A tag first seen months before the collapse is not evidence.
        // Without this the shortlist fills with long-standing noise like
        // `InventoryManagement` and buries the real cause.
        let h = harness().await;
        let t = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let mut f = finding("vehicle_stowed");
        f.last_event_at = Some(t("2026-07-14T21:13:51Z"));
        h.store.upsert_finding(&f).await.unwrap();
        h.tags
            .record(
                "nigel",
                &[TagSighting {
                    shell_tag: "InventoryManagement".into(),
                    first_seen: t("2026-05-20T00:00:00Z"),
                    last_seen: t("2026-08-02T00:00:00Z"),
                    occurrences: 229_796,
                    game_build: None,
                }],
            )
            .await
            .unwrap();

        let (_, body) = send(
            &h.app,
            Request::builder()
                .uri("/v1/admin/parser-health")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(
            body["findings"][0]["candidates"].as_array().unwrap().len(),
            0,
            "a tag predating the collapse must not be offered"
        );
    }

    #[tokio::test]
    async fn a_finding_without_a_collapse_moment_offers_no_candidates() {
        // Legacy rows written before migration 0065 have last_event_at NULL.
        // They must render, just without correlation.
        let h = harness().await;
        h.store
            .upsert_finding(&finding("vehicle_stowed"))
            .await
            .unwrap();

        let (status, body) = send(
            &h.app,
            Request::builder()
                .uri("/v1/admin/parser-health")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["findings"][0]["finding"]["event_type"],
            "vehicle_stowed"
        );
        assert_eq!(
            body["findings"][0]["candidates"].as_array().unwrap().len(),
            0
        );
    }

    #[tokio::test]
    async fn get_reports_null_last_run_before_any_pass() {
        // "No findings" and "the detector never ran" must be distinguishable
        // by the client, which is the whole reason last_run is in the payload.
        let h = harness().await;

        let (status, body) = send(
            &h.app,
            Request::builder()
                .uri("/v1/admin/parser-health")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(body["last_run"].is_null());
        assert_eq!(body["findings"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn acknowledge_marks_the_finding_and_stores_the_note() {
        let h = harness().await;
        h.store
            .upsert_finding(&finding("actor_death"))
            .await
            .unwrap();

        let (status, _) = send(
            &h.app,
            Request::builder()
                .method("POST")
                .uri("/v1/admin/parser-health/actor_death/acknowledge")
                .header("authorization", format!("Bearer {}", h.token))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({ "note": "CIG removed this line" }).to_string(),
                ))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let all = h.store.list_findings().await.unwrap();
        assert_eq!(all[0].status, FindingStatus::Acknowledged);
        assert_eq!(all[0].note.as_deref(), Some("CIG removed this line"));
        assert_eq!(all[0].acknowledged_by.as_deref(), Some("nigel"));
    }

    #[tokio::test]
    async fn resolve_closes_the_finding() {
        let h = harness().await;
        h.store
            .upsert_finding(&finding("vehicle_stowed"))
            .await
            .unwrap();

        let (status, _) = send(
            &h.app,
            Request::builder()
                .method("POST")
                .uri("/v1/admin/parser-health/vehicle_stowed/resolve")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let all = h.store.list_findings().await.unwrap();
        assert_eq!(all[0].status, FindingStatus::Resolved);
        assert_eq!(all[0].resolved_reason.as_deref(), Some("manual"));
    }

    #[tokio::test]
    async fn acknowledging_an_unknown_type_is_404_not_a_silent_ok() {
        let h = harness().await;

        let (status, _) = send(
            &h.app,
            Request::builder()
                .method("POST")
                .uri("/v1/admin/parser-health/never_seen/acknowledge")
                .header("authorization", format!("Bearer {}", h.token))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn malformed_event_type_is_rejected() {
        let h = harness().await;

        let (status, _) = send(
            &h.app,
            Request::builder()
                .method("POST")
                .uri("/v1/admin/parser-health/bad%20type!/resolve")
                .header("authorization", format!("Bearer {}", h.token))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unauthenticated_request_is_rejected() {
        let h = harness().await;

        let resp = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/admin/parser-health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn event_type_validation_accepts_real_types_and_rejects_junk() {
        assert!(valid_event_type("vehicle_stowed"));
        assert!(valid_event_type("mission.objective-v2"));
        assert!(!valid_event_type(""));
        assert!(!valid_event_type("has space"));
        assert!(!valid_event_type("semi;colon"));
        assert!(!valid_event_type(&"x".repeat(EVENT_TYPE_MAX_LEN + 1)));
    }
}
