//! Tray "What's new" panel routes (Phase 8, spec §9).
//!
//! Two routes:
//! - `GET  /v1/me/roadmap/whats-new`        — unread top-level items
//!                                            (auth path) OR the 3
//!                                            most-recent published
//!                                            changelog entries
//!                                            (anonymous fallback).
//! - `POST /v1/me/roadmap/whats-new/seen`   — mark `(item, entry)` as
//!                                            seen for the auth'd
//!                                            user. Returns 204.
//!
//! Server-side state lives in `roadmap_user_read_state` so the panel's
//! "unread" set syncs across a user's devices (spec §9). The anonymous
//! path is provided so an unpaired tray can still show "what shipped
//! recently" without forcing a sign-in — there's just no per-user
//! read-tracking on that path.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::models::{compute_headline_status, RoadmapChangelogEntry, RoadmapItem};
use super::store::RoadmapStore;
use crate::auth::AuthenticatedUser;

// ---------- DTOs -----------------------------------------------------------

/// One card in the tray "What's new" panel. Carries enough metadata
/// for the renderer to show a status chip, a relative-time stamp, and
/// link out to the public detail page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WhatsNewItem {
    /// Stable roadmap item id — used as the `mark_seen` key.
    pub roadmap_item_id: Uuid,
    /// Public slug — used to build the `/roadmap/{slug}` web URL.
    pub slug: String,
    pub title: String,
    /// Aggregated headline status per spec §2.3, e.g. `shipped`.
    pub headline_status: String,
    /// Latest published changelog entry id for this item. The renderer
    /// echoes this back through `mark_seen` so the read-state row
    /// records exactly which entry the user has seen.
    pub latest_changelog_entry_id: Uuid,
    /// When that latest changelog entry was published. Drives the
    /// relative-time chip on the card.
    pub latest_published_at: DateTime<Utc>,
    /// True when the auth'd user has not yet seen the latest entry.
    /// Always `false` on the anonymous fallback path.
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WhatsNewResponse {
    pub items: Vec<WhatsNewItem>,
    /// True iff the request was authenticated. The renderer uses this
    /// to decide whether to show the read-tracking affordance (paired)
    /// or the "recent changes" framing (anonymous).
    pub seen_via_auth: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarkSeenRequest {
    pub roadmap_item_id: Uuid,
    pub changelog_entry_id: Uuid,
}

// ---------- handlers -------------------------------------------------------

const WHATS_NEW_CAP: i64 = 3;

/// GET /v1/me/roadmap/whats-new
///
/// Authenticated path: top 3 unread top-level items, ordered by the
/// freshness of their latest published changelog entry.
///
/// Anonymous path (no bearer / invalid bearer): the 3 most-recent
/// published changelog entries across the public roadmap, framed as
/// "recent changes" — no per-user state.
#[utoipa::path(
    get,
    path = "/v1/me/roadmap/whats-new",
    tag = "roadmap",
    responses(
        (status = 200, description = "Unread (auth) or recent (anon) cards", body = WhatsNewResponse),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn whats_new(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    auth: Option<AuthenticatedUser>,
) -> Response {
    match auth {
        Some(auth) => whats_new_authed(store, auth).await,
        None => whats_new_anonymous(store).await,
    }
}

/// Build a fresh map of `(item_id -> latest_published_entry)` from the
/// store's published-changelog history. Filters out items that have no
/// published entry (a tray card without a publish timestamp would be
/// dishonest).
async fn latest_published_for_items(
    store: &dyn RoadmapStore,
) -> Result<HashMap<Uuid, RoadmapChangelogEntry>, Response> {
    // Cap at 200 — same upper bound the store clamps to. For the panel
    // we only ever surface 3, but we walk a wider window so the
    // "latest per item" pick is robust against drafts being published
    // out of strict order.
    let entries = match store.list_published_changelog(200).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "list_published_changelog failed");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response());
        }
    };
    let mut latest: HashMap<Uuid, RoadmapChangelogEntry> = HashMap::new();
    for entry in entries {
        let Some(ts) = entry.published_at else {
            continue;
        };
        match latest.get(&entry.roadmap_item_id) {
            Some(prev) => {
                let prev_ts = prev.published_at.unwrap_or_else(Utc::now);
                if ts > prev_ts {
                    latest.insert(entry.roadmap_item_id, entry);
                }
            }
            None => {
                latest.insert(entry.roadmap_item_id, entry);
            }
        }
    }
    Ok(latest)
}

/// Headline-status string for one item, derived from its channel rows.
/// Defaults to `proposed` on a store error (per the §2.3 empty rule);
/// errors are warn-logged so the cause isn't lost.
async fn headline_for_item(store: &dyn RoadmapStore, item: &RoadmapItem) -> String {
    let channels = match store.list_channel_statuses(item.id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, item_id = %item.id, "headline channels failed");
            Vec::new()
        }
    };
    compute_headline_status(&channels).as_str().to_string()
}

