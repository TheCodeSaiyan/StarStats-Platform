//! Internal HMAC-keyed bulk-publish endpoint for roadmap changelog
//! drafts (spec §8.5).
//!
//! Sibling of `admin_changelog_routes`. The admin variant gates per-
//! entry publish on the `admin` staff role + Bearer JWT — appropriate
//! for human editorial review-then-publish. This variant authenticates
//! via the same `X-StarStats-Timestamp` + `X-StarStats-Signature`
//! HMAC scheme as `/v1/internal/roadmap/events` — appropriate for CI,
//! where JWTs would expire on every release cadence and force secret
//! rotation. Same secret key (`ROADMAP_CI_EVENT_HMAC_KEY`) so the CI
//! environment doesn't need a second long-lived credential.
//!
//! One route:
//!   - `POST /v1/internal/roadmap/changelog/publish` — bulk publish
//!     drafts scoped to one roadmap item (by slug), optionally
//!     filtered by channel.
//!
//! The endpoint is intentionally bulk-by-slug (not per-entry by
//! changelog row id) because CI doesn't know individual entry ids:
//! it just knows "I shipped slug X to channel Y, please publish the
//! draft(s) that exist for it." Per-entry publish stays on the admin
//! endpoint for human use.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::changelog::{self, ChangelogError};
use super::events;
use super::models::ChannelName;
use super::routes::RoadmapRoutesState;

// Maximum drafts the endpoint will publish per call. Caller may
// request a lower cap via `max_to_publish` but cannot exceed this.
// Caps the worst-case database write fan-out on a stuck-slug.
const HARD_CAP: usize = 50;
const DEFAULT_CAP: usize = 10;

