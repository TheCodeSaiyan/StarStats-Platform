//! Release notes: written by CI, read by the tray and the web (store in
//! `releases.rs`, migration 0072).
//!
//! `POST /v1/internal/releases` takes a release's generated notes from CI,
//! signed with the same `v1.<ts>.<body>` HMAC scheme and key as the roadmap
//! events endpoint (`roadmap::events::verify_event_signature`), so CI needs
//! no second credential. It is mounted only where that key is configured.
//!
//! `GET /v1/releases` is public, for /changelog. `GET /v1/me/releases` adds
//! the caller's unread flag per release, filtered to the channels they
//! follow, for What's New in the tray and on the web.

use crate::api_error::ApiErrorBody;
use crate::auth::{AuthenticatedUser, TokenType};
use crate::releases::{Release, ReleaseStore, ReleaseWrite, CHANNELS, TRACKS};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Reader routes. The store arrives as an Extension like every other store.
pub fn routes() -> Router {
    Router::new()
        .route("/v1/releases", get(list_public))
        .route("/v1/me/releases", get(list_mine))
        .route("/v1/me/releases/{id}/seen", post(mark_seen))
}

#[derive(Clone)]
pub struct ReleaseIngestState {
    pub store: Arc<dyn ReleaseStore>,
    pub hmac_key: Arc<Vec<u8>>,
}

/// The CI ingest route. Carries its own state because it is merged after
/// the app's Extension layers, the same way the roadmap internal routes are.
pub fn internal_router(state: ReleaseIngestState) -> Router {
    Router::new()
        .route("/v1/internal/releases", post(ingest))
        .with_state(state)
}

// -- DTOs -------------------------------------------------------------

