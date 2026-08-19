//! Admin changelog routes (Phase 7, spec §8.4).
//!
//! Three routes, all gated on the `admin` staff role via
//! [`RequireAdmin`]:
//!
//!   - `GET  /v1/admin/roadmap/changelog/drafts`     — list drafts.
//!   - `POST /v1/admin/roadmap/changelog/:id/publish`— flip a draft to
//!                                                     published.
//!   - `POST /v1/admin/roadmap/changelog/:id/edit`   — edit title/body
//!                                                     on a draft.
//!
//! Publish is gated on `admin` rather than `moderator` because the
//! changelog is a user-facing content surface — a public post going
//! out unreviewed is a higher-blast-radius mistake than triaging a
//! report. Phase 9 can soften this if the workflow demands.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::changelog::{self, ChangelogError};
use super::models::RoadmapChangelogEntry;
use super::store::{RoadmapStore, RoadmapStoreError};
use crate::admin_routes::RequireAdmin;

// ---------- DTOs -----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangelogDraftDto {
    pub id: Uuid,
    pub roadmap_item_id: Uuid,
    pub channel: String,
    pub title: String,
    pub body: String,
    pub previous_shipped_sha: Option<String>,
    pub shipped_sha: Option<String>,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub published_by: Option<String>,
}

