//! HTTP handlers for the Submissions domain.
//!
//! Endpoints (all under `/v1/submissions`):
//!   - `GET    /v1/submissions?status=&mine=&limit=&offset=`  list
//!   - `POST   /v1/submissions`                              create
//!   - `GET    /v1/submissions/:id`                          detail
//!   - `POST   /v1/submissions/:id/vote`   { "vote": true }  toggle
//!   - `POST   /v1/submissions/:id/flag`   { "reason": ... } add flag
//!   - `POST   /v1/submissions/:id/withdraw`                 submitter
//!
//! Per-user "once per item" semantics fall out of the composite primary
//! keys on `submission_votes` and `submission_flags`. The vote endpoint
//! takes a single `vote: bool` body and dispatches to insert or delete
//! so the client can implement a toggle in one round-trip.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::restriction_guard::{RequireUnrestricted, Submissions};
use crate::submissions::{
    NewSubmission, PostgresSubmissionStore, Submission, SubmissionFilter, SubmissionSort,
    SubmissionStatus, SubmissionStore, SubmissionWithViewer, WriteOutcome, DESCRIPTION_MAX_LEN,
    FLAG_REASON_MAX_LEN, LABEL_MAX_LEN, PATTERN_MAX_LEN, SAMPLE_LINE_MAX_LEN,
};
use crate::validation::is_valid_event_type;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const LIST_LIMIT_MAX: u32 = 100;
const LIST_LIMIT_MIN: u32 = 1;
const LIST_LIMIT_DEFAULT: u32 = 50;

const ALLOWED_LOG_SOURCES: &[&str] = &["live", "ptu", "eptu"];

pub fn routes(submissions: Arc<PostgresSubmissionStore>) -> Router {
    Router::new()
        .route(
            "/v1/submissions",
            post(create::<PostgresSubmissionStore>).get(list::<PostgresSubmissionStore>),
        )
        .route(
            "/v1/submissions/:id",
            get(detail::<PostgresSubmissionStore>),
        )
        .route(
            "/v1/submissions/:id/vote",
            post(vote::<PostgresSubmissionStore>),
        )
        .route(
            "/v1/submissions/:id/flag",
            post(flag::<PostgresSubmissionStore>),
        )
        .route(
            "/v1/submissions/:id/withdraw",
            post(withdraw::<PostgresSubmissionStore>),
        )
        .with_state(submissions)
}

// -- DTOs ------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubmissionDto {
    pub id: String,
    pub submitter_id: String,
    pub submitter_handle: String,
    pub pattern: String,
    pub proposed_label: String,
    pub description: String,
    pub sample_line: String,
    pub log_source: String,
    pub status: String,
    pub rejection_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub vote_count: i64,
    pub flag_count: i64,
    /// True iff the authenticated viewer has already voted on this
    /// submission. Always false for unauthenticated callers.
    pub viewer_voted: bool,
    pub viewer_flagged: bool,
}

