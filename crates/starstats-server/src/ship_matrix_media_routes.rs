//! Comply-on-request media proxy for RSI Ship Matrix images.
//!
//! `GET /v1/reference/vehicles/{class_name}/media/{idx}` resolves the
//! idx-th image URL from a vehicle row's `metadata.ship_matrix.media`,
//! fetches it from RSI, and streams it back. Proxying (rather than
//! hotlinking) gives three things the design needs:
//!   * an **instant kill-switch** — when `STARSTATS_SHIP_MATRIX_MEDIA`
//!     is off the route 404s, hiding every image with no data redeploy
//!     (the comply-on-request posture);
//!   * **no referrer / IP leakage** from the user's browser to RSI;
//!   * resilience to RSI hotlink-blocking.
//!
//! ## SSRF guard
//!
//! The URL comes from `metadata` (ultimately RSI data), but we still
//! restrict the proxy to `https://*.robertsspaceindustries.com` so it
//! can never be coaxed into fetching an arbitrary host.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};

use crate::reference_data::ReferenceCategory;
use crate::reference_store::ReferenceStore;

/// Upstream image fetch timeout.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Cap on a single proxied image. RSI store renders are < 1 MB; 8 MB is
/// generous headroom and bounds a single allocation.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct MediaState {
    store: Arc<dyn ReferenceStore>,
    http: reqwest::Client,
    /// The admin-managed media kill-switch (DB-backed, mirrored into
    /// this `AtomicBool` and hot-swapped on admin write). When false the
    /// route is dark.
    media_flag: Arc<AtomicBool>,
}

/// Build the media-proxy sub-router. `media_flag` is the shared
/// admin-managed kill-switch handle (also held by the admin router, so
/// an admin toggle takes effect here immediately).
pub fn routes(store: Arc<dyn ReferenceStore>, media_flag: Arc<AtomicBool>) -> Router {
    let http = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // CRITICAL for the SSRF guard: `is_allowed_rsi_url` only checks
        // the INITIAL URL, so redirects MUST be disabled — otherwise an
        // allowlisted RSI URL that 30x-redirects to an internal host
        // would be followed transparently. The proxy fetches a direct
        // image URL and never needs to follow redirects.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("ship matrix media proxy client builder failed");

    let governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(10)
            .burst_size(40)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("ship matrix media governor config builder produced no config"),
    );

    // Deliberately NOT in the OpenAPI spec / no `#[utoipa::path]`: this
    // streams raw image bytes, not a JSON DTO, so it doesn't fit the
    // schema model the data reference routes use. Omission is intentional.
    Router::new()
        .route(
            "/v1/reference/vehicles/:class_name/media/:idx",
            get(proxy_media),
        )
        .with_state(MediaState {
            store,
            http,
            media_flag,
        })
        .layer(GovernorLayer { config: governor })
}

/// Resolve `media[idx]` for a vehicle and stream the image back. 404 for
/// every miss (feature dark, unknown vehicle, no such image, non-RSI
/// host) so a probe can't distinguish the cases.
async fn proxy_media(
    State(st): State<MediaState>,
    Path((class_name, idx)): Path<(String, usize)>,
) -> Response {
    if !st.media_flag.load(Ordering::Relaxed) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let entry = match st
        .store
        .get_entry(ReferenceCategory::Vehicle, &class_name)
        .await
    {
        Ok(Some(e)) => e,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, class_name, "media proxy: store lookup failed");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let Some(url) = entry
        .metadata
        .get("ship_matrix")
        .and_then(|sm| sm.get("media"))
        .and_then(|m| m.get(idx))
        .and_then(|u| u.as_str())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if !is_allowed_rsi_url(url) {
        tracing::warn!(url, "media proxy: refusing non-RSI host");
        return StatusCode::NOT_FOUND.into_response();
    }

    let upstream = match st.http.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(status = %r.status(), url, "media proxy: upstream non-200");
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e, url, "media proxy: upstream fetch failed");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = match upstream.bytes().await {
        Ok(b) if b.len() <= MAX_IMAGE_BYTES => b,
        Ok(b) => {
            tracing::warn!(bytes = b.len(), url, "media proxy: image exceeds cap");
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e, url, "media proxy: body read failed");
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

/// Allow only `https://*.robertsspaceindustries.com` (and the apex).
/// Defence-in-depth against the proxy being steered at an arbitrary host.
fn is_allowed_rsi_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    match parsed.host_str() {
        Some(host) => {
            host == "robertsspaceindustries.com" || host.ends_with(".robertsspaceindustries.com")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_rsi_https_hosts() {
        assert!(is_allowed_rsi_url(
            "https://media.robertsspaceindustries.com/abc/source.jpg"
        ));
        assert!(is_allowed_rsi_url(
            "https://robertsspaceindustries.com/x.png"
        ));
    }

    #[test]
    fn rejects_non_rsi_and_non_https() {
        // Wrong host.
        assert!(!is_allowed_rsi_url("https://evil.example.com/x.jpg"));
        // Look-alike suffix trick.
        assert!(!is_allowed_rsi_url(
            "https://robertsspaceindustries.com.evil.com/x.jpg"
        ));
        // Plain http.
        assert!(!is_allowed_rsi_url(
            "http://media.robertsspaceindustries.com/x.jpg"
        ));
        // Garbage.
        assert!(!is_allowed_rsi_url("not a url"));
    }
}
