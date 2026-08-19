//! Public read API for the roadmap (Phase 5).
//!
//! Three routes, all read-only and public-filter aware:
//! - `GET /v1/roadmap`           — list public items.
//! - `GET /v1/roadmap/:slug`     — fetch one item by slug (404 if
//!                                  private or missing).
//! - `GET /v1/roadmap/changelog` — published changelog entries.
//!                                  Phase 5 returns an empty array;
//!                                  Phase 7 populates this.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::{compute_headline_status, ChannelStatus, RoadmapItem};
use super::store::RoadmapStore;

// ---------- DTOs -----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapItemPublic {
    pub id: uuid::Uuid,
    pub slug: String,
    pub title: String,
    pub summary: Option<String>,
    pub category: Option<String>,
    pub eta_band: Option<String>,
    pub votes: i32,
    pub surfaces: Vec<String>,
    pub public: bool,
    /// Computed from `channels` per spec §2.3.
    pub headline_status: String,
    pub channels: Vec<ChannelStatusPublic>,
    pub content_last_updated: DateTime<Utc>,
    pub pipeline_last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelStatusPublic {
    pub channel: String,
    pub status: String,
    pub build_health: String,
    pub commit_sha: Option<String>,
    pub deployed_at: Option<DateTime<Utc>>,
    pub ci_run_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoadmapListResponse {
    pub items: Vec<RoadmapItemPublic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangelogEntryPublic {
    pub id: uuid::Uuid,
    pub roadmap_item_slug: String,
    pub channel: String,
    pub title: String,
    pub body: String,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangelogResponse {
    pub entries: Vec<ChangelogEntryPublic>,
}

// ---------- mapping helpers ------------------------------------------------

fn channel_to_public(c: ChannelStatus) -> ChannelStatusPublic {
    ChannelStatusPublic {
        channel: c.channel.as_str().to_string(),
        status: c.status.as_str().to_string(),
        build_health: c.build_health.as_str().to_string(),
        commit_sha: c.commit_sha,
        deployed_at: c.deployed_at,
        ci_run_url: c.ci_run_url,
    }
}

fn item_to_public(item: RoadmapItem, channels: Vec<ChannelStatus>) -> RoadmapItemPublic {
    let headline = compute_headline_status(&channels);
    let public_channels = channels.into_iter().map(channel_to_public).collect();
    RoadmapItemPublic {
        id: item.id,
        slug: item.slug,
        title: item.title,
        summary: item.summary,
        category: item.category,
        eta_band: item.eta_band,
        votes: item.votes,
        surfaces: item.surfaces,
        public: item.public,
        headline_status: headline.as_str().to_string(),
        channels: public_channels,
        content_last_updated: item.content_last_updated,
        pipeline_last_updated: item.pipeline_last_updated,
    }
}

// ---------- handlers -------------------------------------------------------

/// GET /v1/roadmap
#[utoipa::path(
    get,
    path = "/v1/roadmap",
    tag = "roadmap",
    responses(
        (status = 200, description = "Public roadmap items", body = RoadmapListResponse),
        (status = 500, description = "Database error"),
    ),
)]
pub async fn list_roadmap(Extension(store): Extension<Arc<dyn RoadmapStore>>) -> Response {
    let items = match store.list_items(true).await {
        Ok(items) => items,
        Err(e) => {
            tracing::warn!(error = %e, "roadmap list failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response();
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let channels = match store.list_channel_statuses(item.id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, item_id = %item.id, "channel list failed");
                Vec::new()
            }
        };
        out.push(item_to_public(item, channels));
    }
    (StatusCode::OK, Json(RoadmapListResponse { items: out })).into_response()
}

/// GET /v1/roadmap/:slug
#[utoipa::path(
    get,
    path = "/v1/roadmap/{slug}",
    tag = "roadmap",
    params(("slug" = String, Path, description = "Roadmap item slug")),
    responses(
        (status = 200, description = "Public roadmap item", body = RoadmapItemPublic),
        (status = 404, description = "Item not found or private"),
        (status = 500, description = "Database error"),
    ),
)]
pub async fn get_roadmap_item(
    Extension(store): Extension<Arc<dyn RoadmapStore>>,
    Path(slug): Path<String>,
) -> Response {
    let item = match store.get_item_by_slug(&slug).await {
        Ok(Some(i)) => i,
        Ok(None) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "roadmap get failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "get failed").into_response();
        }
    };
    // Private items don't leak via the public endpoint (spec §7.1).
    if !item.public {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let channels = store
        .list_channel_statuses(item.id)
        .await
        .unwrap_or_default();
    (StatusCode::OK, Json(item_to_public(item, channels))).into_response()
}

