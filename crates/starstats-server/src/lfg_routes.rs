//! Looking for Group routes (social phase 4). See `lfg.rs` for the model.
//!
//! The board is for signed-in players only: posts carry handles, and a
//! logged-out board would widen who sees them for no benefit to a player
//! looking for crew. Posting and asking to join need a verified RSI handle
//! and an account whose sharing is not restricted. A block, either way,
//! hides a post and refuses a request as if the post did not exist.
//!
//! Moderation: any player can report a post; a moderator resolves the
//! report by dismissing it, removing the post, or removing the post and
//! suspending the host.

use crate::account_restrictions::{AccountRestrictionStore, Restriction};
use crate::admin_routes::RequireModerator;
use crate::api_error::ApiErrorBody;
use crate::audit::{AuditEntry, AuditLog};
use crate::auth::AuthenticatedUser;
use crate::lfg::*;
use crate::notifications::{NotificationKind, NotificationStore};
use crate::restriction_guard::{RequireUnrestricted, Sharing};
use crate::social::SocialStore;
use crate::social_routes::{caller, notify};
use crate::users::{User, UserStore};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/lfg", get(list_posts).post(create_post))
        .route("/v1/lfg/options", get(options))
        .route("/v1/me/lfg/summary", get(my_summary))
        .route("/v1/lfg/{id}", get(get_post).delete(close_post))
        .route("/v1/lfg/{id}/join", post(join).delete(leave))
        .route("/v1/lfg/{id}/members/{handle}", put(respond))
        .route("/v1/lfg/{id}/report", post(report))
        .route("/v1/admin/lfg/reports", get(admin_list_reports))
        .route("/v1/admin/lfg/reports/{id}/resolve", post(admin_resolve))
}

// -- DTOs ------------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLfgPost {
    pub activity: LfgActivity,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub ship: Option<String>,
    pub crew_slots: i16,
    pub voice: LfgVoice,
    pub region: LfgRegion,
    #[serde(default)]
    pub note: Option<String>,
    /// Minutes until the post expires: 15 to 360, 120 if omitted.
    #[serde(default)]
    pub expires_in_minutes: Option<i64>,
}