// ---------- DTOs ----------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct PublishRequest {
    /// Wire-format version; only `1` accepted today.
    pub schema_version: u32,
    /// Idempotency hint from the caller. Not enforced server-side
    /// (per-entry publish is naturally idempotent: re-publishing a
    /// published row returns NotFound and we soft-skip), but logged
    /// for traceability.
    pub event_id: String,
    /// Slug of the roadmap item whose drafts should be published.
    /// Required.
    pub roadmap_slug: String,
    /// Optional channel filter. When set, only drafts whose channel
    /// equals this value are published. When absent, all unpublished
    /// drafts for the item are published.
    #[serde(default)]
    pub channel: Option<String>,
    /// Optional cap on the number of drafts published per call. Clamps
    /// to `HARD_CAP`; defaults to `DEFAULT_CAP` when absent.
    #[serde(default)]
    pub max_to_publish: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PublishedEntry {
    pub id: uuid::Uuid,
    pub channel: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PublishResponse {
    /// Number of drafts successfully published in this call.
    pub published: usize,
    /// Number of drafts skipped (not found / already published races,
    /// channel-filter mismatch, max-cap reached).
    pub skipped: usize,
    /// Per-entry summary for what was published, in publish order.
    pub entries: Vec<PublishedEntry>,
}

// ---------- handler -------------------------------------------------------

/// `POST /v1/internal/roadmap/changelog/publish`
///
/// HMAC-keyed bulk-publish. Verifies the same `v1.<ts>.<body>`
/// signature scheme as `/v1/internal/roadmap/events`, then publishes
/// every draft for the named slug (optionally filtered by channel),
/// up to a per-call cap.
pub async fn publish_drafts(
    State(state): State<RoadmapRoutesState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ts = match headers
        .get("X-StarStats-Timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            tracing::warn!("changelog publish: missing X-StarStats-Timestamp");
            return (StatusCode::UNAUTHORIZED, "missing timestamp").into_response();
        }
    };
    let sig = match headers
        .get("X-StarStats-Signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            tracing::warn!("changelog publish: missing X-StarStats-Signature");
            return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };
    if let Err(e) = events::verify_event_signature(
        state.ci_event_hmac_key.as_ref(),
        &ts,
        sig,
        &body,
        chrono::Utc::now(),
    ) {
        tracing::warn!(error = %e, "changelog publish signature failed");
        return (StatusCode::UNAUTHORIZED, "signature invalid").into_response();
    }

    let payload: PublishRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "changelog publish payload parse failed");
            return (StatusCode::BAD_REQUEST, "bad payload").into_response();
        }
    };
    if payload.schema_version != 1 {
        return (StatusCode::BAD_REQUEST, "unsupported schema_version").into_response();
    }
    if payload.roadmap_slug.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "roadmap_slug required").into_response();
    }

    // Resolve slug → roadmap item. 404 mirrors the admin endpoint's
    // disposition for missing entries and is what the CI script's
    // soft-fail-on-404 already handles.
    let item = match state.store.get_item_by_slug(&payload.roadmap_slug).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            tracing::info!(slug = %payload.roadmap_slug, "changelog publish: slug not found");
            return (StatusCode::NOT_FOUND, "slug not found").into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e, "changelog publish: get_item_by_slug failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "store error").into_response();
        }
    };

    // Parse the optional channel filter eagerly so an invalid value
    // 400s rather than silently filtering to zero matches.
    let channel_filter = match payload.channel.as_deref() {
        Some(s) => match ChannelName::parse(s) {
            Some(c) => Some(c),
            None => {
                return (StatusCode::BAD_REQUEST, "unknown channel").into_response();
            }
        },
        None => None,
    };

    let cap = payload.max_to_publish.unwrap_or(DEFAULT_CAP).min(HARD_CAP);

    let drafts = match state.store.list_changelog_drafts().await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "changelog publish: list_changelog_drafts failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "store error").into_response();
        }
    };

    // Filter to drafts for this item + optional channel. Preserve the
    // store's existing order (most-recently-drafted first per the
    // trait contract) so the response's `entries[]` reads top-down.
    let candidates: Vec<_> = drafts
        .into_iter()
        .filter(|d| d.roadmap_item_id == item.id)
        .filter(|d| match channel_filter {
            Some(c) => d.channel == c,
            None => true,
        })
        .take(cap)
        .collect();

    let total_candidates = candidates.len();
    let mut entries = Vec::with_capacity(total_candidates);
    let mut skipped = 0usize;

    for draft in candidates {
        match changelog::publish_with_notifications(&*state.store, draft.id, "ci").await {
            Ok(published) => entries.push(PublishedEntry {
                id: published.id,
                channel: published.channel.as_str().to_string(),
                title: published.title,
            }),
            // NotFound on publish_with_notifications maps to both
            // "missing entry" (impossible — we just listed it) and
            // "already published" (someone raced us). Treat as soft
            // skip; the operator can ignore.
            Err(ChangelogError::NotFound) => skipped += 1,
            Err(ChangelogError::Store(e)) => {
                tracing::warn!(error = %e, entry_id = %draft.id, "changelog publish: store error mid-batch");
                // Don't roll back successes; this isn't transactional
                // across the batch. Stop here and report partial.
                skipped += 1;
                break;
            }
        }
    }

    tracing::info!(
        event_id = %payload.event_id,
        slug = %payload.roadmap_slug,
        channel = ?payload.channel,
        published = entries.len(),
        skipped,
        "changelog publish: complete"
    );

    let resp = PublishResponse {
        published: entries.len(),
        skipped,
        entries,
    };
    (StatusCode::OK, Json(resp)).into_response()
}

pub fn router(state: RoadmapRoutesState) -> Router {
    Router::new()
        .route(
            "/v1/internal/roadmap/changelog/publish",
            post(publish_drafts),
        )
        .with_state(state)
}