/// What scripts/publish-release-notes.mjs sends.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReleaseIngestBody {
    /// Wire-format version; only `1` is accepted.
    pub schema_version: u32,
    pub track: String,
    pub tag: String,
    pub version: String,
    pub channel: String,
    /// `YYYY-MM-DD`, the tagged commit's date.
    pub date: String,
    #[serde(default)]
    pub summary: String,
    #[schema(value_type = Vec<Object>)]
    pub groups: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReleaseListResponse {
    pub releases: Vec<Release>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MyRelease {
    #[serde(flatten)]
    pub release: Release,
    pub unread: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MyReleasesResponse {
    pub releases: Vec<MyRelease>,
    pub unread_count: usize,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ReleaseQuery {
    /// `tray` or `platform`; both when absent.
    pub track: Option<String>,
    /// Comma-separated channels, e.g. `live` or `alpha,beta,rc,live`.
    /// All channels when absent.
    pub channels: Option<String>,
    /// 1 to 50, default 10.
    pub limit: Option<i64>,
}

fn err(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(ApiErrorBody {
            error: code.to_string(),
            detail: None,
        }),
    )
        .into_response()
}

fn parse_query(q: &ReleaseQuery) -> Result<(Option<String>, Vec<String>), Response> {
    let track = match q.track.as_deref() {
        None | Some("") => None,
        Some(t) if TRACKS.contains(&t) => Some(t.to_string()),
        Some(_) => return Err(err(StatusCode::BAD_REQUEST, "invalid_track")),
    };
    let mut channels = Vec::new();
    for c in q.channels.as_deref().unwrap_or("").split(',') {
        let c = c.trim();
        if c.is_empty() {
            continue;
        }
        if !CHANNELS.contains(&c) {
            return Err(err(StatusCode::BAD_REQUEST, "invalid_channel"));
        }
        channels.push(c.to_string());
    }
    Ok((track, channels))
}

// -- Handlers ---------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/internal/releases",
    tag = "releases",
    operation_id = "releases_ingest",
    request_body = ReleaseIngestBody,
    responses(
        (status = 200, description = "Stored (or replaced by tag)", body = Release),
        (status = 400, description = "Invalid release", body = ApiErrorBody),
        (status = 401, description = "Missing or bad signature"),
    ),
)]
pub async fn ingest(
    State(state): State<ReleaseIngestState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let (Some(ts), Some(sig)) = (
        header("X-StarStats-Timestamp"),
        header("X-StarStats-Signature"),
    ) else {
        return (StatusCode::UNAUTHORIZED, "missing signature").into_response();
    };
    if let Err(e) = crate::roadmap::events::verify_event_signature(
        state.hmac_key.as_ref(),
        ts,
        sig,
        &body,
        chrono::Utc::now(),
    ) {
        tracing::warn!(error = %e, "release ingest signature failed");
        return (StatusCode::UNAUTHORIZED, "signature invalid").into_response();
    }
    let payload: ReleaseIngestBody = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "release ingest payload parse failed");
            return err(StatusCode::BAD_REQUEST, "bad_payload");
        }
    };
    if payload.schema_version != 1 {
        return err(StatusCode::BAD_REQUEST, "unsupported_schema_version");
    }
    let write = match ReleaseWrite::parse(
        &payload.track,
        &payload.tag,
        &payload.version,
        &payload.channel,
        &payload.date,
        &payload.summary,
        payload.groups,
    ) {
        Ok(w) => w,
        Err(reason) => {
            tracing::warn!(reason, tag = %payload.tag, "release ingest rejected");
            return err(StatusCode::BAD_REQUEST, "invalid_release");
        }
    };
    match state.store.upsert(&write).await {
        Ok(r) => {
            tracing::info!(tag = %r.tag, summary = %r.summary, "release notes stored");
            Json(r).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "release upsert failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/releases",
    tag = "releases",
    operation_id = "releases_list_public",
    params(ReleaseQuery),
    responses(
        (status = 200, description = "Releases, newest first", body = ReleaseListResponse),
        (status = 400, description = "Unknown track or channel", body = ApiErrorBody),
    ),
)]
pub async fn list_public(
    Extension(store): Extension<Arc<dyn ReleaseStore>>,
    Query(q): Query<ReleaseQuery>,
) -> Response {
    let (track, channels) = match parse_query(&q) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match store
        .list(track.as_deref(), &channels, q.limit.unwrap_or(10))
        .await
    {
        Ok(releases) => Json(ReleaseListResponse { releases }).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "release list failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

fn player_id(auth: &AuthenticatedUser) -> Result<Uuid, Response> {
    if !matches!(auth.token_type, TokenType::User | TokenType::Device) {
        return Err(err(StatusCode::UNAUTHORIZED, "user_token_required"));
    }
    Uuid::parse_str(&auth.sub).map_err(|_| err(StatusCode::UNAUTHORIZED, "invalid_subject"))
}

#[utoipa::path(
    get,
    path = "/v1/me/releases",
    tag = "releases",
    operation_id = "releases_list_mine",
    params(ReleaseQuery),
    responses(
        (status = 200, description = "Releases with this player's unread flags", body = MyReleasesResponse),
        (status = 400, description = "Unknown track or channel", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub async fn list_mine(
    auth: AuthenticatedUser,
    Extension(store): Extension<Arc<dyn ReleaseStore>>,
    Query(q): Query<ReleaseQuery>,
) -> Response {
    let uid = match player_id(&auth) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let (track, channels) = match parse_query(&q) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let releases = match store
        .list(track.as_deref(), &channels, q.limit.unwrap_or(10))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "release list failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let ids: Vec<Uuid> = releases.iter().map(|r| r.id).collect();
    let seen = match store.seen_ids(uid, &ids).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "release seen lookup failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };
    let releases: Vec<MyRelease> = releases
        .into_iter()
        .map(|r| MyRelease {
            unread: !seen.contains(&r.id),
            release: r,
        })
        .collect();
    let unread_count = releases.iter().filter(|r| r.unread).count();
    Json(MyReleasesResponse {
        releases,
        unread_count,
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/me/releases/{id}/seen",
    tag = "releases",
    operation_id = "releases_mark_seen",
    params(("id" = Uuid, Path, description = "Release id")),
    responses((status = 204, description = "Marked seen")),
    security(("bearer" = [])),
)]
pub async fn mark_seen(
    auth: AuthenticatedUser,
    Extension(store): Extension<Arc<dyn ReleaseStore>>,
    Path(id): Path<Uuid>,
) -> Response {
    let uid = match player_id(&auth) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match store.mark_seen(uid, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "release mark seen failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::releases::test_support::MemoryReleaseStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tower::ServiceExt;

    const SECRET: &[u8] = b"test-release-key";

    fn sign(ts_ms: i64, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET).unwrap();
        mac.update(format!("v1.{ts_ms}.").as_bytes());
        mac.update(body);
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn release_body(tag: &str, channel: &str, date: &str) -> Vec<u8> {
        let track = if tag.starts_with("tray-") {
            "tray"
        } else {
            "platform"
        };
        let version = tag
            .trim_start_matches("tray-")
            .trim_start_matches('v')
            .split('-')
            .next()
            .unwrap()
            .to_string();
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "track": track,
            "tag": tag,
            "version": version,
            "channel": channel,
            "date": date,
            "summary": "1 new",
            "groups": [{ "kind": "New", "lines": [{ "text": "Friends", "surfaces": ["Tray"], "prs": [134], "roadmap": null }] }],
        }))
        .unwrap()
    }

    struct Fx {
        app: Router,
        token: String,
        other: String,
    }

    fn fx() -> Fx {
        let store: Arc<dyn ReleaseStore> = Arc::new(MemoryReleaseStore::new());
        let (issuer, verifier) = fresh_pair();
        let app = routes()
            .layer(Extension(store.clone()))
            .layer(Extension(Arc::new(verifier)))
            .merge(internal_router(ReleaseIngestState {
                store,
                hmac_key: Arc::new(SECRET.to_vec()),
            }));
        let t = |id: Uuid| issuer.sign_user(&id.to_string(), "p").unwrap();
        Fx {
            app,
            token: t(Uuid::now_v7()),
            other: t(Uuid::now_v7()),
        }
    }

    async fn send(app: &Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn ingest_signed(app: &Router, body: Vec<u8>, sig: Option<String>) -> StatusCode {
        let ts = chrono::Utc::now().timestamp_millis();
        let sig = sig.unwrap_or_else(|| sign(ts, &body));
        let req = Request::builder()
            .method("POST")
            .uri("/v1/internal/releases")
            .header("content-type", "application/json")
            .header("X-StarStats-Timestamp", ts.to_string())
            .header("X-StarStats-Signature", sig)
            .body(Body::from(body))
            .unwrap();
        send(app, req).await.0
    }

    fn get(uri: &str, token: Option<&str>) -> Request<Body> {
        let mut r = Request::builder().uri(uri);
        if let Some(t) = token {
            r = r.header("authorization", format!("Bearer {t}"));
        }
        r.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn ingest_requires_a_valid_signature() {
        let f = fx();
        let body = release_body("tray-v0.1.31", "live", "2026-09-26");
        assert_eq!(
            ingest_signed(&f.app, body.clone(), Some("v1=00".into())).await,
            StatusCode::UNAUTHORIZED
        );
        let unsigned = Request::builder()
            .method("POST")
            .uri("/v1/internal/releases")
            .body(Body::from(body.clone()))
            .unwrap();
        assert_eq!(send(&f.app, unsigned).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(ingest_signed(&f.app, body, None).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_rejects_an_invalid_release() {
        let f = fx();
        let mut v: serde_json::Value =
            serde_json::from_slice(&release_body("tray-v0.1.31", "live", "2026-09-26")).unwrap();
        v["channel"] = "nightly".into();
        let status = ingest_signed(&f.app, serde_json::to_vec(&v).unwrap(), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn feeds_filter_by_track_and_channel_and_track_unread() {
        let f = fx();
        for (tag, ch, d) in [
            ("tray-v0.1.30", "live", "2026-09-21"),
            ("tray-v0.1.31-alpha.1", "alpha", "2026-09-26"),
            ("tray-v0.1.31", "live", "2026-09-26"),
            ("v0.1.62", "live", "2026-09-26"),
        ] {
            assert_eq!(
                ingest_signed(&f.app, release_body(tag, ch, d), None).await,
                StatusCode::OK
            );
        }

        let (s, v) = send(&f.app, get("/v1/releases?track=tray&channels=live", None)).await;
        assert_eq!(s, StatusCode::OK);
        let tags: Vec<&str> = v["releases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["tag"].as_str().unwrap())
            .collect();
        assert_eq!(tags, vec!["tray-v0.1.31", "tray-v0.1.30"]);

        let (s, _) = send(&f.app, get("/v1/releases?channels=nightly", None)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        let uri = "/v1/me/releases?track=tray&channels=live";
        let (_, mine) = send(&f.app, get(uri, Some(&f.token))).await;
        assert_eq!(mine["unread_count"], 2);
        let newest = mine["releases"][0]["id"].as_str().unwrap().to_string();
        assert_eq!(mine["releases"][0]["unread"], true);
        assert_eq!(mine["releases"][0]["summary"], "1 new");

        let seen = Request::builder()
            .method("POST")
            .uri(format!("/v1/me/releases/{newest}/seen"))
            .header("authorization", format!("Bearer {}", f.token))
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&f.app, seen).await.0, StatusCode::NO_CONTENT);
        let (_, mine) = send(&f.app, get(uri, Some(&f.token))).await;
        assert_eq!(mine["unread_count"], 1);
        let (_, theirs) = send(&f.app, get(uri, Some(&f.other))).await;
        assert_eq!(theirs["unread_count"], 2, "read state is per player");
    }

    #[tokio::test]
    async fn resending_a_release_replaces_it() {
        let f = fx();
        let body = release_body("v0.1.62", "live", "2026-09-26");
        ingest_signed(&f.app, body.clone(), None).await;
        ingest_signed(&f.app, body, None).await;
        let (_, v) = send(&f.app, get("/v1/releases", None)).await;
        assert_eq!(v["releases"].as_array().unwrap().len(), 1);
    }
}
