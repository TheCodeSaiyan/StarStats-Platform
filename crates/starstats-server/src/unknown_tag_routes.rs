//! `POST /v1/unknown-tags` — opt-in, metadata-only unknown shell-tag report.
//!
//! The tray sends the `<EventName>` shell tags of log lines it could not
//! classify, with first/last sighting and a count. **Never line bodies.**
//! [`crate::unknown_tags::valid_shell_tag`] gates every entry here as well as
//! on the tray, because the tray-side check protects the user's intent and
//! this one protects the server from a modified client.
//!
//! Malformed entries are dropped individually and reported back in
//! `rejected`, rather than failing the batch: one bad tag must not cost the
//! user every other sighting, and a client that starts emitting junk should
//! degrade rather than stop reporting.

use crate::api_error::ApiErrorBody;
use crate::auth::AuthenticatedUser;
use crate::unknown_tags::{sanitise, TagSighting, UnknownTagStore, MAX_TAGS_PER_BATCH};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
    Extension, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportTagsRequest {
    pub tags: Vec<TagSighting>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReportTagsResponse {
    /// Sightings stored.
    pub accepted: usize,
    /// Sightings dropped for failing validation.
    pub rejected: usize,
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

#[utoipa::path(
    post,
    path = "/v1/unknown-tags",
    tag = "parser",
    request_body = ReportTagsRequest,
    responses(
        (status = 200, description = "Sightings recorded", body = ReportTagsResponse),
        (status = 400, description = "Batch too large", body = ApiErrorBody),
        (status = 401, description = "Unauthenticated", body = ApiErrorBody),
    ),
    security(("bearer_auth" = []))
)]
pub async fn report_tags(
    user: AuthenticatedUser,
    Extension(store): Extension<Arc<dyn UnknownTagStore>>,
    Json(body): Json<ReportTagsRequest>,
) -> Response {
    // Reject an oversized batch outright rather than silently truncating —
    // a client sending 10k tags has a bug worth surfacing to it.
    if body.tags.len() > MAX_TAGS_PER_BATCH {
        return err(StatusCode::BAD_REQUEST, "batch_too_large");
    }

    let (kept, rejected) = sanitise(body.tags);
    if rejected > 0 {
        tracing::info!(
            rejected,
            handle = %user.preferred_username,
            "unknown-tag report: dropped invalid sightings"
        );
    }

    // The handle comes off the bearer token, never the request body —
    // the client can't attribute sightings to somebody else.
    if let Err(e) = store.record(&user.preferred_username, &kept).await {
        tracing::error!(error = %e, "unknown-tag report: store failed");
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unknown_tags_unavailable",
        );
    }

    metrics::counter!("starstats_unknown_tag_sightings_total").increment(kept.len() as u64);

    (
        StatusCode::OK,
        Json(ReportTagsResponse {
            accepted: kept.len(),
            rejected,
        }),
    )
        .into_response()
}

pub fn router() -> Router {
    Router::new().route("/v1/unknown-tags", post(report_tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::unknown_tags::test_support::MemoryUnknownTagStore;
    use axum::body::to_bytes;
    use axum::http::Request;
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    struct H {
        app: Router,
        store: Arc<MemoryUnknownTagStore>,
        token: String,
    }

    fn harness() -> H {
        let (issuer, verifier) = fresh_pair();
        let store = Arc::new(MemoryUnknownTagStore::new());
        let store_dyn: Arc<dyn UnknownTagStore> = store.clone();
        let token = issuer
            .sign_user(&Uuid::now_v7().to_string(), "nigel")
            .expect("sign token");
        let app = router()
            .layer(Extension(Arc::new(verifier)))
            .layer(Extension(store_dyn));
        H { app, store, token }
    }

    async fn post(h: &H, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let resp = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/unknown-tags")
                    .header("authorization", format!("Bearer {}", h.token))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn tag(name: &str) -> serde_json::Value {
        json!({
            "shell_tag": name,
            "first_seen": "2026-07-16T00:00:00Z",
            "last_seen": "2026-07-30T00:00:00Z",
            "occurrences": 3307,
        })
    }

    #[tokio::test]
    async fn accepts_a_valid_batch_and_stores_it() {
        let h = harness();

        let (status, body) = post(
            &h,
            json!({ "tags": [tag("LandingArea_UnregisterFromExternalSystems_StowingVehicle")] }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["accepted"], 1);
        assert_eq!(body["rejected"], 0);

        let c = h
            .store
            .candidates(ts("2026-07-01T00:00:00Z"), ts("2026-08-01T00:00:00Z"), 10)
            .await
            .unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].occurrences, 3307);
    }

    #[tokio::test]
    async fn drops_a_body_shaped_entry_but_keeps_the_rest() {
        // The privacy contract has to hold even against a modified client.
        let h = harness();

        let (status, body) = post(
            &h,
            json!({ "tags": [
                tag("GoodTag"),
                tag("[STOWING ON UNREGISTER] LandingArea_X [745597122922]"),
            ]}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["accepted"], 1);
        assert_eq!(body["rejected"], 1);

        let c = h
            .store
            .candidates(ts("2026-07-01T00:00:00Z"), ts("2026-08-01T00:00:00Z"), 10)
            .await
            .unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].shell_tag, "GoodTag");
    }

    #[tokio::test]
    async fn rejects_an_oversized_batch() {
        let h = harness();
        let tags: Vec<serde_json::Value> = (0..MAX_TAGS_PER_BATCH + 1)
            .map(|i| tag(&format!("Tag{i}")))
            .collect();

        let (status, _) = post(&h, json!({ "tags": tags })).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_empty_batch_is_accepted_as_a_no_op() {
        let h = harness();
        let (status, body) = post(&h, json!({ "tags": [] })).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["accepted"], 0);
    }

    #[tokio::test]
    async fn unauthenticated_report_is_rejected() {
        let h = harness();

        let resp = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/unknown-tags")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(json!({"tags": []}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(resp.status(), StatusCode::OK);
    }
}
