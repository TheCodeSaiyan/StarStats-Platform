//! Batch class-name → rich entry resolution endpoint.
//!
//! `POST /v1/reference/resolve` accepts a JSON body of
//! `{ "class_names": ["doom_armor_medium_helmet_02_01_01", ...] }` and returns
//! `{ "resolved": { "doom_armor_medium_helmet_02_01_01": ResolvedEntry } }` —
//! only entries that resolved are present in the map; callers handle misses.
//!
//! Resolution iterates categories `[Item, Weapon, Vehicle, Location]` via
//! `store.get_entry(cat, class)` — first hit wins (suffix-tolerant via the
//! store's `lower(class_name)` index). The response key preserves the
//! original input casing.
//!
//! Auth: any valid `AuthenticatedUser` (user or device token).
//! Cap: >200 class_names → 400 Bad Request.

use crate::auth::AuthenticatedUser;
use crate::reference_data::ReferenceCategory;
use crate::reference_store::ReferenceStore;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct ResolveRequest {
    /// List of raw Star Citizen class identifiers to look up.
    /// Capped at 200 — requests with more entries are rejected with 400.
    pub class_names: Vec<String>,
}

/// Rich resolution result for a single class name.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ResolvedEntry {
    /// Player-friendly display name (e.g. "The Butcher Helmet").
    pub display_name: String,
    /// KB URL slug, if the entry has one.
    pub slug: Option<String>,
    /// Reference category: "item" | "weapon" | "vehicle" | "location".
    pub category: String,
    /// `metadata.classification` — e.g. "FPS.Armor.Helmet".
    pub classification: Option<String>,
    /// `metadata.classification_label` — human-readable form, e.g. "Helmet".
    pub classification_label: Option<String>,
    /// True when the entry has an image the media proxy will actually
    /// serve — a listed image on a non-allowlisted host does not count,
    /// because requesting it can only ever 404.
    pub has_image: bool,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ResolveResponse {
    /// Resolved entries keyed by the supplied class_name.
    /// Only entries that were found in the reference catalogue are
    /// present; missing keys indicate an unknown class_name.
    pub resolved: HashMap<String, ResolvedEntry>,
}

/// Maximum number of class_names accepted per request.
const MAX_CLASS_NAMES: usize = 200;

/// Category search order for resolution: most-specific first so FPS
/// armour/weapons (Item/Weapon) win over a theoretical vehicle match.
const RESOLVE_ORDER: [ReferenceCategory; 4] = [
    ReferenceCategory::Item,
    ReferenceCategory::Weapon,
    ReferenceCategory::Vehicle,
    ReferenceCategory::Location,
];

#[utoipa::path(
    post,
    path = "/v1/reference/resolve",
    tag = "reference",
    request_body = ResolveRequest,
    responses(
        (status = 200, description = "Resolved entries (only found entries present)", body = ResolveResponse),
        (status = 400, description = "Too many class_names (>200)"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
    security(("BearerAuth" = []))
)]
pub async fn resolve_reference_names<R: ReferenceStore>(
    State(store): State<Arc<R>>,
    _user: AuthenticatedUser,
    Json(req): Json<ResolveRequest>,
) -> Response {
    if req.class_names.len() > MAX_CLASS_NAMES {
        return (StatusCode::BAD_REQUEST, "too_many").into_response();
    }

    let mut resolved: HashMap<String, ResolvedEntry> = HashMap::new();

    // One batch query across the resolve-order categories instead of a per-class
    // N-query loop; the store applies category precedence + request-case mapping.
    let matches = store
        .resolve_entries(&RESOLVE_ORDER, &req.class_names)
        .await
        .unwrap_or_default();
    for (req_class, entry) in matches {
        let m = &entry.metadata;
        let classification = m["classification"].as_str().map(str::to_owned);
        let classification_label = m["classification_label"].as_str().map(str::to_owned);
        // `has_image` MUST mean "we can serve it", not "the catalogue lists
        // one". The wiki join carries image URLs from more than one host, and
        // the media proxy is an SSRF allowlist of exactly
        // `media.starcitizen.tools` — so an entry whose only image lives on
        // e.g. `cstone.space` advertised `has_image: true`, the client
        // rendered an <img> for it, and the proxy correctly refused with a
        // 404 every single time. The image degraded fine but the request was
        // never worth making, and it filled consoles with failures that look
        // like a broken feature. Checked against the same predicate the proxy
        // enforces, so the two can't disagree.
        let has_image = m["images"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|img| img.get("original_url"))
            .and_then(|v| v.as_str())
            .is_some_and(crate::reference_media::allowed_image_host);
        resolved.insert(
            req_class,
            ResolvedEntry {
                display_name: entry.display_name,
                slug: entry.slug,
                category: entry.category.as_str().to_owned(),
                classification,
                classification_label,
                has_image,
            },
        );
    }

    (StatusCode::OK, Json(ResolveResponse { resolved })).into_response()
}

