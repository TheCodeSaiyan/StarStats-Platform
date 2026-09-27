//! Crew commend routes (social phase 5). See `commends.rs` for the model.
//!
//! - `GET /v1/me/crew` is your private crew history and the commend
//!   windows open to you.
//! - `PUT /v1/crew/{post_id}/commends/{handle}` gives or changes a commend.
//! - `DELETE /v1/crew/{post_id}/commends/{handle}` withdraws it.
//! - `GET /v1/u/{handle}/commends` is a profile's public totals, behind the
//!   same visibility check as its salutes.
//! - `GET /v1/me/commends` is your own totals.
//!
//! A commend can be given only to someone you flew with on that post,
//! only once the post has ended and for [`COMMEND_WINDOW_HOURS`] after,
//! and only with a verified RSI handle. Nobody learns who gave which: the
//! notification names the kind and the crew's activity, not the giver.

use crate::account_restrictions::AccountRestrictionStore;
use crate::api_error::ApiErrorBody;
use crate::audit::AuditLog;
use crate::auth::AuthenticatedUser;
use crate::commends::{
    CommendKind, CommendRateLimiter, CommendStore, CommendTotal, CrewMate, GivenCommend,
    COMMEND_WINDOW_HOURS, CREW_HISTORY_DAYS,
};
use crate::lfg::{LfgPost, LfgStore};
use crate::notifications::{NotificationKind, NotificationStore};
use crate::restriction_guard::{RequireUnrestricted, Sharing};
use crate::salute_routes::profile_visible;
use crate::share_metadata::ShareMetadataStore;
use crate::social::SocialStore;
use crate::social_routes::{blocked_by_owner, caller, target};
use crate::spicedb::SpicedbClient;
use crate::users::UserStore;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/me/crew", get(my_crew))
        .route(
            "/v1/crew/{post_id}/commends/{handle}",
            put(give).delete(withdraw),
        )
        .route("/v1/u/{handle}/commends", get(profile_commends))
        .route("/v1/me/commends", get(my_commends))
}

/// How far back to look for posts whose window may still be open: the
/// longest a post can stay up, plus the window, with room to spare.
const WINDOW_LOOKBACK_DAYS: i64 = 7;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommendTotals {
    /// Every kind, zeros included. Who gave them is never shown.
    pub totals: Vec<CommendTotal>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GiveCommend {
    pub kind: CommendKind,
}

/// A crewmate in an open commend window.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WindowMate {
    pub handle: String,
    /// What you gave them on this post, if anything.
    pub my_commend: Option<CommendKind>,
}

/// A post whose crew can still commend each other.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommendWindow {
    pub post_id: Uuid,
    pub activity: String,
    pub ended_at: DateTime<Utc>,
    pub closes_at: DateTime<Utc>,
    pub crew: Vec<WindowMate>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CrewOverview {
    /// Open commend windows, soonest to close first.
    pub windows: Vec<CommendWindow>,
    /// Everyone you flew with in the last 90 days, newest first. Private.
    pub history: Vec<CrewMate>,
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

fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "not_found")
}

fn internal(what: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, what, "commend store failed");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
}

/// When the post stopped taking crew: closed by the host, or expired.
/// `None` while it is still open, and for a post a moderator removed,
/// which opens no window.
pub fn ended_at(post: &LfgPost, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if post.removed_at.is_some() {
        return None;
    }
    let end = post
        .closed_at
        .unwrap_or(post.expires_at)
        .min(post.expires_at);
    (end <= now).then_some(end)
}

/// Whether the commend window for this post is open now.
pub fn window_open(post: &LfgPost, now: DateTime<Utc>) -> Result<DateTime<Utc>, &'static str> {
    if post.removed_at.is_some() {
        return Err("not_found");
    }
    let Some(end) = ended_at(post, now) else {
        return Err("post_not_ended");
    };
    let closes = end + Duration::hours(COMMEND_WINDOW_HOURS);
    if now > closes {
        return Err("window_closed");
    }
    Ok(closes)
}

