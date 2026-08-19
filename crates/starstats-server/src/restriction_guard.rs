//! Request guard for account restrictions.
//!
//! Mirrors `admin_routes::extract_with_role` step for step: extract the
//! authenticated user, parse `sub` as a UUID, pull the store off request
//! extensions, load the record, enforce.
//!
//! The guard goes in the ROUTE DEFINITION, not inside handlers. A
//! middleware that mapped paths to capabilities was considered and
//! rejected: a new route would silently get no guard, which is exactly
//! how this project ended up with a "Suspend owner" button that
//! suspended nothing.
//!
//! FAIL CLOSED. Every error path here denies. A restriction check that
//! falls through to "allowed" when the store is unreachable is not a
//! degraded guard, it is an absent one — and it would fail in precisely
//! the circumstances where someone is being actively abusive.

use crate::account_restrictions::{AccountRestrictionStore, Capability};
use crate::auth::AuthenticatedUser;
use axum::async_trait;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Json, Response},
    RequestPartsExt,
};
use std::marker::PhantomData;
use std::sync::Arc;
use uuid::Uuid;

/// Compile-time capability selector, so the route reads
/// `RequireUnrestricted<Sharing>` and a typo is a build error rather
/// than a silently mismatched string.
pub trait CapabilityMarker: Send + Sync + 'static {
    const CAPABILITY: Capability;
}

macro_rules! capability_marker {
    ($name:ident, $variant:expr) => {
        pub struct $name;
        impl CapabilityMarker for $name {
            const CAPABILITY: Capability = $variant;
        }
    };
}

capability_marker!(Ingest, Capability::Ingest);
capability_marker!(Sharing, Capability::Sharing);
capability_marker!(PublicProfile, Capability::PublicProfile);
capability_marker!(Submissions, Capability::Submissions);

/// Extractor that rejects when the caller is barred from `C`.
pub struct RequireUnrestricted<C: CapabilityMarker>(pub AuthenticatedUser, PhantomData<C>);

impl<C: CapabilityMarker> RequireUnrestricted<C> {
    /// Take the authenticated user out of the guard.
    ///
    /// Handlers use this instead of ALSO taking an `AuthenticatedUser`
    /// parameter: the guard has already extracted and verified one, and
    /// a second extractor would verify the same JWT twice per request.
    pub fn into_user(self) -> AuthenticatedUser {
        self.0
    }
}

fn denied(capability: Capability, reason: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": "account_restricted",
            "capability": capability.as_str(),
            // Surfaced deliberately: a bare 403 leaves the user
            // guessing, and the tray has nothing honest to show.
            "reason": reason,
        })),
    )
        .into_response()
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({ "error": "restriction_check_unavailable" })),
    )
        .into_response()
}

