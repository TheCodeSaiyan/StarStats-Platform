//! o7 salute routes (social phase 2). See `salutes.rs` for the model.
//!
//! - `PUT /v1/u/{handle}/salute` salutes a profile the caller can see.
//! - `DELETE /v1/u/{handle}/salute` takes the caller's salute back.
//! - `GET /v1/u/{handle}/salutes` is the public count; signing in adds
//!   whether you saluted.
//! - `GET /v1/me/salutes` is your own count and which friends saluted.
//!
//! A profile you cannot see, one whose owner blocked you, and an unknown
//! handle all answer the same 404, so none of these routes can be used
//! to learn whether a private profile exists.

use crate::account_restrictions::AccountRestrictionStore;
use crate::api_error::ApiErrorBody;
use crate::audit::AuditLog;
use crate::auth::AuthenticatedUser;
use crate::notifications::{NotificationKind, NotificationStore};
use crate::restriction_guard::{RequireUnrestricted, Sharing};
use crate::salutes::{SaluteRateLimiter, SaluteStore};
use crate::share_metadata::ShareMetadataStore;
use crate::sharing_routes::{check_view_with_expiry, public_profile_restricted};
use crate::social::SocialStore;
use crate::social_routes::{blocked_by_owner, caller, notify, target};
use crate::spicedb::SpicedbClient;
use crate::users::UserStore;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, put},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

pub fn routes() -> Router {
    Router::new()
        .route("/v1/u/{handle}/salute", put(salute).delete(unsalute))
        .route("/v1/u/{handle}/salutes", get(salute_summary))
        .route("/v1/me/salutes", get(my_salutes))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SaluteSummary {
    /// Public: salutes on this profile, leaving out accounts whose
    /// sharing is restricted.
    pub count: i64,
    /// Whether the signed-in caller saluted it. Absent when signed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saluted_by_me: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UnsaluteResponse {
    pub saluted: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MySalutes {
    pub count: i64,
    /// Which of your friends saluted you. Nobody else's name is shown.
    pub friends: Vec<String>,
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

/// Whether `viewer` can see `owner`'s profile: their own, a public one
/// that is not restricted, or one shared with them (as a person, an org
/// member or a friend). Mirrors what the profile routes serve.
async fn profile_visible(
    client: &SpicedbClient,
    social: &dyn SocialStore,
    meta: &dyn ShareMetadataStore,
    audit: &dyn AuditLog,
    restrictions: &Arc<dyn AccountRestrictionStore>,
    owner: &str,
    viewer: Option<&str>,
) -> Result<bool, Response> {
    if viewer.is_some_and(|v| v.eq_ignore_ascii_case(owner)) {
        return Ok(true);
    }
    let public = client.has_public_view(owner).await.map_err(|e| {
        tracing::warn!(error = %e, "spicedb public check failed (salutes)");
        err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable")
    })?;
    if public && !public_profile_restricted(restrictions, owner).await? {
        return Ok(true);
    }
    let Some(viewer) = viewer else {
        return Ok(false);
    };
    check_view_with_expiry(client, social, meta, audit, owner, viewer)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "spicedb view check failed (salutes)");
            err(StatusCode::SERVICE_UNAVAILABLE, "spicedb_unavailable")
        })
}