impl From<RoadmapChangelogEntry> for ChangelogDraftDto {
    fn from(e: RoadmapChangelogEntry) -> Self {
        Self {
            id: e.id,
            roadmap_item_id: e.roadmap_item_id,
            channel: e.channel.as_str().to_string(),
            title: e.title,
            body: e.body,
            previous_shipped_sha: e.previous_shipped_sha,
            shipped_sha: e.shipped_sha,
            created_at: e.created_at,
            published_at: e.published_at,
            published_by: e.published_by,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangelogDraftsResponse {
    pub drafts: Vec<ChangelogDraftDto>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct EditChangelogRequest {
    pub title: String,
    pub body: String,
}

// ---------- helpers --------------------------------------------------------

/// Prefer the JWT's `preferred_username` (the human-readable claimed
/// handle); fall back to `sub` if for some reason the claim is blank
/// (defensive — issuer always sets it).
fn publisher_label(auth: &crate::auth::AuthenticatedUser) -> String {
    let handle = auth.preferred_username.trim();
    if handle.is_empty() {
        auth.sub.clone()
    } else {
        handle.to_string()
    }
}

fn store_500(e: RoadmapStoreError, ctx: &'static str) -> Response {
    tracing::warn!(error = %e, ctx, "admin changelog store error");
    (StatusCode::INTERNAL_SERVER_ERROR, "store error").into_response()
}

// ---------- handlers -------------------------------------------------------

/// GET /v1/admin/roadmap/changelog/drafts
#[utoipa::path(
    get,
    path = "/v1/admin/roadmap/changelog/drafts",
    tag = "admin",
    responses(
        (status = 200, description = "Pending draft entries", body = ChangelogDraftsResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn list_drafts(
    _: RequireAdmin,
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
) -> Response {
    match store.list_changelog_drafts().await {
        Ok(rows) => (
            StatusCode::OK,
            Json(ChangelogDraftsResponse {
                drafts: rows.into_iter().map(Into::into).collect(),
            }),
        )
            .into_response(),
        Err(e) => store_500(e, "list_drafts"),
    }
}

/// POST /v1/admin/roadmap/changelog/{id}/publish
#[utoipa::path(
    post,
    path = "/v1/admin/roadmap/changelog/{id}/publish",
    tag = "admin",
    params(("id" = Uuid, Path, description = "Draft entry id")),
    responses(
        (status = 200, description = "Entry published", body = ChangelogDraftDto),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 404, description = "Draft not found or already published"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn publish_draft(
    RequireAdmin(user): RequireAdmin,
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let publisher = publisher_label(&user);
    match changelog::publish_with_notifications(&*store, id, &publisher).await {
        Ok(entry) => (StatusCode::OK, Json(ChangelogDraftDto::from(entry))).into_response(),
        Err(ChangelogError::NotFound) => (StatusCode::NOT_FOUND, "not_found").into_response(),
        Err(ChangelogError::Store(e)) => store_500(e, "publish_draft"),
    }
}

/// POST /v1/admin/roadmap/changelog/{id}/edit
#[utoipa::path(
    post,
    path = "/v1/admin/roadmap/changelog/{id}/edit",
    tag = "admin",
    request_body = EditChangelogRequest,
    params(("id" = Uuid, Path, description = "Draft entry id")),
    responses(
        (status = 200, description = "Draft updated", body = ChangelogDraftDto),
        (status = 400, description = "Empty title or body"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 403, description = "Caller lacks admin role"),
        (status = 404, description = "Draft not found or already published"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn edit_draft(
    _: RequireAdmin,
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Path(id): Path<Uuid>,
    Json(body): Json<EditChangelogRequest>,
) -> Response {
    let title = body.title.trim();
    let body_text = body.body.trim();
    if title.is_empty() || body_text.is_empty() {
        return (StatusCode::BAD_REQUEST, "title_and_body_required").into_response();
    }
    match store.edit_changelog_draft(id, title, body_text).await {
        Ok(entry) => (StatusCode::OK, Json(ChangelogDraftDto::from(entry))).into_response(),
        Err(RoadmapStoreError::NotFound) => (StatusCode::NOT_FOUND, "not_found").into_response(),
        Err(e) => store_500(e, "edit_draft"),
    }
}

// ---------- router ---------------------------------------------------------

/// Build the admin changelog sub-router. The caller layers the
/// `Arc<dyn RoadmapStore>`, `Arc<AuthVerifier>`, and
/// `Arc<dyn StaffRoleStore>` extensions on the parent app.
pub fn router() -> Router {
    Router::new()
        .route("/v1/admin/roadmap/changelog/drafts", get(list_drafts))
        .route(
            "/v1/admin/roadmap/changelog/:id/publish",
            post(publish_draft),
        )
        .route("/v1/admin/roadmap/changelog/:id/edit", post(edit_draft))
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::changelog::draft_for_shipped_transition;
    use super::super::models::ChannelName;
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::UpsertRoadmapItem;
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use crate::staff_roles::test_support::MemoryStaffRoleStore;
    use crate::staff_roles::{StaffRole, StaffRoleStore};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn seed_item(store: &MemoryRoadmapStore, slug: &str, title: &str) -> Uuid {
        let surfaces: Vec<String> = vec![];
        store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: &format!("PVTI_{slug}"),
                slug,
                title,
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public: true,
            })
            .await
            .unwrap()
            .id
    }

    fn build_app(
        store: Arc<dyn RoadmapStore>,
        staff: Arc<dyn StaffRoleStore>,
        verifier: Arc<AuthVerifier>,
    ) -> Router {
        router()
            .layer(Extension(store))
            .layer(Extension(staff))
            .layer(Extension(verifier))
    }

    async fn admin_token(
        staff: &MemoryStaffRoleStore,
        issuer: &TokenIssuer,
        handle: &str,
    ) -> String {
        let user_id = Uuid::now_v7();
        staff
            .grant(user_id, StaffRole::Admin, None, None)
            .await
            .unwrap();
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign admin token")
    }

    fn plain_token(issuer: &TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&Uuid::now_v7().to_string(), handle)
            .expect("sign plain token")
    }

    fn get(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn post_empty(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn post_json(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn drafts_listing_returns_pending_entries_for_admin() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "feature-x", "Feature X").await;
        // Seed two drafts.
        draft_for_shipped_transition(
            &*memory,
            item_id,
            ChannelName::Live,
            None,
            "sha1",
            "Feature X",
        )
        .await
        .unwrap();
        draft_for_shipped_transition(
            &*memory,
            item_id,
            ChannelName::Beta,
            None,
            "sha2",
            "Feature X",
        )
        .await
        .unwrap();
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let tok = admin_token(&staff, &issuer, "admin").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let app = build_app(store, staff_dyn, Arc::new(verifier));

        let resp = app
            .oneshot(get("/v1/admin/roadmap/changelog/drafts", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ChangelogDraftsResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.drafts.len(), 2);
        for d in &body.drafts {
            assert!(d.published_at.is_none());
        }
    }

    #[tokio::test]
    async fn drafts_listing_rejects_non_admin_with_403() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let tok = plain_token(&issuer, "rando");
        let store: Arc<dyn RoadmapStore> = memory;
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let app = build_app(store, staff_dyn, Arc::new(verifier));

        let resp = app
            .oneshot(get("/v1/admin/roadmap/changelog/drafts", &tok))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn publish_moves_draft_to_published_and_returns_payload() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "pub-target", "Publish target").await;
        let draft = draft_for_shipped_transition(
            &*memory,
            item_id,
            ChannelName::Live,
            None,
            "sha1",
            "Publish target",
        )
        .await
        .unwrap();
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let tok = admin_token(&staff, &issuer, "admin-handle").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let app = build_app(store, staff_dyn, Arc::new(verifier));

        let uri = format!("/v1/admin/roadmap/changelog/{}/publish", draft.id);
        let resp = app.oneshot(post_empty(&uri, &tok)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ChangelogDraftDto = serde_json::from_slice(&bytes).unwrap();
        assert!(body.published_at.is_some());
        assert_eq!(body.published_by.as_deref(), Some("admin-handle"));
    }

    #[tokio::test]
    async fn publish_already_published_returns_404() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "dbl", "Dbl publish").await;
        let draft = draft_for_shipped_transition(
            &*memory,
            item_id,
            ChannelName::Live,
            None,
            "sha1",
            "Dbl publish",
        )
        .await
        .unwrap();
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let tok = admin_token(&staff, &issuer, "admin").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let verifier_arc = Arc::new(verifier);
        let app = router()
            .layer(Extension(store.clone()))
            .layer(Extension(staff_dyn.clone()))
            .layer(Extension(verifier_arc.clone()));

        let uri = format!("/v1/admin/roadmap/changelog/{}/publish", draft.id);
        let resp = app.oneshot(post_empty(&uri, &tok)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Second publish on the same draft -- already published.
        let app2 = router()
            .layer(Extension(store))
            .layer(Extension(staff_dyn))
            .layer(Extension(verifier_arc));
        let resp = app2.oneshot(post_empty(&uri, &tok)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn edit_updates_title_and_body_on_draft() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "edit-tgt", "Edit target").await;
        let draft = draft_for_shipped_transition(
            &*memory,
            item_id,
            ChannelName::Live,
            None,
            "sha1",
            "Edit target",
        )
        .await
        .unwrap();
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let tok = admin_token(&staff, &issuer, "admin").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let app = build_app(store, staff_dyn, Arc::new(verifier));

        let uri = format!("/v1/admin/roadmap/changelog/{}/edit", draft.id);
        let resp = app
            .oneshot(post_json(
                &uri,
                &tok,
                serde_json::json!({"title": "Hand-edited", "body": "Curated copy."}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ChangelogDraftDto = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.title, "Hand-edited");
        assert_eq!(body.body, "Curated copy.");
    }

    #[tokio::test]
    async fn edit_rejects_empty_title_or_body() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let item_id = seed_item(&memory, "empty", "Empty edit").await;
        let draft = draft_for_shipped_transition(
            &*memory,
            item_id,
            ChannelName::Live,
            None,
            "sha1",
            "Empty edit",
        )
        .await
        .unwrap();
        let staff = Arc::new(MemoryStaffRoleStore::new());
        let (issuer, verifier) = fresh_pair();
        let tok = admin_token(&staff, &issuer, "admin").await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let staff_dyn: Arc<dyn StaffRoleStore> = staff;
        let app = build_app(store, staff_dyn, Arc::new(verifier));

        let uri = format!("/v1/admin/roadmap/changelog/{}/edit", draft.id);
        let resp = app
            .oneshot(post_json(
                &uri,
                &tok,
                serde_json::json!({"title": "   ", "body": "x"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
