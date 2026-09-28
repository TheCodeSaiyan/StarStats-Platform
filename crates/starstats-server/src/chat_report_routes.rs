//! Chat report routes (social phase 6). See `chat_reports.rs`.
//!
//! - `POST /v1/chat/reports`: a player reports another player in a room
//!   they share, revealing the messages they choose.
//! - `GET /v1/admin/chat/reports`: the moderation queue.
//! - `POST /v1/admin/chat/reports/{id}/resolve`: dismiss, restrict the
//!   reported player from chat, or suspend them. Both enforcing outcomes
//!   take the player out of every chat room at once.

use crate::account_restrictions::{AccountRestrictionStore, Restriction};
use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::AuthenticatedUser;
use crate::chat_reports::{
    ChatReport, ChatReportReason, ChatReportStatus, ChatReportStore, NewChatReport,
    RevealedMessage, DETAILS_MAX, MAX_MESSAGE_CHARS, MAX_REVEALED, REPORTS_PER_DAY,
    RESOLUTION_NOTE_MAX,
};
use crate::chat_rooms::ChatRooms;
use crate::social_routes::caller;
use crate::users::{validate_handle, UserStore};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/chat/reports", post(report))
        .route("/v1/admin/chat/reports", get(admin_list))
        .route("/v1/admin/chat/reports/{id}/resolve", post(admin_resolve))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportChat {
    pub room_id: String,
    /// The player being reported; they must be in the room.
    pub reported_handle: String,
    pub reason: ChatReportReason,
    #[serde(default)]
    pub details: Option<String>,
    /// The messages the reporter chooses to reveal, 1 to 20, all sent by
    /// the reported player.
    pub messages: Vec<RevealedMessage>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChatReportFiled {
    pub id: Uuid,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ChatReportsQuery {
    /// `open` (default), `dismissed`, `chat_restricted`, `user_suspended`
    /// or `all`.
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatReportList {
    pub reports: Vec<ChatReport>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveChatReport {
    /// `dismissed`, `chat_restricted` or `user_suspended`.
    pub outcome: String,
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

fn internal(what: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, what, "chat report failed");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
}

/// Trimmed optional text: too long or carrying control characters is a 400.
fn text(field: &str, v: Option<String>, max: usize) -> Result<Option<String>, Response> {
    let Some(s) = v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if s.chars().count() > max {
        return Err(err(StatusCode::BAD_REQUEST, &format!("{field}_too_long")));
    }
    if s.chars().any(|c| c.is_control() && c != '\n') {
        return Err(err(StatusCode::BAD_REQUEST, &format!("{field}_invalid")));
    }
    Ok(Some(s))
}

async fn audit_best_effort(
    audit: &dyn AuditLog,
    actor: &str,
    action: &str,
    payload: serde_json::Value,
) {
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: None,
            actor_handle: Some(actor.to_string()),
            action: action.to_string(),
            payload,
        })
        .await
    {
        tracing::warn!(error = %e, action, "audit log append failed");
    }
}

/// Report a player in a chat you share. Only messages you reveal are sent,
/// and only moderators see them.
#[utoipa::path(
    post,
    path = "/v1/chat/reports",
    tag = "chat",
    operation_id = "chat_report",
    request_body = ReportChat,
    responses(
        (status = 200, description = "Reported", body = ChatReportFiled),
        (status = 400, description = "Invalid report", body = ApiErrorBody),
        (status = 404, description = "Not a chat you share", body = ApiErrorBody),
        (status = 429, description = "Too many reports today", body = ApiErrorBody),
        (status = 503, description = "Chat is not available", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn report(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(reports): Extension<Arc<dyn ChatReportStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(rooms): Extension<Arc<Option<ChatRooms>>>,
    Json(body): Json<ReportChat>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(rooms) = rooms.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "chat_unavailable");
    };
    let reported = body.reported_handle.trim();
    if !validate_handle(reported) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    if reported.eq_ignore_ascii_case(&me.claimed_handle) {
        return err(StatusCode::BAD_REQUEST, "cannot_report_self");
    }
    let details = match text("details", body.details, DETAILS_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if body.messages.is_empty() || body.messages.len() > MAX_REVEALED {
        return err(StatusCode::BAD_REQUEST, "messages_count");
    }
    let expected_sender = rooms.user_id(reported);
    for m in &body.messages {
        if !m.event_id.starts_with('$') || m.event_id.len() > 255 {
            return err(StatusCode::BAD_REQUEST, "messages_invalid");
        }
        if m.text.chars().count() > MAX_MESSAGE_CHARS || m.text.trim().is_empty() {
            return err(StatusCode::BAD_REQUEST, "messages_invalid");
        }
        // Only the reported player's own messages: a report is about them.
        if m.sender != expected_sender {
            return err(StatusCode::BAD_REQUEST, "message_not_from_reported");
        }
    }
    // Both must be in the room: you report what you saw, in a chat you share.
    for h in [me.claimed_handle.as_str(), reported] {
        match rooms.store.is_member(&body.room_id, h).await {
            Ok(true) => {}
            Ok(false) => return err(StatusCode::NOT_FOUND, "not_found"),
            Err(e) => return internal("is_member", e),
        }
    }
    match reports
        .count_since(&me.claimed_handle, Utc::now() - Duration::hours(24))
        .await
    {
        Ok(n) if n >= REPORTS_PER_DAY => return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "chat report rate count failed; skipping gate"),
    }
    let r = match reports
        .create(NewChatReport {
            reporter: &me.claimed_handle,
            reported,
            room_id: &body.room_id,
            reason: body.reason,
            details: details.as_deref(),
            messages: &body.messages,
        })
        .await
    {
        Ok(r) => r,
        Err(e) => return internal("create", e),
    };
    // The audit trail names the report, never the revealed text.
    audit_best_effort(
        audit.as_ref(),
        &me.claimed_handle,
        "chat.reported",
        serde_json::json!({
            "report_id": r.id,
            "reported_handle": r.reported_handle,
            "reason": r.reason.as_str(),
            "messages": r.messages.len(),
        }),
    )
    .await;
    Json(ChatReportFiled { id: r.id }).into_response()
}

#[utoipa::path(
    get,
    path = "/v1/admin/chat/reports",
    tag = "admin",
    operation_id = "admin_chat_reports",
    params(ChatReportsQuery),
    responses(
        (status = 200, description = "Reports, newest first", body = ChatReportList),
        (status = 403, description = "Not a moderator"),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_list(
    RequireModerator(_user): RequireModerator,
    Extension(reports): Extension<Arc<dyn ChatReportStore>>,
    Query(q): Query<ChatReportsQuery>,
) -> Response {
    let status = match q.status.as_deref() {
        None | Some("") => Some(ChatReportStatus::Open),
        Some("all") => None,
        Some(s) => match ChatReportStatus::parse(s) {
            Some(s) => Some(s),
            None => return err(StatusCode::BAD_REQUEST, "invalid_status"),
        },
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    match reports.list(status, limit, offset).await {
        Ok(reports) => Json(ChatReportList { reports }).into_response(),
        Err(e) => internal("list", e),
    }
}

/// Resolve a report. `chat_restricted` adds a chat restriction to whatever
/// the player already has; `user_suspended` suspends them outright. Both
/// take them out of every chat room. Enforce first, then record, so a
/// failed enforcement leaves the report open to retry.
#[utoipa::path(
    post,
    path = "/v1/admin/chat/reports/{id}/resolve",
    tag = "admin",
    operation_id = "admin_chat_resolve",
    params(("id" = Uuid, Path, description = "Report id")),
    request_body = ResolveChatReport,
    responses(
        (status = 200, description = "Resolved", body = ChatReport),
        (status = 400, description = "Unknown outcome, or note too long", body = ApiErrorBody),
        (status = 404, description = "No such report", body = ApiErrorBody),
        (status = 409, description = "Already resolved, or the player is gone", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn admin_resolve(
    RequireModerator(moderator): RequireModerator,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(reports): Extension<Arc<dyn ChatReportStore>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    chat: Option<Extension<Arc<Option<ChatRooms>>>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ResolveChatReport>,
) -> Response {
    let outcome = match ChatReportStatus::parse(&body.outcome) {
        Some(s) if s != ChatReportStatus::Open => s,
        _ => return err(StatusCode::BAD_REQUEST, "invalid_outcome"),
    };
    let note = match text("note", body.note, RESOLUTION_NOTE_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let by = moderator.preferred_username.clone();
    let now = Utc::now();
    let pending = match reports.get(id).await {
        Ok(Some(r)) if r.status == ChatReportStatus::Open => r,
        Ok(Some(_)) => return err(StatusCode::CONFLICT, "already_resolved"),
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => return internal("get", e),
    };
    if outcome != ChatReportStatus::Dismissed {
        let target = match users.find_by_handle(&pending.reported_handle).await {
            Ok(Some(u)) => u,
            Ok(None) => return err(StatusCode::CONFLICT, "user_not_found"),
            Err(e) => return internal("find_by_handle", e),
        };
        let reason = note
            .clone()
            .unwrap_or_else(|| format!("After chat report {}", pending.id));
        let restriction = if outcome == ChatReportStatus::UserSuspended {
            Restriction {
                ingest_blocked: true,
                sharing_blocked: true,
                public_profile_blocked: true,
                submissions_blocked: true,
                chat_blocked: true,
                reason,
                restricted_by: by.clone(),
                restricted_at: now,
                expires_at: None,
            }
        } else {
            // Add chat to whatever is already there; a chat restriction
            // must not lift a limit another moderator set.
            match restrictions.effective(target.id).await {
                Ok(Some(existing)) => Restriction {
                    chat_blocked: true,
                    ..existing
                },
                Ok(None) => Restriction {
                    ingest_blocked: false,
                    sharing_blocked: false,
                    public_profile_blocked: false,
                    submissions_blocked: false,
                    chat_blocked: true,
                    reason,
                    restricted_by: by.clone(),
                    restricted_at: now,
                    expires_at: None,
                },
                Err(e) => return internal("effective", e),
            }
        };
        if let Err(e) = restrictions.upsert(target.id, &restriction).await {
            return internal("upsert", e);
        }
        if let Some(rooms) = crate::chat_rooms::from_ext(&chat) {
            crate::chat_rooms::best_effort(
                "remove_everywhere",
                rooms.remove_everywhere(&target.claimed_handle, "Restricted from chat"),
            )
            .await;
        }
    }
    let report = match reports
        .resolve(id, &by, outcome, note.as_deref(), now)
        .await
    {
        Ok(r) => r,
        Err(e) => return internal("resolve", e),
    };
    audit_best_effort(
        audit.as_ref(),
        &by,
        "chat.report_resolved",
        serde_json::json!({
            "report_id": report.id,
            "outcome": outcome.as_str(),
            "reported_handle": report.reported_handle,
        }),
    )
    .await;
    Json(report).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_restrictions::test_support::MemoryAccountRestrictionStore;
    use crate::account_restrictions::Capability;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::TokenIssuer;
    use crate::chat_reports::test_support::MemoryChatReportStore;
    use crate::chat_rooms::test_support::{rooms, MemoryChatRoomStore, RecordingMatrix};
    use crate::chat_rooms::{ChatRoomStore, RoomKind};
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    const ROOM: &str = "!room:starstats.app";

    struct F {
        app: Router,
        issuer: TokenIssuer,
        users: Arc<MemoryUserStore>,
        staff: Arc<MemoryStaffRoleStore>,
        reports: Arc<MemoryChatReportStore>,
        restrictions: Arc<MemoryAccountRestrictionStore>,
        room_store: Arc<MemoryChatRoomStore>,
        matrix: Arc<RecordingMatrix>,
    }

    fn fixture() -> F {
        let users = Arc::new(MemoryUserStore::new());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let reports = Arc::new(MemoryChatReportStore::new());
        let restrictions = Arc::new(MemoryAccountRestrictionStore::new());
        let (issuer, verifier) = fresh_pair();
        let (chat_rooms, room_store, matrix) = rooms();
        let chat_rooms: Arc<Option<ChatRooms>> = Arc::new(Arc::try_unwrap(chat_rooms).ok());
        let app = routes()
            .layer(Extension(users.clone() as Arc<dyn UserStore>))
            .layer(Extension(staff.clone() as Arc<dyn StaffRoleStore>))
            .layer(Extension(reports.clone() as Arc<dyn ChatReportStore>))
            .layer(Extension(
                restrictions.clone() as Arc<dyn AccountRestrictionStore>
            ))
            .layer(Extension(
                Arc::new(MemoryAuditLog::default()) as Arc<dyn AuditLog>
            ))
            .layer(Extension(chat_rooms))
            .layer(Extension(Arc::new(verifier)));
        F {
            app,
            issuer,
            users,
            staff,
            reports,
            restrictions,
            room_store,
            matrix,
        }
    }

    impl F {
        async fn user(&self, handle: &str) -> (String, Uuid) {
            let phc = hash_password("password-123-abcdef").unwrap();
            let u = self
                .users
                .create(&format!("{handle}@example.com"), &phc, handle)
                .await
                .unwrap();
            (
                self.issuer.sign_user(&u.id.to_string(), handle).unwrap(),
                u.id,
            )
        }

        async fn in_room(&self, handles: &[&str]) {
            self.room_store
                .insert_room(ROOM, RoomKind::Dm, None, None)
                .await
                .unwrap();
            for h in handles {
                self.room_store.add_member(ROOM, h).await.unwrap();
            }
        }

        async fn call(
            &self,
            method: &str,
            uri: &str,
            token: &str,
            body: Option<serde_json::Value>,
        ) -> (StatusCode, serde_json::Value) {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"));
            let body = match body {
                Some(v) => {
                    req = req.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let resp = self
                .app
                .clone()
                .oneshot(req.body(body).unwrap())
                .await
                .unwrap();
            let status = resp.status();
            let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
            )
        }
    }

    fn report_body(sender: &str) -> serde_json::Value {
        serde_json::json!({
            "room_id": ROOM,
            "reported_handle": "Mallory",
            "reason": "harassment",
            "details": "kept going after I asked them to stop",
            "messages": [{
                "event_id": "$e1",
                "sender": sender,
                "sent_at": "2026-09-28T12:00:00Z",
                "text": "something unkind"
            }]
        })
    }

    #[tokio::test]
    async fn a_player_reports_someone_in_a_chat_they_share() {
        let f = fixture();
        let (alice, _) = f.user("Alice").await;
        f.user("Mallory").await;
        f.in_room(&["alice", "mallory"]).await;
        let (s, v) = f
            .call(
                "POST",
                "/v1/chat/reports",
                &alice,
                Some(report_body("@mallory:starstats.app")),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let stored = f.reports.all();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].messages[0].text, "something unkind");
        assert_eq!(stored[0].reporter_handle, "Alice");
    }

    #[tokio::test]
    async fn only_shared_rooms_and_only_the_reported_players_messages() {
        let f = fixture();
        let (alice, _) = f.user("Alice").await;
        let (carol, _) = f.user("Carol").await;
        f.user("Mallory").await;
        f.in_room(&["alice", "mallory"]).await;
        let (s, v) = f
            .call(
                "POST",
                "/v1/chat/reports",
                &carol,
                Some(report_body("@mallory:starstats.app")),
            )
            .await;
        assert_eq!(
            (s, v["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("not_found")),
            "not in the room"
        );
        let (s, v) = f
            .call(
                "POST",
                "/v1/chat/reports",
                &alice,
                Some(report_body("@alice:starstats.app")),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "message_not_from_reported");
        let mut body = report_body("@mallory:starstats.app");
        body["messages"] = serde_json::json!([]);
        let (s, v) = f.call("POST", "/v1/chat/reports", &alice, Some(body)).await;
        assert_eq!(
            (s, v["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("messages_count"))
        );
        let mut body = report_body("@mallory:starstats.app");
        body["reported_handle"] = serde_json::json!("alice");
        let (s, v) = f.call("POST", "/v1/chat/reports", &alice, Some(body)).await;
        assert_eq!(
            (s, v["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("cannot_report_self"))
        );
        assert!(f.reports.all().is_empty());
    }

    #[tokio::test]
    async fn five_reports_a_day() {
        let f = fixture();
        let (alice, _) = f.user("Alice").await;
        f.user("Mallory").await;
        f.in_room(&["alice", "mallory"]).await;
        for _ in 0..REPORTS_PER_DAY {
            let (s, _) = f
                .call(
                    "POST",
                    "/v1/chat/reports",
                    &alice,
                    Some(report_body("@mallory:starstats.app")),
                )
                .await;
            assert_eq!(s, StatusCode::OK);
        }
        let (s, _) = f
            .call(
                "POST",
                "/v1/chat/reports",
                &alice,
                Some(report_body("@mallory:starstats.app")),
            )
            .await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn restricting_from_chat_keeps_other_limits_and_empties_their_rooms() {
        let f = fixture();
        let (alice, _) = f.user("Alice").await;
        let (_, mallory_id) = f.user("Mallory").await;
        let (moderator, mod_id) = f.user("Mod").await;
        f.staff
            .grant(mod_id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        f.in_room(&["alice", "mallory"]).await;
        // An earlier, unrelated limit a moderator set.
        f.restrictions
            .upsert(
                mallory_id,
                &Restriction {
                    ingest_blocked: false,
                    sharing_blocked: true,
                    public_profile_blocked: false,
                    submissions_blocked: false,
                    chat_blocked: false,
                    reason: "spammed shares".into(),
                    restricted_by: "earlier".into(),
                    restricted_at: Utc::now(),
                    expires_at: None,
                },
            )
            .await
            .unwrap();
        f.call(
            "POST",
            "/v1/chat/reports",
            &alice,
            Some(report_body("@mallory:starstats.app")),
        )
        .await;
        let id = f.reports.all()[0].id;

        let (s, _) = f.call("GET", "/v1/admin/chat/reports", &alice, None).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "players cannot read the queue");
        let (_, v) = f
            .call("GET", "/v1/admin/chat/reports", &moderator, None)
            .await;
        assert_eq!(v["reports"][0]["messages"][0]["text"], "something unkind");

        let (s, v) = f
            .call(
                "POST",
                &format!("/v1/admin/chat/reports/{id}/resolve"),
                &moderator,
                Some(serde_json::json!({ "outcome": "chat_restricted", "note": "harassment in DMs" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let r = f.restrictions.effective(mallory_id).await.unwrap().unwrap();
        assert!(r.blocks(Capability::Chat));
        assert!(r.blocks(Capability::Sharing), "the earlier limit survives");
        assert!(
            !f.room_store.is_member(ROOM, "mallory").await.unwrap(),
            "out of the room"
        );
        assert!(f
            .matrix
            .calls()
            .iter()
            .any(|c| c.starts_with("kick") && c.ends_with("@mallory:starstats.app")));

        let (s, v) = f
            .call(
                "POST",
                &format!("/v1/admin/chat/reports/{id}/resolve"),
                &moderator,
                Some(serde_json::json!({ "outcome": "dismissed" })),
            )
            .await;
        assert_eq!(
            (s, v["error"].as_str()),
            (StatusCode::CONFLICT, Some("already_resolved"))
        );
    }
}
