//! Public-beta waitlist: the join endpoint, plus the moderator console.
//!
//! `POST /v1/waitlist` is unauthenticated by necessity — the whole point
//! is that the person does not have an account yet. It is anti-enumeration
//! in the same way as `magic/start`: the response shape never
//! distinguishes "you were already on the list" from "you were just
//! added", so it cannot be used to probe whether an address is registered.

use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::mail::Mailer;
use crate::waitlist::{QueueStatus, SignupOutcome, WaitlistStore};
use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Build the waitlist sub-router. `WaitlistStore` + `Mailer` extensions
/// are layered on the outer router in `main`.
pub fn routes() -> Router {
    Router::new()
        .route("/v1/waitlist", post(join))
        .route("/v1/waitlist/status", get(status))
        .route("/v1/admin/waitlist", get(admin_list))
        .route("/v1/admin/waitlist/admit", post(admin_admit))
        .route("/v1/admin/waitlist/resend", post(admin_resend))
        .route("/v1/admin/waitlist/delete", post(admin_delete))
        .route(
            "/v1/admin/waitlist/config",
            get(admin_get_config).put(admin_set_config),
        )
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WaitlistJoinRequest {
    pub email: String,
    /// Free-text channel attribution ("reddit", "spectrum", "discord").
    /// Optional: a signup with no attribution is still a signup.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WaitlistJoinResponse {
    /// Always true when the request was accepted. Anti-enumeration: the
    /// shape does not distinguish "already on the list" from "just added".
    pub joined: bool,
    /// 1-based queue position, or null when admitted immediately.
    pub position: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WaitlistStatusResponse {
    /// Whether invite-only beta gating is active. When true, the web
    /// front-door overlay shows and `/v1/auth/signup` requires an invite.
    pub gate_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WaitlistEntryApi {
    pub id: String,
    pub email: String,
    pub source: Option<String>,
    pub created_at: String,
    pub admitted_at: Option<String>,
    /// RFC3339 timestamp of when the invite was redeemed (an account now
    /// exists), or `None`. The console badges the row and disables its
    /// delete checkbox when this is set — a UX hint only, not a guard:
    /// `delete_batch`'s SQL predicate stays the sole authority.
    pub invite_consumed_at: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdminListQuery {
    /// "queued" (default) or "admitted".
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdmitRequest {
    pub ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AdmitResponse {
    /// How many were actually admitted. Lower than `ids.len()` when a row
    /// was already admitted — a double-click must not re-mint an invite
    /// over a live one.
    pub admitted: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ResendResponse {
    /// How many invites were actually put on the wire. This counts
    /// SUCCESSFUL sends, not matched rows — resend exists precisely
    /// because a send failed, so reporting "resent 1" while the transport
    /// is still broken would be the green-while-doing-nothing lie this
    /// endpoint is meant to end. Rows that aren't admitted are skipped
    /// silently; a row that matched but failed to send is NOT counted.
    pub resent: i64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WaitlistDeleteResponse {
    /// Ids actually removed.
    pub deleted: Vec<String>,
    /// Ids refused because the invite was already redeemed. Ids that never
    /// existed appear in neither list.
    pub blocked: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WaitlistConfigApi {
    pub cap: i64,
    pub gate_enabled: bool,
}

fn error(
    status: StatusCode,
    code: &'static str,
    detail: Option<String>,
) -> axum::response::Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail,
        }),
    )
        .into_response()
}

/// Deliberately minimal: exactly one `@`, something either side, a dot in
/// the domain. Full RFC 5322 validation is a tarpit and delivery is the
/// real check — this only rejects obvious junk before we store it.
fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 3 || s.len() > 254 {
        return false;
    }
    let mut parts = s.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

#[utoipa::path(
    post,
    path = "/v1/waitlist",
    tag = "waitlist",
    operation_id = "waitlist_join",
    request_body = WaitlistJoinRequest,
    responses(
        (status = 200, description = "Joined; admitted immediately or queued", body = WaitlistJoinResponse),
        (status = 400, description = "Malformed email", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn join(
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Json(req): Json<WaitlistJoinRequest>,
) -> impl IntoResponse {
    if !looks_like_email(&req.email) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_email",
            Some("that does not look like an email address".into()),
        );
    }

    let outcome = match store.signup(&req.email, req.source.as_deref()).await {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(error = %e, "waitlist signup failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "signup_failed", None);
        }
    };

    match outcome {
        SignupOutcome::Admitted { ref invite_token } => {
            // Best-effort, like every other send path — a failed mail must
            // not 500 a signup already committed. But it is logged at
            // ERROR, not WARN: a silent failure here is a person who
            // believes they are in the beta and never hears from us again,
            // and the row looks perfectly admitted from the DB side.
            if let Err(e) = mailer.send_waitlist_invite(&req.email, invite_token).await {
                tracing::error!(
                    error = %format!("{e:#}"),
                    email = %req.email,
                    "waitlist invite send FAILED — admitted user has no link; re-admit from /admin/waitlist"
                );
            }
            Json(WaitlistJoinResponse {
                joined: true,
                position: None,
            })
            .into_response()
        }
        SignupOutcome::Queued { position } | SignupOutcome::AlreadyQueued { position } => {
            Json(WaitlistJoinResponse {
                joined: true,
                position: Some(position),
            })
            .into_response()
        }
        SignupOutcome::AlreadyAdmitted => Json(WaitlistJoinResponse {
            joined: true,
            position: None,
        })
        .into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/v1/waitlist/status",
    tag = "waitlist",
    operation_id = "waitlist_status",
    responses(
        (status = 200, description = "Current beta gate state", body = WaitlistStatusResponse),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
)]
pub async fn status(Extension(store): Extension<Arc<dyn WaitlistStore>>) -> impl IntoResponse {
    match store.gate_enabled().await {
        Ok(gate_enabled) => Json(WaitlistStatusResponse { gate_enabled }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "waitlist gate_enabled read failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "status_failed", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/waitlist",
    tag = "waitlist",
    operation_id = "waitlist_admin_list",
    params(
        ("status" = Option<String>, Query, description = "queued (default) | admitted"),
        ("limit" = Option<i64>, Query, description = "max rows, default 100, capped 500"),
    ),
    responses(
        (status = 200, description = "Queue rows", body = Vec<WaitlistEntryApi>),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_list(
    RequireModerator(_user): RequireModerator,
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
    Query(q): Query<AdminListQuery>,
) -> impl IntoResponse {
    let status = match q.status.as_deref() {
        Some("admitted") => QueueStatus::Admitted,
        _ => QueueStatus::Queued,
    };
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    match store.list(status, limit).await {
        Ok(rows) => Json(
            rows.into_iter()
                .map(|r| WaitlistEntryApi {
                    id: r.id.to_string(),
                    email: r.email,
                    source: r.source,
                    created_at: r.created_at.to_rfc3339(),
                    admitted_at: r.admitted_at.map(|t| t.to_rfc3339()),
                    invite_consumed_at: r.invite_consumed_at.map(|t| t.to_rfc3339()),
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "waitlist admin list failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "list_failed", None)
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/waitlist/admit",
    tag = "waitlist",
    operation_id = "waitlist_admin_admit",
    request_body = AdmitRequest,
    responses(
        (status = 200, description = "Count actually admitted", body = AdmitResponse),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_admit(
    RequireModerator(_user): RequireModerator,
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Json(req): Json<AdmitRequest>,
) -> impl IntoResponse {
    let ids: Vec<Uuid> = req
        .ids
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    let invites = match store.admit_batch(&ids).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "waitlist admit_batch failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "admit_failed", None);
        }
    };

    let admitted = invites.len() as i64;
    for inv in &invites {
        if let Err(e) = mailer
            .send_waitlist_invite(&inv.email, &inv.invite_token)
            .await
        {
            tracing::error!(
                error = %format!("{e:#}"),
                email = %inv.email,
                "waitlist invite send FAILED for an admitted user — they have no link"
            );
        }
    }
    Json(AdmitResponse { admitted }).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/waitlist/resend",
    tag = "waitlist",
    operation_id = "waitlist_admin_resend",
    request_body = AdmitRequest,
    responses(
        (status = 200, description = "Count actually re-sent", body = ResendResponse),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_resend(
    RequireModerator(_user): RequireModerator,
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
    Extension(mailer): Extension<Arc<dyn Mailer>>,
    Json(req): Json<AdmitRequest>,
) -> impl IntoResponse {
    let ids: Vec<Uuid> = req
        .ids
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    // Read-only fetch of the EXISTING tokens — no re-mint, so the links
    // already in inboxes stay valid. Recovers the exact failure mode
    // where an auto-admit minted a token but the invite mail never sent
    // (e.g. the SMTP transport was down at admit time).
    let invites = match store.resend_batch(&ids).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "waitlist resend_batch failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "resend_failed", None);
        }
    };

    // Count SUCCESSFUL sends, not matched rows: the whole reason to press
    // resend is that a send failed, so a failure here must lower the
    // reported number rather than hide behind an optimistic count.
    let mut resent = 0i64;
    for inv in &invites {
        if let Err(e) = mailer
            .send_waitlist_invite(&inv.email, &inv.invite_token)
            .await
        {
            tracing::error!(
                error = %format!("{e:#}"),
                email = %inv.email,
                "waitlist invite RESEND failed — the mail transport is still broken"
            );
        } else {
            resent += 1;
        }
    }
    Json(ResendResponse { resent }).into_response()
}

#[utoipa::path(
    post,
    path = "/v1/admin/waitlist/delete",
    tag = "waitlist",
    operation_id = "waitlist_admin_delete",
    request_body = AdmitRequest,
    responses(
        (status = 200, description = "Which ids were deleted vs refused", body = WaitlistDeleteResponse),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_delete(
    RequireModerator(user): RequireModerator,
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(req): Json<AdmitRequest>,
) -> impl IntoResponse {
    let ids: Vec<Uuid> = req
        .ids
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    match store.delete_batch(&ids).await {
        Ok(outcome) => {
            // Best-effort per the project invariant: an audit hiccup must
            // never poison the response. Only emitted when something was
            // actually removed — a fully-blocked batch changed nothing.
            if !outcome.deleted.is_empty() {
                if let Err(e) = audit
                    .append(AuditEntry {
                        actor_sub: Some(user.sub.clone()),
                        actor_handle: Some(user.preferred_username.clone()),
                        action: "waitlist.deleted".to_string(),
                        payload: serde_json::json!({
                            "deleted": outcome.deleted.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                            "blocked": outcome.blocked.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                        }),
                    })
                    .await
                {
                    tracing::warn!(error = %e, "waitlist delete audit append failed");
                }
            }
            Json(WaitlistDeleteResponse {
                deleted: outcome.deleted.iter().map(|id| id.to_string()).collect(),
                blocked: outcome.blocked.iter().map(|id| id.to_string()).collect(),
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "waitlist delete_batch failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "delete_failed", None)
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/waitlist/config",
    tag = "waitlist",
    operation_id = "waitlist_admin_get_config",
    responses(
        (status = 200, description = "Current cap + gate state", body = WaitlistConfigApi),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_get_config(
    RequireModerator(_user): RequireModerator,
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
) -> impl IntoResponse {
    // Read both, or say so. A console that renders a default cap because
    // one read failed would show a number that is not the one enforcing
    // admissions — the worst possible lie for this particular page.
    let cap = match store.cap().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "waitlist cap read failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "config_failed", None);
        }
    };
    let gate_enabled = match store.gate_enabled().await {
        Ok(g) => g,
        Err(e) => {
            tracing::error!(error = %e, "waitlist gate_enabled read failed");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "config_failed", None);
        }
    };
    Json(WaitlistConfigApi { cap, gate_enabled }).into_response()
}

#[utoipa::path(
    put,
    path = "/v1/admin/waitlist/config",
    tag = "waitlist",
    operation_id = "waitlist_admin_set_config",
    request_body = WaitlistConfigApi,
    responses(
        (status = 200, description = "Saved config as stored", body = WaitlistConfigApi),
        (status = 403, description = "Not a moderator", body = ApiErrorBody),
        (status = 500, description = "Server error", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_set_config(
    RequireModerator(user): RequireModerator,
    Extension(store): Extension<Arc<dyn WaitlistStore>>,
    Json(cfg): Json<WaitlistConfigApi>,
) -> impl IntoResponse {
    let by = Uuid::parse_str(&user.sub).ok();
    let cap = cfg.cap.clamp(0, 100_000);
    match store.set_config(cap, cfg.gate_enabled, by).await {
        Ok(()) => {
            tracing::info!(
                cap,
                gate_enabled = cfg.gate_enabled,
                "waitlist config changed"
            );
            // Echo what was STORED, not what was sent — the clamp above
            // may have changed it, and a UI that shows the requested
            // value would quietly disagree with the database.
            Json(WaitlistConfigApi {
                cap,
                gate_enabled: cfg.gate_enabled,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "waitlist set_config failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::mail::test_support::RecordingMailer;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use crate::waitlist::test_support::MemoryWaitlistStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    /// Only the public join route — the admin routes need a staff-role
    /// extension that these tests deliberately do not wire.
    fn app(store: Arc<dyn WaitlistStore>, mailer: Arc<dyn Mailer>) -> Router {
        Router::new()
            .route("/v1/waitlist", post(join))
            .route("/v1/waitlist/status", get(status))
            .layer(Extension(store))
            .layer(Extension(mailer))
    }

    fn join_req(json: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/waitlist")
            .header("content-type", "application/json")
            .body(Body::from(json.to_string()))
            .unwrap()
    }

    // -- Test 1: under cap → admitted + mailed ------------------------

    #[tokio::test]
    async fn joining_under_cap_admits_and_mails_an_invite() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(1));
        let mailer = Arc::new(RecordingMailer::default());
        let app = app(store, mailer.clone());

        let res = app
            .oneshot(join_req(r#"{"email":"a@example.com"}"#))
            .await
            .unwrap();

        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["joined"], true);
        assert_eq!(body["position"], serde_json::Value::Null);
        // Admitted with no email = a person who thinks they are in and
        // never hears from us. Assert the send was attempted.
        assert_eq!(mailer.waitlist_invites().len(), 1);
        assert_eq!(mailer.waitlist_invites()[0].0, "a@example.com");
    }

    // -- Test 2: at cap → queued, no mail -----------------------------

    #[tokio::test]
    async fn joining_at_cap_queues_and_reports_position_without_mailing() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(0));
        let mailer = Arc::new(RecordingMailer::default());
        let app = app(store, mailer.clone());

        let res = app
            .oneshot(join_req(r#"{"email":"a@example.com"}"#))
            .await
            .unwrap();

        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["joined"], true);
        assert_eq!(body["position"], 1);
        // No invite exists yet — mailing one would be a lie.
        assert_eq!(mailer.waitlist_invites().len(), 0);
    }

    // -- Test 3: junk in ---------------------------------------------

    #[tokio::test]
    async fn a_malformed_email_is_rejected() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(1));
        let app = app(store, Arc::new(RecordingMailer::default()));

        let res = app
            .oneshot(join_req(r#"{"email":"not-an-email"}"#))
            .await
            .unwrap();

        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_email");
    }

    #[tokio::test]
    async fn an_empty_email_is_rejected() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(1));
        let app = app(store, Arc::new(RecordingMailer::default()));
        let res = app.oneshot(join_req(r#"{"email":""}"#)).await.unwrap();
        let (status, _) = read_body(res).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // -- Test 4: resubmitting is not an error -------------------------

    #[tokio::test]
    async fn resubmitting_returns_the_same_position_and_does_not_remail() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(0));
        let mailer = Arc::new(RecordingMailer::default());
        let app = app(store, mailer.clone());

        let first = app
            .clone()
            .oneshot(join_req(r#"{"email":"a@example.com"}"#))
            .await
            .unwrap();
        let (_, b1) = read_body(first).await;

        let second = app
            .oneshot(join_req(r#"{"email":"a@example.com"}"#))
            .await
            .unwrap();
        let (status, b2) = read_body(second).await;

        // Forgetting you signed up must not look like a failure.
        assert_eq!(status, StatusCode::OK);
        assert_eq!(b1["position"], 1);
        assert_eq!(b2["position"], 1);
        assert_eq!(mailer.waitlist_invites().len(), 0);
    }

    // -- Test 5: anti-enumeration ------------------------------------

    #[tokio::test]
    async fn an_already_admitted_address_is_indistinguishable_from_a_fresh_admit() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(2));
        let mailer = Arc::new(RecordingMailer::default());
        let app = app(store, mailer.clone());

        let fresh = app
            .clone()
            .oneshot(join_req(r#"{"email":"a@example.com"}"#))
            .await
            .unwrap();
        let (s1, b1) = read_body(fresh).await;

        let repeat = app
            .oneshot(join_req(r#"{"email":"a@example.com"}"#))
            .await
            .unwrap();
        let (s2, b2) = read_body(repeat).await;

        // Identical status AND body: a probe cannot tell whether an
        // address is already on the list.
        assert_eq!(s1, s2);
        assert_eq!(b1, b2);
    }

    // -- Test 6: source attribution ----------------------------------

    #[tokio::test]
    async fn a_source_is_accepted_and_absence_of_one_is_fine() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(5));
        let app = app(store, Arc::new(RecordingMailer::default()));

        let with = app
            .clone()
            .oneshot(join_req(r#"{"email":"a@example.com","source":"reddit"}"#))
            .await
            .unwrap();
        assert_eq!(read_body(with).await.0, StatusCode::OK);

        let without = app
            .oneshot(join_req(r#"{"email":"b@example.com"}"#))
            .await
            .unwrap();
        assert_eq!(read_body(without).await.0, StatusCode::OK);
    }

    // -- Test 7: status reflects the gate flag ------------------------

    #[tokio::test]
    async fn status_returns_gate_flag_off() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::open());
        let mailer = Arc::new(RecordingMailer::default());
        let res = app(store, mailer)
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/waitlist/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["gate_enabled"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn status_reflects_gate_on() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(1));
        let mailer = Arc::new(RecordingMailer::default());
        let res = app(store, mailer)
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/waitlist/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["gate_enabled"], serde_json::json!(true));
    }

    // -- Test 8: the email sniff test --------------------------------

    #[test]
    fn looks_like_email_accepts_real_addresses_and_rejects_junk() {
        assert!(looks_like_email("a@example.com"));
        assert!(looks_like_email("first.last+tag@sub.example.co.uk"));
        assert!(looks_like_email("  padded@example.com  "));
        assert!(!looks_like_email("no-at-sign"));
        assert!(!looks_like_email("@example.com"));
        assert!(!looks_like_email("a@nodot"));
        assert!(!looks_like_email("a@.example.com"));
        assert!(!looks_like_email("a@example.com."));
        assert!(!looks_like_email("two@at@signs.com"));
        assert!(!looks_like_email(""));
    }

    // -- Admin: delete_batch wiring ------------------------------------
    //
    // RequireModerator needs a real JWT (Extension<Arc<AuthVerifier>>)
    // plus a StaffRoleStore extension plus an active moderator grant —
    // `app()` above deliberately skips all of that for the public routes.
    // This mirrors the `build_app`/`moderator_token` idiom used by
    // `admin_submission_routes.rs`, `admin_sharing_routes.rs`,
    // `admin_parser_rules.rs`, `admin_inference_rules.rs`,
    // `admin_parser_submissions.rs`, and `appearance_routes.rs::admin_app`.

    fn admin_test_router(
        store: Arc<dyn WaitlistStore>,
        staff: Arc<dyn StaffRoleStore>,
        verifier: Arc<AuthVerifier>,
        audit: Arc<dyn AuditLog>,
    ) -> Router {
        Router::new()
            .route("/v1/admin/waitlist", get(admin_list))
            .route("/v1/admin/waitlist/delete", post(admin_delete))
            .layer(Extension(store))
            .layer(Extension(staff))
            .layer(Extension(verifier))
            .layer(Extension(audit))
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

    #[tokio::test]
    async fn admin_delete_separates_deleted_from_blocked() {
        let store = Arc::new(MemoryWaitlistStore::with_cap(0));
        store.signup("drop@example.com", None).await.unwrap();
        store.signup("keep@example.com", None).await.unwrap();
        let rows = store.list(QueueStatus::Queued, 10).await.unwrap();
        let drop = rows
            .iter()
            .find(|r| r.email == "drop@example.com")
            .unwrap()
            .id;
        let keep = rows
            .iter()
            .find(|r| r.email == "keep@example.com")
            .unwrap()
            .id;
        let invites = store.admit_batch(&[keep]).await.unwrap();
        store.redeem_invite(&invites[0].invite_token).await.unwrap();

        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "mod").await;
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let app = admin_test_router(store.clone(), staff.clone(), Arc::new(verifier), audit);

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/waitlist/delete")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "ids": [drop.to_string(), keep.to_string()] })
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let body: WaitlistDeleteResponse = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body.deleted, vec![drop.to_string()]);
        assert_eq!(body.blocked, vec![keep.to_string()]);
    }

    // -- Admin: list exposes invite_consumed_at (F2) ---------------------
    //
    // `admin_list` never selected this column until now; a redeemed row
    // rendered identically to a deletable one, so the console had no way
    // to badge it or disable its checkbox before a refused delete looked
    // like a dead button.

    #[tokio::test]
    async fn admin_list_exposes_invite_consumed_at_for_redeemed_rows_only() {
        let store = Arc::new(MemoryWaitlistStore::with_cap(2));
        let SignupOutcome::Admitted { invite_token } =
            store.signup("redeemed@example.com", None).await.unwrap()
        else {
            panic!("expected admitted");
        };
        store.signup("live@example.com", None).await.unwrap();
        store.redeem_invite(&invite_token).await.unwrap();

        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "mod").await;
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let app = admin_test_router(store.clone(), staff.clone(), Arc::new(verifier), audit);

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/admin/waitlist?status=admitted")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        let rows: Vec<WaitlistEntryApi> = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let redeemed = rows
            .iter()
            .find(|r| r.email == "redeemed@example.com")
            .unwrap();
        let live = rows.iter().find(|r| r.email == "live@example.com").unwrap();
        assert!(redeemed.invite_consumed_at.is_some());
        assert!(live.invite_consumed_at.is_none());
    }

    // -- Admin: audit emission -------------------------------------------

    #[tokio::test]
    async fn admin_delete_emits_a_waitlist_deleted_audit_entry_with_actor_identity() {
        let store = Arc::new(MemoryWaitlistStore::with_cap(0));
        store.signup("drop@example.com", None).await.unwrap();
        let id = store.list(QueueStatus::Queued, 10).await.unwrap()[0].id;

        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "mod").await;
        let audit = Arc::new(MemoryAuditLog::default());
        let audit_dyn: Arc<dyn AuditLog> = audit.clone();
        let app = admin_test_router(store.clone(), staff.clone(), Arc::new(verifier), audit_dyn);

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/waitlist/delete")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "ids": [id.to_string()] }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "waitlist.deleted");
        // A destructive action's audit row is close to worthless without
        // knowing who did it — assert both actor fields, not just presence.
        assert_eq!(entries[0].actor_handle.as_deref(), Some("mod"));
        assert!(entries[0].actor_sub.is_some());
        assert_eq!(
            entries[0].payload["deleted"],
            serde_json::json!([id.to_string()])
        );
        assert_eq!(entries[0].payload["blocked"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn admin_delete_returns_200_when_everything_was_blocked() {
        let store = Arc::new(MemoryWaitlistStore::with_cap(0));
        store.signup("real@example.com", None).await.unwrap();
        let id = store.list(QueueStatus::Queued, 10).await.unwrap()[0].id;
        let invites = store.admit_batch(&[id]).await.unwrap();
        store.redeem_invite(&invites[0].invite_token).await.unwrap();

        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = moderator_token(&staff, &issuer, "mod").await;
        let audit = Arc::new(MemoryAuditLog::default());
        let audit_dyn: Arc<dyn AuditLog> = audit.clone();
        let app = admin_test_router(store.clone(), staff.clone(), Arc::new(verifier), audit_dyn);

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/waitlist/delete")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "ids": [id.to_string()] }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // A partial batch has no honest single status code, so the outcome
        // lives in the body — an all-blocked batch is still a 200.
        assert_eq!(res.status(), StatusCode::OK);
        // A fully-blocked batch changed nothing, so nothing is audited.
        assert!(audit.snapshot().is_empty());
    }

    // -- Admin: auth gating ---------------------------------------------
    //
    // Missing token vs authenticated-non-moderator are two different
    // rejections at two different layers: `AuthenticatedUser`'s extractor
    // fails closed on a missing/malformed bearer token before the
    // moderator-role check ever runs (`AuthError` renders uniformly as
    // 401, see auth.rs), while `RequireModerator` renders an authenticated
    // caller who lacks the role as 403. Mirrors
    // `admin_parser_submissions.rs::list_rejects_missing_bearer_token`,
    // the same-shaped test in this crate's other admin-route suites.

    #[tokio::test]
    async fn admin_delete_rejects_missing_bearer_token() {
        let store: Arc<dyn WaitlistStore> = Arc::new(MemoryWaitlistStore::with_cap(0));
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (_issuer, verifier) = fresh_pair();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let app = admin_test_router(store, staff, Arc::new(verifier), audit);

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/waitlist/delete")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({ "ids": [] }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// Mints a token for a fresh UUID with NO staff grant at all -- the
    /// user simply has no row in `MemoryStaffRoleStore`, so
    /// `list_active_for_user` returns an empty set and `has(Moderator)`
    /// is false. Mirrors
    /// `admin_parser_submissions.rs::plain_token` /
    /// `appearance_routes.rs::issue_token` with a non-granted user.
    fn plain_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&Uuid::now_v7().to_string(), handle)
            .expect("sign plain token")
    }

    #[tokio::test]
    async fn admin_delete_rejects_non_moderator_with_403() {
        // This is the load-bearing gate for a destructive endpoint: the
        // missing-token test above only proves authentication is
        // enforced, not authorization. Swap RequireModerator for a
        // weaker extractor and that test keeps passing while this one
        // catches it.
        let store = Arc::new(MemoryWaitlistStore::with_cap(0));
        store.signup("survivor@example.com", None).await.unwrap();
        let id = store.list(QueueStatus::Queued, 10).await.unwrap()[0].id;

        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = plain_token(&issuer, "rando");
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let app = admin_test_router(store.clone(), staff, Arc::new(verifier), audit);

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/waitlist/delete")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "ids": [id.to_string()] }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // A 403 with the row already gone would be the worst possible
        // outcome -- confirm the rejected request never reached the store.
        let queued = store.list(QueueStatus::Queued, 10).await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].id, id);
    }
}
