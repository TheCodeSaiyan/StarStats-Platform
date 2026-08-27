//! Same-origin media proxy for item/weapon KB images.
//!
//! `GET /v1/reference/{category}/{class}/media/{idx}` resolves the
//! idx-th image URL from an item or weapon row's `metadata.images[idx].original_url`,
//! fetches it from `media.starcitizen.tools`, and streams the bytes back.
//!
//! Only `item` and `weapon` categories are served — `vehicle` and `location`
//! images are either absent or served via the Ship Matrix media proxy.
//!
//! ## SSRF guard
//!
//! The URL comes from `metadata` (ultimately community-wiki data), but
//! the proxy is restricted to `media.starcitizen.tools` only. Any other
//! host — including look-alike suffixes like
//! `media.starcitizen.tools.evil.com` — returns 404. Redirects are
//! disabled so a redirect chain cannot escape the allowlist.
//!
//! ## Route collision analysis
//!
//! The route shape is `/:category/:class/media/:idx` — five segments deep.
//! Existing routes at three segments (`/:category/:class_name`) or four
//! segments (`/:category/by-class/:class_name`, `/:category/slug/:slug`,
//! `/:category/stats`, `/:category/compare`, `/:category/cohort`) never
//! match five-segment paths. No collision.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::sync::Arc;

use crate::reference_data::ReferenceCategory;
use crate::reference_store::ReferenceStore;

/// Upstream image fetch timeout.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Cap on a single proxied image. Wiki renders are typically < 1 MB;
/// 8 MB is generous headroom and bounds a single allocation.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Return true iff `url` has scheme `https` AND host exactly
/// `media.starcitizen.tools` (case-insensitive).
///
/// Look-alike suffixes such as `media.starcitizen.tools.evil.com` are
/// rejected by the exact-host comparison. Plain `http` is rejected by the
/// scheme check.
pub fn allowed_image_host(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            if u.scheme() != "https" {
                return None;
            }
            u.host_str()
                .map(|h| h.eq_ignore_ascii_case("media.starcitizen.tools"))
        })
        .unwrap_or(false)
}

/// Categories eligible for the media proxy (item + weapon only).
fn parse_media_category(raw: &str) -> Option<ReferenceCategory> {
    match raw {
        "item" => Some(ReferenceCategory::Item),
        "weapon" => Some(ReferenceCategory::Weapon),
        _ => None,
    }
}

#[derive(Clone)]
pub(crate) struct MediaState {
    store: Arc<dyn ReferenceStore>,
    http: reqwest::Client,
}

/// Build the item/weapon media-proxy sub-router.
pub fn routes(store: Arc<dyn ReferenceStore>) -> Router {
    let http = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // Redirects MUST be disabled — the SSRF guard only checks the
        // initial URL; a redirect to an internal host would bypass it.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reference media proxy client builder failed");

    Router::new()
        .route(
            "/v1/reference/:category/:class_name/media/:idx",
            get(proxy_reference_media),
        )
        .with_state(MediaState { store, http })
}