async fn whats_new_authed(store: Arc<dyn RoadmapStore>, auth: AuthenticatedUser) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => {
            return (StatusCode::UNAUTHORIZED, "invalid_subject").into_response();
        }
    };
    let items = match store
        .list_top_level_items_with_changelog(WHATS_NEW_CAP * 5)
        .await
    {
        // Walk a wider window than the cap so we can drop already-seen
        // items and still surface 3 fresh ones.
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(error = %e, "list_top_level_items_with_changelog failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response();
        }
    };
    let latest = match latest_published_for_items(&*store).await {
        Ok(map) => map,
        Err(resp) => return resp,
    };

    let mut out: Vec<WhatsNewItem> = Vec::with_capacity(WHATS_NEW_CAP as usize);
    for item in items {
        let Some(latest_entry) = latest.get(&item.id).cloned() else {
            // The store said this item has a publish, but our walk
            // didn't surface it — skip defensively rather than emit
            // a card with a fabricated entry id.
            continue;
        };
        let Some(latest_at) = latest_entry.published_at else {
            continue;
        };
        let read = match store.get_user_read_state(user_id, item.id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "get_user_read_state failed");
                None
            }
        };
        let unread = match read {
            None => true,
            Some(rs) => rs.last_seen_changelog_entry_id != Some(latest_entry.id),
        };
        if !unread {
            continue;
        }
        let headline = headline_for_item(&*store, &item).await;
        out.push(WhatsNewItem {
            roadmap_item_id: item.id,
            slug: item.slug,
            title: item.title,
            headline_status: headline,
            latest_changelog_entry_id: latest_entry.id,
            latest_published_at: latest_at,
            unread: true,
        });
        if out.len() >= WHATS_NEW_CAP as usize {
            break;
        }
    }

    (
        StatusCode::OK,
        Json(WhatsNewResponse {
            items: out,
            seen_via_auth: true,
        }),
    )
        .into_response()
}

async fn whats_new_anonymous(store: Arc<dyn RoadmapStore>) -> Response {
    // Anonymous: surface the 3 most-recent published changelog entries
    // across the public roadmap. No read-tracking; `unread` is always
    // false so the renderer styles them as "recent" rather than
    // "new for you".
    let entries = match store.list_published_changelog(50).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "list_published_changelog failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response();
        }
    };
    // Resolve item slug + publicness via list_items (matches the
    // public_routes.rs changelog pattern). Inline cache prevents an
    // N+1 when several entries share the same item.
    let all_items = store.list_items(false).await.unwrap_or_default();
    let mut item_by_id: HashMap<Uuid, RoadmapItem> = HashMap::new();
    for it in all_items {
        item_by_id.insert(it.id, it);
    }

    // Pick at most one entry per item (the freshest), so the panel
    // doesn't repeat the same card with two timestamps.
    let mut seen_items: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut out: Vec<WhatsNewItem> = Vec::with_capacity(WHATS_NEW_CAP as usize);
    for entry in entries {
        let Some(published_at) = entry.published_at else {
            continue;
        };
        let Some(item) = item_by_id.get(&entry.roadmap_item_id) else {
            continue;
        };
        if !item.public || item.parent_id.is_some() {
            continue;
        }
        if !seen_items.insert(item.id) {
            continue;
        }
        let headline = headline_for_item(&*store, item).await;
        out.push(WhatsNewItem {
            roadmap_item_id: item.id,
            slug: item.slug.clone(),
            title: item.title.clone(),
            headline_status: headline,
            latest_changelog_entry_id: entry.id,
            latest_published_at: published_at,
            unread: false,
        });
        if out.len() >= WHATS_NEW_CAP as usize {
            break;
        }
    }

    (
        StatusCode::OK,
        Json(WhatsNewResponse {
            items: out,
            seen_via_auth: false,
        }),
    )
        .into_response()
}

