//! Roadmap HTTP routes — Phase 3 lands just the GitHub Projects v2
//! webhook receiver. Public read API + voting routes come in
//! Phases 5 and 6.
//!
//! The webhook route is mounted under `/v1/internal/roadmap/...`
//! (not part of the public OpenAPI spec — `#[utoipa::path]` is
//! intentionally omitted).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use super::events::{self, AuditSink, CiEventError, CiEventPayload, TracingAuditSink};
use super::github_graphql::GitHubReader;
use super::store::RoadmapStore;
use super::sync;

/// Shared state for the roadmap routes. Cloned per request via
/// `State(...)`; the inner `Arc`s make that cheap.
#[derive(Clone)]
pub struct RoadmapRoutesState {
    pub store: Arc<dyn RoadmapStore>,
    pub reader: Arc<dyn GitHubReader>,
    pub webhook_hmac_key: Arc<Vec<u8>>,
    pub ci_event_hmac_key: Arc<Vec<u8>>,
    pub audit: Arc<dyn AuditSink>,
    /// ProjectV2 node id this server is bound to. Used by the
    /// `issues` webhook branch to filter linked Project items to
    /// just our Project (an Issue may belong to multiple Projects
    /// across the org; we only care about ours).
    pub project_id: Arc<String>,
}

impl RoadmapRoutesState {
    /// Construct with the default `TracingAuditSink`. Phase 7 will
    /// swap this for the real audit log.
    pub fn new(
        store: Arc<dyn RoadmapStore>,
        reader: Arc<dyn GitHubReader>,
        webhook_hmac_key: Vec<u8>,
        ci_event_hmac_key: Vec<u8>,
        project_id: String,
    ) -> Self {
        Self {
            store,
            reader,
            webhook_hmac_key: Arc::new(webhook_hmac_key),
            ci_event_hmac_key: Arc::new(ci_event_hmac_key),
            audit: Arc::new(TracingAuditSink),
            project_id: Arc::new(project_id),
        }
    }
}

/// Router for the roadmap pipeline's internal routes. Returned as
/// an `axum::Router` so the caller composes it under the desired
/// path prefix. The bulk-publish endpoint lives in a sibling module
/// (`internal_changelog_routes`) and is merged in here.
pub fn router(state: RoadmapRoutesState) -> Router {
    Router::new()
        .route("/v1/internal/roadmap/github-webhook", post(github_webhook))
        .route("/v1/internal/roadmap/events", post(ci_event))
        .with_state(state.clone())
        .merge(super::internal_changelog_routes::router(state))
}

/// `POST /v1/internal/roadmap/github-webhook`
///
/// Verifies `X-Hub-Signature-256` against the raw body using
/// the configured HMAC key, then dispatches to
/// [`sync::handle_webhook_verified`]. Returns 204 on success, 401 on
/// signature failure, 400 on payload parse error, 502 if a downstream
/// GitHub/store call fails.
pub async fn github_webhook(
    State(state): State<RoadmapRoutesState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Signature header is required.
    let sig = match headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            tracing::warn!("roadmap webhook: missing X-Hub-Signature-256");
            return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
        }
    };
    if let Err(e) =
        sync::verify_github_webhook_signature(state.webhook_hmac_key.as_ref(), &body, sig)
    {
        tracing::warn!(error = %e, "roadmap webhook signature failed");
        return (StatusCode::UNAUTHORIZED, "signature invalid").into_response();
    }
    // `X-GitHub-Event` identifies which webhook fired (e.g.
    // `projects_v2_item`, `issues`). Missing is treated as
    // bad-request — well-formed GitHub deliveries always include it,
    // so absence indicates a malformed caller, not a benign event we
    // should ignore.
    let event = match headers.get("X-GitHub-Event").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => {
            tracing::warn!("roadmap webhook: missing X-GitHub-Event header");
            return (StatusCode::BAD_REQUEST, "missing event header").into_response();
        }
    };
    let project_id = state.project_id.as_str();
    match sync::handle_webhook_verified(&*state.store, &*state.reader, &event, project_id, &body)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(sync::SyncError::PayloadParse(msg)) => {
            tracing::warn!(error = %msg, event, "roadmap webhook payload parse failed");
            (StatusCode::BAD_REQUEST, "bad payload").into_response()
        }
        Err(sync::SyncError::UnknownAction(action)) => {
            // An unknown action isn't an error per se — GitHub may
            // ship new lifecycle events. Log and 204 so the webhook
            // delivery doesn't retry forever.
            tracing::info!(event, action, "roadmap webhook: ignored unknown action");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(sync::SyncError::UnknownEvent(name)) => {
            // Org webhook may fan out events we haven't wired up
            // (pull_request, push, etc.). 204 so the delivery doesn't
            // retry; the operator can either narrow the subscription
            // in the GitHub UI or wire a new branch in handle_webhook_verified.
            tracing::info!(event = name, "roadmap webhook: ignored unknown event");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(other) => {
            tracing::warn!(error = %other, event, "roadmap webhook dispatch failed");
            (StatusCode::BAD_GATEWAY, "downstream failure").into_response()
        }
    }
}

/// `POST /v1/internal/roadmap/events`
///
/// Inbound from the CI pipeline. Verifies the v1 HMAC signature over
/// `v1.<timestamp_ms>.<body>` (spec §4.5), parses the JSON payload,
/// then dispatches to `events::ingest_event`. Returns 204 on success
/// (including the idempotent-duplicate path), 400 on payload errors,
/// 401 on signature failure, 404 if the referenced item is missing.
pub async fn ci_event(
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
            tracing::warn!("ci event: missing X-StarStats-Timestamp");
            return (StatusCode::UNAUTHORIZED, "missing timestamp").into_response();
        }
    };
    let sig = match headers
        .get("X-StarStats-Signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            tracing::warn!("ci event: missing X-StarStats-Signature");
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
        tracing::warn!(error = %e, "ci event signature failed");
        return (StatusCode::UNAUTHORIZED, "signature invalid").into_response();
    }
    let payload: CiEventPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "ci event payload parse failed");
            return (StatusCode::BAD_REQUEST, "bad payload").into_response();
        }
    };
    match events::ingest_event(
        &*state.store,
        Some(state.reader.as_ref()),
        state.audit.as_ref(),
        &payload,
    )
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(CiEventError::ItemNotFound) => {
            (StatusCode::NOT_FOUND, "item not found").into_response()
        }
        Err(CiEventError::MissingEventId)
        | Err(CiEventError::MissingIdentifier)
        | Err(CiEventError::SchemaVersionUnsupported(_))
        | Err(CiEventError::UnknownChannel(_))
        | Err(CiEventError::UnknownStatus(_)) => {
            (StatusCode::BAD_REQUEST, "bad request").into_response()
        }
        Err(other) => {
            tracing::warn!(error = %other, "ci event ingest failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
        }
    }
}

// Phase 3/4 don't include route-layer integration tests — signature
// verification + ingest dispatch are covered in the underlying
// modules. Phase 5 will add full route-layer tests once there's a
// non-trivial response shape to assert against.

// Compile-time anchor so Extension-style imports stay live until
// Phase 5 grows real handler params.
#[allow(dead_code)]
fn _extension_anchor(_e: Extension<()>) {}