/// A post as a player sees it on the board.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LfgPostView {
    #[serde(flatten)]
    pub post: LfgPost,
    /// Whether the host's RSI handle is verified.
    pub host_verified: bool,
    /// The caller's own standing on this post, if any.
    pub my_status: Option<MemberStatus>,
    pub is_host: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LfgListResponse {
    pub posts: Vec<LfgPostView>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LfgPostDetail {
    #[serde(flatten)]
    pub view: LfgPostView,
    /// The host sees everyone who asked; accepted crew see the crew; others
    /// see nobody.
    pub members: Vec<LfgMember>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LfgOptions {
    pub activities: Vec<LfgActivity>,
    pub systems: Vec<String>,
    pub voices: Vec<LfgVoice>,
    pub regions: Vec<LfgRegion>,
    pub crew_min: i16,
    pub crew_max: i16,
    pub expiry_default_minutes: i64,
    pub expiry_min_minutes: i64,
    pub expiry_max_minutes: i64,
}

/// What the Crew badge shows: players waiting on the caller's open post.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LfgSummary {
    /// Whether the caller has an open post.
    pub hosting: bool,
    /// Players who asked to join it and have not been answered.
    pub pending_requests: i64,
    /// The open post, so the tray can close it when the game exits.
    pub post_id: Option<Uuid>,
    /// When the open post was made. The tray leaves alone a post made
    /// after the game closed, which is a plan for later, not a leftover.
    pub opened_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    pub activity: Option<String>,
    pub system: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RespondBody {
    /// `accept`, `decline` or `remove`. Remove also stops them asking again.
    pub action: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportLfgPost {
    pub reason: LfgReportReason,
    #[serde(default)]
    pub details: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ReportsQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LfgReportList {
    pub reports: Vec<LfgReport>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveLfgReport {
    /// `dismissed`, `post_removed` or `user_suspended`.
    pub outcome: String,
    #[serde(default)]
    pub note: Option<String>,
}

// -- Helpers ---------------------------------------------------------------------

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

fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "not_found")
}

fn store_err(e: LfgError, call: &str) -> Response {
    match e {
        LfgError::NotFound => not_found(),
        LfgError::AlreadyMember => err(StatusCode::CONFLICT, "already_asked"),
        LfgError::Removed => err(StatusCode::FORBIDDEN, "removed_from_group"),
        LfgError::AlreadyResolved => err(StatusCode::CONFLICT, "already_resolved"),
        other => {
            tracing::error!(error = %other, call, "lfg store failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// A short single-line text field: trimmed, empty is none, too long is an
/// error, and control characters are refused.
fn text(field: &str, v: Option<String>, max: usize) -> Result<Option<String>, Response> {
    let Some(s) = v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if s.chars().count() > max {
        return Err(err(StatusCode::BAD_REQUEST, &format!("{field}_too_long")));
    }
    if s.chars().any(char::is_control) {
        return Err(err(StatusCode::BAD_REQUEST, &format!("{field}_invalid")));
    }
    Ok(Some(s))
}

/// Whether either player has blocked the other. Fails closed.
async fn blocked_either(social: &dyn SocialStore, a: &str, b: &str) -> Result<bool, Response> {
    match (social.is_blocked(a, b).await, social.is_blocked(b, a).await) {
        (Ok(x), Ok(y)) => Ok(x || y),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(error = %e, "lfg: block lookup failed");
            Err(err(
                StatusCode::SERVICE_UNAVAILABLE,
                "block_check_unavailable",
            ))
        }
    }
}

async fn view(
    users: &dyn UserStore,
    store: &dyn LfgStore,
    me: &User,
    post: LfgPost,
) -> LfgPostView {
    let host_verified = matches!(
        users.find_by_handle(&post.host_handle).await,
        Ok(Some(u)) if u.rsi_verified_at.is_some()
    );
    let is_host = post.host_handle.eq_ignore_ascii_case(&me.claimed_handle);
    let my_status = if is_host {
        None
    } else {
        store
            .member(post.id, &me.claimed_handle)
            .await
            .ok()
            .flatten()
            .map(|m| m.status)
    };
    LfgPostView {
        post,
        host_verified,
        my_status,
        is_host,
    }
}

/// The post, if it exists and neither side has blocked the other.
async fn visible_post(
    store: &dyn LfgStore,
    social: &dyn SocialStore,
    me: &User,
    id: Uuid,
) -> Result<LfgPost, Response> {
    let post = match store.get_post(id).await {
        Ok(Some(p)) if p.removed_at.is_none() => p,
        Ok(_) => return Err(not_found()),
        Err(e) => return Err(store_err(e, "get_post")),
    };
    if blocked_either(social, &post.host_handle, &me.claimed_handle).await? {
        return Err(not_found());
    }
    Ok(post)
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

// -- Player routes -----------------------------------------------------------------

/// What a post form offers: every vocabulary and limit, so the web and the
/// tray never hard-code them.
#[utoipa::path(
    get,
    path = "/v1/lfg/options",
    tag = "lfg",
    operation_id = "lfg_options",
    responses((status = 200, description = "Choices for a post", body = LfgOptions)),
)]
pub async fn options() -> Response {
    Json(LfgOptions {
        activities: LfgActivity::ALL.to_vec(),
        systems: starstats_core::location_classifier::known_systems()
            .into_iter()
            .map(str::to_string)
            .collect(),
        voices: LfgVoice::ALL.to_vec(),
        regions: LfgRegion::ALL.to_vec(),
        crew_min: CREW_MIN,
        crew_max: CREW_MAX,
        expiry_default_minutes: EXPIRY_DEFAULT_MINUTES,
        expiry_min_minutes: EXPIRY_MIN_MINUTES,
        expiry_max_minutes: EXPIRY_MAX_MINUTES,
    })
    .into_response()
}

/// The Crew badge: how many players are waiting on your open post. Cheap
/// enough for every page load (one post, its member rows).
#[utoipa::path(
    get,
    path = "/v1/me/lfg/summary",
    tag = "lfg",
    operation_id = "lfg_my_summary",
    responses((status = 200, description = "Your Looking for Group summary", body = LfgSummary)),
    security(("bearer" = [])),
)]
pub async fn my_summary(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let post = match store
        .open_post_for_host(&me.claimed_handle, Utc::now())
        .await
    {
        Ok(p) => p,
        Err(e) => return store_err(e, "open_post_for_host"),
    };
    let Some(post) = post else {
        return Json(LfgSummary {
            hosting: false,
            pending_requests: 0,
            post_id: None,
            opened_at: None,
        })
        .into_response();
    };
    match store.members(post.id).await {
        Ok(members) => Json(LfgSummary {
            hosting: true,
            pending_requests: members
                .iter()
                .filter(|m| m.status == MemberStatus::Requested)
                .count() as i64,
            post_id: Some(post.id),
            opened_at: Some(post.created_at),
        })
        .into_response(),
        Err(e) => store_err(e, "members"),
    }
}

/// Open posts, newest first. Posts from anyone you blocked, or who blocked
/// you, are left out.
#[utoipa::path(
    get,
    path = "/v1/lfg",
    tag = "lfg",
    operation_id = "lfg_list",
    params(ListQuery),
    responses((status = 200, description = "Open posts", body = LfgListResponse)),
    security(("bearer" = [])),
)]
pub async fn list_posts(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let activity = match q.activity.as_deref().filter(|s| !s.is_empty()) {
        None => None,
        Some(a) => match LfgActivity::parse(a) {
            Some(a) => Some(a),
            None => return err(StatusCode::BAD_REQUEST, "invalid_activity"),
        },
    };
    let posts = match store
        .list_open(
            Utc::now(),
            activity,
            q.system.as_deref().filter(|s| !s.is_empty()),
            200,
        )
        .await
    {
        Ok(p) => p,
        Err(e) => return store_err(e, "list_open"),
    };
    let mut out = Vec::with_capacity(posts.len());
    for p in posts {
        match blocked_either(social.as_ref(), &p.host_handle, &me.claimed_handle).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(r) => return r,
        }
        out.push(view(users.as_ref(), store.as_ref(), &me, p).await);
    }
    Json(LfgListResponse { posts: out }).into_response()
}