/// GET /v1/roadmap/changelog
///
/// Phase 7 wires this to the published-changelog rows. Public items
/// only — an entry whose underlying `RoadmapItem` is private (or
/// soft-deleted) is dropped from the response. Hard cap at 50 entries
/// per response (the front-end paginates client-side once the list
/// grows).
#[utoipa::path(
    get,
    path = "/v1/roadmap/changelog",
    tag = "roadmap",
    responses(
        (status = 200, description = "Published changelog entries", body = ChangelogResponse),
        (status = 500, description = "Database error"),
    ),
)]
pub async fn list_changelog(Extension(store): Extension<Arc<dyn RoadmapStore>>) -> Response {
    let entries = match store.list_published_changelog(50).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "list_published_changelog failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response();
        }
    };
    // Resolve each entry's item slug; drop entries whose item is
    // missing or non-public (slug is omitted from the row to keep
    // the DB write narrow). A tiny per-id cache prevents N+1 within
    // a single response, since multiple drafts can target the same
    // item.
    let mut cache: std::collections::HashMap<uuid::Uuid, Option<RoadmapItem>> =
        std::collections::HashMap::new();
    let mut out: Vec<ChangelogEntryPublic> = Vec::with_capacity(entries.len());
    for entry in entries {
        let cached_item = match cache.get(&entry.roadmap_item_id) {
            Some(c) => c.clone(),
            None => {
                // get_item_by_slug is by slug; we have an id. The
                // store doesn't expose get_by_id, but list_items
                // returns all -- we use list_items(false) once and
                // probe in-memory below. Inlined here keeps the
                // pattern self-contained.
                let all = store.list_items(false).await.unwrap_or_default();
                for it in all.into_iter() {
                    cache.insert(it.id, Some(it));
                }
                cache.entry(entry.roadmap_item_id).or_insert(None).clone()
            }
        };
        let Some(item) = cached_item else { continue };
        if !item.public {
            continue;
        }
        let Some(published_at) = entry.published_at else {
            // Defensive: list_published_changelog filters on
            // `published_at IS NOT NULL`, so this is structurally
            // unreachable. Skip rather than unwrap.
            continue;
        };
        out.push(ChangelogEntryPublic {
            id: entry.id,
            roadmap_item_slug: item.slug,
            channel: entry.channel.as_str().to_string(),
            title: entry.title,
            body: entry.body,
            published_at,
        });
    }
    (StatusCode::OK, Json(ChangelogResponse { entries: out })).into_response()
}

// ---------- router ---------------------------------------------------------

/// Build the public read router. The caller wires
/// `Arc<dyn RoadmapStore>` as an Extension on the parent app.
pub fn router() -> Router {
    Router::new()
        .route("/v1/roadmap", get(list_roadmap))
        // The longer prefix MUST come before /:slug so axum matches
        // `/changelog` as a literal path rather than as a slug.
        .route("/v1/roadmap/changelog", get(list_changelog))
        .route("/v1/roadmap/:slug", get(get_roadmap_item))
}