#[async_trait]
impl<S, C> FromRequestParts<S> for RequireUnrestricted<C>
where
    S: Send + Sync,
    C: CapabilityMarker,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user = parts
            .extract::<AuthenticatedUser>()
            .await
            .map_err(IntoResponse::into_response)?;

        // A malformed sub is not a reason to let the request through.
        let user_id = Uuid::parse_str(&user.sub).map_err(|_| {
            tracing::error!(sub = %user.sub, "restriction guard: unparseable sub; denying");
            unavailable()
        })?;

        let store = parts
            .extensions
            .get::<Arc<dyn AccountRestrictionStore>>()
            .cloned()
            .ok_or_else(|| {
                tracing::error!(
                    "AccountRestrictionStore extension missing on a restriction-gated route; \
                     denying. This is a wiring bug: the route is unprotected without it."
                );
                unavailable()
            })?;

        match store.effective(user_id).await {
            Ok(Some(restriction)) if restriction.blocks(C::CAPABILITY) => {
                Err(denied(C::CAPABILITY, &restriction.reason))
            }
            Ok(_) => Ok(RequireUnrestricted(user, PhantomData)),
            Err(e) => {
                tracing::error!(error = %e, %user_id, "restriction lookup failed; denying");
                Err(unavailable())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_restrictions::{test_support::MemoryAccountRestrictionStore, Restriction};
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use axum::{body::Body, http::Request, routing::post, Router};
    use chrono::{Duration, Utc};
    use tower::ServiceExt;

    async fn ok_handler(_: RequireUnrestricted<Sharing>) -> &'static str {
        "ok"
    }

    async fn ingest_handler(_: RequireUnrestricted<Ingest>) -> &'static str {
        "ok"
    }

    fn restriction(sharing: bool, expires: Option<chrono::DateTime<Utc>>) -> Restriction {
        Restriction {
            ingest_blocked: false,
            sharing_blocked: sharing,
            public_profile_blocked: false,
            submissions_blocked: false,
            reason: "spamming share invites".into(),
            restricted_by: "modhandle".into(),
            restricted_at: Utc::now(),
            expires_at: expires,
        }
    }

    fn app(store: Arc<dyn AccountRestrictionStore>, verifier: AuthVerifier) -> Router {
        Router::new()
            .route("/sharing", post(ok_handler))
            .route("/ingest", post(ingest_handler))
            .layer(axum::Extension(store))
            .layer(axum::Extension(Arc::new(verifier)))
    }

    fn req(path: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn token_for(issuer: &TokenIssuer, id: Uuid) -> String {
        issuer.sign_user(&id.to_string(), "TestUser").unwrap()
    }

    #[tokio::test]
    async fn unrestricted_user_passes() {
        let id = Uuid::now_v7();
        let (issuer, verifier) = fresh_pair();
        let store: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::new());
        let resp = app(store, verifier)
            .oneshot(req("/sharing", &token_for(&issuer, id)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn restricted_capability_is_denied_with_the_reason() {
        let id = Uuid::now_v7();
        let (issuer, verifier) = fresh_pair();
        let store: Arc<dyn AccountRestrictionStore> = Arc::new(
            MemoryAccountRestrictionStore::new().with_restriction(id, restriction(true, None)),
        );
        let resp = app(store, verifier)
            .oneshot(req("/sharing", &token_for(&issuer, id)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "account_restricted");
        assert_eq!(body["capability"], "sharing");
        // The reason reaches the caller so the tray can say something
        // truthful instead of showing a bare permission error.
        assert_eq!(body["reason"], "spamming share invites");
    }

    #[tokio::test]
    async fn a_different_capability_is_not_denied() {
        // This is what separates "limit" from "suspend". If the guard
        // ignored its capability parameter, every targeted limit would
        // silently become a full suspension.
        let id = Uuid::now_v7();
        let (issuer, verifier) = fresh_pair();
        let store: Arc<dyn AccountRestrictionStore> = Arc::new(
            MemoryAccountRestrictionStore::new().with_restriction(id, restriction(true, None)),
        );
        let resp = app(store, verifier)
            .oneshot(req("/ingest", &token_for(&issuer, id)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn expired_restriction_does_not_deny() {
        let id = Uuid::now_v7();
        let (issuer, verifier) = fresh_pair();
        let expired = restriction(true, Some(Utc::now() - Duration::days(1)));
        let store: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::new().with_restriction(id, expired));
        let resp = app(store, verifier)
            .oneshot(req("/sharing", &token_for(&issuer, id)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn store_error_denies_rather_than_allowing() {
        // THE load-bearing test of this module. A restriction check
        // that permits when it cannot reach its store is not a guard.
        let id = Uuid::now_v7();
        let (issuer, verifier) = fresh_pair();
        let store: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::failing());
        let resp = app(store, verifier)
            .oneshot(req("/sharing", &token_for(&issuer, id)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn missing_store_extension_denies() {
        // A wiring mistake must be loud, not permissive.
        let id = Uuid::now_v7();
        let (issuer, verifier) = fresh_pair();
        let router = Router::new()
            .route("/sharing", post(ok_handler))
            .layer(axum::Extension(Arc::new(verifier)));
        let resp = router
            .oneshot(req("/sharing", &token_for(&issuer, id)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unauthenticated_request_is_rejected_before_any_store_lookup() {
        let (_issuer, verifier) = fresh_pair();
        let store: Arc<dyn AccountRestrictionStore> =
            Arc::new(MemoryAccountRestrictionStore::failing());
        let resp = app(store, verifier)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sharing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