/// Post a call for crew. One open post per host; a verified RSI handle is
/// needed, because the board is global and a handle is what players copy
/// into the game.
#[utoipa::path(
    post,
    path = "/v1/lfg",
    tag = "lfg",
    operation_id = "lfg_create",
    request_body = CreateLfgPost,
    responses(
        (status = 200, description = "Posted", body = LfgPostView),
        (status = 400, description = "A field is invalid", body = ApiErrorBody),
        (status = 403, description = "RSI handle not verified, or sharing restricted", body = ApiErrorBody),
        (status = 409, description = "You already have an open post", body = ApiErrorBody),
        (status = 429, description = "Too many posts today", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn create_post(
    guard: RequireUnrestricted<Sharing>,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(body): Json<CreateLfgPost>,
) -> Response {
    let auth = guard.into_user();
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if me.rsi_verified_at.is_none() {
        return err(StatusCode::FORBIDDEN, "rsi_handle_not_verified");
    }
    if !(CREW_MIN..=CREW_MAX).contains(&body.crew_slots) {
        return err(StatusCode::BAD_REQUEST, "invalid_crew_slots");
    }
    let minutes = body.expires_in_minutes.unwrap_or(EXPIRY_DEFAULT_MINUTES);
    if !(EXPIRY_MIN_MINUTES..=EXPIRY_MAX_MINUTES).contains(&minutes) {
        return err(StatusCode::BAD_REQUEST, "invalid_expiry");
    }
    let system = match body
        .system
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(s) => match starstats_core::location_classifier::canonical_system(s) {
            Some(c) => Some(c.to_string()),
            None => return err(StatusCode::BAD_REQUEST, "invalid_system"),
        },
    };
    let location = match text("location", body.location, LOCATION_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let ship = match text("ship", body.ship, SHIP_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let note = match text("note", body.note, NOTE_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };

    let now = Utc::now();
    match store.open_post_for_host(&me.claimed_handle, now).await {
        Ok(Some(_)) => return err(StatusCode::CONFLICT, "already_posting"),
        Ok(None) => {}
        Err(e) => return store_err(e, "open_post_for_host"),
    }
    match store
        .count_posts_since(&me.claimed_handle, now - day())
        .await
    {
        Ok(n) if n >= POSTS_PER_DAY => return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        Ok(_) => {}
        Err(e) => return store_err(e, "count_posts_since"),
    }
    let post = match store
        .create_post(NewLfgPost {
            host_handle: me.claimed_handle.clone(),
            activity: body.activity,
            system,
            location,
            ship,
            crew_slots: body.crew_slots,
            voice: body.voice,
            region: body.region,
            note,
            expires_at: now + Duration::minutes(minutes),
        })
        .await
    {
        Ok(p) => p,
        Err(e) => return store_err(e, "create_post"),
    };
    audit_best_effort(
        audit.as_ref(),
        &me.claimed_handle,
        "lfg.posted",
        serde_json::json!({ "post_id": post.id, "activity": post.activity.as_str() }),
    )
    .await;
    Json(view(users.as_ref(), store.as_ref(), &me, post).await).into_response()
}

#[utoipa::path(
    get,
    path = "/v1/lfg/{id}",
    tag = "lfg",
    operation_id = "lfg_get",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "The post", body = LfgPostDetail),
        (status = 404, description = "No such post, or not one you can see", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn get_post(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let post = match visible_post(store.as_ref(), social.as_ref(), &me, id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let v = view(users.as_ref(), store.as_ref(), &me, post).await;
    let members = if v.is_host || v.my_status == Some(MemberStatus::Accepted) {
        match store.members(id).await {
            Ok(all) if v.is_host => all,
            Ok(all) => all
                .into_iter()
                .filter(|m| m.status == MemberStatus::Accepted)
                .collect(),
            Err(e) => return store_err(e, "members"),
        }
    } else {
        Vec::new()
    };
    Json(LfgPostDetail { view: v, members }).into_response()
}

/// The host closes their post.
#[utoipa::path(
    delete,
    path = "/v1/lfg/{id}",
    tag = "lfg",
    operation_id = "lfg_close",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Closed"),
        (status = 404, description = "Not your post", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn close_post(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match store.get_post(id).await {
        Ok(Some(p)) if p.host_handle.eq_ignore_ascii_case(&me.claimed_handle) => {}
        Ok(_) => return not_found(),
        Err(e) => return store_err(e, "get_post"),
    }
    match store.close_post(id, Utc::now()).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_err(e, "close_post"),
    }
}

/// Ask to join. The host is notified.
#[utoipa::path(
    post,
    path = "/v1/lfg/{id}/join",
    tag = "lfg",
    operation_id = "lfg_join",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 200, description = "Asked", body = LfgMember),
        (status = 400, description = "Your own post", body = ApiErrorBody),
        (status = 403, description = "Not verified, restricted, or removed from this group", body = ApiErrorBody),
        (status = 404, description = "No such open post, or not one you can see", body = ApiErrorBody),
        (status = 409, description = "Already asked, or the group is full", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn join(
    guard: RequireUnrestricted<Sharing>,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let auth = guard.into_user();
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if me.rsi_verified_at.is_none() {
        return err(StatusCode::FORBIDDEN, "rsi_handle_not_verified");
    }
    let post = match visible_post(store.as_ref(), social.as_ref(), &me, id).await {
        Ok(p) if p.is_open(Utc::now()) => p,
        Ok(_) => return not_found(),
        Err(r) => return r,
    };
    if post.host_handle.eq_ignore_ascii_case(&me.claimed_handle) {
        return err(StatusCode::BAD_REQUEST, "own_post");
    }
    if post.crew_count >= i64::from(post.crew_slots) {
        return err(StatusCode::CONFLICT, "group_full");
    }
    let member = match store.request_join(id, &me.claimed_handle).await {
        Ok(m) => m,
        Err(e) => return store_err(e, "request_join"),
    };
    notify(
        social.as_ref(),
        notes.as_ref(),
        &post.host_handle,
        &me,
        NotificationKind::LfgJoinRequest,
        serde_json::json!({ "post_id": id, "rsi_verified": true }),
    )
    .await;
    Json(member).into_response()
}

/// Withdraw a request, or leave a group you are in.
#[utoipa::path(
    delete,
    path = "/v1/lfg/{id}/join",
    tag = "lfg",
    operation_id = "lfg_leave",
    params(("id" = Uuid, Path, description = "Post id")),
    responses(
        (status = 204, description = "Left"),
        (status = 404, description = "Not in this group", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn leave(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match store.member(id, &me.claimed_handle).await {
        Ok(Some(m)) if matches!(m.status, MemberStatus::Requested | MemberStatus::Accepted) => {}
        Ok(_) => return not_found(),
        Err(e) => return store_err(e, "member"),
    }
    match store
        .set_member_status(id, &me.claimed_handle, MemberStatus::Left, Utc::now())
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_err(e, "set_member_status"),
    }
}

/// The host accepts, declines or removes someone. Accepting notifies them,
/// so they can copy the host's handle into the in-game invite.
#[utoipa::path(
    put,
    path = "/v1/lfg/{id}/members/{handle}",
    tag = "lfg",
    operation_id = "lfg_respond",
    params(
        ("id" = Uuid, Path, description = "Post id"),
        ("handle" = String, Path, description = "The player to respond to"),
    ),
    request_body = RespondBody,
    responses(
        (status = 200, description = "Their new standing", body = LfgMember),
        (status = 400, description = "Unknown action", body = ApiErrorBody),
        (status = 404, description = "Not your post, or they did not ask", body = ApiErrorBody),
        (status = 409, description = "The group is full, or the post has ended", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn respond(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Extension(commends): Extension<Arc<dyn crate::commends::CommendStore>>,
    Path((id, handle)): Path<(Uuid, String)>,
    Json(body): Json<RespondBody>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let status = match body.action.as_str() {
        "accept" => MemberStatus::Accepted,
        "decline" => MemberStatus::Declined,
        "remove" => MemberStatus::Removed,
        _ => return err(StatusCode::BAD_REQUEST, "invalid_action"),
    };
    let post = match store.get_post(id).await {
        Ok(Some(p)) if p.host_handle.eq_ignore_ascii_case(&me.claimed_handle) => p,
        Ok(_) => return not_found(),
        Err(e) => return store_err(e, "get_post"),
    };
    let current = match store.member(id, &handle).await {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => return store_err(e, "member"),
    };
    if status == MemberStatus::Accepted {
        if !post.is_open(Utc::now()) {
            return err(StatusCode::CONFLICT, "post_ended");
        }
        if current.status != MemberStatus::Requested {
            return err(StatusCode::CONFLICT, "not_requested");
        }
        if post.crew_count >= i64::from(post.crew_slots) {
            return err(StatusCode::CONFLICT, "group_full");
        }
    }
    let updated = match store
        .set_member_status(id, &current.handle, status, Utc::now())
        .await
    {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => return store_err(e, "set_member_status"),
    };
    record_crew_change(
        commends.as_ref(),
        store.as_ref(),
        &post,
        current.status,
        &updated,
    )
    .await;
    if status == MemberStatus::Accepted {
        let host_verified = me.rsi_verified_at.is_some();
        notify(
            social.as_ref(),
            notes.as_ref(),
            &updated.handle,
            &me,
            NotificationKind::LfgJoinAccepted,
            serde_json::json!({ "post_id": id, "rsi_verified": host_verified }),
        )
        .await;
    }
    Json(updated).into_response()
}

/// Keep crew history in step with the group. An accepted player flew with
/// the host and everyone already in the crew (a player who left still
/// flew); a player removed after being accepted did not, and loses their
/// history and commends on this post. Best-effort: the response has
/// already been decided, and a missed row only costs a commend.
async fn record_crew_change(
    commends: &dyn crate::commends::CommendStore,
    store: &dyn LfgStore,
    post: &LfgPost,
    before: MemberStatus,
    after: &LfgMember,
) {
    match after.status {
        MemberStatus::Accepted => {
            let members = match store.members(post.id).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "crew history: members failed");
                    return;
                }
            };
            let others = std::iter::once(post.host_handle.clone()).chain(
                members
                    .into_iter()
                    .filter(|m| matches!(m.status, MemberStatus::Accepted | MemberStatus::Left))
                    .filter(|m| !m.handle.eq_ignore_ascii_case(&after.handle))
                    .map(|m| m.handle),
            );
            for other in others {
                if let Err(e) = commends
                    .record_crew(
                        post.id,
                        post.activity.as_str(),
                        &after.handle,
                        &other,
                        Utc::now(),
                    )
                    .await
                {
                    tracing::warn!(error = %e, "crew history: record failed");
                }
            }
        }
        MemberStatus::Removed if before == MemberStatus::Accepted => {
            if let Err(e) = commends.forget_crew(post.id, &after.handle).await {
                tracing::warn!(error = %e, "crew history: forget failed");
            }
        }
        _ => {}
    }
}

/// Report a post to the moderators. The post is kept as it was when
/// reported, even if it later expires.
#[utoipa::path(
    post,
    path = "/v1/lfg/{id}/report",
    tag = "lfg",
    operation_id = "lfg_report",
    params(("id" = Uuid, Path, description = "Post id")),
    request_body = ReportLfgPost,
    responses(
        (status = 200, description = "Reported"),
        (status = 400, description = "Details too long, or your own post", body = ApiErrorBody),
        (status = 404, description = "No such post", body = ApiErrorBody),
        (status = 429, description = "Too many reports today", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn report(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReportLfgPost>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let details = match text("details", body.details, REPORT_DETAILS_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let post = match visible_post(store.as_ref(), social.as_ref(), &me, id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if post.host_handle.eq_ignore_ascii_case(&me.claimed_handle) {
        return err(StatusCode::BAD_REQUEST, "own_post");
    }
    match store
        .count_reports_since(&me.claimed_handle, Utc::now() - day())
        .await
    {
        Ok(n) if n >= REPORTS_PER_DAY => return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "lfg report rate count failed; skipping gate"),
    }
    let r = match store
        .create_report(&post, &me.claimed_handle, body.reason, details.as_deref())
        .await
    {
        Ok(r) => r,
        Err(e) => return store_err(e, "create_report"),
    };
    audit_best_effort(
        audit.as_ref(),
        &me.claimed_handle,
        "lfg.reported",
        serde_json::json!({ "report_id": r.id, "post_id": id, "reason": r.reason.as_str() }),
    )
    .await;
    StatusCode::OK.into_response()
}

// -- Moderation ------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/admin/lfg/reports",
    tag = "admin",
    operation_id = "admin_lfg_reports",
    params(ReportsQuery),
    responses(
        (status = 200, description = "Reports, newest first", body = LfgReportList),
        (status = 403, description = "Not a moderator"),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_list_reports(
    RequireModerator(_user): RequireModerator,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Query(q): Query<ReportsQuery>,
) -> Response {
    let status = match q.status.as_deref() {
        None | Some("") => Some(LfgReportStatus::Open),
        Some("all") => None,
        Some(s) => match LfgReportStatus::parse(s) {
            Some(s) => Some(s),
            None => return err(StatusCode::BAD_REQUEST, "invalid_status"),
        },
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    match store.list_reports(status, limit, offset).await {
        Ok(reports) => Json(LfgReportList { reports }).into_response(),
        Err(e) => store_err(e, "list_reports"),
    }
}

/// Resolve a report. `post_removed` takes the post down; `user_suspended`
/// takes it down and suspends the host, as the share report queue does.
#[utoipa::path(
    post,
    path = "/v1/admin/lfg/reports/{id}/resolve",
    tag = "admin",
    operation_id = "admin_lfg_resolve",
    params(("id" = Uuid, Path, description = "Report id")),
    request_body = ResolveLfgReport,
    responses(
        (status = 200, description = "Resolved", body = LfgReport),
        (status = 400, description = "Unknown outcome, or note too long", body = ApiErrorBody),
        (status = 404, description = "No such report", body = ApiErrorBody),
        (status = 409, description = "Already resolved", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn admin_resolve(
    RequireModerator(moderator): RequireModerator,
    Extension(store): Extension<Arc<dyn LfgStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ResolveLfgReport>,
) -> Response {
    let outcome = match LfgReportStatus::parse(&body.outcome) {
        Some(s) if s != LfgReportStatus::Open => s,
        _ => return err(StatusCode::BAD_REQUEST, "invalid_outcome"),
    };
    let note = match text("note", body.note, RESOLUTION_NOTE_MAX) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let by = moderator.preferred_username.clone();
    let now = Utc::now();
    let pending = match store.get_report(id).await {
        Ok(Some(r)) if r.status == LfgReportStatus::Open => r,
        Ok(Some(_)) => return err(StatusCode::CONFLICT, "already_resolved"),
        Ok(None) => return not_found(),
        Err(e) => return store_err(e, "get_report"),
    };
    // Enforce first, then record. Recording a decision is not taking it,
    // and a report marked resolved with nothing enforced could never be
    // retried from the queue.
    if matches!(
        outcome,
        LfgReportStatus::PostRemoved | LfgReportStatus::UserSuspended
    ) {
        let reason = note
            .clone()
            .unwrap_or_else(|| format!("Removed after LFG report {}", pending.id));
        if let Err(e) = store.remove_post(pending.post_id, &by, &reason, now).await {
            tracing::error!(error = %e, "lfg: post removal after report failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "removal_failed");
        }
    }
    if outcome == LfgReportStatus::UserSuspended {
        let restriction = Restriction {
            ingest_blocked: true,
            sharing_blocked: true,
            public_profile_blocked: true,
            submissions_blocked: true,
            reason: note
                .clone()
                .unwrap_or_else(|| format!("Suspended after LFG report {}", pending.id)),
            restricted_by: by.clone(),
            restricted_at: now,
            // Lifted explicitly by a moderator, as from the share queue.
            expires_at: None,
        };
        match restrictions
            .upsert_by_handle(&pending.host_handle, &restriction)
            .await
        {
            Ok(Some(_)) => {}
            // No account under that handle any more: nothing was
            // suspended, so do not record that it was.
            Ok(None) => return err(StatusCode::CONFLICT, "host_not_found"),
            Err(e) => {
                tracing::error!(error = %e, "lfg: suspension after report failed");
                return err(StatusCode::INTERNAL_SERVER_ERROR, "suspension_failed");
            }
        }
    }
    let report = match store
        .resolve_report(id, &by, outcome, note.as_deref(), now)
        .await
    {
        Ok(r) => r,
        Err(e) => return store_err(e, "resolve_report"),
    };
    audit_best_effort(
        audit.as_ref(),
        &by,
        "lfg.report_resolved",
        serde_json::json!({
            "report_id": report.id,
            "post_id": report.post_id,
            "outcome": outcome.as_str(),
            "host_handle": report.host_handle,
        }),
    )
    .await;
    Json(report).into_response()
}

/// Delete ended posts once they are no longer useful, daily. A post with
/// an open report is kept until the report is resolved.
pub fn spawn_purge_loop(
    store: Arc<dyn LfgStore>,
    commends: Arc<dyn crate::commends::CommendStore>,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        loop {
            tick.tick().await;
            let before = Utc::now() - Duration::days(RETAIN_ENDED_DAYS);
            match store.purge_ended(before).await {
                Ok(n) => tracing::info!(purged = n, "lfg retention sweep"),
                Err(e) => tracing::warn!(error = %e, "lfg retention sweep failed"),
            }
            let before = Utc::now() - Duration::days(crate::commends::CREW_HISTORY_DAYS);
            match commends.purge_crew_before(before).await {
                Ok(n) => tracing::info!(purged = n, "crew history retention sweep"),
                Err(e) => tracing::warn!(error = %e, "crew history retention sweep failed"),
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_restrictions::test_support::MemoryAccountRestrictionStore;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::TokenIssuer;
    use crate::lfg::test_support::MemoryLfgStore;
    use crate::notifications::test_support::MemoryNotificationStore;
    use crate::social::test_support::MemorySocialStore;
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    struct Fixture {
        app: Router,
        issuer: TokenIssuer,
        users: Arc<MemoryUserStore>,
        social: Arc<MemorySocialStore>,
        notes: Arc<MemoryNotificationStore>,
        staff: Arc<MemoryStaffRoleStore>,
        restrictions: Arc<MemoryAccountRestrictionStore>,
        commends: Arc<crate::commends::test_support::MemoryCommendStore>,
    }

    fn fixture() -> Fixture {
        let users = Arc::new(MemoryUserStore::new());
        let social = Arc::new(MemorySocialStore::new());
        let notes = Arc::new(MemoryNotificationStore::new());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let restrictions = Arc::new(MemoryAccountRestrictionStore::new());
        let (issuer, verifier) = fresh_pair();
        let users_dyn: Arc<dyn UserStore> = users.clone();
        let social_dyn: Arc<dyn SocialStore> = social.clone();
        let notes_dyn: Arc<dyn NotificationStore> = notes.clone();
        let staff_dyn: Arc<dyn StaffRoleStore> = staff.clone();
        let restrictions_dyn: Arc<dyn AccountRestrictionStore> = restrictions.clone();
        let store: Arc<dyn LfgStore> = Arc::new(MemoryLfgStore::new());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let commends = Arc::new(crate::commends::test_support::MemoryCommendStore::new());
        let commends_dyn: Arc<dyn crate::commends::CommendStore> = commends.clone();
        let app = routes()
            .merge(crate::commend_routes::routes())
            .layer(Extension(commends_dyn))
            .layer(Extension(Arc::new(
                crate::commends::CommendRateLimiter::new(),
            )))
            .layer(Extension(users_dyn))
            .layer(Extension(social_dyn))
            .layer(Extension(notes_dyn))
            .layer(Extension(staff_dyn))
            .layer(Extension(restrictions_dyn))
            .layer(Extension(store))
            .layer(Extension(audit))
            .layer(Extension(Arc::new(verifier)));
        Fixture {
            app,
            issuer,
            users,
            social,
            notes,
            staff,
            restrictions,
            commends,
        }
    }

    impl Fixture {
        async fn user(&self, handle: &str, verified: bool) -> String {
            let phc = hash_password("password-123-abcdef").unwrap();
            let u = self
                .users
                .create(&format!("{handle}@example.com"), &phc, handle)
                .await
                .unwrap();
            if verified {
                self.users.mark_rsi_verified(u.id).await.unwrap();
            }
            self.issuer.sign_user(&u.id.to_string(), handle).unwrap()
        }

        async fn moderator(&self, handle: &str) -> String {
            let token = self.user(handle, true).await;
            let id = self.users.find_by_handle(handle).await.unwrap().unwrap().id;
            self.staff
                .grant(id, StaffRole::Moderator, None, None)
                .await
                .unwrap();
            token
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

        async fn post(&self, token: &str, crew: i16) -> String {
            let (s, v) = self
                .call(
                    "POST",
                    "/v1/lfg",
                    token,
                    Some(serde_json::json!({
                        "activity": "mining",
                        "system": "stanton",
                        "ship": "Prospector",
                        "crew_slots": crew,
                        "voice": "optional",
                        "region": "eu",
                        "note": "Quantanium run, bring a Vulture",
                    })),
                )
                .await;
            assert_eq!(s, StatusCode::OK, "{v}");
            v["id"].as_str().unwrap().to_string()
        }
    }

    #[tokio::test]
    async fn posting_needs_a_verified_handle_and_valid_fields() {
        let f = fixture();
        let unverified = f.user("Newbie", false).await;
        let host = f.user("Host", true).await;
        let body = |system: &str, crew: i16| {
            serde_json::json!({
                "activity": "mining", "system": system, "crew_slots": crew,
                "voice": "none", "region": "any",
            })
        };
        let (s, v) = f
            .call("POST", "/v1/lfg", &unverified, Some(body("Pyro", 2)))
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "rsi_handle_not_verified");
        let (s, v) = f
            .call("POST", "/v1/lfg", &host, Some(body("Arrakis", 2)))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid_system");
        let (s, v) = f
            .call("POST", "/v1/lfg", &host, Some(body("Pyro", 0)))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "invalid_crew_slots");

        let id = f.post(&host, 2).await;
        let (_, v) = f.call("GET", &format!("/v1/lfg/{id}"), &host, None).await;
        assert_eq!(v["system"], "Stanton", "the system is stored canonically");
        assert_eq!(v["is_host"], true);
        let (s, v) = f
            .call("POST", "/v1/lfg", &host, Some(body("Pyro", 2)))
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "one open post at a time");
        assert_eq!(v["error"], "already_posting");
    }

    #[tokio::test]
    async fn joining_is_asked_accepted_and_notified() {
        let f = fixture();
        let host = f.user("Host", true).await;
        let bob = f.user("Bob", true).await;
        let carol = f.user("Carol", true).await;
        let id = f.post(&host, 1).await;

        let (s, v) = f
            .call("POST", &format!("/v1/lfg/{id}/join"), &bob, None)
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["status"], "requested");
        assert_eq!(f.notes.all_for("Host").len(), 1, "the host is told");

        // Only the host sees who asked.
        let (_, v) = f.call("GET", &format!("/v1/lfg/{id}"), &carol, None).await;
        assert_eq!(v["members"], serde_json::json!([]));
        let (_, v) = f.call("GET", &format!("/v1/lfg/{id}"), &host, None).await;
        assert_eq!(v["members"][0]["handle"], "Bob");

        let (s, v) = f
            .call(
                "PUT",
                &format!("/v1/lfg/{id}/members/bob"),
                &host,
                Some(serde_json::json!({ "action": "accept" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["status"], "accepted");
        assert_eq!(f.notes.all_for("Bob").len(), 1, "the joiner is told");

        // One slot, now taken.
        let (s, v) = f
            .call("POST", &format!("/v1/lfg/{id}/join"), &carol, None)
            .await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "group_full");
    }

    #[tokio::test]
    async fn the_summary_counts_players_waiting_on_my_post() {
        let f = fixture();
        let host = f.user("Host", true).await;
        let bob = f.user("Bob", true).await;
        let carol = f.user("Carol", true).await;
        let (_, v) = f.call("GET", "/v1/me/lfg/summary", &host, None).await;
        assert_eq!(
            v,
            serde_json::json!({
                "hosting": false,
                "pending_requests": 0,
                "post_id": null,
                "opened_at": null,
            })
        );

        let id = f.post(&host, 3).await;
        let (_, v) = f.call("GET", "/v1/me/lfg/summary", &host, None).await;
        assert_eq!(v["post_id"], id.as_str(), "names the open post");
        assert!(v["opened_at"].is_string());
        f.call("POST", &format!("/v1/lfg/{id}/join"), &bob, None)
            .await;
        f.call("POST", &format!("/v1/lfg/{id}/join"), &carol, None)
            .await;
        let (_, v) = f.call("GET", "/v1/me/lfg/summary", &host, None).await;
        assert_eq!(v["pending_requests"], 2);

        // Answering one takes it off the badge.
        f.call(
            "PUT",
            &format!("/v1/lfg/{id}/members/bob"),
            &host,
            Some(serde_json::json!({ "action": "accept" })),
        )
        .await;
        let (_, v) = f.call("GET", "/v1/me/lfg/summary", &host, None).await;
        assert_eq!(v["pending_requests"], 1);

        // A joiner is not hosting anything.
        let (_, v) = f.call("GET", "/v1/me/lfg/summary", &bob, None).await;
        assert_eq!(v["hosting"], false);
    }

    #[tokio::test]
    async fn a_removed_player_cannot_ask_again_but_one_who_left_can() {
        let f = fixture();
        let host = f.user("Host", true).await;
        let bob = f.user("Bob", true).await;
        let id = f.post(&host, 3).await;
        f.call("POST", &format!("/v1/lfg/{id}/join"), &bob, None)
            .await;
        let (s, _) = f
            .call("DELETE", &format!("/v1/lfg/{id}/join"), &bob, None)
            .await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (s, _) = f
            .call("POST", &format!("/v1/lfg/{id}/join"), &bob, None)
            .await;
        assert_eq!(s, StatusCode::OK);
        f.call(
            "PUT",
            &format!("/v1/lfg/{id}/members/Bob"),
            &host,
            Some(serde_json::json!({ "action": "remove" })),
        )
        .await;
        let (s, v) = f
            .call("POST", &format!("/v1/lfg/{id}/join"), &bob, None)
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "removed_from_group");
    }

    #[tokio::test]
    async fn a_block_hides_the_post_both_ways() {
        let f = fixture();
        let host = f.user("Host", true).await;
        let pest = f.user("Pest", true).await;
        let blocker = f.user("Blocker", true).await;
        let id = f.post(&host, 3).await;
        f.social.block("Host", "Pest").await.unwrap();
        f.social.block("Blocker", "Host").await.unwrap();

        for token in [&pest, &blocker] {
            let (_, v) = f.call("GET", "/v1/lfg", token, None).await;
            assert_eq!(v["posts"], serde_json::json!([]));
            let (s, _) = f.call("GET", &format!("/v1/lfg/{id}"), token, None).await;
            assert_eq!(s, StatusCode::NOT_FOUND);
            let (s, _) = f
                .call("POST", &format!("/v1/lfg/{id}/join"), token, None)
                .await;
            assert_eq!(s, StatusCode::NOT_FOUND, "the same 404 as a missing post");
        }
    }

    #[tokio::test]
    async fn a_report_reaches_moderators_and_their_decision_is_enforced() {
        let f = fixture();
        let host = f.user("Host", true).await;
        let bob = f.user("Bob", true).await;
        let moderator = f.moderator("Mod").await;
        let host_id = f.users.find_by_handle("Host").await.unwrap().unwrap().id;
        f.restrictions.add_handle(host_id, "Host");
        let id = f.post(&host, 3).await;

        let (s, _) = f
            .call(
                "POST",
                &format!("/v1/lfg/{id}/report"),
                &bob,
                Some(serde_json::json!({ "reason": "spam", "details": "selling credits" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK);

        let (s, _) = f.call("GET", "/v1/admin/lfg/reports", &bob, None).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "players cannot read the queue");
        let (s, v) = f
            .call("GET", "/v1/admin/lfg/reports", &moderator, None)
            .await;
        assert_eq!(s, StatusCode::OK);
        let report = &v["reports"][0];
        assert_eq!(
            report["post_snapshot"]["note"],
            "Quantanium run, bring a Vulture"
        );
        let report_id = report["id"].as_str().unwrap().to_string();

        let (s, v) = f
            .call(
                "POST",
                &format!("/v1/admin/lfg/reports/{report_id}/resolve"),
                &moderator,
                Some(serde_json::json!({ "outcome": "user_suspended", "note": "credit seller" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let (s, _) = f.call("GET", &format!("/v1/lfg/{id}"), &bob, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "the post is taken down");
        let r = f.restrictions.effective(host_id).await.unwrap();
        assert!(
            r.is_some_and(|r| r.sharing_blocked),
            "the host is suspended"
        );

        let (s, v) = f
            .call(
                "POST",
                &format!("/v1/admin/lfg/reports/{report_id}/resolve"),
                &moderator,
                Some(serde_json::json!({ "outcome": "dismissed" })),
            )
            .await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "already_resolved");
    }

    async fn crew_of_three(f: &Fixture) -> (String, String, String, String) {
        let host = f.user("Host", true).await;
        let bob = f.user("Bob", true).await;
        let carol = f.user("Carol", true).await;
        let id = f.post(&host, 3).await;
        for (who, handle) in [(&bob, "bob"), (&carol, "carol")] {
            f.call("POST", &format!("/v1/lfg/{id}/join"), who, None)
                .await;
            let (s, v) = f
                .call(
                    "PUT",
                    &format!("/v1/lfg/{id}/members/{handle}"),
                    &host,
                    Some(serde_json::json!({ "action": "accept" })),
                )
                .await;
            assert_eq!(s, StatusCode::OK, "{v}");
        }
        (host, bob, carol, id)
    }

    #[tokio::test]
    async fn accepted_crew_flew_together_and_a_requester_did_not() {
        use crate::commends::CommendStore;
        let f = fixture();
        let (host, _bob, _carol, id) = crew_of_three(&f).await;
        let dave = f.user("Dave", true).await;
        f.call("POST", &format!("/v1/lfg/{id}/join"), &dave, None)
            .await;
        let post: Uuid = id.parse().unwrap();
        assert!(f.commends.flew_together(post, "Bob", "Host").await.unwrap());
        assert!(
            f.commends
                .flew_together(post, "Carol", "Bob")
                .await
                .unwrap(),
            "crewmates, not just the host"
        );
        assert!(!f
            .commends
            .flew_together(post, "Dave", "Host")
            .await
            .unwrap());

        // Removed after being accepted: did not fly.
        f.call(
            "PUT",
            &format!("/v1/lfg/{id}/members/carol"),
            &host,
            Some(serde_json::json!({ "action": "remove" })),
        )
        .await;
        assert!(!f
            .commends
            .flew_together(post, "Carol", "Host")
            .await
            .unwrap());
        assert!(f.commends.flew_together(post, "Bob", "Host").await.unwrap());
    }

    #[tokio::test]
    async fn crew_commend_once_the_post_ends_and_nobody_learns_who() {
        let f = fixture();
        let (host, bob, carol, id) = crew_of_three(&f).await;
        let uri = format!("/v1/crew/{id}/commends/host");
        let body = Some(serde_json::json!({ "kind": "great_pilot" }));

        let (s, v) = f.call("PUT", &uri, &bob, body.clone()).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert_eq!(v["error"], "post_not_ended");

        let (s, _) = f
            .call("DELETE", &format!("/v1/lfg/{id}"), &host, None)
            .await;
        assert!(s.is_success(), "{s}");
        let before = f.notes.all_for("Host").len();

        let (s, v) = f.call("PUT", &uri, &bob, body.clone()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["kind"], "great_pilot");
        let told = f.notes.all_for("Host");
        assert_eq!(told.len(), before + 1, "the host is told");
        let n = told
            .iter()
            .find(|n| n.kind == NotificationKind::Commend)
            .unwrap();
        assert_eq!(n.actor_handle, None, "without a name");
        assert_eq!(n.payload["kind"], "great_pilot");

        // Changing the word does not notify again.
        let (s, _) = f
            .call(
                "PUT",
                &uri,
                &bob,
                Some(serde_json::json!({ "kind": "good_comms" })),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(f.notes.all_for("Host").len(), before + 1);
        f.call("PUT", &uri, &carol, body.clone()).await;

        let (_, v) = f.call("GET", "/v1/me/commends", &host, None).await;
        let count = |k: &str| {
            v["totals"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["kind"] == k)
                .unwrap()["count"]
                .clone()
        };
        assert_eq!(count("good_comms"), 1);
        assert_eq!(count("great_pilot"), 1);
        assert_eq!(count("reliable"), 0);

        // Bob sees the window, his crewmates, and what he gave.
        let (_, v) = f.call("GET", "/v1/me/crew", &bob, None).await;
        let w = &v["windows"][0];
        assert_eq!(w["post_id"], id.as_str());
        assert_eq!(w["crew"].as_array().unwrap().len(), 2);
        let host_row = w["crew"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["handle"] == "Host")
            .unwrap();
        assert_eq!(host_row["my_commend"], "good_comms");
        assert_eq!(v["history"].as_array().unwrap().len(), 2);

        let (s, _) = f.call("DELETE", &uri, &bob, None).await;
        assert_eq!(s, StatusCode::NO_CONTENT);
        let (_, v) = f.call("GET", "/v1/me/commends", &host, None).await;
        assert_eq!(
            v["totals"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["count"] != 0)
                .count(),
            1,
            "only Carol's is left"
        );
    }

    #[tokio::test]
    async fn only_crewmates_can_commend_and_never_themselves() {
        let f = fixture();
        let (host, bob, _carol, id) = crew_of_three(&f).await;
        let dave = f.user("Dave", true).await;
        f.call("POST", &format!("/v1/lfg/{id}/join"), &dave, None)
            .await;
        f.call("DELETE", &format!("/v1/lfg/{id}"), &host, None)
            .await;
        let body = Some(serde_json::json!({ "kind": "reliable" }));

        let (s, v) = f
            .call(
                "PUT",
                &format!("/v1/crew/{id}/commends/host"),
                &dave,
                body.clone(),
            )
            .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "asked but never flew");
        assert_eq!(v["error"], "not_found");
        let (s, v) = f
            .call(
                "PUT",
                &format!("/v1/crew/{id}/commends/BOB"),
                &bob,
                body.clone(),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "cannot_commend_self");
        let (s, _) = f
            .call(
                "PUT",
                &format!("/v1/crew/{id}/commends/ghost"),
                &bob,
                body.clone(),
            )
            .await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = f
            .call(
                "PUT",
                &format!("/v1/crew/{id}/commends/host"),
                &bob,
                Some(serde_json::json!({ "kind": "toxic" })),
            )
            .await;
        assert!(s.is_client_error(), "no word outside the list: {s}");
    }

    #[tokio::test]
    async fn an_unverified_crewmate_cannot_commend() {
        use crate::commends::CommendStore;
        let f = fixture();
        let host = f.user("Host", true).await;
        let eve = f.user("Eve", false).await;
        let id = f.post(&host, 2).await;
        f.call("DELETE", &format!("/v1/lfg/{id}"), &host, None)
            .await;
        // Joining needs a verified handle, so put her in the crew directly.
        f.commends
            .record_crew(id.parse().unwrap(), "mining", "Eve", "Host", Utc::now())
            .await
            .unwrap();
        let (s, v) = f
            .call(
                "PUT",
                &format!("/v1/crew/{id}/commends/host"),
                &eve,
                Some(serde_json::json!({ "kind": "reliable" })),
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(v["error"], "rsi_handle_not_verified");
    }

    fn ended_post(closed: Option<i64>, expires: i64, removed: bool) -> LfgPost {
        let now = Utc::now();
        LfgPost {
            id: Uuid::new_v4(),
            host_handle: "Host".into(),
            activity: LfgActivity::Mining,
            system: None,
            location: None,
            ship: None,
            crew_slots: 2,
            voice: LfgVoice::Optional,
            region: LfgRegion::Eu,
            note: None,
            created_at: now - Duration::hours(100),
            expires_at: now + Duration::hours(expires),
            closed_at: closed.map(|h| now + Duration::hours(h)),
            removed_at: removed.then_some(now),
            crew_count: 1,
        }
    }

    #[test]
    fn the_commend_window_is_the_48_hours_after_the_post_ends() {
        use crate::commend_routes::window_open;
        let now = Utc::now();
        assert_eq!(
            window_open(&ended_post(None, 1, false), now).unwrap_err(),
            "post_not_ended"
        );
        assert!(
            window_open(&ended_post(Some(-47), 3, false), now).is_ok(),
            "closed"
        );
        assert!(
            window_open(&ended_post(None, -47, false), now).is_ok(),
            "expired"
        );
        assert_eq!(
            window_open(&ended_post(Some(-49), 3, false), now).unwrap_err(),
            "window_closed"
        );
        assert_eq!(
            window_open(&ended_post(Some(-1), 3, true), now).unwrap_err(),
            "not_found",
            "a post a moderator removed opens no window"
        );
    }
}