/// Salute a profile. Idempotent: saluting twice is one salute, and only
/// the first notifies the owner.
#[utoipa::path(
    put,
    path = "/v1/u/{handle}/salute",
    tag = "social",
    operation_id = "social_salute",
    params(("handle" = String, Path, description = "Profile to salute")),
    responses(
        (status = 200, description = "Saluted", body = SaluteSummary),
        (status = 400, description = "Your own profile", body = ApiErrorBody),
        (status = 403, description = "RSI handle not verified, or sharing restricted", body = ApiErrorBody),
        (status = 404, description = "No such profile, or not one you can see", body = ApiErrorBody),
        (status = 429, description = "Too many salutes", body = ApiErrorBody),
        (status = 503, description = "SpiceDB unavailable", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[allow(clippy::too_many_arguments)]
pub async fn salute(
    guard: RequireUnrestricted<Sharing>,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(notes): Extension<Arc<dyn NotificationStore>>,
    Extension(salutes): Extension<Arc<dyn SaluteStore>>,
    Extension(limiter): Extension<Arc<SaluteRateLimiter>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(meta): Extension<Arc<dyn ShareMetadataStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Path(handle): Path<String>,
) -> Response {
    let auth = guard.into_user();
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    // The count is public, so a salute has to cost something to fake:
    // each one needs its own RSI account taken through verification.
    if me.rsi_verified_at.is_none() {
        return err(StatusCode::FORBIDDEN, "rsi_handle_not_verified");
    }
    let them = match target(&handle, users.as_ref()).await {
        Ok(u) => u,
        Err(r) if r.status() == StatusCode::BAD_REQUEST => return r,
        Err(_) => return not_found(),
    };
    if them.id == me.id {
        return err(StatusCode::BAD_REQUEST, "cannot_salute_self");
    }
    if !limiter.check(&me.claimed_handle) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    }
    match blocked_by_owner(
        social.as_ref(),
        &them.claimed_handle,
        Some(&me.claimed_handle),
    )
    .await
    {
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
        &them.claimed_handle,
        Some(&me.claimed_handle),
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return not_found(),
        Err(r) => return r,
    }

    let created = match salutes
        .salute(&me.claimed_handle, &them.claimed_handle)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "salute write failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    if created {
        notify(
            social.as_ref(),
            notes.as_ref(),
            &them.claimed_handle,
            &me,
            NotificationKind::Salute,
            serde_json::json!({}),
        )
        .await;
    }
    match salutes.count(&them.claimed_handle).await {
        Ok(count) => Json(SaluteSummary {
            count,
            saluted_by_me: Some(true),
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "salute count failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// Take your salute back. Idempotent. Answers without a count, so it
/// cannot be used to read the count of a profile you cannot see.
#[utoipa::path(
    delete,
    path = "/v1/u/{handle}/salute",
    tag = "social",
    operation_id = "social_unsalute",
    params(("handle" = String, Path, description = "Profile to unsalute")),
    responses(
        (status = 200, description = "Not saluting", body = UnsaluteResponse),
        (status = 400, description = "Invalid handle", body = ApiErrorBody),
        (status = 429, description = "Too many salutes", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn unsalute(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(salutes): Extension<Arc<dyn SaluteStore>>,
    Extension(limiter): Extension<Arc<SaluteRateLimiter>>,
    Path(handle): Path<String>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let handle = handle.trim();
    if !crate::users::validate_handle(handle) {
        return err(StatusCode::BAD_REQUEST, "invalid_handle");
    }
    // Shares the salute budget: saluting and unsaluting in a loop is how
    // someone would flood the owner with notifications.
    if !limiter.check(&me.claimed_handle) {
        return err(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    }
    if let Err(e) = salutes.unsalute(&me.claimed_handle, handle).await {
        tracing::error!(error = %e, "unsalute failed");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }
    Json(UnsaluteResponse { saluted: false }).into_response()
}

/// A profile's salute count. Public for a public profile; signed in, it
/// also covers profiles shared with you and says whether you saluted.
#[utoipa::path(
    get,
    path = "/v1/u/{handle}/salutes",
    tag = "social",
    operation_id = "social_salute_summary",
    params(("handle" = String, Path, description = "Profile")),
    responses(
        (status = 200, description = "Salute count", body = SaluteSummary),
        (status = 404, description = "No such profile, or not one you can see", body = ApiErrorBody),
        (status = 503, description = "SpiceDB unavailable", body = ApiErrorBody),
    ),
)]
#[allow(clippy::too_many_arguments)]
pub async fn salute_summary(
    viewer: Option<AuthenticatedUser>,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(salutes): Extension<Arc<dyn SaluteStore>>,
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
    let count = match salutes.count(&owner.claimed_handle).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "salute count failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let saluted_by_me = match viewer_handle {
        Some(v) => match salutes.has_saluted(v, &owner.claimed_handle).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::warn!(error = %e, "has_saluted failed; omitting saluted_by_me");
                None
            }
        },
        None => None,
    };
    Json(SaluteSummary {
        count,
        saluted_by_me,
    })
    .into_response()
}

/// Your own salutes: the count, and which of your friends saluted you.
#[utoipa::path(
    get,
    path = "/v1/me/salutes",
    tag = "social",
    operation_id = "social_my_salutes",
    responses((status = 200, description = "Your salutes", body = MySalutes)),
    security(("bearer" = [])),
)]
pub async fn my_salutes(
    auth: AuthenticatedUser,
    Extension(users): Extension<Arc<dyn UserStore>>,
    Extension(social): Extension<Arc<dyn SocialStore>>,
    Extension(salutes): Extension<Arc<dyn SaluteStore>>,
) -> Response {
    let me = match caller(&auth, users.as_ref()).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let count = match salutes.count(&me.claimed_handle).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = %e, "salute count failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let friends: Vec<String> = match social.list_friends(&me.claimed_handle).await {
        Ok(f) => f.into_iter().map(|f| f.handle).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "list_friends failed; returning no friend salutes");
            Vec::new()
        }
    };
    let friends = match salutes.saluters_among(&me.claimed_handle, &friends).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "saluters_among failed; returning no friend salutes");
            Vec::new()
        }
    };
    Json(MySalutes { count, friends }).into_response()
}
