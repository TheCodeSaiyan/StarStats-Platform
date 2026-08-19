//! `/v1/users/me/profile-layout` routes — owner-only GET + PUT.
//!
//! GET returns either the stored layout or `null` and a `source` field
//! telling the client whether the base of the response is the stored
//! row or a "needs default" signal. The PROJECTION against the
//! registry (appending missing widgets as enabled:false) happens
//! web-side, so the server stays unaware of the registry — keeps
//! schema evolution additive.

use crate::audit::{AuditEntry, AuditLog, ACTION_PROFILE_LAYOUT_UPDATED};
use crate::auth::AuthenticatedUser;
use crate::profile_layout::{LayoutEntry, LayoutSurface, ProfileLayoutStore};
use axum::{
    extract::{Extension, Query},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Hard caps to keep payloads tiny. The server is defensive against
/// the client; UI should never exceed these by design.
const MAX_ENTRIES: usize = 32;
const MAX_ID_LEN: usize = 64;
/// Free-grid bounds (M7). The web layer places widgets on a 24-column
/// grid; the server is defensive against a client sending nonsense.
const GRID_COLS: u32 = 24;
const MAX_ROWS: u32 = 512;

/// Reject obviously-invalid free-grid geometry. Geometry is OPTIONAL —
/// a legacy `{id,enabled,size}` entry (all `None`) always passes. When
/// present, a width/height must be at least 1 and the tile must fit the
/// grid horizontally. Vertical extent is capped so a hostile client
/// can't request an unbounded canvas.
fn geometry_in_bounds(entry: &LayoutEntry) -> bool {
    if let Some(w) = entry.w {
        if w == 0 || w > GRID_COLS {
            return false;
        }
    }
    if let Some(x) = entry.x {
        if x >= GRID_COLS {
            return false;
        }
        // If both x and w are set, the tile must not overflow the grid.
        if let Some(w) = entry.w {
            if x + w > GRID_COLS {
                return false;
            }
        }
    }
    if let Some(h) = entry.h {
        if h == 0 || h > MAX_ROWS {
            return false;
        }
    }
    if let Some(y) = entry.y {
        if y > MAX_ROWS {
            return false;
        }
    }
    true
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProfileLayoutResponse {
    /// The owner's stored layout. `null` means "use default" — the
    /// client (web) projects against the registry.
    pub layout: Option<Vec<LayoutEntry>>,
    /// `"stored"` when the row exists, `"default"` when the column is
    /// NULL and the client should fall back to DEFAULT_LAYOUT.
    pub source: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateProfileLayoutRequest {
    pub layout: Option<Vec<LayoutEntry>>,
}

/// Optional `?surface=` selector. Defaults to `profile` so the legacy
/// route shape is unchanged.
#[derive(Debug, Deserialize)]
pub struct LayoutSurfaceQuery {
    #[serde(default)]
    pub surface: LayoutSurface,
}

/// Build the `/v1/users/me/profile-layout` sub-router.
pub fn routes() -> Router {
    Router::new().route(
        "/v1/users/me/profile-layout",
        get(get_profile_layout).put(put_profile_layout),
    )
}

/// GET /v1/users/me/profile-layout
///
/// Returns the owner's stored layout, or `null` + `source:"default"` if
/// the column is unset. Validation and registry projection are left to
/// the web layer so schema evolution stays additive.
#[utoipa::path(
    get,
    path = "/v1/users/me/profile-layout",
    params(
        ("surface" = Option<crate::profile_layout::LayoutSurface>, Query,
         description = "Which layout surface: 'profile' (default) or 'home'"),
    ),
    responses(
        (status = 200, description = "Owner's profile layout (null = use default)", body = ProfileLayoutResponse),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    tag = "profile-layout",
)]
pub async fn get_profile_layout(
    auth: AuthenticatedUser,
    Query(q): Query<LayoutSurfaceQuery>,
    Extension(store): Extension<Arc<dyn ProfileLayoutStore>>,
) -> Result<Json<ProfileLayoutResponse>, StatusCode> {
    let stored = store
        .get(q.surface, &auth.preferred_username)
        .await
        .map_err(|e| {
            tracing::warn!(err = %e, "profile_layout get failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let source = if stored.is_some() {
        "stored"
    } else {
        "default"
    }
    .to_string();
    Ok(Json(ProfileLayoutResponse {
        layout: stored,
        source,
    }))
}

/// PUT /v1/users/me/profile-layout
///
/// Overwrites the stored layout. Validation is hand-rolled (serde
/// already enforces the WidgetSize enum; length caps and id charset
/// are checked here). Validation errors return **400 BAD_REQUEST** —
/// never 401, which would trigger the client auto-logout interceptor.
#[utoipa::path(
    put,
    path = "/v1/users/me/profile-layout",
    request_body = UpdateProfileLayoutRequest,
    params(
        ("surface" = Option<crate::profile_layout::LayoutSurface>, Query,
         description = "Which layout surface: 'profile' (default) or 'home'"),
    ),
    responses(
        (status = 200, description = "Layout written; response carries canonical form", body = ProfileLayoutResponse),
        (status = 400, description = "Validation error (oversized array, bad id chars, etc.)"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    tag = "profile-layout",
)]
pub async fn put_profile_layout(
    auth: AuthenticatedUser,
    Query(q): Query<LayoutSurfaceQuery>,
    Extension(store): Extension<Arc<dyn ProfileLayoutStore>>,
    Extension(audit): Extension<Arc<dyn AuditLog>>,
    Json(req): Json<UpdateProfileLayoutRequest>,
) -> Result<Json<ProfileLayoutResponse>, StatusCode> {
    // Validate length + per-id charset + id length. WidgetSize enum
    // is already enforced by serde at the deserialization boundary.
    if let Some(layout) = &req.layout {
        if layout.len() > MAX_ENTRIES {
            return Err(StatusCode::BAD_REQUEST);
        }
        for entry in layout {
            if entry.id.is_empty() || entry.id.len() > MAX_ID_LEN {
                return Err(StatusCode::BAD_REQUEST);
            }
            if !entry
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            if !geometry_in_bounds(entry) {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    // Fetch prior layout for the audit diff. Best-effort — a failure
    // here yields a less-useful diff string, not a request failure.
    let prev = store
        .get(q.surface, &auth.preferred_username)
        .await
        .ok()
        .flatten();

    store
        .put(q.surface, &auth.preferred_username, req.layout.as_deref())
        .await
        .map_err(|e| {
            tracing::warn!(err = %e, "profile_layout put failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let next = req.layout.clone();
    let diff = render_diff(prev.as_deref(), next.as_deref());
    let prev_count = prev.as_ref().map_or(0, |v| v.len());
    let next_count = next.as_ref().map_or(0, |v| v.len());

    // Best-effort audit emission per docs/ENGINEERING.md — a hiccup here must
    // never poison the response.
    if let Err(e) = audit
        .append(AuditEntry {
            actor_sub: Some(auth.sub.clone()),
            actor_handle: Some(auth.preferred_username.clone()),
            action: ACTION_PROFILE_LAYOUT_UPDATED.to_string(),
            payload: serde_json::json!({
                "surface": q.surface,
                "prev_count": prev_count,
                "next_count": next_count,
                "diff": diff,
            }),
        })
        .await
    {
        tracing::warn!(err = %e, "audit emit failed for profile_layout.updated");
    }

    let source = if next.is_some() { "stored" } else { "default" }.to_string();
    Ok(Json(ProfileLayoutResponse {
        layout: next,
        source,
    }))
}

/// Compact diff string for the audit row. Format is stable so log
/// readers can grep for change kinds. Examples:
///   `"moved:sessions 1→4, disabled:entities, sized:heatmap=expanded"`
///   `"no-op"` when nothing changed.
fn render_diff(prev: Option<&[LayoutEntry]>, next: Option<&[LayoutEntry]>) -> String {
    let prev = prev.unwrap_or(&[]);
    let next = next.unwrap_or(&[]);
    let mut parts: Vec<String> = Vec::new();

    // Enabled/disabled toggles and size changes for entries present in both.
    for entry in next {
        if let Some(p) = prev.iter().find(|p| p.id == entry.id) {
            if p.enabled != entry.enabled {
                parts.push(format!(
                    "{}:{}",
                    if entry.enabled { "enabled" } else { "disabled" },
                    entry.id,
                ));
            }
            if p.size != entry.size {
                parts.push(format!("sized:{}={}", entry.id, entry.size.as_str()));
            }
        } else {
            parts.push(format!("added:{}", entry.id));
        }
    }
    // Removals: entries present in prev but absent in next.
    for entry in prev {
        if !next.iter().any(|n| n.id == entry.id) {
            parts.push(format!("removed:{}", entry.id));
        }
    }
    // Position moves for entries present in both.
    for (new_pos, entry) in next.iter().enumerate() {
        if let Some(old_pos) = prev.iter().position(|p| p.id == entry.id) {
            if old_pos != new_pos {
                parts.push(format!("moved:{} {}→{}", entry.id, old_pos, new_pos));
            }
        }
    }

    if parts.is_empty() {
        "no-op".to_string()
    } else {
        parts.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::MemoryAuditLog;
    use crate::auth::test_support::fresh_pair;
    use crate::profile_layout::test_support::MemoryProfileLayoutStore;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Build a minimal test app: the two profile-layout routes, the
    /// ProfileLayoutStore Extension, the AuditLog Extension, and an
    /// AuthVerifier Extension so the JWT extractor works.
    /// Auth is exercised via real JWTs minted by `fresh_pair()` —
    /// the same pattern used in sharing_routes tests.
    fn test_app(
        store: Arc<dyn ProfileLayoutStore>,
        audit: Arc<dyn AuditLog>,
        verifier: Arc<crate::auth::AuthVerifier>,
    ) -> axum::Router {
        routes()
            .layer(Extension(store))
            .layer(Extension(audit))
            .layer(Extension(verifier))
    }

    /// Mint a bearer token for a handle. `sub` is a fresh UUID so
    /// each call produces a unique principal.
    fn mint_token(issuer: &crate::auth::TokenIssuer, handle: &str) -> String {
        issuer
            .sign_user(&Uuid::new_v4().to_string(), handle)
            .expect("sign_user")
    }

    async fn read_body(resp: axum::response::Response) -> (StatusCode, serde_json::Value) {
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    // -- Test 1: GET returns source:"default" for a fresh user --------

    #[tokio::test]
    async fn get_returns_default_when_unset() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(store, audit, Arc::new(verifier));

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let (status, body) = read_body(res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "default");
        assert!(body["layout"].is_null());
    }

    // -- Test 2: PUT then GET roundtrips the saved layout --------------

    #[tokio::test]
    async fn put_then_get_roundtrips() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let verifier = Arc::new(verifier);
        let app = test_app(store, audit, verifier.clone());

        let put_body = serde_json::to_vec(&serde_json::json!({
            "layout": [
                { "id": "sessions", "enabled": true, "size": "compact" },
                { "id": "heatmap",  "enabled": true, "size": "compact" },
            ]
        }))
        .unwrap();

        let put_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(put_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_res.status(), StatusCode::OK);

        let get_res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let (status, body) = read_body(get_res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "stored");
        let layout = body["layout"].as_array().unwrap();
        assert_eq!(layout.len(), 2);
        assert_eq!(layout[0]["id"], "sessions");
        assert_eq!(layout[1]["id"], "heatmap");
    }

    // -- Test 3: PUT with >MAX_ENTRIES entries returns 400 -------------

    #[tokio::test]
    async fn put_rejects_oversized_array() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(store, audit, Arc::new(verifier));

        // MAX_ENTRIES + 1 entries — one over the cap.
        let layout: Vec<serde_json::Value> = (0..=MAX_ENTRIES)
            .map(|i| {
                serde_json::json!({ "id": format!("w{i}"), "enabled": true, "size": "compact" })
            })
            .collect();
        let body = serde_json::to_vec(&serde_json::json!({ "layout": layout })).unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        // MUST be 400, not 401 — client interceptors auto-logout on 401.
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    // -- Test 4: PUT with an id longer than MAX_ID_LEN returns 400 ----

    #[tokio::test]
    async fn put_rejects_oversized_id() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(store, audit, Arc::new(verifier));

        let bad_id = "x".repeat(MAX_ID_LEN + 1);
        let body = serde_json::to_vec(&serde_json::json!({
            "layout": [{ "id": bad_id, "enabled": true, "size": "compact" }]
        }))
        .unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        // MUST be 400, not 401.
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_accepts_valid_geometry_and_round_trips() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(store, audit, Arc::new(verifier));

        let body = serde_json::to_vec(&serde_json::json!({
            "layout": [
                { "id": "heatmap", "enabled": true, "size": "expanded",
                  "x": 0, "y": 0, "w": 24, "h": 8 },
                { "id": "orgs", "enabled": true, "size": "compact",
                  "x": 0, "y": 8, "w": 6, "h": 6 },
            ]
        }))
        .unwrap();

        let put_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_res.status(), StatusCode::OK);

        // The stored layout keeps the geometry verbatim.
        let get_res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(get_res).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["layout"][0]["w"], 24);
        assert_eq!(body["layout"][0]["h"], 8);
        assert_eq!(body["layout"][1]["x"], 0);
        assert_eq!(body["layout"][1]["y"], 8);
    }

    #[tokio::test]
    async fn put_rejects_out_of_bounds_geometry() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(store, audit, Arc::new(verifier));

        // x + w overflows the 24-col grid → 400 (not 401).
        let body = serde_json::to_vec(&serde_json::json!({
            "layout": [
                { "id": "heatmap", "enabled": true, "size": "compact",
                  "x": 20, "y": 0, "w": 12, "h": 6 },
            ]
        }))
        .unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    // -- Test 5: PUT emits an audit row with correct action and diff ---

    #[tokio::test]
    async fn put_emits_audit_row() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(
            store,
            audit.clone() as Arc<dyn AuditLog>,
            Arc::new(verifier),
        );

        let put_body = serde_json::to_vec(&serde_json::json!({
            "layout": [
                { "id": "sessions", "enabled": true, "size": "compact" },
                { "id": "heatmap",  "enabled": true, "size": "expanded" },
            ]
        }))
        .unwrap();

        let put_res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(put_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(put_res.status(), StatusCode::OK);

        // Verify the audit row was emitted with correct fields.
        let entries = audit.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, ACTION_PROFILE_LAYOUT_UPDATED);
        assert_eq!(entries[0].actor_handle.as_deref(), Some("Alice"));

        // Verify the diff includes the added items (empty → 2 entries).
        let diff_str = entries[0]
            .payload
            .get("diff")
            .and_then(|v| v.as_str())
            .expect("diff must be a string in payload");
        assert!(
            diff_str.contains("added:sessions") && diff_str.contains("added:heatmap"),
            "diff should mention added items, got: {}",
            diff_str
        );
    }

    // -- Test 6: PUT ?surface=home is isolated from the profile surface --

    #[tokio::test]
    async fn put_home_surface_is_isolated_from_profile() {
        let store: Arc<dyn ProfileLayoutStore> = Arc::new(MemoryProfileLayoutStore::default());
        let audit: Arc<dyn AuditLog> = Arc::new(MemoryAuditLog::default());
        let (issuer, verifier) = fresh_pair();
        let token = mint_token(&issuer, "Alice");
        let app = test_app(store, audit, Arc::new(verifier));

        let put_body = serde_json::to_vec(&serde_json::json!({
            "layout": [{ "id": "heatmap", "enabled": true, "size": "expanded" }]
        }))
        .unwrap();

        // Write to the HOME surface.
        let put_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v1/users/me/profile-layout?surface=home")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(put_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_res.status(), StatusCode::OK);

        // The PROFILE surface (default, no query) must still be empty.
        let get_profile = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/profile-layout")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(get_profile).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "default");
        assert!(body["layout"].is_null());

        // The HOME surface returns what we wrote.
        let get_home = app
            .oneshot(
                Request::builder()
                    .uri("/v1/users/me/profile-layout?surface=home")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = read_body(get_home).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "stored");
        assert_eq!(body["layout"].as_array().unwrap()[0]["id"], "heatmap");
    }
}