pub fn router<R: ReferenceStore>(store: Arc<R>) -> Router {
    Router::new()
        .route("/v1/reference/resolve", post(resolve_reference_names::<R>))
        .with_state(store)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::fresh_pair;
    use crate::reference_data::{ReferenceCategory, ReferenceEntry};
    use crate::reference_store::test_support::MemoryReferenceStore;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    fn make_entry(
        category: ReferenceCategory,
        class_name: &str,
        display_name: &str,
    ) -> ReferenceEntry {
        ReferenceEntry {
            category,
            class_name: class_name.to_owned(),
            display_name: display_name.to_owned(),
            slug: None,
            metadata: serde_json::json!({}),
        }
    }

    /// `has_image` must mean "we can serve it", not "one is listed".
    ///
    /// The wiki join carries image URLs from more than one host, and the media
    /// proxy is an SSRF allowlist of exactly `media.starcitizen.tools`. Two
    /// real items in the catalogue — `qrt_specialist_heavy_core_01_01_01` and
    /// `grin_multitool_energy_01_mag` — have their only image on
    /// `cstone.space`, so they advertised `has_image: true`, the client
    /// rendered an <img>, and the proxy refused with a 404 every time. It
    /// degraded correctly and was never worth requesting.
    #[tokio::test]
    async fn an_image_we_cannot_serve_is_not_advertised() {
        let store = Arc::new(MemoryReferenceStore::new());
        // Verbatim shape of the real entry, including the host.
        store
            .upsert_entries(&[
                ReferenceEntry {
                    category: ReferenceCategory::Item,
                    class_name: "qrt_specialist_heavy_core_01_01_01".to_owned(),
                    display_name: "Antium Core".to_owned(),
                    slug: Some("antium-core".to_owned()),
                    metadata: serde_json::json!({
                        "images": [{
                            "original_url":
                                "https://cstone.space/uifimages/3c09b16a-0c8b-41df-84c9-e0a4a271c0fa.png"
                        }]
                    }),
                },
                // A second entry the proxy CAN serve, so the assertion below
                // is about the host and not the field being broken outright.
                make_item_with_metadata(
                    "doom_armor_medium_helmet_02_01_01",
                    "The Butcher Helmet",
                    Some("the-butcher-helmet"),
                ),
            ])
            .await
            .unwrap();

        let (issuer, verifier) = fresh_pair();
        let token = mint_user_token(&issuer);
        let app = build_app(store, Arc::new(verifier));

        let (status, body) = post_json(
            app,
            serde_json::json!({
                "class_names": [
                    "qrt_specialist_heavy_core_01_01_01",
                    "doom_armor_medium_helmet_02_01_01"
                ]
            }),
            &token,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let resolved = &body["resolved"];
        assert_eq!(
            resolved["qrt_specialist_heavy_core_01_01_01"]["has_image"], false,
            "an image on a non-allowlisted host cannot be served, so it must \
             not be advertised — the client would render an <img> that 404s",
        );
        assert_eq!(
            resolved["doom_armor_medium_helmet_02_01_01"]["has_image"], true,
            "an allowlisted image must still be advertised",
        );
    }

    fn make_item_with_metadata(
        class_name: &str,
        display_name: &str,
        slug: Option<&str>,
    ) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Item,
            class_name: class_name.to_owned(),
            display_name: display_name.to_owned(),
            slug: slug.map(str::to_owned),
            metadata: serde_json::json!({
                "classification": "FPS.Armor.Helmet",
                "classification_label": "Helmet",
                "images": [{"original_url": "https://media.starcitizen.tools/x.jpg"}]
            }),
        }
    }

    fn mint_user_token(issuer: &crate::auth::TokenIssuer) -> String {
        issuer
            .sign_user(&uuid::Uuid::new_v4().to_string(), "TestUser")
            .expect("sign_user failed")
    }

    fn build_app(
        store: Arc<MemoryReferenceStore>,
        verifier: Arc<crate::auth::AuthVerifier>,
    ) -> axum::Router {
        Router::new()
            .route(
                "/v1/reference/resolve",
                post(resolve_reference_names::<MemoryReferenceStore>),
            )
            .with_state(store)
            .layer(axum::Extension(verifier))
    }

    async fn post_json(
        app: axum::Router,
        body: serde_json::Value,
        token: &str,
    ) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/reference/resolve")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    // -----------------------------------------------------------------------
    // New rich-resolve tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn resolves_rich_entry_and_omits_unknown() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[make_item_with_metadata(
                "doom_armor_medium_helmet_02_01_01",
                "The Butcher Helmet",
                Some("the-butcher-helmet"),
            )])
            .await
            .unwrap();

        let (issuer, verifier) = fresh_pair();
        let token = mint_user_token(&issuer);
        let app = build_app(store, Arc::new(verifier));

        let (status, body) = post_json(
            app,
            serde_json::json!({
                "class_names": ["doom_armor_medium_helmet_02_01_01", "nope_x"]
            }),
            &token,
        )
        .await;

        assert_eq!(status, StatusCode::OK);

        let resolved = &body["resolved"];

        // Hit: full rich fields
        let entry = &resolved["doom_armor_medium_helmet_02_01_01"];
        assert_eq!(entry["display_name"], "The Butcher Helmet");
        assert_eq!(entry["slug"], "the-butcher-helmet");
        assert_eq!(entry["category"], "item");
        assert_eq!(entry["classification"], "FPS.Armor.Helmet");
        assert_eq!(entry["classification_label"], "Helmet");
        assert_eq!(entry["has_image"], true);

        // Miss: must not appear
        assert!(
            resolved.get("nope_x").is_none(),
            "nope_x should not appear in resolved"
        );
    }

    // -----------------------------------------------------------------------
    // Retained guard tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_more_than_200_class_names() {
        let store = Arc::new(MemoryReferenceStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = mint_user_token(&issuer);
        let app = build_app(store, Arc::new(verifier));

        let class_names: Vec<String> = (0..201).map(|i| format!("item_{i}")).collect();
        let (status, _) = post_json(
            app,
            serde_json::json!({ "class_names": class_names }),
            &token,
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn accepts_exactly_200_class_names() {
        let store = Arc::new(MemoryReferenceStore::new());
        let (issuer, verifier) = fresh_pair();
        let token = mint_user_token(&issuer);
        let app = build_app(store, Arc::new(verifier));

        let class_names: Vec<String> = (0..200).map(|i| format!("item_{i}")).collect();
        let (status, body) = post_json(
            app,
            serde_json::json!({ "class_names": class_names }),
            &token,
        )
        .await;

        // 200 entries is within the cap; the store has no data so
        // `resolved` is empty, but the request itself is accepted.
        assert_eq!(status, StatusCode::OK);
        assert!(body["resolved"]
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn returns_401_without_auth_token() {
        let store = Arc::new(MemoryReferenceStore::new());
        let (_, verifier) = fresh_pair();
        let app = build_app(store, Arc::new(verifier));

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/reference/resolve")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({ "class_names": [] })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn resolves_across_multiple_categories() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[
                make_entry(
                    ReferenceCategory::Vehicle,
                    "AEGS_Avenger_Stalker",
                    "Avenger Stalker",
                ),
                make_entry(
                    ReferenceCategory::Weapon,
                    "klwe_lasercannon_s2",
                    "Sledge II",
                ),
                make_entry(ReferenceCategory::Item, "item_fps_ammo_01", "FPS Ammo"),
            ])
            .await
            .unwrap();

        let (issuer, verifier) = fresh_pair();
        let token = mint_user_token(&issuer);
        let app = build_app(store, Arc::new(verifier));

        let (status, body) = post_json(
            app,
            serde_json::json!({
                "class_names": [
                    "AEGS_Avenger_Stalker",
                    "klwe_lasercannon_s2",
                    "item_fps_ammo_01",
                    "nonexistent"
                ]
            }),
            &token,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let resolved = &body["resolved"];
        assert_eq!(
            resolved["AEGS_Avenger_Stalker"]["display_name"],
            "Avenger Stalker"
        );
        assert_eq!(resolved["AEGS_Avenger_Stalker"]["category"], "vehicle");
        assert_eq!(resolved["klwe_lasercannon_s2"]["display_name"], "Sledge II");
        assert_eq!(resolved["klwe_lasercannon_s2"]["category"], "weapon");
        assert_eq!(resolved["item_fps_ammo_01"]["display_name"], "FPS Ammo");
        assert_eq!(resolved["item_fps_ammo_01"]["category"], "item");
        assert!(resolved.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn missing_metadata_fields_are_null() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[make_entry(
                ReferenceCategory::Weapon,
                "rsi_p4ar_01",
                "P4-AR",
            )])
            .await
            .unwrap();

        let (issuer, verifier) = fresh_pair();
        let token = mint_user_token(&issuer);
        let app = build_app(store, Arc::new(verifier));

        let (status, body) = post_json(
            app,
            serde_json::json!({ "class_names": ["rsi_p4ar_01"] }),
            &token,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let entry = &body["resolved"]["rsi_p4ar_01"];
        assert_eq!(entry["display_name"], "P4-AR");
        assert_eq!(entry["category"], "weapon");
        assert!(entry["slug"].is_null());
        assert!(entry["classification"].is_null());
        assert!(entry["classification_label"].is_null());
        assert_eq!(entry["has_image"], false);
    }
}