/// Resolve `metadata.images[idx].original_url` and stream the image.
///
/// Returns 404 for:
/// - unknown or disallowed category (not `item` / `weapon`)
/// - class_name not found in the store
/// - `idx` out of range
/// - missing `original_url` field
/// - host not exactly `media.starcitizen.tools`
///
/// Returns 502 on upstream fetch/read errors.
#[utoipa::path(
    get,
    path = "/v1/reference/{category}/{class_name}/media/{idx}",
    tag = "reference",
    operation_id = "reference_proxy_media",
    params(
        ("category" = String, Path, description = "One of: item, weapon"),
        ("class_name" = String, Path, description = "Entry class_name (case-insensitive)"),
        ("idx" = usize, Path, description = "Zero-based index into metadata.images"),
    ),
    responses(
        (status = 200, description = "Image bytes (content-type forwarded from upstream, Cache-Control: public max-age=86400)"),
        (status = 404, description = "Unknown category, class not found, index out of range, or host not allowlisted"),
        (status = 502, description = "Upstream fetch error or non-success response"),
    ),
)]
pub async fn proxy_reference_media(
    State(st): State<MediaState>,
    Path((category, class_name, idx)): Path<(String, String, usize)>,
) -> Response {
    let Some(cat) = parse_media_category(&category) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let entry = match st.store.get_entry(cat, &class_name).await {
        Ok(Some(e)) => e,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, class_name, "reference media proxy: store lookup failed");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let Some(url) = entry
        .metadata
        .get("images")
        .and_then(|arr| arr.get(idx))
        .and_then(|img| img.get("original_url"))
        .and_then(|v| v.as_str())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if !allowed_image_host(url) {
        tracing::warn!(url, "reference media proxy: refusing non-allowlisted host");
        return StatusCode::NOT_FOUND.into_response();
    }

    let upstream = match st.http.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(status = %r.status(), url, "reference media proxy: upstream non-200");
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e, url, "reference media proxy: upstream fetch failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    // REFUSE AN OVERSIZED IMAGE BEFORE DOWNLOADING IT.
    //
    // The cap used to be enforced only after `bytes().await`, which reads the
    // whole body into memory first — so it rejected the response without ever
    // having bounded the allocation it exists to bound. A real 10.75 MB PNG in
    // the catalogue (`Pembroke_-_Backback.png`) was buffered in full on every
    // request and then thrown away.
    if let Some(len) = upstream.content_length() {
        if len > MAX_IMAGE_BYTES as u64 {
            tracing::warn!(
                bytes = len,
                url,
                "reference media proxy: declared length exceeds cap; not fetching body"
            );
            return StatusCode::NOT_FOUND.into_response();
        }
    }

    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = match upstream.bytes().await {
        Ok(b) if b.len() <= MAX_IMAGE_BYTES => b,
        Ok(b) => {
            // Backstop for a response that declared no Content-Length.
            //
            // NOT a 502. The upstream answered perfectly well; WE decline to
            // serve an image this big. Reporting our own policy as a gateway
            // failure sent operators looking for a broken upstream, and told
            // the browser to treat a permanent condition as a server error.
            // 404 is what "there is no image here for you" means, and it is
            // what the client's onError already handles.
            tracing::warn!(
                bytes = b.len(),
                url,
                "reference media proxy: image exceeds cap"
            );
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e, url, "reference media proxy: body read failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
        ],
        Body::from(bytes),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_data::{ReferenceCategory, ReferenceEntry};
    use crate::reference_store::test_support::MemoryReferenceStore;
    use axum::http::Request;
    use tower::ServiceExt;

    // -----------------------------------------------------------------------
    // SSRF host-guard unit tests (RED → GREEN)
    // -----------------------------------------------------------------------

    #[test]
    fn allowed_image_host_accepts_starcitizen_tools_media() {
        assert!(
            allowed_image_host("https://media.starcitizen.tools/x.jpg"),
            "exact match must be accepted"
        );
        assert!(
            allowed_image_host("https://MEDIA.STARCITIZEN.TOOLS/x.jpg"),
            "case-insensitive host match must be accepted"
        );
        assert!(
            allowed_image_host("https://media.starcitizen.tools/path/to/image.png?size=800"),
            "path + query must be accepted when host is exact"
        );
    }

    #[test]
    fn allowed_image_host_rejects_evil_hosts() {
        // Wrong host entirely
        assert!(
            !allowed_image_host("https://evil.com/x.jpg"),
            "unrelated host must be rejected"
        );
        // Trailing-host trick: subdomain-of-evil that looks like the target
        assert!(
            !allowed_image_host("http://media.starcitizen.tools.evil.com/x"),
            "trailing-host suffix trick must be rejected"
        );
        // Same trailing-host trick with https — isolates the host-equality guard
        assert!(
            !allowed_image_host("https://media.starcitizen.tools.evil.com/x"),
            "https trailing-host suffix trick must be rejected (host-equality guard)"
        );
        // Plain http (not https)
        assert!(
            !allowed_image_host("http://media.starcitizen.tools/x.jpg"),
            "http scheme must be rejected"
        );
        // Garbage
        assert!(
            !allowed_image_host("not a url"),
            "garbage input must be rejected"
        );
        // Empty
        assert!(!allowed_image_host(""), "empty string must be rejected");
    }

    // -----------------------------------------------------------------------
    // Route-layer tests (Memory store, no live upstream)
    // -----------------------------------------------------------------------

    fn make_item_with_images(class_name: &str) -> ReferenceEntry {
        ReferenceEntry {
            category: ReferenceCategory::Item,
            class_name: class_name.to_owned(),
            display_name: "Test Item".to_owned(),
            slug: None,
            metadata: serde_json::json!({
                "images": [
                    {"original_url": "https://media.starcitizen.tools/foo.jpg"},
                    {"original_url": "https://media.starcitizen.tools/bar.jpg"}
                ]
            }),
        }
    }

    fn test_app(store: Arc<MemoryReferenceStore>) -> axum::Router {
        let store_dyn: Arc<dyn ReferenceStore> = store;
        routes(store_dyn)
    }

    async fn get_status(app: axum::Router, uri: &str) -> axum::http::StatusCode {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    #[tokio::test]
    async fn missing_class_name_returns_404() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);

        let status = get_status(app, "/v1/reference/item/missing_class/media/0").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn idx_out_of_range_returns_404() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[make_item_with_images("doom_armor_medium_helmet_01")])
            .await
            .unwrap();
        let app = test_app(store);

        // Index 99 is past the 2-element images array
        let status = get_status(
            app,
            "/v1/reference/item/doom_armor_medium_helmet_01/media/99",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn disallowed_category_returns_404() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);

        // "location" is not a permitted category for this proxy
        let status = get_status(app, "/v1/reference/location/some_place/media/0").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn vehicle_category_returns_404() {
        let store = Arc::new(MemoryReferenceStore::new());
        let app = test_app(store);

        // "vehicle" images are served by the ship_matrix_media_routes proxy
        let status = get_status(app, "/v1/reference/vehicle/AEGS_Avenger_Stalker/media/0").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn entry_without_images_field_returns_404() {
        let store = Arc::new(MemoryReferenceStore::new());
        store
            .upsert_entries(&[ReferenceEntry {
                category: ReferenceCategory::Item,
                class_name: "no_images_item".to_owned(),
                display_name: "No Images".to_owned(),
                slug: None,
                metadata: serde_json::json!({}),
            }])
            .await
            .unwrap();
        let app = test_app(store);

        let status = get_status(app, "/v1/reference/item/no_images_item/media/0").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn parse_media_category_accepts_item_and_weapon() {
        assert_eq!(parse_media_category("item"), Some(ReferenceCategory::Item));
        assert_eq!(
            parse_media_category("weapon"),
            Some(ReferenceCategory::Weapon)
        );
    }

    #[test]
    fn parse_media_category_rejects_vehicle_and_location() {
        assert_eq!(parse_media_category("vehicle"), None);
        assert_eq!(parse_media_category("location"), None);
        assert_eq!(parse_media_category("npc"), None);
    }
}
