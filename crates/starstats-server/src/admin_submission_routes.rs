//! Moderator HTTP handlers for the Submissions domain.
//!
//! Endpoints (all behind [`RequireModerator`] -- admins inherit):
//!   - `POST /v1/admin/submissions/{id}/accept`
//!   - `POST /v1/admin/submissions/{id}/reject`        body: `{"reason": "..."}`
//!   - `POST /v1/admin/submissions/{id}/dismiss-flag`
//!   - `GET  /v1/admin/submissions/queue?status=&limit=&offset=`
//!
//! Each mutation reads the prior state, enforces the legal-transition
//! set in the store, and -- when the row actually changes -- writes one
//! `audit_log` row. Idempotent no-ops (accept-on-already-accepted, etc.)
//! return the same response shape with `was_changed: false` and DO NOT
//! emit an audit row, so a moderator's hammer-click on the accept button
//! doesn't pollute the chain.
//!
//! The router builder is generic over [`SubmissionStore`] so route tests
//! can drive it with `MemorySubmissionStore`. Wave B (main.rs glue)
//! mounts this under the `/v1/admin` prefix and merges it into the
//! existing admin sub-router from `admin_routes::router()`.

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::submission_routes::SubmissionDto;
use crate::submissions::{
    AdminQueueFilter, Submission, SubmissionError, SubmissionStore, SubmissionTransition,
    SubmissionWithViewer, FLAG_REASON_MAX_LEN,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const QUEUE_LIMIT_DEFAULT: u32 = 50;
const QUEUE_LIMIT_MAX: u32 = 100;
const QUEUE_LIMIT_MIN: u32 = 1;

/// Cap on a moderator's reject reason. Re-uses the user-flag cap so the
/// two free-text fields share the same TOAST budget.
const REJECT_REASON_MAX_LEN: usize = FLAG_REASON_MAX_LEN;

/// Build the admin submissions sub-router. Caller is responsible for
/// installing the `Arc<dyn AuditLog>`, `Arc<AuthVerifier>`, and
/// `Arc<dyn StaffRoleStore>` extensions before mounting.
pub fn router<S: SubmissionStore>(store: Arc<S>) -> Router {
    Router::new()
        .route("/v1/admin/submissions/:id/accept", post(accept::<S>))
        .route("/v1/admin/submissions/:id/reject", post(reject::<S>))
        .route(
            "/v1/admin/submissions/:id/dismiss-flag",
            post(dismiss_flag::<S>),
        )
        .route("/v1/admin/submissions/queue", get(queue::<S>))
        .with_state(store)
}

// -- DTOs ------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubmissionTransitionResponse {
    pub id: String,
    pub previous_status: String,
    pub new_status: String,
    /// `false` for idempotent no-ops (already in the target state). The
    /// UI uses this to suppress the "moved to X" toast on hammer clicks.
    pub was_changed: bool,
}

