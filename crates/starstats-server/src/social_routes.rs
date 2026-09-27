//! Friends, blocks, mutes and the notifications inbox (social phase 1).
//!
//! The caller's handle is resolved from the token's `sub` through the
//! user store rather than read from `preferred_username`: a device
//! token (the tray) carries the device label there, not the handle.
//! Resolving through the store also means a renamed account acts under
//! its current handle, not the one baked into an older token.
//!
//! Privacy rules that shape the handlers:
//!   * A request to someone who blocked you is stored and reported back
//!     as `requested`, exactly like any other. It never reaches them.
//!     Telling the sender would make a block detectable.
//!   * Accepting, declining or cancelling someone else's request is a
//!     404, not a 403, so request ids cannot be probed.
//!   * Mutes only silence notifications; nothing else changes.
//!
//! Audit emission is best-effort, as everywhere in this server.

use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::{AuthenticatedUser, TokenType};
use crate::friend_sync;
use crate::notifications::{Notification, NotificationKind, NotificationStore, PAGE_LIMIT_MAX};
use crate::rsi_org_store::RsiOrgStore;
use crate::salutes::SaluteStore;
use crate::share_metadata::ShareMetadataStore;
use crate::social::{
    request_rate_limit_window, Friend, FriendRequest, FriendRequestPolicy, FriendRequestStatus,
    ListedHandle, SocialError, SocialStore, REQUEST_RATE_LIMIT_PER_WINDOW,
};
use crate::spicedb::SpicedbClient;
use crate::users::{validate_handle, User, UserStore};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/me/friends", get(list_friends))
        .route("/v1/me/friends/requests", post(send_request))
        .route("/v1/me/friends/requests/{id}/accept", post(accept_request))
        .route(
            "/v1/me/friends/requests/{id}/decline",
            post(decline_request),
        )
        .route("/v1/me/friends/requests/{id}/cancel", post(cancel_request))
        .route(
            "/v1/me/friends/{handle}",
            axum::routing::delete(remove_friend),
        )
        .route("/v1/me/blocks", get(list_blocks))
        .route(
            "/v1/me/blocks/{handle}",
            put(block_user).delete(unblock_user),
        )
        .route("/v1/me/mutes", get(list_mutes))
        .route("/v1/me/mutes/{handle}", put(mute_user).delete(unmute_user))
        .route("/v1/me/social/settings", put(update_settings))
        .route("/v1/me/notifications", get(list_notifications))
        .route("/v1/me/notifications/read", post(mark_notifications_read))
}