// ---------- tests ----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::store::test_support::MemoryRoadmapStore;
    use super::super::store::{UpsertChannelStatus, UpsertRoadmapItem};
    use super::*;
    use crate::roadmap::models::{BuildHealth, ChannelName, RoadmapStatus};
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn seed(store: &MemoryRoadmapStore, slug: &str, public: bool, channels: &[ChannelName]) {
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
        for &c in channels {
            store
                .upsert_channel_status(UpsertChannelStatus {
                    roadmap_item_id: item.id,
                    channel: c,
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
        }
    }

    fn app(store: Arc<dyn RoadmapStore>) -> Router {
        router().layer(Extension(store))
    }

    #[tokio::test]
    async fn list_returns_only_public_items() {
        let store = Arc::new(MemoryRoadmapStore::new());
        seed(&store, "alpha-pub", true, &[ChannelName::Live]).await;
        seed(&store, "beta-priv", false, &[ChannelName::Live]).await;
        let resp = app(store)
            .oneshot(
                Request::builder()
                    .uri("/v1/roadmap")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: RoadmapListResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.items.len(), 1);
        assert_eq!(body.items[0].slug, "alpha-pub");
        assert_eq!(body.items[0].headline_status, "shipped");
    }

    #[tokio::test]
    async fn get_returns_404_for_private_item() {
        let store = Arc::new(MemoryRoadmapStore::new());
        seed(&store, "secret", false, &[]).await;
        let resp = app(store)
            .oneshot(
                Request::builder()
                    .uri("/v1/roadmap/secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_returns_item_with_channel_chips() {
        let store = Arc::new(MemoryRoadmapStore::new());
        seed(
            &store,
            "voting-ui",
            true,
            &[ChannelName::Live, ChannelName::Beta],
        )
        .await;
        let resp = app(store)
            .oneshot(
                Request::builder()
                    .uri("/v1/roadmap/voting-ui")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: RoadmapItemPublic = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.channels.len(), 2);
        assert_eq!(body.headline_status, "shipped");
    }

    #[tokio::test]
    async fn changelog_returns_published_entries() {
        use crate::roadmap::changelog::{draft_for_shipped_transition, publish_with_notifications};

        let memory = Arc::new(MemoryRoadmapStore::new());
        seed(&memory, "pub-item", true, &[ChannelName::Live]).await;
        let item = memory.get_item_by_slug("pub-item").await.unwrap().unwrap();

        // Draft + publish one entry; leave a second as a draft.
        let pub_entry = draft_for_shipped_transition(
            &*memory,
            item.id,
            ChannelName::Live,
            None,
            "abc1234",
            "pub-item",
        )
        .await
        .unwrap();
        publish_with_notifications(&*memory, pub_entry.id, "admin")
            .await
            .unwrap();
        let _draft = draft_for_shipped_transition(
            &*memory,
            item.id,
            ChannelName::Beta,
            None,
            "draft-sha",
            "pub-item",
        )
        .await
        .unwrap();

        let store: Arc<dyn RoadmapStore> = memory;
        let resp = app(store)
            .oneshot(
                Request::builder()
                    .uri("/v1/roadmap/changelog")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ChangelogResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.entries.len(), 1, "drafts excluded; only published");
        assert_eq!(body.entries[0].roadmap_item_slug, "pub-item");
        assert_eq!(body.entries[0].channel, "live");
    }

    #[tokio::test]
    async fn changelog_filters_out_private_items() {
        use crate::roadmap::changelog::{draft_for_shipped_transition, publish_with_notifications};

        let memory = Arc::new(MemoryRoadmapStore::new());
        seed(&memory, "secret", false, &[ChannelName::Live]).await;
        let item = memory.get_item_by_slug("secret").await.unwrap().unwrap();
        let draft = draft_for_shipped_transition(
            &*memory,
            item.id,
            ChannelName::Live,
            None,
            "sha",
            "secret",
        )
        .await
        .unwrap();
        publish_with_notifications(&*memory, draft.id, "admin")
            .await
            .unwrap();

        let store: Arc<dyn RoadmapStore> = memory;
        let resp = app(store)
            .oneshot(
                Request::builder()
                    .uri("/v1/roadmap/changelog")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: ChangelogResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(
            body.entries.is_empty(),
            "private items don't leak via the public changelog"
        );
    }
}