impl From<SubmissionTransition> for SubmissionTransitionResponse {
    fn from(t: SubmissionTransition) -> Self {
        Self {
            id: t.id.to_string(),
            previous_status: t.previous_status,
            new_status: t.new_status,
            was_changed: t.was_changed,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RejectRequest {
    /// Free-text rationale for rejection. Capped at
    /// [`REJECT_REASON_MAX_LEN`].
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AdminQueueResponse {
    pub items: Vec<SubmissionDto>,
    /// Whether the next page would have more rows. Computed by asking
    /// the store for `limit + 1` and trimming -- cheaper than a
    /// separate COUNT query.
    pub has_more: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct QueueParams {
    /// `review` (default), `flagged`, or `all`. Case-insensitive.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_queue_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_queue_limit() -> u32 {
    QUEUE_LIMIT_DEFAULT
}

// -- Helpers ---------------------------------------------------------

fn err(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

fn parse_id(raw: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(raw).map_err(|_| err(StatusCode::BAD_REQUEST, "invalid_id"))
}

fn parse_moderator_id(moderator: &RequireModerator) -> Result<Uuid, Response> {
    Uuid::parse_str(&moderator.0.sub).map_err(|_| err(StatusCode::UNAUTHORIZED, "invalid_subject"))
}

fn parse_queue_filter(raw: Option<&str>) -> Result<AdminQueueFilter, Response> {
    match raw.map(|s| s.to_ascii_lowercase()) {
        None => Ok(AdminQueueFilter::Review),
        Some(s) => match s.as_str() {
            "review" => Ok(AdminQueueFilter::Review),
            "flagged" => Ok(AdminQueueFilter::Flagged),
            "all" => Ok(AdminQueueFilter::All),
            _ => Err(err(StatusCode::BAD_REQUEST, "invalid_status")),
        },
    }
}

fn submission_to_dto(s: Submission) -> SubmissionDto {
    SubmissionDto::from(SubmissionWithViewer {
        submission: s,
        viewer_voted: false,
        viewer_flagged: false,
    })
}

/// Map a [`SubmissionError`] from a transition call to an HTTP response.
fn transition_error_response(e: SubmissionError) -> Response {
    match e {
        SubmissionError::NotFound => err(StatusCode::NOT_FOUND, "not_found"),
        SubmissionError::IllegalTransition { .. } => {
            err(StatusCode::CONFLICT, "illegal_transition")
        }
        other => {
            tracing::error!(error = %other, "admin submission transition failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "transition_failed").into_response()
        }
    }
}

/// Best-effort audit append. A failed audit write does NOT roll back
/// the state transition (Postgres is the system of record for the
/// transition itself; the audit chain is for forensics and we'd rather
/// have the state transition land than reject the moderator's action).
async fn record_audit(
    audit: &Arc<dyn AuditLog>,
    moderator: &RequireModerator,
    action: &str,
    payload: serde_json::Value,
) {
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(moderator.0.sub.clone()),
            actor_handle: Some(moderator.0.preferred_username.clone()),
            action: action.to_string(),
            payload,
        })
        .await
    {
        tracing::warn!(
            error = %e,
            action = %action,
            "audit log append failed for admin submission action"
        );
    }
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/admin/submissions/{id}/accept",
    tag = "admin-submissions",
    operation_id = "admin_submissions_accept",
    params(("id" = String, Path, description = "Submission UUID")),
    responses(
        (status = 200, description = "Submission accepted (or already accepted)", body = SubmissionTransitionResponse),
        (status = 400, description = "Invalid id", body = ApiErrorBody),
        (status = 403, description = "Caller is not a moderator", body = ApiErrorBody),
        (status = 404, description = "Submission not found", body = ApiErrorBody),
        (status = 409, description = "Submission is in a terminal state", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn accept<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    moderator: RequireModerator,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let moderator_id = match parse_moderator_id(&moderator) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let transition = match store.accept_submission(id, moderator_id).await {
        Ok(t) => t,
        Err(e) => return transition_error_response(e),
    };

    if transition.was_changed {
        record_audit(
            &audit,
            &moderator,
            "admin.submission.accept",
            serde_json::json!({
                "submission_id": transition.id.to_string(),
                "previous_status": transition.previous_status,
                "new_status": transition.new_status,
            }),
        )
        .await;
    }

    (
        StatusCode::OK,
        Json(SubmissionTransitionResponse::from(transition)),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/submissions/{id}/reject",
    tag = "admin-submissions",
    operation_id = "admin_submissions_reject",
    params(("id" = String, Path, description = "Submission UUID")),
    request_body = RejectRequest,
    responses(
        (status = 200, description = "Submission rejected (or already rejected)", body = SubmissionTransitionResponse),
        (status = 400, description = "Invalid id or reason", body = ApiErrorBody),
        (status = 403, description = "Caller is not a moderator", body = ApiErrorBody),
        (status = 404, description = "Submission not found", body = ApiErrorBody),
        (status = 409, description = "Submission is in a terminal state", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn reject<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    moderator: RequireModerator,
    Path(id_str): Path<String>,
    Json(req): Json<RejectRequest>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let moderator_id = match parse_moderator_id(&moderator) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let reason = req.reason.trim();
    if reason.is_empty() {
        return err(StatusCode::BAD_REQUEST, "missing_reason");
    }
    if reason.len() > REJECT_REASON_MAX_LEN {
        return err(StatusCode::BAD_REQUEST, "reason_too_long");
    }

    let transition = match store.reject_submission(id, moderator_id, reason).await {
        Ok(t) => t,
        Err(e) => return transition_error_response(e),
    };

    if transition.was_changed {
        record_audit(
            &audit,
            &moderator,
            "admin.submission.reject",
            serde_json::json!({
                "submission_id": transition.id.to_string(),
                "previous_status": transition.previous_status,
                "new_status": transition.new_status,
                "reason": reason,
            }),
        )
        .await;
    }

    (
        StatusCode::OK,
        Json(SubmissionTransitionResponse::from(transition)),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/submissions/{id}/dismiss-flag",
    tag = "admin-submissions",
    operation_id = "admin_submissions_dismiss_flag",
    params(("id" = String, Path, description = "Submission UUID")),
    responses(
        (status = 200, description = "Flag dismissed; submission returned to review", body = SubmissionTransitionResponse),
        (status = 400, description = "Invalid id", body = ApiErrorBody),
        (status = 403, description = "Caller is not a moderator", body = ApiErrorBody),
        (status = 404, description = "Submission not found", body = ApiErrorBody),
        (status = 409, description = "Submission is in a terminal state", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn dismiss_flag<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    moderator: RequireModerator,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let moderator_id = match parse_moderator_id(&moderator) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let transition = match store.dismiss_flag(id, moderator_id).await {
        Ok(t) => t,
        Err(e) => return transition_error_response(e),
    };

    if transition.was_changed {
        record_audit(
            &audit,
            &moderator,
            "admin.submission.dismiss_flag",
            serde_json::json!({
                "submission_id": transition.id.to_string(),
                "previous_status": transition.previous_status,
                "new_status": transition.new_status,
            }),
        )
        .await;
    }

    (
        StatusCode::OK,
        Json(SubmissionTransitionResponse::from(transition)),
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/submissions/queue",
    tag = "admin-submissions",
    operation_id = "admin_submissions_queue",
    params(QueueParams),
    responses(
        (status = 200, description = "Page of moderator-actionable submissions", body = AdminQueueResponse),
        (status = 400, description = "Invalid filter", body = ApiErrorBody),
        (status = 403, description = "Caller is not a moderator", body = ApiErrorBody),
    ),
    security(("BearerAuth" = []))
)]
pub async fn queue<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    _moderator: RequireModerator,
    Query(params): Query<QueueParams>,
) -> Response {
    let filter = match parse_queue_filter(params.status.as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let limit = params.limit.clamp(QUEUE_LIMIT_MIN, QUEUE_LIMIT_MAX) as i64;
    let offset = params.offset as i64;

    // Ask for one extra row so we can compute `has_more` cheaply.
    let mut rows = match store.list_admin_queue(filter, limit + 1, offset).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "admin queue list failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "queue_failed").into_response();
        }
    };
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.truncate(limit as usize);
    }

    let items: Vec<SubmissionDto> = rows.into_iter().map(submission_to_dto).collect();
    (StatusCode::OK, Json(AdminQueueResponse { items, has_more })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use crate::submissions::test_support::MemorySubmissionStore;
    use crate::submissions::{NewSubmission, SubmissionStatus, AUTO_FLAG_THRESHOLD};
    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    /// Build a router with the full extension stack: verifier (so the
    /// JWT extractor works), staff-role store (so RequireModerator can
    /// evaluate the gate), and audit log (so handlers can append).
    fn build_app(
        store: Arc<MemorySubmissionStore>,
        audit: Arc<MemoryAuditLog>,
        staff: Arc<MemoryStaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        let audit_dyn: Arc<dyn AuditLog> = audit;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        router::<MemorySubmissionStore>(store)
            .layer(Extension(verifier))
            .layer(Extension(audit_dyn))
            .layer(Extension(staff_dyn))
    }

    fn issue_token(issuer: &TokenIssuer, user_id: Uuid, handle: &str) -> String {
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign user token")
    }

    async fn json_body<T: for<'de> serde::Deserialize<'de>>(resp: Response) -> T {
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "decode {}: {} (body={})",
                std::any::type_name::<T>(),
                e,
                String::from_utf8_lossy(&bytes)
            )
        })
    }

    /// Seed one submission authored by `submitter` and force its status
    /// to `target` directly through the in-memory store. Cheaper than
    /// driving the lifecycle through public methods for every test.
    async fn seed_submission(
        store: &MemorySubmissionStore,
        submitter: Uuid,
        label: &str,
        target: SubmissionStatus,
    ) -> Uuid {
        let s = store
            .create(NewSubmission {
                submitter_id: submitter,
                pattern: label,
                proposed_label: label,
                description: "seed",
                sample_line: "seed",
                log_source: "live",
            })
            .await
            .expect("create");
        if target != SubmissionStatus::Review {
            // Drive the in-memory row directly to the target state via
            // existing public methods rather than poking private fields.
            // For `Flagged` we use the auto-escalation path; everything
            // else here is exercised through the new admin transitions.
            if target == SubmissionStatus::Flagged {
                for i in 0..AUTO_FLAG_THRESHOLD {
                    let flagger = Uuid::now_v7();
                    store
                        .flag(s.id, flagger, Some(&format!("seed-{i}")))
                        .await
                        .unwrap();
                }
            } else if target == SubmissionStatus::Accepted {
                store.accept_submission(s.id, Uuid::now_v7()).await.unwrap();
            } else {
                panic!("seed_submission: unsupported target {:?}", target);
            }
        }
        s.id
    }

    /// Common fixture: build store + verifier + audit + staff and seed
    /// `mod_user` (with moderator role) and `plain_user` (no role).
    async fn fixture() -> (
        Arc<MemorySubmissionStore>,
        Arc<MemoryAuditLog>,
        Arc<MemoryStaffRoleStore>,
        Router,
        TokenIssuer,
        Uuid, // mod_user
        Uuid, // plain_user
        Uuid, // submitter
    ) {
        let store = Arc::new(MemorySubmissionStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();

        let mod_user = Uuid::now_v7();
        let plain_user = Uuid::now_v7();
        let submitter = Uuid::now_v7();
        store.add_user(mod_user, "mod");
        store.add_user(plain_user, "plain");
        store.add_user(submitter, "submitter");

        // Only mod_user gets the role.
        staff
            .grant(mod_user, StaffRole::Moderator, None, None)
            .await
            .unwrap();

        let app = build_app(
            store.clone(),
            audit.clone(),
            staff.clone(),
            Arc::new(verifier),
        );
        (
            store, audit, staff, app, issuer, mod_user, plain_user, submitter,
        )
    }

    #[tokio::test]
    async fn accept_review_succeeds_and_writes_audit() {
        let (store, audit, _staff, app, issuer, mod_user, _plain, submitter) = fixture().await;
        let id = seed_submission(&store, submitter, "x_event", SubmissionStatus::Review).await;
        let token = issue_token(&issuer, mod_user, "mod");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/admin/submissions/{id}/accept"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: SubmissionTransitionResponse = json_body(resp).await;
        assert!(body.was_changed);
        assert_eq!(body.previous_status, "review");
        assert_eq!(body.new_status, "accepted");

        // Submission row reflects the new state.
        let detail = store.find_by_id(id, None).await.unwrap().unwrap();
        assert_eq!(detail.submission.status, SubmissionStatus::Accepted);

        // Exactly one audit row, with the expected action + actor.
        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "admin.submission.accept");
        assert_eq!(
            entries[0].actor_sub.as_deref(),
            Some(mod_user.to_string().as_str())
        );
        assert_eq!(entries[0].actor_handle.as_deref(), Some("mod"));
    }

    #[tokio::test]
    async fn accept_already_accepted_is_idempotent_no_audit() {
        let (store, audit, _staff, app, issuer, mod_user, _plain, submitter) = fixture().await;
        let id = seed_submission(&store, submitter, "x_event", SubmissionStatus::Accepted).await;
        let token = issue_token(&issuer, mod_user, "mod");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/admin/submissions/{id}/accept"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: SubmissionTransitionResponse = json_body(resp).await;
        assert!(!body.was_changed, "second accept must be a no-op");
        assert_eq!(body.new_status, "accepted");

        // Seed used the public store API, which never writes audit on
        // its own. So the audit log starts at 0; after the idempotent
        // 200 OK it must still be 0.
        let entries = audit.snapshot();
        assert_eq!(entries.len(), 0);
    }

    #[tokio::test]
    async fn reject_writes_reason_to_db() {
        let (store, audit, _staff, app, issuer, mod_user, _plain, submitter) = fixture().await;
        let id = seed_submission(&store, submitter, "x_event", SubmissionStatus::Review).await;
        let token = issue_token(&issuer, mod_user, "mod");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/admin/submissions/{id}/reject"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                json!({"reason": "duplicates rule X"}).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: SubmissionTransitionResponse = json_body(resp).await;
        assert!(body.was_changed);
        assert_eq!(body.new_status, "rejected");

        let detail = store.find_by_id(id, None).await.unwrap().unwrap();
        assert_eq!(detail.submission.status, SubmissionStatus::Rejected);
        assert_eq!(
            detail.submission.rejection_reason.as_deref(),
            Some("duplicates rule X")
        );

        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "admin.submission.reject");
        assert_eq!(
            entries[0].payload.get("reason").and_then(|v| v.as_str()),
            Some("duplicates rule X")
        );
    }

    #[tokio::test]
    async fn dismiss_flag_returns_to_review() {
        let (store, audit, _staff, app, issuer, mod_user, _plain, submitter) = fixture().await;
        let id = seed_submission(&store, submitter, "x_event", SubmissionStatus::Flagged).await;
        let token = issue_token(&issuer, mod_user, "mod");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/admin/submissions/{id}/dismiss-flag"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: SubmissionTransitionResponse = json_body(resp).await;
        assert!(body.was_changed);
        assert_eq!(body.previous_status, "flagged");
        assert_eq!(body.new_status, "review");

        let detail = store.find_by_id(id, None).await.unwrap().unwrap();
        assert_eq!(detail.submission.status, SubmissionStatus::Review);

        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "admin.submission.dismiss_flag");
    }

    #[tokio::test]
    async fn non_moderator_gets_403() {
        let (store, audit, _staff, app, issuer, _mod_user, plain_user, submitter) = fixture().await;
        let id = seed_submission(&store, submitter, "x_event", SubmissionStatus::Review).await;
        let token = issue_token(&issuer, plain_user, "plain");

        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/admin/submissions/{id}/accept"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // Submission state must not have moved.
        let detail = store.find_by_id(id, None).await.unwrap().unwrap();
        assert_eq!(detail.submission.status, SubmissionStatus::Review);

        // No audit row for the rejected attempt.
        assert!(audit.snapshot().is_empty());
    }

    #[tokio::test]
    async fn queue_filter_review() {
        let (store, _audit, _staff, app, issuer, mod_user, _plain, submitter) = fixture().await;

        // 2 review + 2 flagged + 1 accepted.
        seed_submission(&store, submitter, "r1", SubmissionStatus::Review).await;
        seed_submission(&store, submitter, "r2", SubmissionStatus::Review).await;
        seed_submission(&store, submitter, "f1", SubmissionStatus::Flagged).await;
        seed_submission(&store, submitter, "f2", SubmissionStatus::Flagged).await;
        seed_submission(&store, submitter, "a1", SubmissionStatus::Accepted).await;

        let token = issue_token(&issuer, mod_user, "mod");
        let req = Request::builder()
            .method("GET")
            .uri("/v1/admin/submissions/queue?status=review")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AdminQueueResponse = json_body(resp).await;
        assert_eq!(body.items.len(), 2, "review filter -> 2 items");
        for item in &body.items {
            assert_eq!(item.status, "review");
        }
        assert!(!body.has_more);
    }

    #[tokio::test]
    async fn queue_filter_all() {
        let (store, _audit, _staff, app, issuer, mod_user, _plain, submitter) = fixture().await;

        // Same seeding as the review test.
        seed_submission(&store, submitter, "r1", SubmissionStatus::Review).await;
        seed_submission(&store, submitter, "r2", SubmissionStatus::Review).await;
        seed_submission(&store, submitter, "f1", SubmissionStatus::Flagged).await;
        seed_submission(&store, submitter, "f2", SubmissionStatus::Flagged).await;
        seed_submission(&store, submitter, "a1", SubmissionStatus::Accepted).await;

        let token = issue_token(&issuer, mod_user, "mod");
        let req = Request::builder()
            .method("GET")
            .uri("/v1/admin/submissions/queue?status=all")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: AdminQueueResponse = json_body(resp).await;
        // 2 review + 2 flagged = 4. Accepted is intentionally excluded.
        assert_eq!(body.items.len(), 4, "all filter -> review + flagged only");
        for item in &body.items {
            assert!(
                item.status == "review" || item.status == "flagged",
                "unexpected status in queue: {}",
                item.status
            );
        }
    }
}