/// Your crew history and the commend windows open to you.
#[utoipa::path(
    get,
    path = "/v1/me/crew",
    tag = "lfg",
    operation_id = "crew_mine",
    responses((status = 200, description = "Your crew", body = CrewOverview)),
    security(("bearer" = [])),
)]
pub async fn my_crew(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(lfg): Extension<Arc<dyn LfgStore>>,
    Extension(commends): Extension<Arc<dyn CommendStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let now = Utc::now();
    let history = match commends
        .crew_of(&me.claimed_handle, now - Duration::days(CREW_HISTORY_DAYS))
        .await
    {
        Ok(h) => h,
        Err(e) => return internal("crew_of", e),
    };

    // Posts recent enough that their window may be open, in history order.
    let recent = now - Duration::days(WINDOW_LOOKBACK_DAYS);
    let mut crews: Vec<(Uuid, Vec<&CrewMate>)> = Vec::new();
    for mate in history.iter().filter(|m| m.flew_at >= recent) {
        match crews.iter_mut().find(|(id, _)| *id == mate.post_id) {
            Some((_, v)) => v.push(mate),
            None => crews.push((mate.post_id, vec![mate])),
        }
    }
    let ids: Vec<Uuid> = crews.iter().map(|(id, _)| *id).collect();
    let given: HashMap<(Uuid, String), CommendKind> =
        match commends.given(&me.claimed_handle, &ids).await {
            Ok(g) => g
                .into_iter()
                .map(|g| ((g.post_id, g.recipient.to_lowercase()), g.kind))
                .collect(),
            Err(e) => return internal("given", e),
        };

    let mut windows = Vec::new();
    for (post_id, mates) in crews {
        let post = match lfg.get_post(post_id).await {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => return internal("get_post", e),
        };
        let Ok(closes_at) = window_open(&post, now) else {
            continue;
        };
        let Some(ended) = ended_at(&post, now) else {
            continue;
        };
        windows.push(CommendWindow {
            post_id,
            activity: post.activity.as_str().to_string(),
            ended_at: ended,
            closes_at,
            crew: mates
                .into_iter()
                .map(|m| WindowMate {
                    handle: m.handle.clone(),
                    my_commend: given.get(&(post_id, m.handle.to_lowercase())).copied(),
                })
                .collect(),
        });
    }
    windows.sort_by_key(|w| w.closes_at);
    Json(CrewOverview { windows, history }).into_response()
}

/// The checks giving and withdrawing share: a real crewmate on a post
/// whose window is open, neither side having blocked the other.
#[allow(clippy::too_many_arguments)]
async fn crewmate_on_open_post(
    users: &dyn UserStore,
    social: &dyn SocialStore,
    lfg: &dyn LfgStore,
    commends: &dyn CommendStore,
    me: &str,
    post_id: Uuid,
    handle: &str,
) -> Result<(crate::users::User, LfgPost), Response> {
    let them = match target(handle, users).await {
        Ok(u) => u,
        Err(r) if r.status() == StatusCode::BAD_REQUEST => return Err(r),
        Err(_) => return Err(not_found()),
    };
    if them.claimed_handle.eq_ignore_ascii_case(me) {
        return Err(err(StatusCode::BAD_REQUEST, "cannot_commend_self"));
    }
    for (a, b) in [
        (me, them.claimed_handle.as_str()),
        (them.claimed_handle.as_str(), me),
    ] {
        match social.is_blocked(a, b).await {
            Ok(false) => {}
            Ok(true) => return Err(not_found()),
            Err(e) => return Err(internal("is_blocked", e)),
        }
    }
    match commends
        .flew_together(post_id, me, &them.claimed_handle)
        .await
    {
        Ok(true) => {}
        Ok(false) => return Err(not_found()),
        Err(e) => return Err(internal("flew_together", e)),
    }
    let post = match lfg.get_post(post_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Err(not_found()),
        Err(e) => return Err(internal("get_post", e)),
    };
    match window_open(&post, Utc::now()) {
        Ok(_) => Ok((them, post)),
        Err("not_found") => Err(not_found()),
        Err(code) => Err(err(StatusCode::CONFLICT, code)),
    }
}