// ---------- tests ---------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roadmap::changelog::draft_for_shipped_transition;
    use crate::roadmap::events::{CiEventError, TracingAuditSink};
    use crate::roadmap::github_graphql::{GitHubError, GitHubReader, ProjectItem};
    use crate::roadmap::models::{ChannelName, RoadmapStatus};
    use crate::roadmap::store::test_support::MemoryRoadmapStore;
    use crate::roadmap::store::{RoadmapStore, UpsertChannelStatus, UpsertRoadmapItem};
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    type HmacSha256 = Hmac<Sha256>;

    const SECRET: &[u8] = b"test-hmac-key";

    fn sign(ts_ms: i64, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(SECRET).unwrap();
        mac.update(format!("v1.{}.", ts_ms).as_bytes());
        mac.update(body);
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    struct NullReader;
    #[async_trait]
    impl GitHubReader for NullReader {
        async fn list_project_items(
            &self,
            _project_id: &str,
        ) -> Result<Vec<ProjectItem>, GitHubError> {
            Ok(Vec::new())
        }
        async fn get_project_item(&self, _item_id: &str) -> Result<ProjectItem, GitHubError> {
            Err(GitHubError::Schema("no item".into()))
        }
        async fn list_project_item_ids_for_issue(
            &self,
            _issue_id: &str,
            _project_id: &str,
        ) -> Result<Vec<String>, GitHubError> {
            Ok(Vec::new())
        }
    }

    async fn seed_item_with_draft(
        slug: &str,
        channel: ChannelName,
    ) -> (Arc<MemoryRoadmapStore>, Uuid) {
        let store = Arc::new(MemoryRoadmapStore::new());
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
                public: true,
            })
            .await
            .unwrap();
        store
            .upsert_channel_status(UpsertChannelStatus {
                roadmap_item_id: item.id,
                channel,
                status: RoadmapStatus::Shipped,
                build_health: crate::roadmap::models::BuildHealth::Passing,
                build_id: None,
                commit_sha: Some("deadbeef"),
                deployed_at: None,
                ci_run_url: None,
                previous_shipped_sha: None,
                last_event_id: None,
            })
            .await
            .unwrap();
        let draft = draft_for_shipped_transition(&*store, item.id, channel, None, "deadbeef", slug)
            .await
            .unwrap();
        (store, draft.id)
    }

    fn state_with(store: Arc<dyn RoadmapStore>) -> RoadmapRoutesState {
        RoadmapRoutesState {
            store,
            reader: Arc::new(NullReader),
            webhook_hmac_key: Arc::new(b"unused-webhook".to_vec()),
            ci_event_hmac_key: Arc::new(SECRET.to_vec()),
            audit: Arc::new(TracingAuditSink),
            project_id: Arc::new("PVT_test".to_string()),
        }
    }

    async fn call(
        state: RoadmapRoutesState,
        body: &[u8],
        ts: Option<i64>,
        sig: Option<String>,
    ) -> (StatusCode, Vec<u8>) {
        let app = router(state);
        let mut req = Request::builder()
            .method("POST")
            .uri("/v1/internal/roadmap/changelog/publish")
            .header("content-type", "application/json");
        if let Some(t) = ts {
            req = req.header("X-StarStats-Timestamp", t.to_string());
        }
        if let Some(s) = sig {
            req = req.header("X-StarStats-Signature", s);
        }
        let resp = app
            .oneshot(req.body(Body::from(body.to_vec())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, bytes)
    }

    fn body_for(slug: &str, channel: Option<&str>) -> Vec<u8> {
        let mut v = serde_json::json!({
            "schema_version": 1,
            "event_id": Uuid::new_v4().to_string(),
            "roadmap_slug": slug,
        });
        if let Some(c) = channel {
            v["channel"] = serde_json::Value::String(c.to_string());
        }
        serde_json::to_vec(&v).unwrap()
    }

    #[tokio::test]
    async fn publishes_one_draft_when_one_matches() {
        let (store, _draft_id) = seed_item_with_draft("kappa", ChannelName::Live).await;
        let state = state_with(store);
        let body = body_for("kappa", None);
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, resp_bytes) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::OK);
        let resp: PublishResponse = serde_json::from_slice(&resp_bytes).unwrap();
        assert_eq!(resp.published, 1);
        assert_eq!(resp.skipped, 0);
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].channel, "live");
    }

    #[tokio::test]
    async fn returns_zero_published_when_no_drafts_for_slug() {
        // seed item but no draft
        let store = Arc::new(MemoryRoadmapStore::new());
        let surfaces: Vec<String> = vec![];
        store
            .upsert_item(UpsertRoadmapItem {
                github_project_item_id: "PVTI_empty",
                slug: "empty",
                title: "empty",
                summary: None,
                category: None,
                eta_band: None,
                surfaces: &surfaces,
                parent_id: None,
                links: None,
                public: true,
            })
            .await
            .unwrap();
        let state = state_with(store);
        let body = body_for("empty", None);
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, resp_bytes) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::OK);
        let resp: PublishResponse = serde_json::from_slice(&resp_bytes).unwrap();
        assert_eq!(resp.published, 0);
        assert!(resp.entries.is_empty());
    }

    #[tokio::test]
    async fn returns_404_when_slug_missing() {
        let store: Arc<dyn RoadmapStore> = Arc::new(MemoryRoadmapStore::new());
        let state = state_with(store);
        let body = body_for("nope", None);
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, _) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn channel_filter_narrows_to_matching_channel() {
        let (store, _) = seed_item_with_draft("filterme", ChannelName::Live).await;
        // Add a second draft on a different channel for the same item.
        let item = store.get_item_by_slug("filterme").await.unwrap().unwrap();
        store
            .upsert_channel_status(UpsertChannelStatus {
                roadmap_item_id: item.id,
                channel: ChannelName::Beta,
                status: RoadmapStatus::Shipped,
                build_health: crate::roadmap::models::BuildHealth::Passing,
                build_id: None,
                commit_sha: Some("cafef00d"),
                deployed_at: None,
                ci_run_url: None,
                previous_shipped_sha: None,
                last_event_id: None,
            })
            .await
            .unwrap();
        draft_for_shipped_transition(
            &*store,
            item.id,
            ChannelName::Beta,
            None,
            "cafef00d",
            "filterme",
        )
        .await
        .unwrap();

        let state = state_with(store);
        let body = body_for("filterme", Some("live"));
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, resp_bytes) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::OK);
        let resp: PublishResponse = serde_json::from_slice(&resp_bytes).unwrap();
        assert_eq!(resp.published, 1);
        assert_eq!(resp.entries[0].channel, "live");
    }

    #[tokio::test]
    async fn unknown_channel_in_filter_returns_400() {
        let (store, _) = seed_item_with_draft("ch400", ChannelName::Live).await;
        let state = state_with(store);
        let body = body_for("ch400", Some("nonsense"));
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, _) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_signature_returns_401() {
        let (store, _) = seed_item_with_draft("nosig", ChannelName::Live).await;
        let state = state_with(store);
        let body = body_for("nosig", None);
        let ts = chrono::Utc::now().timestamp_millis();

        let (status, _) = call(state, &body, Some(ts), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_timestamp_returns_401() {
        let (store, _) = seed_item_with_draft("nots", ChannelName::Live).await;
        let state = state_with(store);
        let body = body_for("nots", None);
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, _) = call(state, &body, None, Some(sig)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_signature_returns_401() {
        let (store, _) = seed_item_with_draft("badsig", ChannelName::Live).await;
        let state = state_with(store);
        let body = body_for("badsig", None);
        let ts = chrono::Utc::now().timestamp_millis();
        let mut mac = HmacSha256::new_from_slice(b"different-key").unwrap();
        mac.update(format!("v1.{}.", ts).as_bytes());
        mac.update(&body);
        let sig = format!("v1={}", hex::encode(mac.finalize().into_bytes()));

        let (status, _) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn max_to_publish_caps_the_batch() {
        // Seed item with 3 drafts across 3 channels
        let (store, _) = seed_item_with_draft("cap-me", ChannelName::Live).await;
        let item = store.get_item_by_slug("cap-me").await.unwrap().unwrap();
        for (ch, sha) in [
            (ChannelName::Beta, "1111111"),
            (ChannelName::Alpha, "2222222"),
        ] {
            store
                .upsert_channel_status(UpsertChannelStatus {
                    roadmap_item_id: item.id,
                    channel: ch,
                    status: RoadmapStatus::Shipped,
                    build_health: crate::roadmap::models::BuildHealth::Passing,
                    build_id: None,
                    commit_sha: Some(sha),
                    deployed_at: None,
                    ci_run_url: None,
                    previous_shipped_sha: None,
                    last_event_id: None,
                })
                .await
                .unwrap();
            draft_for_shipped_transition(&*store, item.id, ch, None, sha, "cap-me")
                .await
                .unwrap();
        }

        let state = state_with(store);
        let mut body_v = serde_json::json!({
            "schema_version": 1,
            "event_id": Uuid::new_v4().to_string(),
            "roadmap_slug": "cap-me",
            "max_to_publish": 2usize,
        });
        let body = serde_json::to_vec(&body_v).unwrap();
        let _ = &mut body_v; // suppress unused-mut lint
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, resp_bytes) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::OK);
        let resp: PublishResponse = serde_json::from_slice(&resp_bytes).unwrap();
        assert_eq!(resp.published, 2);
    }

    #[tokio::test]
    async fn rejects_unsupported_schema_version() {
        let store: Arc<dyn RoadmapStore> = Arc::new(MemoryRoadmapStore::new());
        let state = state_with(store);
        let body = serde_json::to_vec(&serde_json::json!({
            "schema_version": 99,
            "event_id": Uuid::new_v4().to_string(),
            "roadmap_slug": "any",
        }))
        .unwrap();
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sign(ts, &body);

        let (status, _) = call(state, &body, Some(ts), Some(sig)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // Quiet the unused-warning on the CiEventError import path. The
    // type isn't directly referenced in tests, but is the error
    // returned by verify_event_signature when used in error
    // assertions; importing it keeps future test extensions ergonomic.
    #[allow(dead_code)]
    fn _force_use(_: CiEventError) {}
}