// -- DTOs -------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct FriendsResponse {
    pub friends: Vec<Friend>,
    /// Pending requests addressed to the caller. Requests from users
    /// the caller has blocked are left out.
    pub incoming: Vec<FriendRequest>,
    /// Pending requests the caller has sent.
    pub outgoing: Vec<FriendRequest>,
    pub friend_request_policy: FriendRequestPolicy,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendFriendRequestBody {
    pub handle: String,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendFriendRequestOutcome {
    /// A pending request was created.
    Requested,
    /// The other user had already asked; both requests are settled and
    /// the two are now friends.
    BecameFriends,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SendFriendRequestResponse {
    pub outcome: SendFriendRequestOutcome,
    /// The caller's request, when one was created.
    pub request: Option<FriendRequest>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FriendRequestResponse {
    pub request: FriendRequest,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BlocksResponse {
    pub blocks: Vec<ListedHandle>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MutesResponse {
    pub mutes: Vec<ListedHandle>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SocialSettings {
    pub friend_request_policy: FriendRequestPolicy,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct NotificationsQuery {
    /// Only notifications created strictly after this instant.
    pub since: Option<DateTime<Utc>>,
    /// Page size, 1 to 200. Default 50.
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NotificationsResponse {
    pub items: Vec<Notification>,
    /// Unread across the whole inbox, not just this page.
    pub unread_count: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkNotificationsReadBody {
    /// Notifications to mark read. Ignored when `all` is true.
    #[serde(default)]
    pub ids: Vec<Uuid>,
    /// Mark every unread notification read.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MarkNotificationsReadResponse {
    pub updated: u64,
    pub unread_count: i64,
}

// -- Helpers ----------------------------------------------------------

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

fn social_err(e: SocialError, context: &'static str) -> Response {
    match e {
        SocialError::NotFound => err(StatusCode::NOT_FOUND, "request_not_found"),
        SocialError::NotPending => err(StatusCode::CONFLICT, "request_not_pending"),
        SocialError::DuplicatePending => err(StatusCode::CONFLICT, "request_pending"),
        other => {
            tracing::error!(error = %other, context, "social store failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// The signed-in user behind the token, by id. Accepts user and device
/// tokens; refuses the short-lived TOTP interim token, which must only
/// ever reach the second-factor endpoint.
pub(crate) async fn caller(
    auth: &AuthenticatedUser,
    users: &dyn UserStore,
) -> Result<User, Response> {
    if !matches!(auth.token_type, TokenType::User | TokenType::Device) {
        return Err(err(StatusCode::UNAUTHORIZED, "user_token_required"));
    }
    let Ok(id) = Uuid::parse_str(&auth.sub) else {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid_subject"));
    };
    match users.find_by_id(id).await {
        Ok(Some(u)) => Ok(u),
        Ok(None) => Err(err(StatusCode::UNAUTHORIZED, "unknown_user")),
        Err(e) => {
            tracing::error!(error = %e, "caller lookup failed");
            Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal"))
        }
    }
}

/// Resolve another user by handle to their canonical record.
pub(crate) async fn target(handle: &str, users: &dyn UserStore) -> Result<User, Response> {
    let handle = handle.trim();
    if !validate_handle(handle) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_handle"));
    }
    match users.find_by_handle(handle).await {
        Ok(Some(u)) => Ok(u),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "user_not_found")),
        Err(e) => {
            tracing::error!(error = %e, "target lookup failed");
            Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal"))
        }
    }
}

/// Whether the signed-in `viewer` is blocked by `owner`. A block hides
/// the owner's profile from them the way a private profile is hidden,
/// so callers answer 404. An anonymous viewer is never blocked: a block
/// is between two accounts, and this is not a way to hide a public
/// profile from someone who signs out.
///
/// Fails CLOSED with 503: showing the profile to the person it was
/// hidden from is the one outcome this exists to prevent.
pub(crate) async fn blocked_by_owner(
    social: &dyn SocialStore,
    owner: &str,
    viewer: Option<&str>,
) -> Result<bool, Response> {
    let Some(viewer) = viewer else {
        return Ok(false);
    };
    if viewer.eq_ignore_ascii_case(owner) {
        return Ok(false);
    }
    social.is_blocked(owner, viewer).await.map_err(|e| {
        tracing::error!(error = %e, "block lookup failed (profile view); refusing to serve");
        err(StatusCode::SERVICE_UNAVAILABLE, "block_check_unavailable")
    })
}

/// Whether two users share an org, for the `org_mates` request policy:
/// an RSI org in both users' latest snapshots (by `sid`), or a
/// StarStats org in any role. RSI is checked first because it needs
/// only Postgres. `Err` means SpiceDB could not answer and no RSI org
/// matched, so the answer is unknown rather than no.
async fn share_an_org(
    a: &User,
    b: &User,
    spicedb: &Option<SpicedbClient>,
    rsi_orgs: &dyn RsiOrgStore,
) -> Result<bool, ()> {
    let sids = |snap: Option<crate::rsi_org_store::RsiOrgsSnapshot>| -> Vec<String> {
        snap.map(|s| {
            s.orgs
                .into_iter()
                .map(|o| o.sid.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
    };
    match (
        rsi_orgs.latest_for_user(a.id).await,
        rsi_orgs.latest_for_user(b.id).await,
    ) {
        (Ok(x), Ok(y)) => {
            let (x, y) = (sids(x), sids(y));
            if x.iter().any(|sid| y.contains(sid)) {
                return Ok(true);
            }
        }
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!(error = %e, "org_mates: rsi org snapshot read failed")
        }
    }
    let Some(client) = spicedb.as_ref() else {
        return Err(());
    };
    match (
        client.list_orgs_for_user(&a.claimed_handle).await,
        client.list_orgs_for_user(&b.claimed_handle).await,
    ) {
        (Ok(x), Ok(y)) => Ok(x.iter().any(|slug| y.contains(slug))),
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!(error = %e, "org_mates: spicedb org lookup failed");
            Err(())
        }
    }
}

async fn audit_best_effort(
    audit: &dyn AuditLog,
    me: &User,
    action: &'static str,
    payload: serde_json::Value,
) {
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(me.id.to_string()),
            actor_handle: Some(me.claimed_handle.clone()),
            action: action.to_string(),
            payload,
        })
        .await
    {
        tracing::warn!(error = %e, action, "audit log append failed");
    }
}

/// Deliver a notification unless the recipient has blocked or muted
/// the actor. Best-effort: a failed notification must not undo the
/// action that caused it.
pub(crate) async fn notify(
    social: &dyn SocialStore,
    notes: &dyn NotificationStore,
    recipient: &str,
    actor: &User,
    kind: NotificationKind,
    payload: serde_json::Value,
) {
    let actor_handle = actor.claimed_handle.as_str();
    let silenced = match (
        social.is_blocked(recipient, actor_handle).await,
        social.is_muted(recipient, actor_handle).await,
    ) {
        (Ok(b), Ok(m)) => b || m,
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!(error = %e, "notify: block/mute check failed; not notifying");
            true
        }
    };
    if silenced {
        return;
    }
    if let Err(e) = notes
        .create(recipient, kind, Some(actor_handle), payload)
        .await
    {
        tracing::warn!(error = %e, kind = kind.as_str(), "notification create failed");
    }
}

// -- Friends ----------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/me/friends",
    tag = "social",
    operation_id = "social_list_friends",
    responses(
        (status = 200, description = "Friends and pending requests", body = FriendsResponse),
        (status = 401, description = "Not signed in", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn list_friends(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let h = me.claimed_handle.as_str();
    let (friends, incoming, outgoing, policy) = match (
        social.list_friends(h).await,
        social.list_incoming(h).await,
        social.list_outgoing(h).await,
        social.get_policy(h).await,
    ) {
        (Ok(f), Ok(i), Ok(o), Ok(p)) => (f, i, o, p),
        (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => {
            return social_err(e, "list_friends")
        }
    };
    Json(FriendsResponse {
        friends,
        incoming,
        outgoing,
        friend_request_policy: policy,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/me/friends/requests",
    tag = "social",
    operation_id = "social_send_friend_request",
    request_body = SendFriendRequestBody,
    responses(
        (status = 200, description = "Request sent, or already reciprocated", body = SendFriendRequestResponse),
        (status = 400, description = "Invalid handle, or your own handle", body = ApiErrorBody),
        (status = 403, description = "That user is not accepting requests", body = ApiErrorBody),
        (status = 404, description = "No such user", body = ApiErrorBody),
        (status = 409, description = "Already friends, request pending, or you have blocked them", body = ApiErrorBody),
        (status = 429, description = "Too many requests sent recently", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn send_request(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(rsi_orgs): Extension<Arc<dyn RsiOrgStore>>,
    Json(body): Json<SendFriendRequestBody>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let them = match target(&body.handle, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if them.id == me.id {
        return err(StatusCode::BAD_REQUEST, "cannot_friend_self");
    }
    let (mine, theirs) = (me.claimed_handle.as_str(), them.claimed_handle.as_str());

    match social.are_friends(mine, theirs).await {
        Ok(true) => return err(StatusCode::CONFLICT, "already_friends"),
        Ok(false) => {}
        Err(e) => return social_err(e, "send_request.are_friends"),
    }
    match social.is_blocked(mine, theirs).await {
        Ok(true) => return err(StatusCode::CONFLICT, "you_blocked_user"),
        Ok(false) => {}
        Err(e) => return social_err(e, "send_request.caller_blocked"),
    }

    let since = Utc::now() - request_rate_limit_window();
    match social.count_recent_requests(mine, since).await {
        Ok(n) if n >= REQUEST_RATE_LIMIT_PER_WINDOW => {
            return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited")
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "friend request rate-limit count failed; skipping gate")
        }
    }

    // The policy is checked before the block, and a block is then
    // treated exactly like acceptance, so a blocked sender sees what a
    // stranger would see.
    match social.get_policy(theirs).await {
        Ok(FriendRequestPolicy::Nobody) => {
            return err(StatusCode::FORBIDDEN, "not_accepting_requests")
        }
        Ok(FriendRequestPolicy::OrgMates) => {
            match share_an_org(&me, &them, spicedb.as_ref(), rsi_orgs.as_ref()).await {
                Ok(true) => {}
                Ok(false) => return err(StatusCode::FORBIDDEN, "not_accepting_requests"),
                Err(()) => return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable"),
            }
        }
        Ok(FriendRequestPolicy::Everyone) => {}
        Err(e) => return social_err(e, "send_request.policy"),
    }
    let they_blocked_me = match social.is_blocked(theirs, mine).await {
        Ok(b) => b,
        Err(e) => return social_err(e, "send_request.target_blocked"),
    };

    // They already asked us: settle both and become friends. A pending
    // reverse request cannot exist alongside their block, because a
    // block cancels pending requests both ways.
    if !they_blocked_me {
        match social.find_pending(theirs, mine).await {
            Ok(Some(reverse)) => {
                if let Err(e) = social
                    .resolve_request(reverse.id, FriendRequestStatus::Accepted)
                    .await
                {
                    return social_err(e, "send_request.accept_reverse");
                }
                if let Err(e) = social.add_friendship(mine, theirs).await {
                    return social_err(e, "send_request.add_friendship");
                }
                friend_sync::apply(spicedb.as_ref(), mine, theirs, true).await;
                notify(
                    social.as_ref(),
                    notes.as_ref(),
                    theirs,
                    &me,
                    NotificationKind::FriendAccepted,
                    serde_json::json!({ "rsi_verified": me.rsi_verified_at.is_some() }),
                )
                .await;
                audit_best_effort(
                    audit.as_ref(),
                    &me,
                    "friend.accepted",
                    serde_json::json!({ "request_id": reverse.id, "other_handle": theirs }),
                )
                .await;
                return Json(SendFriendRequestResponse {
                    outcome: SendFriendRequestOutcome::BecameFriends,
                    request: None,
                })
                .into_response();
            }
            Ok(None) => {}
            Err(e) => return social_err(e, "send_request.find_reverse"),
        }
    }

    let request = match social.create_request(mine, theirs).await {
        Ok(r) => r,
        Err(e) => return social_err(e, "send_request.create"),
    };
    if !they_blocked_me {
        notify(
            social.as_ref(),
            notes.as_ref(),
            theirs,
            &me,
            NotificationKind::FriendRequest,
            serde_json::json!({
                "request_id": request.id,
                "rsi_verified": me.rsi_verified_at.is_some(),
            }),
        )
        .await;
    }
    audit_best_effort(
        audit.as_ref(),
        &me,
        "friend.requested",
        serde_json::json!({ "request_id": request.id, "other_handle": theirs }),
    )
    .await;
    Json(SendFriendRequestResponse {
        outcome: SendFriendRequestOutcome::Requested,
        request: Some(request),
    })
    .into_response()
}

/// Load a pending-or-not request and check the caller is on the side
/// allowed to act on it. Anything else is a 404.
async fn request_for(
    social: &dyn SocialStore,
    id: Uuid,
    me: &User,
    as_recipient: bool,
) -> Result<FriendRequest, Response> {
    let req = match social.get_request(id).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, "request_not_found")),
        Err(e) => return Err(social_err(e, "request_for")),
    };
    let side = if as_recipient {
        &req.recipient_handle
    } else {
        &req.requester_handle
    };
    if !side.eq_ignore_ascii_case(&me.claimed_handle) {
        return Err(err(StatusCode::NOT_FOUND, "request_not_found"));
    }
    // A request from someone the recipient blocked is hidden from them,
    // so it cannot be acted on either.
    if as_recipient {
        match social
            .is_blocked(&me.claimed_handle, &req.requester_handle)
            .await
        {
            Ok(true) => return Err(err(StatusCode::NOT_FOUND, "request_not_found")),
            Ok(false) => {}
            Err(e) => return Err(social_err(e, "request_for.blocked")),
        }
    }
    Ok(req)
}

#[utoipa::path(
    post,
    path = "/v1/me/friends/requests/{id}/accept",
    tag = "social",
    operation_id = "social_accept_friend_request",
    params(("id" = Uuid, Path, description = "Friend request id")),
    responses(
        (status = 200, description = "Accepted; the two are now friends", body = FriendRequestResponse),
        (status = 404, description = "No such request addressed to you", body = ApiErrorBody),
        (status = 409, description = "Request is no longer pending", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn accept_request(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Path(id): Path<Uuid>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req = match request_for(social.as_ref(), id, &me, true).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let req = match social
        .resolve_request(req.id, FriendRequestStatus::Accepted)
        .await
    {
        Ok(r) => r,
        Err(e) => return social_err(e, "accept.resolve"),
    };
    if let Err(e) = social
        .add_friendship(&req.requester_handle, &req.recipient_handle)
        .await
    {
        return social_err(e, "accept.add_friendship");
    }
    friend_sync::apply(
        spicedb.as_ref(),
        &req.requester_handle,
        &req.recipient_handle,
        true,
    )
    .await;
    notify(
        social.as_ref(),
        notes.as_ref(),
        &req.requester_handle,
        &me,
        NotificationKind::FriendAccepted,
        serde_json::json!({ "rsi_verified": me.rsi_verified_at.is_some() }),
    )
    .await;
    audit_best_effort(
        audit.as_ref(),
        &me,
        "friend.accepted",
        serde_json::json!({ "request_id": req.id, "other_handle": req.requester_handle }),
    )
    .await;
    Json(FriendRequestResponse { request: req }).into_response()
}

#[allow(clippy::too_many_arguments)]
async fn resolve_quietly(
    auth: AuthenticatedUser,
    users: Arc<dyn UserStore>,
    social: Arc<dyn SocialStore>,
    audit: Arc<dyn AuditLog>,
    id: Uuid,
    as_recipient: bool,
    to: FriendRequestStatus,
    action: &'static str,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let req = match request_for(social.as_ref(), id, &me, as_recipient).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let req = match social.resolve_request(req.id, to).await {
        Ok(r) => r,
        Err(e) => return social_err(e, action),
    };
    audit_best_effort(
        audit.as_ref(),
        &me,
        action,
        serde_json::json!({ "request_id": req.id }),
    )
    .await;
    Json(FriendRequestResponse { request: req }).into_response()
}

/// Declining is silent: the sender is not told, and simply sees the
/// request disappear from their pending list.
#[utoipa::path(
    post,
    path = "/v1/me/friends/requests/{id}/decline",
    tag = "social",
    operation_id = "social_decline_friend_request",
    params(("id" = Uuid, Path, description = "Friend request id")),
    responses(
        (status = 200, description = "Declined", body = FriendRequestResponse),
        (status = 404, description = "No such request addressed to you", body = ApiErrorBody),
        (status = 409, description = "Request is no longer pending", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn decline_request(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
) -> Response {
    resolve_quietly(
        auth,
        users,
        social,
        audit,
        id,
        true,
        FriendRequestStatus::Declined,
        "friend.declined",
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/me/friends/requests/{id}/cancel",
    tag = "social",
    operation_id = "social_cancel_friend_request",
    params(("id" = Uuid, Path, description = "Friend request id")),
    responses(
        (status = 200, description = "Cancelled", body = FriendRequestResponse),
        (status = 404, description = "No such request sent by you", body = ApiErrorBody),
        (status = 409, description = "Request is no longer pending", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn cancel_request(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
) -> Response {
    resolve_quietly(
        auth,
        users,
        social,
        audit,
        id,
        false,
        FriendRequestStatus::Cancelled,
        "friend.request_cancelled",
    )
    .await
}

#[utoipa::path(
    delete,
    path = "/v1/me/friends/{handle}",
    tag = "social",
    operation_id = "social_remove_friend",
    params(("handle" = String, Path, description = "Friend's handle")),
    responses(
        (status = 204, description = "No longer friends"),
        (status = 400, description = "Invalid handle", body = ApiErrorBody),
        (status = 404, description = "Not friends with that user", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn remove_friend(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let handle = handle.trim();
    if !validate_handle(handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    match social.remove_friendship(&me.claimed_handle, handle).await {
        Ok(true) => {
            // SpiceDB ids are case-sensitive: delete under the spelling
            // the tuples were written with, not the one in the URL.
            let theirs = match users.find_by_handle(handle).await {
                Ok(Some(u)) => u.claimed_handle,
                _ => handle.to_string(),
            };
            friend_sync::apply(spicedb.as_ref(), &me.claimed_handle, &theirs, false).await;
            audit_best_effort(
                audit.as_ref(),
                &me,
                "friend.removed",
                serde_json::json!({ "other_handle": handle }),
            )
            .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => err(StatusCode::NOT_FOUND, "not_friends"),
        Err(e) => social_err(e, "remove_friend"),
    }
}

// -- Blocks -----------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/me/blocks",
    tag = "social",
    operation_id = "social_list_blocks",
    responses((status = 200, description = "Users you have blocked", body = BlocksResponse)),
    security(("bearer" = [])),
)]
pub async fn list_blocks(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match social.list_blocks(&me.claimed_handle).await {
        Ok(blocks) => Json(BlocksResponse { blocks }).into_response(),
        Err(e) => social_err(e, "list_blocks"),
    }
}

/// Block a user. Removes any friendship, cancels pending requests in
/// both directions, clears their notifications from your inbox and
/// revokes a direct stats share you had given them. Their future
/// requests are hidden from you and they are not told.
#[utoipa::path(
    put,
    path = "/v1/me/blocks/{handle}",
    tag = "social",
    operation_id = "social_block_user",
    params(("handle" = String, Path, description = "Handle to block")),
    responses(
        (status = 204, description = "Blocked"),
        (status = 400, description = "Invalid handle, or your own handle", body = ApiErrorBody),
        (status = 404, description = "No such user", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn block_user(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
    Extension(salutes): Extension<Arc<dyn SaluteStore>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let them = match target(&handle, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if them.id == me.id {
        return err(StatusCode::BAD_REQUEST, "cannot_block_self");
    }
    let (mine, theirs) = (me.claimed_handle.as_str(), them.claimed_handle.as_str());

    // The block row is the one write that must land; everything after
    // it is cleanup, and the block alone already hides them.
    if let Err(e) = social.block(mine, theirs).await {
        return social_err(e, "block");
    }
    if let Err(e) = social.remove_friendship(mine, theirs).await {
        tracing::warn!(error = %e, "block: remove_friendship failed");
    }
    // Removes their view of a friends share. Unconditional: a failed
    // remove_friendship above must not leave the tuples behind.
    friend_sync::apply(spicedb.as_ref(), mine, theirs, false).await;
    // Their salute on my profile goes, and mine on theirs.
    if let Err(e) = salutes.delete_between(mine, theirs).await {
        tracing::warn!(error = %e, "block: salute delete failed");
    }
    if let Err(e) = social.cancel_pending_between(mine, theirs).await {
        tracing::warn!(error = %e, "block: cancel_pending_between failed");
    }
    if let Err(e) = notes.delete_from_actor(mine, theirs).await {
        tracing::warn!(error = %e, "block: delete_from_actor failed");
    }
    // Revoke the stats share I gave them, if any. SpiceDB being down
    // must not stop the block; the share can still be removed from
    // /sharing, and the warning names the pair's first half only.
    if let Some(client) = spicedb.as_ref() {
        match client.delete_share_with_user(mine, theirs).await {
            Ok(()) => {
                if let Err(e) = meta.delete(mine, theirs).await {
                    tracing::warn!(error = %e, "block: share_metadata delete failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "block: share revoke failed"),
        }
    }
    audit_best_effort(
        audit.as_ref(),
        &me,
        "user.blocked",
        serde_json::json!({ "other_handle": theirs }),
    )
    .await;
    StatusCode::NO_CONTENT.into_response()
}

#[utoipa::path(
    delete,
    path = "/v1/me/blocks/{handle}",
    tag = "social",
    operation_id = "social_unblock_user",
    params(("handle" = String, Path, description = "Handle to unblock")),
    responses(
        (status = 204, description = "Unblocked"),
        (status = 400, description = "Invalid handle", body = ApiErrorBody),
        (status = 404, description = "That user is not blocked", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn unblock_user(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let handle = handle.trim();
    if !validate_handle(handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    match social.unblock(&me.claimed_handle, handle).await {
        Ok(true) => {
            audit_best_effort(
                audit.as_ref(),
                &me,
                "user.unblocked",
                serde_json::json!({ "other_handle": handle }),
            )
            .await;
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => err(StatusCode::NOT_FOUND, "not_blocked"),
        Err(e) => social_err(e, "unblock"),
    }
}

// -- Mutes ------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/me/mutes",
    tag = "social",
    operation_id = "social_list_mutes",
    responses((status = 200, description = "Users you have muted", body = MutesResponse)),
    security(("bearer" = [])),
)]
pub async fn list_mutes(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match social.list_mutes(&me.claimed_handle).await {
        Ok(mutes) => Json(MutesResponse { mutes }).into_response(),
        Err(e) => social_err(e, "list_mutes"),
    }
}

/// Mute a user: their friend requests and other activity stop
/// producing notifications for you. Nothing else changes.
#[utoipa::path(
    put,
    path = "/v1/me/mutes/{handle}",
    tag = "social",
    operation_id = "social_mute_user",
    params(("handle" = String, Path, description = "Handle to mute")),
    responses(
        (status = 204, description = "Muted"),
        (status = 400, description = "Invalid handle, or your own handle", body = ApiErrorBody),
        (status = 404, description = "No such user", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn mute_user(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let them = match target(&handle, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if them.id == me.id {
        return err(StatusCode::BAD_REQUEST, "cannot_mute_self");
    }
    match social.mute(&me.claimed_handle, &them.claimed_handle).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => social_err(e, "mute"),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/me/mutes/{handle}",
    tag = "social",
    operation_id = "social_unmute_user",
    params(("handle" = String, Path, description = "Handle to unmute")),
    responses(
        (status = 204, description = "Unmuted"),
        (status = 400, description = "Invalid handle", body = ApiErrorBody),
        (status = 404, description = "That user is not muted", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn unmute_user(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let handle = handle.trim();
    if !validate_handle(handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    match social.unmute(&me.claimed_handle, handle).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "not_muted"),
        Err(e) => social_err(e, "unmute"),
    }
}

// -- Settings ---------------------------------------------------------

#[utoipa::path(
    put,
    path = "/v1/me/social/settings",
    tag = "social",
    operation_id = "social_update_settings",
    request_body = SocialSettings,
    responses(
        (status = 200, description = "Settings as stored", body = SocialSettings),
        (status = 422, description = "Unknown policy value"),
    ),
    security(("bearer" = [])),
)]
pub async fn update_settings(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Json(body): Json<SocialSettings>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Err(e) = social
        .set_policy(&me.claimed_handle, body.friend_request_policy)
        .await
    {
        return social_err(e, "update_settings");
    }
    // Read back so the response reflects what was stored, not what was
    // asked for.
    match social.get_policy(&me.claimed_handle).await {
        Ok(p) => Json(SocialSettings {
            friend_request_policy: p,
        })
        .into_response(),
        Err(e) => social_err(e, "update_settings.read_back"),
    }
}

// -- Notifications ----------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/me/notifications",
    tag = "social",
    operation_id = "social_list_notifications",
    params(NotificationsQuery),
    responses(
        (status = 200, description = "Newest first, plus the inbox unread count", body = NotificationsResponse),
        (status = 401, description = "Not signed in", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn list_notifications(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Query(q): Query<NotificationsQuery>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let limit = q.limit.unwrap_or(50).clamp(1, PAGE_LIMIT_MAX);
    match (
        notes.list(&me.claimed_handle, q.since, limit).await,
        notes.unread_count(&me.claimed_handle).await,
    ) {
        (Ok(items), Ok(unread_count)) => Json(NotificationsResponse {
            items,
            unread_count,
        })
        .into_response(),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(error = %e, "list_notifications failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

#[utoipa::path(
    post,
    path = "/v1/me/notifications/read",
    tag = "social",
    operation_id = "social_mark_notifications_read",
    request_body = MarkNotificationsReadBody,
    responses(
        (status = 200, description = "Rows updated and the new unread count", body = MarkNotificationsReadResponse),
        (status = 400, description = "Neither ids nor all given", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn mark_notifications_read(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Json(body): Json<MarkNotificationsReadBody>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if !body.all && body.ids.is_empty() {
        return err(StatusCode::BAD_REQUEST, "nothing_to_mark");
    }
    if body.ids.len() > PAGE_LIMIT_MAX as usize {
        return err(StatusCode::BAD_REQUEST, "too_many_ids");
    }
    let updated = if body.all {
        notes.mark_all_read(&me.claimed_handle).await
    } else {
        notes.mark_read(&me.claimed_handle, &body.ids).await
    };
    match (updated, notes.unread_count(&me.claimed_handle).await) {
        (Ok(updated), Ok(unread_count)) => Json(MarkNotificationsReadResponse {
            updated,
            unread_count,
        })
        .into_response(),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(error = %e, "mark_notifications_read failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_restrictions::test_support::MemoryAccountRestrictionStore;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::TokenIssuer;
    use crate::devices::test_support::MemoryDeviceStore;
    use crate::devices::DeviceStore;
    use crate::notifications::test_support::MemoryNotificationStore;
    use crate::rsi_org_store::test_support::MemoryRsiOrgStore;
    use crate::rsi_verify::RsiOrg;
    use crate::salutes::test_support::MemorySaluteStore;
    use crate::share_metadata::test_support::MemoryShareMetadataStore;
    use crate::social::test_support::MemorySocialStore;
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use chrono::Duration as ChronoDuration;
    use tower::ServiceExt;

    struct Fixture {
        app: Router,
        issuer: TokenIssuer,
        users: Arc<MemoryUserStore>,
        social: Arc<MemorySocialStore>,
        notes: Arc<MemoryNotificationStore>,
        devices: Arc<MemoryDeviceStore>,
        rsi_orgs: Arc<MemoryRsiOrgStore>,
        salutes: Arc<MemorySaluteStore>,
    }

    fn fixture() -> Fixture {
        fixture_with(None)
    }

    fn fixture_with(spicedb: Option<SpicedbClient>) -> Fixture {
        let users = Arc::new(MemoryUserStore::new());
        let social = Arc::new(MemorySocialStore::new());
        let notes = Arc::new(MemoryNotificationStore::new());
        let devices = Arc::new(MemoryDeviceStore::new());
        let (issuer, verifier) = fresh_pair();
        let users_dyn: Arc<dyn UserStore> = users.clone();
        let social_dyn: Arc<dyn SocialStore> = social.clone();
        let notes_dyn: Arc<dyn NotificationStore> = notes.clone();
        let devices_dyn: Arc<dyn DeviceStore> = devices.clone();
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let meta: Arc<dyn ShareMetadataStore> = Arc::new(MemoryShareMetadataStore::default());
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(spicedb);
        let rsi_orgs = Arc::new(MemoryRsiOrgStore::new());
        let rsi_orgs_dyn: Arc<dyn RsiOrgStore> = rsi_orgs.clone();
        let salutes = Arc::new(MemorySaluteStore::new());
        let salutes_dyn: Arc<dyn SaluteStore> = salutes.clone();
        let app = routes()
            .merge(crate::salute_routes::routes())
            .layer(Extension(salutes_dyn))
            .layer(Extension(
                Arc::new(crate::salutes::SaluteRateLimiter::new()),
            ))
            .layer(Extension(Arc::new(MemoryAccountRestrictionStore::new())
                as Arc<
                    dyn crate::account_restrictions::AccountRestrictionStore,
                >))
            .layer(Extension(users_dyn))
            .layer(Extension(rsi_orgs_dyn))
            .layer(Extension(social_dyn))
            .layer(Extension(notes_dyn))
            .layer(Extension(audit))
            .layer(Extension(meta))
            .layer(Extension(spicedb))
            .layer(Extension(devices_dyn))
            .layer(Extension(Arc::new(verifier)));
        Fixture {
            app,
            issuer,
            users,
            social,
            notes,
            devices,
            rsi_orgs,
            salutes,
        }
    }

    impl Fixture {
        /// Create a user and return a user token for them.
        async fn user(&self, handle: &str, verified: bool) -> String {
            let phc = hash_password("password-123-abcdef").unwrap();
            let u = self
                .users
                .create(&format!("{handle}@example.com"), &phc, handle)
                .await
                .unwrap();
            if verified {
                self.users.mark_rsi_verified(u.id).await.unwrap();
                self.social.mark_verified(handle);
            }
            self.issuer
                .sign_user(&u.id.to_string(), handle)
                .expect("sign user token")
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
            let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, v)
        }

        async fn request(&self, token: &str, to: &str) -> (StatusCode, serde_json::Value) {
            self.call(
                "POST",
                "/v1/me/friends/requests",
                token,
                Some(serde_json::json!({ "handle": to })),
            )
            .await
        }
    }

    #[tokio::test]
    async fn request_then_accept_makes_friends_both_ways_and_notifies() {
        let f = fixture();
        let alice = f.user("Alice", true).await;
        let bob = f.user("Bob", true).await;

        // Lower-case input resolves to the canonical handle.
        let (s, v) = f.request(&alice, "bob").await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["outcome"], "requested");
        assert_eq!(v["request"]["recipient_handle"], "Bob");
        let id = v["request"]["id"].as_str().unwrap().to_string();

        let bob_inbox = f.notes.all_for("Bob");
        assert_eq!(bob_inbox.len(), 1);
        assert_eq!(bob_inbox[0].kind, NotificationKind::FriendRequest);
        assert_eq!(bob_inbox[0].payload["request_id"], id.as_str());
        assert_eq!(bob_inbox[0].payload["rsi_verified"], true);

        let (_, v) = f.call("GET", "/v1/me/friends", &bob, None).await;
        assert_eq!(v["incoming"].as_array().unwrap().len(), 1);

        let accept = format!("/v1/me/friends/requests/{id}/accept");
        let (s, _) = f.call("POST", &accept, &bob, None).await;
        assert_eq!(s, StatusCode::OK);

        let (_, a) = f.call("GET", "/v1/me/friends", &alice, None).await;
        assert_eq!(a["friends"][0]["handle"], "Bob");
        assert_eq!(a["friends"][0]["rsi_verified"], true);
        assert!(a["outgoing"].as_array().unwrap().is_empty());
        let (_, b) = f.call("GET", "/v1/me/friends", &bob, None).await;
        assert_eq!(b["friends"][0]["handle"], "Alice");

        let alice_inbox = f.notes.all_for("Alice");
        assert_eq!(alice_inbox.len(), 1);
        assert_eq!(alice_inbox[0].kind, NotificationKind::FriendAccepted);

        // A second accept is a conflict, not a second friendship.
        let (s, v) = f.call("POST", &accept, &bob, None).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "request_not_pending");

        let (s, v) = f.request(&alice, "Bob").await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "already_friends");
    }

    #[tokio::test]
    async fn crossing_requests_become_friends() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        f.request(&alice, "bob").await;
        let (s, v) = f.request(&bob, "alice").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["outcome"], "became_friends");
        assert!(f.social.are_friends("alice", "bob").await.unwrap());
        let (_, a) = f.call("GET", "/v1/me/friends", &alice, None).await;
        assert!(a["outgoing"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_the_recipient_can_accept_and_others_get_404() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        f.user("bob", false).await;
        let mallory = f.user("mallory", false).await;
        let (_, v) = f.request(&alice, "bob").await;
        let id = v["request"]["id"].as_str().unwrap().to_string();
        let accept = format!("/v1/me/friends/requests/{id}/accept");
        let cancel = format!("/v1/me/friends/requests/{id}/cancel");

        for who in [&alice, &mallory] {
            let (s, v) = f.call("POST", &accept, who, None).await;
            assert_eq!(s, StatusCode::NOT_FOUND);
            assert_eq!(v["error"], "request_not_found");
        }
        // The sender can cancel their own request; a stranger cannot.
        let (s, _) = f.call("POST", &cancel, &mallory, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, v) = f.call("POST", &cancel, &alice, None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["request"]["status"], "cancelled");
    }

    #[tokio::test]
    async fn a_blocked_sender_sees_success_but_nothing_arrives() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let troll = f.user("troll", false).await;
        let (s, _) = f.call("PUT", "/v1/me/blocks/troll", &alice, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);

        let (s, v) = f.request(&troll, "alice").await;
        assert_eq!(s, StatusCode::OK, "a block must not be detectable");
        assert_eq!(v["outcome"], "requested");
        assert!(f.notes.all_for("alice").is_empty());
        let (_, a) = f.call("GET", "/v1/me/friends", &alice, None).await;
        assert!(a["incoming"].as_array().unwrap().is_empty());

        // And the hidden request cannot be accepted by id.
        let id = v["request"]["id"].as_str().unwrap().to_string();
        let accept = format!("/v1/me/friends/requests/{id}/accept");
        let (s, _) = f.call("POST", &accept, &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert!(!f.social.are_friends("alice", "troll").await.unwrap());
    }

    #[tokio::test]
    async fn block_unfriends_cancels_requests_and_clears_their_notifications() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        f.social.add_friendship("alice", "bob").await.unwrap();
        // A stale request plus the notification it produced.
        f.social.create_request("bob", "alice").await.unwrap();
        f.notes
            .create(
                "alice",
                NotificationKind::FriendRequest,
                Some("bob"),
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let (s, _) = f.call("PUT", "/v1/me/blocks/BOB", &alice, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        assert!(!f.social.are_friends("alice", "bob").await.unwrap());
        assert!(f
            .social
            .find_pending("bob", "alice")
            .await
            .unwrap()
            .is_none());
        assert!(f.notes.all_for("alice").is_empty());

        // The blocker cannot send them a request without unblocking.
        let (s, v) = f.request(&alice, "bob").await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "you_blocked_user");

        let (_, v) = f.call("GET", "/v1/me/blocks", &alice, None).await;
        assert_eq!(v["blocks"][0]["handle"], "bob");
        let (s, _) = f.call("DELETE", "/v1/me/blocks/bob", &alice, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (s, _) = f.call("DELETE", "/v1/me/blocks/bob", &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // Once unblocked, bob can ask again.
        let (s, _) = f.request(&bob, "alice").await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn mute_silences_notifications_but_not_the_request() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        let (s, _) = f.call("PUT", "/v1/me/mutes/bob", &alice, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (s, _) = f.request(&bob, "alice").await;
        assert_eq!(s, StatusCode::OK);
        assert!(f.notes.all_for("alice").is_empty());
        let (_, a) = f.call("GET", "/v1/me/friends", &alice, None).await;
        assert_eq!(a["incoming"].as_array().unwrap().len(), 1);
        let (_, v) = f.call("GET", "/v1/me/mutes", &alice, None).await;
        assert_eq!(v["mutes"][0]["handle"], "bob");
    }

    // -- Salutes ---------------------------------------------------------

    #[tokio::test]
    async fn salute_gates_come_before_any_profile_lookup() {
        let f = fixture();
        let unverified = f.user("alice", false).await;
        let bob = f.user("bob", true).await;
        f.user("carol", true).await;

        // The count is public, so an unverified account cannot add to it.
        let (s, v) = f.call("PUT", "/v1/u/carol/salute", &unverified, None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "rsi_handle_not_verified");

        let (s, v) = f.call("PUT", "/v1/u/BOB/salute", &bob, None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "cannot_salute_self");

        // An unknown handle and a hidden profile answer alike.
        let (s, v) = f.call("PUT", "/v1/u/ghost/salute", &bob, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "not_found");

        // Past the gates, visibility needs SpiceDB; without it, 503.
        let (s, v) = f.call("PUT", "/v1/u/carol/salute", &bob, None).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "spicedb_unavailable");
        assert_eq!(f.salutes.count("carol").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_blocked_viewer_sees_no_salutes_and_cannot_salute() {
        let f = fixture();
        let alice = f.user("alice", true).await;
        let bob = f.user("bob", true).await;
        f.salutes.salute("alice", "bob").await.unwrap();
        f.salutes.salute("bob", "alice").await.unwrap();

        let (s, _) = f.call("PUT", "/v1/me/blocks/alice", &bob, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        assert_eq!(f.salutes.count("bob").await.unwrap(), 0, "hers on his goes");
        assert_eq!(
            f.salutes.count("alice").await.unwrap(),
            0,
            "and his on hers"
        );

        // The same 404 as a stranger, before SpiceDB is ever asked.
        let (s, v) = f.call("PUT", "/v1/u/bob/salute", &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "not_found");
        let (s, _) = f.call("GET", "/v1/u/bob/salutes", &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn my_salutes_names_friends_only() {
        let f = fixture();
        let bob = f.user("bob", true).await;
        f.social.add_friendship("bob", "alice").await.unwrap();
        f.social.add_friendship("bob", "carol").await.unwrap();
        f.salutes.salute("alice", "bob").await.unwrap();
        f.salutes.salute("stranger", "bob").await.unwrap();

        let (s, v) = f.call("GET", "/v1/me/salutes", &bob, None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["count"], 2, "the count includes everyone");
        assert_eq!(
            v["friends"],
            serde_json::json!(["alice"]),
            "the names do not"
        );
    }

    #[tokio::test]
    async fn saluting_and_unsaluting_share_one_rate_limit() {
        let f = fixture();
        let alice = f.user("alice", true).await;
        let bursts = crate::salutes::SALUTE_BURST as usize;
        for _ in 0..bursts {
            let (s, _) = f.call("DELETE", "/v1/u/bob/salute", &alice, None).await;
            assert_eq!(s, StatusCode::OK);
        }
        let (s, v) = f.call("DELETE", "/v1/u/bob/salute", &alice, None).await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(v["error"], "rate_limited");
    }

    /// The whole flow against a real SpiceDB: a public profile can be
    /// saluted by anyone verified, a private one only by someone it is
    /// shared with, and only the first salute notifies. Skipped without
    /// one (see `spicedb::test_live`).
    #[tokio::test]
    async fn live_spicedb_salutes_follow_what_the_viewer_can_see() {
        let Some(c) = crate::spicedb::test_live::live_spicedb().await else {
            return;
        };
        let f = fixture_with(Some(c.clone()));
        let alice = f.user("Alice", true).await;
        f.user("Pub", true).await;
        f.user("Priv", true).await;
        f.user("Shared", true).await;
        c.write_public_view("Pub").await.unwrap();
        // Written before any check touches it: SpiceDB reuses a check
        // result for a few seconds, so checking a profile before and
        // after sharing it would read the old answer.
        c.write_share_with_friends("Shared").await.unwrap();
        c.write_friendship("Shared", "Alice").await.unwrap();

        let (s, v) = f.call("PUT", "/v1/u/pub/salute", &alice, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["count"], 1);
        assert_eq!(v["saluted_by_me"], true);
        let (s, _) = f.call("PUT", "/v1/u/Pub/salute", &alice, None).await;
        assert_eq!(s, StatusCode::OK);
        let salutes: Vec<_> = f
            .notes
            .all_for("Pub")
            .into_iter()
            .filter(|n| n.kind == NotificationKind::Salute)
            .collect();
        assert_eq!(salutes.len(), 1, "saluting twice notifies once");

        // Signed out, a public profile's count is still readable.
        let (s, v) = f.call("GET", "/v1/u/pub/salutes", "", None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["count"], 1);
        assert!(v.get("saluted_by_me").is_none());

        // A private profile is invisible; one shared with friends is not.
        let (s, _) = f.call("PUT", "/v1/u/priv/salute", &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = f.call("GET", "/v1/u/priv/salutes", "", None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, v) = f.call("PUT", "/v1/u/shared/salute", &alice, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let (s, v) = f.call("GET", "/v1/u/shared/salutes", &alice, None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["saluted_by_me"], true);
        let (s, _) = f.call("GET", "/v1/u/shared/salutes", "", None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "signed out, it is private again");
    }

    /// The routes keep SpiceDB's friend tuples in step: written on
    /// accept, deleted on unfriend and on block. Needs a real SpiceDB
    /// (see `spicedb::test_live`); skipped without one.
    #[tokio::test]
    async fn live_spicedb_friend_routes_write_and_delete_tuples() {
        let Some(c) = crate::spicedb::test_live::live_spicedb().await else {
            return;
        };
        let f = fixture_with(Some(c.clone()));
        let alice = f.user("Alice", false).await;
        let bob = f.user("Bob", false).await;
        let tuples = || async {
            let mut t = c.list_friend_tuples().await.unwrap();
            t.sort();
            t
        };
        let pair = vec![
            ("Alice".to_string(), "Bob".to_string()),
            ("Bob".to_string(), "Alice".to_string()),
        ];

        let (_, v) = f.request(&alice, "bob").await;
        assert!(tuples().await.is_empty(), "a request alone writes nothing");
        let id = v["request"]["id"].as_str().unwrap().to_string();
        let (s, _) = f
            .call(
                "POST",
                &format!("/v1/me/friends/requests/{id}/accept"),
                &bob,
                None,
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(
            tuples().await,
            pair,
            "accept writes both, spelled as claimed"
        );

        // Unfriend by a differently-cased handle still finds the tuples.
        let (s, _) = f.call("DELETE", "/v1/me/friends/BOB", &alice, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        assert!(tuples().await.is_empty(), "unfriend deletes both");

        // Crossing requests make friends too, then a block unmakes it.
        let (s, _) = f.request(&alice, "bob").await;
        assert_eq!(s, StatusCode::OK);
        let (s, v) = f.request(&bob, "alice").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["outcome"], "became_friends");
        assert_eq!(tuples().await, pair);
        let (s, _) = f.call("PUT", "/v1/me/blocks/alice", &bob, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        assert!(tuples().await.is_empty(), "block deletes both");
    }

    #[tokio::test]
    async fn policy_org_mates_admits_a_shared_rsi_org() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let carol = f.user("carol", false).await;
        let bob = f.user("bob", false).await;
        let (s, v) = f
            .call(
                "PUT",
                "/v1/me/social/settings",
                &bob,
                Some(serde_json::json!({ "friend_request_policy": "org_mates" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["friend_request_policy"], "org_mates");

        let org = |sid: &str| RsiOrg {
            sid: sid.into(),
            name: sid.into(),
            rank: None,
            is_main: false,
        };
        for (handle, sid) in [("bob", "MINERS"), ("alice", "miners"), ("carol", "HAULERS")] {
            let id = f.users.find_by_handle(handle).await.unwrap().unwrap().id;
            f.rsi_orgs.save(id, &[org(sid)]).await.unwrap();
        }

        // Alice shares Bob's RSI org (sids compare case-insensitively).
        let (s, _) = f.request(&alice, "bob").await;
        assert_eq!(s, StatusCode::OK);

        // Carol shares no RSI org, and with SpiceDB unreachable her
        // StarStats orgs cannot be checked: the answer is unknown, so
        // she is told to retry rather than told Bob refuses.
        let (s, v) = f.request(&carol, "bob").await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["error"], "spicedb_unavailable");
    }

    #[tokio::test]
    async fn policy_nobody_refuses_and_self_and_unknown_are_rejected() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        let (s, v) = f
            .call(
                "PUT",
                "/v1/me/social/settings",
                &bob,
                Some(serde_json::json!({ "friend_request_policy": "nobody" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["friend_request_policy"], "nobody");

        let (s, v) = f.request(&alice, "bob").await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "not_accepting_requests");

        let (s, v) = f.request(&alice, "ALICE").await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "cannot_friend_self");

        let (s, v) = f.request(&alice, "ghost").await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "user_not_found");

        let (s, v) = f.request(&alice, "bad$handle").await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid_handle");
    }

    #[tokio::test]
    async fn sending_is_rate_limited() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        for i in 0..REQUEST_RATE_LIMIT_PER_WINDOW {
            f.social
                .create_request("alice", &format!("u{i}"))
                .await
                .unwrap();
        }
        f.user("bob", false).await;
        let (s, v) = f.request(&alice, "bob").await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(v["error"], "rate_limited");
    }

    #[tokio::test]
    async fn device_tokens_act_as_the_owner_not_the_device_label() {
        let f = fixture();
        f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        let alice_user = f.users.find_by_handle("alice").await.unwrap().unwrap();
        let pairing = f
            .devices
            .create_pairing(alice_user.id, "Hangar PC", ChronoDuration::minutes(5))
            .await
            .unwrap();
        let redeemed = f.devices.redeem(&pairing.code).await.unwrap();
        let device = f
            .issuer
            .sign_device(
                &alice_user.id.to_string(),
                &redeemed.label,
                redeemed.device_id,
            )
            .unwrap();

        let (s, v) = f.request(&device, "bob").await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["request"]["requester_handle"], "alice");
        let (_, b) = f.call("GET", "/v1/me/friends", &bob, None).await;
        assert_eq!(b["incoming"][0]["requester_handle"], "alice");
    }

    #[tokio::test]
    async fn unfriend_removes_both_sides() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        f.social.add_friendship("alice", "bob").await.unwrap();
        let (s, _) = f.call("DELETE", "/v1/me/friends/Bob", &alice, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (_, b) = f.call("GET", "/v1/me/friends", &bob, None).await;
        assert!(b["friends"].as_array().unwrap().is_empty());
        let (s, v) = f.call("DELETE", "/v1/me/friends/bob", &alice, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "not_friends");
    }

    #[tokio::test]
    async fn notifications_list_since_and_mark_read() {
        let f = fixture();
        let alice = f.user("alice", false).await;
        let bob = f.user("bob", false).await;
        let carol = f.user("carol", false).await;
        f.request(&bob, "alice").await;
        f.request(&carol, "alice").await;

        let (s, v) = f.call("GET", "/v1/me/notifications", &alice, None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["unread_count"], 2);
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["actor_handle"], "carol", "newest first");
        let first = items[1]["id"].as_str().unwrap().to_string();
        let read = "/v1/me/notifications/read";

        // Another user cannot mark alice's notification read.
        let body = serde_json::json!({ "ids": [first] });
        let (_, v) = f.call("POST", read, &bob, Some(body.clone())).await;
        assert_eq!(v["updated"], 0);

        let (_, v) = f.call("POST", read, &alice, Some(body)).await;
        assert_eq!(v["updated"], 1);
        assert_eq!(v["unread_count"], 1);

        let (s, v) = f
            .call("POST", read, &alice, Some(serde_json::json!({})))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "nothing_to_mark");

        let (_, v) = f
            .call(
                "POST",
                read,
                &alice,
                Some(serde_json::json!({ "all": true })),
            )
            .await;
        assert_eq!(v["unread_count"], 0);

        // `Z` form keeps the timestamp free of a URL-unsafe `+`.
        let future = (Utc::now() + ChronoDuration::minutes(1)).format("%Y-%m-%dT%H:%M:%SZ");
        let uri = format!("/v1/me/notifications?since={future}");
        let (s, v) = f.call("GET", &uri, &alice, None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["items"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unauthenticated_is_rejected() {
        let f = fixture();
        let resp = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/me/friends")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.status().is_client_error());
    }
}
