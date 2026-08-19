//! Moderator endpoints for account restrictions.
//!
//!   PUT    /v1/admin/users/:id/restrictions   — restrict / suspend
//!   DELETE /v1/admin/users/:id/restrictions   — reinstate
//!
//! Moderator-gated for both: moderators triage the abuse queue, so
//! requiring an admin for every case would mean escalating everything.
//! Deletion (the irreversible action) stays admin-only and lives
//! elsewhere.
//!
//! Suspension REVOKES existing shares rather than merely blocking new
//! ones. That is not incidental — the share-report dialog has promised
//! it since before it was true, and blocking creation while leaving
//! current grants live would leave the promise still broken.
//!
//! Revocation is NOT undone by reinstating. The grants are deleted, not
//! paused; the user re-creates them. The admin UI says so explicitly so
//! nobody reads Reinstate as an undo button.

use crate::account_restrictions::{AccountRestrictionStore, Restriction};
use crate::admin_routes::RequireModerator;
use crate::audit::{AuditEntry, AuditLog};
use crate::spicedb::SpicedbClient;
use crate::staff_roles::{StaffRole, StaffRoleStore};
use crate::users::UserStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::put,
    Extension, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

const REASON_MAX_LEN: usize = 280;

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct RestrictionRequest {
    #[serde(default)]
    pub ingest_blocked: bool,
    #[serde(default)]
    pub sharing_blocked: bool,
    #[serde(default)]
    pub public_profile_blocked: bool,
    #[serde(default)]
    pub submissions_blocked: bool,
    /// Required, non-empty. Shown to the restricted user, so it is
    /// both the audit record and user-facing copy.
    pub reason: String,
    /// `None` means "until lifted".
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AdminRestrictionDto {
    pub ingest_blocked: bool,
    pub sharing_blocked: bool,
    pub public_profile_blocked: bool,
    pub submissions_blocked: bool,
    pub reason: String,
    pub restricted_by: String,
    pub restricted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// True when every capability is blocked. Derived, not stored —
    /// there is no separate suspended flag to drift out of sync.
    pub is_suspension: bool,
    /// How many share grants were revoked as part of this action.
    /// Zero for a targeted limit; also zero when SpiceDB is unavailable,
    /// which is why the field is reported rather than assumed.
    pub shares_revoked: u32,
}

fn err(status: StatusCode, code: &str) -> Response {
    (status, Json(serde_json::json!({ "error": code }))).into_response()
}

impl AdminRestrictionDto {
    pub fn from_restriction(r: &Restriction, shares_revoked: u32) -> Self {
        Self {
            ingest_blocked: r.ingest_blocked,
            sharing_blocked: r.sharing_blocked,
            public_profile_blocked: r.public_profile_blocked,
            submissions_blocked: r.submissions_blocked,
            reason: r.reason.clone(),
            restricted_by: r.restricted_by.clone(),
            restricted_at: r.restricted_at,
            expires_at: r.expires_at,
            is_suspension: r.is_suspension(),
            shares_revoked,
        }
    }
}

/// Revoke every outbound share the handle owns, and drop their public
/// profile relationship.
///
/// Returns how many user-share grants went away. Best-effort per grant:
/// one failing delete must not abort the rest, or a single stuck
/// relationship would leave the remaining shares live.
async fn revoke_all_shares(client: &SpicedbClient, handle: &str) -> u32 {
    let mut revoked = 0u32;

    match client.list_share_with_user(handle).await {
        Ok(recipients) => {
            for recipient in recipients {
                match client.delete_share_with_user(handle, &recipient).await {
                    Ok(()) => revoked += 1,
                    Err(e) => {
                        tracing::error!(error = %e, %handle, %recipient, "suspend: share revoke failed")
                    }
                }
            }
        }
        Err(e) => tracing::error!(error = %e, %handle, "suspend: listing user shares failed"),
    }

    match client.list_share_with_org(handle).await {
        Ok(orgs) => {
            for slug in orgs {
                if let Err(e) = client.delete_share_with_org(handle, &slug).await {
                    tracing::error!(error = %e, %handle, %slug, "suspend: org share revoke failed");
                }
            }
        }
        Err(e) => tracing::error!(error = %e, %handle, "suspend: listing org shares failed"),
    }

    // A suspended account's profile must not stay publicly listed. The
    // read filters already hide it, but leaving the SpiceDB row would
    // mean the profile silently reappears the moment the restriction
    // expires or is lifted.
    if let Err(e) = client.delete_public_view(handle).await {
        tracing::error!(error = %e, %handle, "suspend: delete_public_view failed");
    }

    revoked
}

/// Shared by the PUT handler and the share-report resolution path.
///
/// `actor_*` describe the moderator; `target_*` the account. Returns
/// the stored restriction plus the number of shares revoked.
#[allow(clippy::too_many_arguments)]
pub async fn apply_restriction(
    restrictions: &Arc<dyn AccountRestrictionStore>,
    spicedb: &Arc<Option<SpicedbClient>>,
    audit: &Arc<dyn AuditLog>,
    target_user_id: Uuid,
    target_handle: &str,
    actor_sub: &str,
    actor_handle: &str,
    restriction: Restriction,
) -> Result<(Restriction, u32), Response> {
    if let Err(e) = restrictions.upsert(target_user_id, &restriction).await {
        tracing::error!(error = %e, %target_user_id, "restriction upsert failed");
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "internal"));
    }

    // Only a full suspension revokes. A targeted limit (e.g. "stop them
    // sharing") blocks new grants via the request guard and leaves the
    // existing ones, which is what "limit" means.
    let mut shares_revoked = 0u32;
    if restriction.is_suspension() {
        match spicedb.as_ref() {
            Some(client) => shares_revoked = revoke_all_shares(client, target_handle).await,
            None => tracing::error!(
                %target_handle,
                "suspension applied but SpiceDB unavailable; existing shares NOT revoked"
            ),
        }
    }

    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(actor_sub.to_string()),
            actor_handle: Some(actor_handle.to_string()),
            action: "account.restricted".to_string(),
            payload: serde_json::json!({
                "target_user_id": target_user_id,
                "target_handle": target_handle,
                "ingest_blocked": restriction.ingest_blocked,
                "sharing_blocked": restriction.sharing_blocked,
                "public_profile_blocked": restriction.public_profile_blocked,
                "submissions_blocked": restriction.submissions_blocked,
                "is_suspension": restriction.is_suspension(),
                "reason": restriction.reason,
                "expires_at": restriction.expires_at,
                "shares_revoked": shares_revoked,
            }),
        })
        .await
    {
        tracing::warn!(error = %e, "audit append failed (account.restricted)");
    }

    Ok((restriction, shares_revoked))
}