/// Commend a crewmate, or change the word. Only a new commend notifies
/// them, and the notification does not say who gave it.
#[utoipa::path(
    put,
    path = "/v1/crew/{post_id}/commends/{handle}",
    tag = "lfg",
    operation_id = "crew_commend",
    params(
        ("post_id" = Uuid, Path, description = "The post you flew on"),
        ("handle" = String, Path, description = "The crewmate to commend"),
    ),
    request_body = GiveCommend,
    responses(
        (status = 200, description = "Commended", body = GivenCommend),
        (status = 400, description = "Yourself, or an invalid handle", body = ApiErrorBody),
        (status = 403, description = "RSI handle not verified, or sharing restricted", body = ApiErrorBody),
        (status = 404, description = "Not someone you flew with on this post", body = ApiErrorBody),
        (status = 409, description = "The post has not ended, or the window has closed", body = ApiErrorBody),
        (status = 429, description = "Too many commends", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn give(
    guard: RequireUnrestricted<Sharing>,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(lfg): Extension<Arc<dyn LfgStore>>,
    Extension(commends): Extension<Arc<dyn CommendStore>>,
    Extension(limiter): Extension<Arc<CommendRateLimiter>>,
    Path((post_id, handle)): Path<(Uuid, String)>,
    Json(body): Json<GiveCommend>,
) -> Response {
    let auth = guard.into_user();
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    // Totals are public, so a commend has to cost something to fake.
    if me.rsi_verified_at.is_none() {
        return err(StatusCode::FORBIDDEN, "rsi_handle_not_verified");
    }
    if !limiter.check(&me.claimed_handle) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    }
    let (them, post) = match crewmate_on_open_post(
        users.as_ref(),
        social.as_ref(),
        lfg.as_ref(),
        commends.as_ref(),
        &me.claimed_handle,
        post_id,
        &handle,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    let created = match commends
        .set(
            post_id,
            &me.claimed_handle,
            &them.claimed_handle,
            body.kind,
            Utc::now(),
        )
        .await
    {
        Ok(c) => c,
        Err(e) => return internal("set", e),
    };
    if created {
        // Anonymous by design, but a player who muted the giver still
        // does not want to hear from them.
        let muted = social
            .is_muted(&them.claimed_handle, &me.claimed_handle)
            .await
            .unwrap_or(true);
        if !muted {
            if let Err(e) = notes
                .create(
                    &them.claimed_handle,
                    NotificationKind::Commend,
                    None,
                    serde_json::json!({
                        "kind": body.kind.as_str(),
                        "post_id": post_id,
                        "activity": post.activity.as_str(),
                    }),
                )
                .await
            {
                tracing::warn!(error = %e, "commend notification failed");
            }
        }
    }
    Json(GivenCommend {
        post_id,
        recipient: them.claimed_handle,
        kind: body.kind,
    })
    .into_response()
}

/// Withdraw a commend while the window is open. Idempotent.
#[utoipa::path(
    delete,
    path = "/v1/crew/{post_id}/commends/{handle}",
    tag = "lfg",
    operation_id = "crew_withdraw_commend",
    params(
        ("post_id" = Uuid, Path, description = "The post you flew on"),
        ("handle" = String, Path, description = "The crewmate"),
    ),
    responses(
        (status = 204, description = "Withdrawn"),
        (status = 404, description = "Not someone you flew with on this post", body = ApiErrorBody),
        (status = 409, description = "The window has closed", body = ApiErrorBody),
        (status = 429, description = "Too many commends", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn withdraw(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(lfg): Extension<Arc<dyn LfgStore>>,
    Extension(commends): Extension<Arc<dyn CommendStore>>,
    Extension(limiter): Extension<Arc<CommendRateLimiter>>,
    Path((post_id, handle)): Path<(Uuid, String)>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if !limiter.check(&me.claimed_handle) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    }
    let (them, _) = match crewmate_on_open_post(
        users.as_ref(),
        social.as_ref(),
        lfg.as_ref(),
        commends.as_ref(),
        &me.claimed_handle,
        post_id,
        &handle,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    match commends
        .withdraw(post_id, &me.claimed_handle, &them.claimed_handle)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal("withdraw", e),
    }
}

/// A profile's public commend totals. Answers 404 wherever the profile's
/// salutes would, so it cannot be used to probe a private profile.
#[utoipa::path(
    get,
    path = "/v1/u/{handle}/commends",
    tag = "social",
    operation_id = "social_profile_commends",
    params(("handle" = String, Path, description = "Profile")),
    responses(
        (status = 200, description = "Commend totals", body = CommendTotals),
        (status = 404, description = "No such profile, or not one you can see", body = ApiErrorBody),
        (status = 503, description = "SpiceDB unavailable", body = ApiErrorBody),
    ),
)]
#[allow(clippy::too_many_arguments)]
pub async fn profile_commends(
    viewer: Option<AuthenticatedUser>,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(commends): Extension<Arc<dyn CommendStore>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Path(handle): Path<String>,
) -> Response {
    let owner = match target(&handle, users.as_ref()).await {
        Ok(u) => u,
        Err(r) if r.status() == StatusCode::BAD_REQUEST => return r,
        Err(_) => return not_found(),
    };
    let viewer_handle = viewer.as_ref().map(|v| v.preferred_username.as_str());
    match blocked_by_owner(social.as_ref(), &owner.claimed_handle, viewer_handle).await {
        Ok(true) => return not_found(),
        Ok(false) => {}
        Err(r) => return r,
    }
    let Some(client) = spicedb.as_ref() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable");
    };
    match profile_visible(
        client,
        social.as_ref(),
        meta.as_ref(),
        audit.as_ref(),
        &restrictions,
        &owner.claimed_handle,
        viewer_handle,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found(),
        Err(r) => return r,
    }
    match commends.totals(&owner.claimed_handle).await {
        Ok(totals) => Json(CommendTotals { totals }).into_response(),
        Err(e) => internal("totals", e),
    }
}

/// Your own commend totals.
#[utoipa::path(
    get,
    path = "/v1/me/commends",
    tag = "social",
    operation_id = "social_my_commends",
    responses((status = 200, description = "Your commend totals", body = CommendTotals)),
    security(("bearer" = [])),
)]
pub async fn my_commends(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(commends): Extension<Arc<dyn CommendStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    match commends.totals(&me.claimed_handle).await {
        Ok(totals) => Json(CommendTotals { totals }).into_response(),
        Err(e) => internal("totals", e),
    }
}