/// POST /v1/me/roadmap/whats-new/seen
#[utoipa::path(
    post,
    path = "/v1/me/roadmap/whats-new/seen",
    tag = "roadmap",
    request_body = MarkSeenRequest,
    responses(
        (status = 204, description = "Read-state recorded"),
        (status = 401, description = "Authentication required"),
        (status = 500, description = "Database error"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn mark_seen(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    auth: AuthenticatedUser,
    Json(body): Json<MarkSeenRequest>,
) -> Response {
    let user_id = match Uuid::parse_str(&auth.sub) {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid_subject").into_response(),
    };
    if let Err(e) = store
        .upsert_user_read_state(user_id, body.roadmap_item_id, Some(body.changelog_entry_id))
        .await
    {
        tracing::warn!(error = %e, "upsert_user_read_state failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------- router ---------------------------------------------------------

/// Build the "What's new" sub-router. The caller layers
/// `Arc<dyn RoadmapStore>` + `Arc<AuthVerifier>` as Extensions.
pub fn router() -> Router {
    Router::new()
        .route("/v1/me/roadmap/whats-new", get(whats_new))
        .route("/v1/me/roadmap/whats-new/seen", post(mark_seen))
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::changelog::{draft_for_shipped_transition, publish_with_notifications};
    use super::super::models::{BuildHealth, ChannelName, RoadmapStatus};
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::{UpsertChannelStatus, UpsertRoadmapItem};
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::auth::{AuthVerifier, TokenIssuer};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn seed_item_with_published_entry(
        store: &MemoryRoadmapStore,
        slug: &str,
        public: bool,
    ) -> (Uuid, Uuid) {
        let surfaces: Vec<String> = vec![];
        let item = store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: &format!("PVTI_{slug}"),
                slug,
                title: slug,
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public,
            })
            .await
            .unwrap();
        store
            .upsert_channel_status(UpsertChannelStatus {
                roadmap_item_id: item.id,
                channel: ChannelName::Live,
                status: RoadmapStatus::Shipped,
                build_health: BuildHealth::Passing,
                build_id: None,
                commit_sha: Some("deadbeef"),
                deployed_at: None,
                ci_run_url: None,
                previous_shipped_sha: None,
                last_event_id: None,
            })
            .await
            .unwrap();
        let draft =
            draft_for_shipped_transition(store, item.id, ChannelName::Live, None, "sha", slug)
                .await
                .unwrap();
        let published = publish_with_notifications(store, draft.id, "admin")
            .await
            .unwrap();
        (item.id, published.id)
    }

    fn build_app(store: Arc<dyn RoadmapStore>, verifier: Arc<AuthVerifier>) -> Router {
        router().layer(Extension(store)).layer(Extension(verifier))
    }

    fn issue_token(issuer: &TokenIssuer, user_id: Uuid, handle: &str) -> String {
        issuer
            .sign_user(&user_id.to_string(), handle)
            .expect("sign user token")
    }

    #[tokio::test]
    async fn anonymous_returns_recent_published_entries() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let (_, _) = seed_item_with_published_entry(&memory, "alpha", true).await;
        let (_, _) = seed_item_with_published_entry(&memory, "beta", true).await;

        let store: Arc<dyn RoadmapStore> = memory;
        let (_issuer, verifier) = fresh_pair();
        let app = build_app(store, Arc::new(verifier));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/me/roadmap/whats-new")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: WhatsNewResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(
            !body.seen_via_auth,
            "anonymous path advertises no auth-backed state"
        );
        assert_eq!(body.items.len(), 2);
        // No read-tracking on anon path -- all `unread = false`.
        for item in &body.items {
            assert!(!item.unread);
        }
    }

    #[tokio::test]
    async fn authed_unread_items_surface() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let (_item_a, _) = seed_item_with_published_entry(&memory, "feature-a", true).await;
        let (_item_b, _) = seed_item_with_published_entry(&memory, "feature-b", true).await;
        let store: Arc<dyn RoadmapStore> = memory;
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "tester");
        let app = build_app(store, Arc::new(verifier));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/me/roadmap/whats-new")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: WhatsNewResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(body.seen_via_auth);
        assert_eq!(body.items.len(), 2, "both fresh items show as unread");
        for item in &body.items {
            assert!(item.unread);
        }
    }

    #[tokio::test]
    async fn already_seen_items_are_hidden_for_authed_user() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let (item_id, entry_id) = seed_item_with_published_entry(&memory, "feature-c", true).await;
        let user_id = Uuid::now_v7();
        // Pre-seed: user already saw the latest entry for this item.
        memory
            .upsert_user_read_state(user_id, item_id, Some(entry_id))
            .await
            .unwrap();

        let store: Arc<dyn RoadmapStore> = memory;
        let (issuer, verifier) = fresh_pair();
        let token = issue_token(&issuer, user_id, "tester");
        let app = build_app(store, Arc::new(verifier));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/me/roadmap/whats-new")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: WhatsNewResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(body.seen_via_auth);
        assert!(
            body.items.is_empty(),
            "item the user has already seen should be hidden"
        );
    }

    #[tokio::test]
    async fn mark_seen_flips_unread_state() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let (item_id, entry_id) = seed_item_with_published_entry(&memory, "feature-d", true).await;
        let store: Arc<dyn RoadmapStore> = memory.clone();
        let (issuer, verifier) = fresh_pair();
        let user_id = Uuid::now_v7();
        let token = issue_token(&issuer, user_id, "tester");
        let app = build_app(store.clone(), Arc::new(verifier));

        // Confirm unread upfront.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/me/roadmap/whats-new")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: WhatsNewResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.items.len(), 1);

        // POST /seen.
        let payload = serde_json::to_vec(&MarkSeenRequest {
            roadmap_item_id: item_id,
            changelog_entry_id: entry_id,
        })
        .unwrap();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/me/roadmap/whats-new/seen")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Now the panel should be empty.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/me/roadmap/whats-new")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: WhatsNewResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(body.items.is_empty(), "mark-seen should clear the panel");
    }

    #[tokio::test]
    async fn mark_seen_requires_auth() {
        let memory = Arc::new(MemoryRoadmapStore::new());
        let (item_id, entry_id) = seed_item_with_published_entry(&memory, "feature-e", true).await;
        let store: Arc<dyn RoadmapStore> = memory;
        let (_issuer, verifier) = fresh_pair();
        let app = build_app(store, Arc::new(verifier));

        let payload = serde_json::to_vec(&MarkSeenRequest {
            roadmap_item_id: item_id,
            changelog_entry_id: entry_id,
        })
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/me/roadmap/whats-new/seen")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