#[utoipa::path(
    put,
    path = "/v1/admin/users/{id}/restrictions",
    tag = "admin",
    params(("id" = String, Path, description = "User UUID")),
    request_body = RestrictionRequest,
    responses(
        (status = 200, description = "Restriction applied", body = AdminRestrictionDto),
        (status = 400, description = "Missing reason, or targeting yourself"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role, or target is an admin"),
        (status = 404, description = "User not found"),
    ),
    security(("BearerAuth" = []))
)]
#[allow(clippy::too_many_arguments)]
pub async fn set_restrictions<U: UserStore>(
    RequireModerator(actor): RequireModerator,
    State(users): State<Arc<U>>,
    Extension(staff): Extension<Arc<dyn StaffRoleStore>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id_str): Path<String>,
    Json(body): Json<RestrictionRequest>,
) -> Response {
    let reason = body.reason.trim();
    if reason.is_empty() {
        return err(StatusCode::BAD_REQUEST, "reason_required");
    }
    if reason.chars().count() > REASON_MAX_LEN {
        return err(StatusCode::BAD_REQUEST, "reason_too_long");
    }
    if !body.ingest_blocked
        && !body.sharing_blocked
        && !body.public_profile_blocked
        && !body.submissions_blocked
    {
        // An all-false restriction is a second way to spell
        // "unrestricted". Reinstate (DELETE) is the one way.
        return err(StatusCode::BAD_REQUEST, "no_capabilities_selected");
    }

    let Ok(target_id) = Uuid::parse_str(&id_str) else {
        return err(StatusCode::NOT_FOUND, "user_not_found");
    };

    // Self-protection, mirroring cannot_revoke_own_admin.
    if actor.sub == id_str {
        return err(StatusCode::BAD_REQUEST, "cannot_restrict_yourself");
    }

    let target = match users.find_by_id(target_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return err(StatusCode::NOT_FOUND, "user_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    match staff.list_active_for_user(target_id).await {
        Ok(roles) if roles.has(StaffRole::Admin) => {
            return err(StatusCode::FORBIDDEN, "cannot_restrict_an_admin");
        }
        Ok(_) => {}
        Err(e) => {
            // Fail closed: if we cannot tell whether the target is an
            // admin, do not restrict them.
            tracing::error!(error = %e, "staff role lookup failed; refusing to restrict");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }

    let restriction = Restriction {
        ingest_blocked: body.ingest_blocked,
        sharing_blocked: body.sharing_blocked,
        public_profile_blocked: body.public_profile_blocked,
        submissions_blocked: body.submissions_blocked,
        reason: reason.to_string(),
        restricted_by: actor.preferred_username.clone(),
        restricted_at: Utc::now(),
        expires_at: body.expires_at,
    };

    match apply_restriction(
        &restrictions,
        &spicedb,
        &audit,
        target_id,
        &target.claimed_handle,
        &actor.sub,
        &actor.preferred_username,
        restriction,
    )
    .await
    {
        Ok((stored, revoked)) => (
            StatusCode::OK,
            Json(AdminRestrictionDto::from_restriction(&stored, revoked)),
        )
            .into_response(),
        Err(resp) => resp,
    }
}

#[utoipa::path(
    delete,
    path = "/v1/admin/users/{id}/restrictions",
    tag = "admin",
    params(("id" = String, Path, description = "User UUID")),
    responses(
        (status = 200, description = "Reinstated (or already unrestricted)"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks moderator role"),
        (status = 404, description = "User not found"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn clear_restrictions<U: UserStore>(
    RequireModerator(actor): RequireModerator,
    State(users): State<Arc<U>>,
    Extension(restrictions): Extension<Arc<dyn AccountRestrictionStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Path(id_str): Path<String>,
) -> Response {
    let Ok(target_id) = Uuid::parse_str(&id_str) else {
        return err(StatusCode::NOT_FOUND, "user_not_found");
    };

    let target = match users.find_by_id(target_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return err(StatusCode::NOT_FOUND, "user_not_found"),
        Err(e) => {
            tracing::error!(error = %e, "find_by_id failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    let removed = match restrictions.lift(target_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, %target_id, "restriction lift failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    if removed {
        if let Err(e) = audit
            .append(AuditEntry {
                actor_sub: Some(actor.sub.clone()),
                actor_handle: Some(actor.preferred_username.clone()),
                action: "account.reinstated".to_string(),
                payload: serde_json::json!({
                    "target_user_id": target_id,
                    "target_handle": target.claimed_handle,
                    // Revoked shares are NOT restored — the grants were
                    // deleted, not paused.
                    "shares_restored": 0,
                }),
            })
            .await
        {
            tracing::warn!(error = %e, "audit append failed (account.reinstated)");
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "reinstated": removed })),
    )
        .into_response()
}

pub fn router<U: UserStore>(users: Arc<U>) -> Router {
    Router::new()
        .route(
            "/v1/admin/users/:id/restrictions",
            put(set_restrictions::<U>).delete(clear_restrictions::<U>),
        )
        .with_state(users)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_restrictions::test_support::MemoryAccountRestrictionStore;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::users::hash_password;
    use crate::users::test_support::MemoryUserStore;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    struct Env {
        app: Router,
        restrictions: Arc<dyn AccountRestrictionStore>,
        issuer: TokenIssuer,
        staff: Arc<MemoryStaffRoleStore>,
        users: Arc<MemoryUserStore>,
    }

    async fn env() -> Env {
        let users = Arc::new(MemoryUserStore::default());
        let staff_mem = Arc::new(MemoryStaffRoleStore::new());
        let staff: Arc<dyn StaffRoleStore> = staff_mem.clone();
        let restrictions: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::new());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        // SpiceDB absent: suspension still applies, it just cannot
        // revoke. The handler logs that loudly and reports
        // shares_revoked = 0 rather than implying it revoked.
        let spicedb: Arc<Option<SpicedbClient>> = Arc::new(None);
        let (issuer, verifier) = fresh_pair();

        let app = router(users.clone())
            .layer(Extension(staff))
            .layer(Extension(restrictions.clone()))
            .layer(Extension(audit))
            .layer(Extension(spicedb))
            .layer(Extension(Arc::new(verifier) as Arc<AuthVerifier>));

        Env {
            app,
            restrictions,
            issuer,
            staff: staff_mem,
            users,
        }
    }

    async fn seed_user(users: &MemoryUserStore, email: &str, handle: &str) -> Uuid {
        let phc = hash_password("password-123-abcdef").unwrap();
        users.create(email, &phc, handle).await.unwrap().id
    }

    async fn moderator(env: &Env, handle: &str) -> (Uuid, String) {
        let id = Uuid::now_v7();
        env.staff
            .grant(id, StaffRole::Moderator, None, None)
            .await
            .unwrap();
        (id, env.issuer.sign_user(&id.to_string(), handle).unwrap())
    }

    fn put_req(token: &str, target: Uuid, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("PUT")
            .uri(format!("/v1/admin/users/{target}/restrictions"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    fn suspend_body() -> serde_json::Value {
        json!({
            "ingest_blocked": true,
            "sharing_blocked": true,
            "public_profile_blocked": true,
            "submissions_blocked": true,
            "reason": "harassment"
        })
    }

    #[tokio::test]
    async fn moderator_can_restrict_and_the_store_reflects_it() {
        // The assertion that matters is the STORED state, not the 200.
        // A handler that returned 200 and wrote nothing is exactly the
        // bug this whole feature replaces.
        let env = env().await;
        let target = seed_user(&env.users, "t@example.test", "Target").await;
        let (_, token) = moderator(&env, "modhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, target, suspend_body()))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let stored = env
            .restrictions
            .effective(target)
            .await
            .unwrap()
            .expect("restriction should be stored");
        assert!(stored.is_suspension());
        assert_eq!(stored.reason, "harassment");
        assert_eq!(stored.restricted_by, "modhandle");
    }

    #[tokio::test]
    async fn reinstating_deletes_the_row() {
        let env = env().await;
        let target = seed_user(&env.users, "t@example.test", "Target").await;
        let (_, token) = moderator(&env, "modhandle").await;
        env.app
            .clone()
            .oneshot(put_req(&token, target, suspend_body()))
            .await
            .unwrap();

        let resp = env
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/admin/users/{target}/restrictions"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(env.restrictions.effective(target).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cannot_restrict_yourself() {
        let env = env().await;
        let (mod_id, token) = moderator(&env, "modhandle").await;
        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, mod_id, suspend_body()))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cannot_restrict_an_admin() {
        let env = env().await;
        let target = seed_user(&env.users, "a@example.test", "AdminTarget").await;
        env.staff
            .grant(target, StaffRole::Admin, None, None)
            .await
            .unwrap();
        let (_, token) = moderator(&env, "modhandle").await;

        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, target, suspend_body()))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(env.restrictions.effective(target).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reason_is_required_and_nothing_is_written_without_one() {
        let env = env().await;
        let target = seed_user(&env.users, "t@example.test", "Target").await;
        let (_, token) = moderator(&env, "modhandle").await;
        let body = json!({ "sharing_blocked": true, "reason": "   " });
        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, target, body))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(env.restrictions.effective(target).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_all_false_restriction_is_rejected() {
        // All-false would be a second way to spell "unrestricted".
        // Reinstate (DELETE) is the one way.
        let env = env().await;
        let target = seed_user(&env.users, "t@example.test", "Target").await;
        let (_, token) = moderator(&env, "modhandle").await;
        let body = json!({ "reason": "nothing in particular" });
        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, target, body))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_targeted_limit_is_not_a_suspension() {
        let env = env().await;
        let target = seed_user(&env.users, "t@example.test", "Target").await;
        let (_, token) = moderator(&env, "modhandle").await;
        let body = json!({ "sharing_blocked": true, "reason": "spamming invites" });

        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, target, body))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let stored = env.restrictions.effective(target).await.unwrap().unwrap();
        assert!(!stored.is_suspension());
        assert!(stored.sharing_blocked);
        assert!(!stored.ingest_blocked);
    }

    #[tokio::test]
    async fn a_plain_user_cannot_restrict_anyone() {
        let env = env().await;
        let target = seed_user(&env.users, "t@example.test", "Target").await;
        let token = env
            .issuer
            .sign_user(&Uuid::now_v7().to_string(), "nobody")
            .unwrap();
        let resp = env
            .app
            .clone()
            .oneshot(put_req(&token, target, suspend_body()))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(env.restrictions.effective(target).await.unwrap().is_none());
    }
}