impl From<SubmissionWithViewer> for SubmissionDto {
    fn from(s: SubmissionWithViewer) -> Self {
        let SubmissionWithViewer {
            submission,
            viewer_voted,
            viewer_flagged,
        } = s;
        let Submission {
            id,
            submitter_id,
            submitter_handle,
            pattern,
            proposed_label,
            description,
            sample_line,
            log_source,
            status,
            rejection_reason,
            created_at,
            updated_at,
            vote_count,
            flag_count,
        } = submission;
        Self {
            id: id.to_string(),
            submitter_id: submitter_id.to_string(),
            submitter_handle,
            pattern,
            proposed_label,
            description,
            sample_line,
            log_source,
            status: status.as_str().to_string(),
            rejection_reason,
            created_at,
            updated_at,
            vote_count,
            flag_count,
            viewer_voted,
            viewer_flagged,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Filter by lifecycle status. Omit for "all".
    #[serde(default)]
    pub status: Option<String>,
    /// Restrict to submissions the caller authored.
    #[serde(default)]
    pub mine: Option<bool>,
    /// Sort order: `newest` (default), `oldest`, or `votes`.
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default = "default_list_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_list_limit() -> u32 {
    LIST_LIMIT_DEFAULT
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListResponse {
    pub submissions: Vec<SubmissionDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSubmissionRequest {
    pub pattern: String,
    pub proposed_label: String,
    pub description: String,
    pub sample_line: String,
    pub log_source: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateSubmissionResponse {
    pub submission: SubmissionDto,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VoteRequest {
    /// `true` to record / keep a vote; `false` to retract one.
    pub vote: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VoteResponse {
    pub voted: bool,
    pub vote_count: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FlagRequest {
    /// Optional free-text reason. Capped at FLAG_REASON_MAX_LEN.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FlagResponse {
    pub flag_count: i64,
    pub status: String,
    /// `true` only on the request that crossed the auto-escalation
    /// threshold; lets the UI surface a one-shot toast.
    pub escalated: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WithdrawResponse {
    pub submission: SubmissionDto,
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

fn parse_user_id(auth: &AuthenticatedUser) -> Result<Uuid, Response> {
    Uuid::parse_str(&auth.sub).map_err(|_| err(StatusCode::UNAUTHORIZED, "invalid_subject"))
}

fn parse_id(raw: &str) -> Result<Uuid, Response> {
    Uuid::parse_str(raw).map_err(|_| err(StatusCode::BAD_REQUEST, "invalid_id"))
}

// -- Handlers --------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/submissions",
    tag = "submissions",
    operation_id = "submissions_list",
    params(ListParams),
    responses(
        (status = 200, description = "Page of submissions matching the filter", body = ListResponse),
        (status = 400, description = "Invalid filter", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
    Query(params): Query<ListParams>,
) -> Response {
    let viewer_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    let status = match params.status.as_deref() {
        Some(s) => match SubmissionStatus::parse(s) {
            Some(parsed) => Some(parsed),
            None => return err(StatusCode::BAD_REQUEST, "invalid_status"),
        },
        None => None,
    };

    let sort = match params.sort.as_deref() {
        Some(s) => match SubmissionSort::parse(s) {
            Some(parsed) => parsed,
            None => return err(StatusCode::BAD_REQUEST, "invalid_sort"),
        },
        None => SubmissionSort::default(),
    };

    let limit = params.limit.clamp(LIST_LIMIT_MIN, LIST_LIMIT_MAX) as i64;
    let offset = params.offset as i64;

    let filter = SubmissionFilter {
        status,
        mine_only: params.mine.unwrap_or(false),
        sort,
    };

    match store.list(filter, Some(viewer_id), limit, offset).await {
        Ok(rows) => (
            StatusCode::OK,
            Json(ListResponse {
                submissions: rows.into_iter().map(SubmissionDto::from).collect(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "list submissions failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "list_failed").into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/submissions",
    tag = "submissions",
    operation_id = "submissions_create",
    request_body = CreateSubmissionRequest,
    responses(
        (status = 201, description = "Submission created", body = CreateSubmissionResponse),
        (status = 400, description = "Validation failed", body = ApiErrorBody),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn create<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    guard: RequireUnrestricted<Submissions>,
    Json(req): Json<CreateSubmissionRequest>,
) -> Response {
    // The guard already extracted and verified the JWT; taking a second
    // `AuthenticatedUser` parameter would verify it twice per request.
    let auth = guard.into_user();
    let viewer_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    let pattern = req.pattern.trim();
    let label = req.proposed_label.trim();
    let description = req.description.trim();
    let sample = req.sample_line.trim();
    let log_source = req.log_source.trim().to_lowercase();

    if pattern.is_empty() || pattern.len() > PATTERN_MAX_LEN {
        return err(StatusCode::BAD_REQUEST, "invalid_pattern");
    }
    if label.is_empty() || label.len() > LABEL_MAX_LEN || !is_valid_event_type(label) {
        // Reuse the existing event_type slug rule so submitted labels
        // can land directly as parser rule names without a second
        // validation pass at acceptance time.
        return err(StatusCode::BAD_REQUEST, "invalid_label");
    }
    if description.is_empty() || description.len() > DESCRIPTION_MAX_LEN {
        return err(StatusCode::BAD_REQUEST, "invalid_description");
    }
    if sample.is_empty() || sample.len() > SAMPLE_LINE_MAX_LEN {
        return err(StatusCode::BAD_REQUEST, "invalid_sample_line");
    }
    if !ALLOWED_LOG_SOURCES.contains(&log_source.as_str()) {
        return err(StatusCode::BAD_REQUEST, "invalid_log_source");
    }

    let new = NewSubmission {
        submitter_id: viewer_id,
        pattern,
        proposed_label: label,
        description,
        sample_line: sample,
        log_source: &log_source,
    };

    match store.create(new).await {
        Ok(s) => {
            let dto = SubmissionDto::from(SubmissionWithViewer {
                submission: s,
                viewer_voted: false,
                viewer_flagged: false,
            });
            (
                StatusCode::CREATED,
                Json(CreateSubmissionResponse { submission: dto }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "create submission failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "create_failed").into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/submissions/{id}",
    tag = "submissions",
    operation_id = "submissions_detail",
    params(("id" = String, Path, description = "Submission UUID")),
    responses(
        (status = 200, description = "Submission detail", body = SubmissionDto),
        (status = 400, description = "Invalid id", body = ApiErrorBody),
        (status = 404, description = "Not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn detail<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let viewer_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    match store.find_by_id(id, Some(viewer_id)).await {
        Ok(Some(s)) => (StatusCode::OK, Json(SubmissionDto::from(s))).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "submission detail failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "detail_failed").into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/submissions/{id}/vote",
    tag = "submissions",
    operation_id = "submissions_vote",
    params(("id" = String, Path, description = "Submission UUID")),
    request_body = VoteRequest,
    responses(
        (status = 200, description = "Vote toggled", body = VoteResponse),
        (status = 400, description = "Invalid id", body = ApiErrorBody),
        (status = 404, description = "Not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn vote<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
    Path(id_str): Path<String>,
    Json(req): Json<VoteRequest>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let viewer_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    let result = if req.vote {
        store.vote(id, viewer_id).await
    } else {
        store.unvote(id, viewer_id).await
    };
    let _ = match result {
        Ok(WriteOutcome::Inserted) | Ok(WriteOutcome::AlreadyExists) => (),
        Err(e) => {
            tracing::error!(error = %e, "vote toggle failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "vote_failed").into_response();
        }
    };

    // Re-read so the response carries the post-write count + viewer
    // state. Two round-trips per vote is fine; this isn't a hot path.
    match store.find_by_id(id, Some(viewer_id)).await {
        Ok(Some(s)) => (
            StatusCode::OK,
            Json(VoteResponse {
                voted: s.viewer_voted,
                vote_count: s.submission.vote_count,
            }),
        )
            .into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "vote re-read failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "vote_failed").into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/submissions/{id}/flag",
    tag = "submissions",
    operation_id = "submissions_flag",
    params(("id" = String, Path, description = "Submission UUID")),
    request_body = FlagRequest,
    responses(
        (status = 200, description = "Flag recorded", body = FlagResponse),
        (status = 400, description = "Invalid id or reason too long", body = ApiErrorBody),
        (status = 404, description = "Not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn flag<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
    Path(id_str): Path<String>,
    Json(req): Json<FlagRequest>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let viewer_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    let reason = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(r) = reason {
        if r.len() > FLAG_REASON_MAX_LEN {
            return err(StatusCode::BAD_REQUEST, "reason_too_long");
        }
    }

    let outcome = match store.flag(id, viewer_id, reason).await {
        Ok(o) => o,
        Err(crate::submissions::SubmissionError::NotFound) => {
            return err(StatusCode::NOT_FOUND, "not_found")
        }
        Err(e) => {
            tracing::error!(error = %e, "flag failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "flag_failed").into_response();
        }
    };

    // Read back the post-write status so the UI doesn't have to guess
    // the lifecycle effect of the flag (it might have just escalated).
    let status = match store.find_by_id(id, Some(viewer_id)).await {
        Ok(Some(s)) => s.submission.status.as_str().to_string(),
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => {
            tracing::error!(error = %e, "flag re-read failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "flag_failed").into_response();
        }
    };

    (
        StatusCode::OK,
        Json(FlagResponse {
            flag_count: outcome.flag_count,
            status,
            escalated: outcome.escalated,
        }),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/submissions/{id}/withdraw",
    tag = "submissions",
    operation_id = "submissions_withdraw",
    params(("id" = String, Path, description = "Submission UUID")),
    responses(
        (status = 200, description = "Submission withdrawn", body = WithdrawResponse),
        (status = 400, description = "Already past review", body = ApiErrorBody),
        (status = 403, description = "Not your submission", body = ApiErrorBody),
        (status = 404, description = "Not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn withdraw<S: SubmissionStore>(
    State(store): State<Arc<S>>,
    auth: AuthenticatedUser,
    Path(id_str): Path<String>,
) -> Response {
    let id = match parse_id(&id_str) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let viewer_id = match parse_user_id(&auth) {
        Ok(id) => id,
        Err(r) => return r,
    };

    match store.withdraw(id, viewer_id).await {
        Ok(s) => {
            let dto = SubmissionDto::from(SubmissionWithViewer {
                submission: s,
                viewer_voted: false,
                viewer_flagged: false,
            });
            (StatusCode::OK, Json(WithdrawResponse { submission: dto })).into_response()
        }
        Err(crate::submissions::SubmissionError::NotFound) => {
            err(StatusCode::NOT_FOUND, "not_found")
        }
        Err(crate::submissions::SubmissionError::Forbidden) => {
            err(StatusCode::FORBIDDEN, "not_submitter")
        }
        Err(crate::submissions::SubmissionError::BadState) => {
            err(StatusCode::BAD_REQUEST, "bad_state")
        }
        Err(e) => {
            tracing::error!(error = %e, "withdraw failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "withdraw_failed").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::submissions::test_support::MemorySubmissionStore;
    use crate::submissions::AUTO_FLAG_THRESHOLD;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Extension;
    use serde_json::json;
    use tower::ServiceExt;

    fn router(store: Arc<MemorySubmissionStore>, verifier: Arc<AuthVerifier>) -> Router {
        Router::new()
            .route(
                "/v1/submissions",
                post(create::<MemorySubmissionStore>).get(list::<MemorySubmissionStore>),
            )
            .route("/v1/submissions/:id", get(detail::<MemorySubmissionStore>))
            .route(
                "/v1/submissions/:id/vote",
                post(vote::<MemorySubmissionStore>),
            )
            .route(
                "/v1/submissions/:id/flag",
                post(flag::<MemorySubmissionStore>),
            )
            .route(
                "/v1/submissions/:id/withdraw",
                post(withdraw::<MemorySubmissionStore>),
            )
            .layer(Extension(verifier))
            .layer(Extension({
                // Restriction-gated routes deny when this extension is
                // absent, so every test router that mounts one needs an
                // (unrestricted) store. That is the guard being loud
                // about a wiring gap rather than silently permitting.
                let s: std::sync::Arc<dyn crate::account_restrictions::AccountRestrictionStore> =
                    std::sync::Arc::new(
                        crate::account_restrictions::test_support::MemoryAccountRestrictionStore::new(),
                    );
                s
            }))
            .with_state(store)
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

    fn create_payload() -> serde_json::Value {
        json!({
            "pattern": "<X> *",
            "proposed_label": "x_event",
            "description": "An x event.",
            "sample_line": "<X> hello world",
            "log_source": "live",
        })
    }

    #[tokio::test]
    async fn create_then_get_back() {
        let store = Arc::new(MemorySubmissionStore::default());
        let alice_id = Uuid::now_v7();
        store.add_user(alice_id, "alice");
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, alice_id, "alice");
        let app = router(store, Arc::new(verifier));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(create_payload().to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: CreateSubmissionResponse = json_body(resp).await;
        assert_eq!(body.submission.proposed_label, "x_event");
        assert_eq!(body.submission.status, "review");
        assert_eq!(body.submission.vote_count, 0);

        let req = Request::builder()
            .method("GET")
            .uri(format!("/v1/submissions/{}", body.submission.id))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let detail: SubmissionDto = json_body(resp).await;
        assert_eq!(detail.id, body.submission.id);
    }

    #[tokio::test]
    async fn create_rejects_bad_label_slug() {
        let store = Arc::new(MemorySubmissionStore::default());
        let alice_id = Uuid::now_v7();
        store.add_user(alice_id, "alice");
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, alice_id, "alice");
        let app = router(store, Arc::new(verifier));

        let bad = json!({
            "pattern": "<X>",
            "proposed_label": "BadLabel",
            "description": "x",
            "sample_line": "x",
            "log_source": "live",
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/submissions")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(bad.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: ApiErrorBody = json_body(resp).await;
        assert_eq!(body.error, "invalid_label");
    }

    #[tokio::test]
    async fn vote_toggles_via_body_flag() {
        let store = Arc::new(MemorySubmissionStore::default());
        let alice_id = Uuid::now_v7();
        store.add_user(alice_id, "alice");
        let bob_id = Uuid::now_v7();
        store.add_user(bob_id, "bob");
        let (issuer, verifier) = fresh_pair();
        let app = router(store.clone(), Arc::new(verifier));

        let alice_token = issue_token(&issuer, alice_id, "alice");
        let bob_token = issue_token(&issuer, bob_id, "bob");

        // Alice creates.
        let req = Request::builder()
            .method("POST")
            .uri("/v1/submissions")
            .header("authorization", format!("Bearer {alice_token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(create_payload().to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let created: CreateSubmissionResponse = json_body(resp).await;

        // Bob votes.
        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/submissions/{}/vote", created.submission.id))
            .header("authorization", format!("Bearer {bob_token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json!({"vote": true}).to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v: VoteResponse = json_body(resp).await;
        assert!(v.voted);
        assert_eq!(v.vote_count, 1);

        // Bob retracts.
        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/submissions/{}/vote", created.submission.id))
            .header("authorization", format!("Bearer {bob_token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json!({"vote": false}).to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let v: VoteResponse = json_body(resp).await;
        assert!(!v.voted);
        assert_eq!(v.vote_count, 0);
    }

    #[tokio::test]
    async fn three_distinct_flags_escalate_status() {
        let store = Arc::new(MemorySubmissionStore::default());
        let alice_id = Uuid::now_v7();
        store.add_user(alice_id, "alice");
        let (issuer, verifier) = fresh_pair();
        let alice_token = issue_token(&issuer, alice_id, "alice");
        let app = router(store.clone(), Arc::new(verifier.clone()));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/submissions")
            .header("authorization", format!("Bearer {alice_token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(create_payload().to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let created: CreateSubmissionResponse = json_body(resp).await;

        // Issue tokens for AUTO_FLAG_THRESHOLD distinct users and have
        // each flag once. Only the threshold-th flag should fire
        // `escalated: true`.
        let mut last_escalated = false;
        for i in 0..AUTO_FLAG_THRESHOLD {
            let uid = Uuid::now_v7();
            store.add_user(uid, &format!("flagger{i}"));
            let tok = issue_token(&issuer, uid, &format!("flagger{i}"));
            let req = Request::builder()
                .method("POST")
                .uri(format!("/v1/submissions/{}/flag", created.submission.id))
                .header("authorization", format!("Bearer {tok}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"reason": "broken"}).to_string(),
                ))
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let f: FlagResponse = json_body(resp).await;
            assert_eq!(f.flag_count, i + 1);
            last_escalated = f.escalated;
            if i + 1 < AUTO_FLAG_THRESHOLD {
                assert!(!f.escalated, "early flag should not escalate");
            }
        }
        assert!(last_escalated, "threshold flag should escalate");

        // Detail endpoint reflects the new status.
        let req = Request::builder()
            .method("GET")
            .uri(format!("/v1/submissions/{}", created.submission.id))
            .header("authorization", format!("Bearer {alice_token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let detail: SubmissionDto = json_body(resp).await;
        assert_eq!(detail.status, "flagged");
    }

    #[tokio::test]
    async fn withdraw_only_by_submitter() {
        let store = Arc::new(MemorySubmissionStore::default());
        let alice_id = Uuid::now_v7();
        store.add_user(alice_id, "alice");
        let bob_id = Uuid::now_v7();
        store.add_user(bob_id, "bob");
        let (issuer, verifier) = fresh_pair();
        let alice_token = issue_token(&issuer, alice_id, "alice");
        let bob_token = issue_token(&issuer, bob_id, "bob");
        let app = router(store, Arc::new(verifier));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/submissions")
            .header("authorization", format!("Bearer {alice_token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(create_payload().to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let created: CreateSubmissionResponse = json_body(resp).await;

        // Bob tries -> 403.
        let req = Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/submissions/{}/withdraw",
                created.submission.id
            ))
            .header("authorization", format!("Bearer {bob_token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // Alice succeeds.
        let req = Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/submissions/{}/withdraw",
                created.submission.id
            ))
            .header("authorization", format!("Bearer {alice_token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: WithdrawResponse = json_body(resp).await;
        assert_eq!(body.submission.status, "withdrawn");
    }
}
