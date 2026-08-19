//! Tauri 2 auto-update manifest endpoint.
//!
//! Tauri's updater plugin polls a `GET` endpoint shaped like
//! `/{target}/{arch}/{current_version}` and expects either:
//!
//!  * `204 No Content` — no update is available; client stays put.
//!  * `200 OK` with the unified manifest JSON — client downloads the
//!    bundle for its `target/arch` and verifies the minisign signature.
//!
//! We surface the same contract here under `/v1/updater/...` so it
//! lives alongside the rest of the versioned API. The manifest itself
//! is a JSON file on disk (path resolved at startup from
//! `STARSTATS_UPDATER_MANIFEST_PATH`); whatever release tooling drops
//! signed bundles into the homelab also drops the manifest. We read it
//! per request — manifests are tiny (~1KB) and update infrequently, so
//! avoiding caching keeps the publish path "edit the file, done".
//!
//! Behaviour matrix:
//!  * manifest path absent on disk -> 204 (treat as "no update yet").
//!  * manifest version equals the client's `current_version` -> 204.
//!  * manifest version differs from `current_version` -> 200 + body.
//!  * manifest path set but file is unreadable / malformed JSON -> 503,
//!    with a `tracing::error!` so ops sees the misconfiguration.
//!
//! Version comparison: this crate doesn't depend on `semver`, so we use
//! string equality. That's safe for Tauri's monotonic release tags and
//! avoids a parse step that could turn a legitimate "new version"
//! signal into a 503 if the manifest carries an unparseable string.

use crate::config::UpdaterConfig;
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use utoipa::ToSchema;

/// One row of the `platforms` map in a Tauri 2 unified manifest. The
/// updater plugin expects `signature` + `url` keys verbatim (any extra
/// keys are tolerated, but these two are mandatory). We deserialize +
/// reserialize so a malformed manifest is caught early and surfaces as
/// a 503 instead of a confusing client-side parse error.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlatformBundle {
    /// Detached minisign signature of the bundle (base64). The Tauri
    /// updater verifies this against the public key embedded in the
    /// app at build time.
    pub signature: String,
    /// HTTPS URL of the bundle archive. Must be reachable from the
    /// client; in the homelab this is a Traefik-fronted GitHub release
    /// asset URL.
    pub url: String,
}

/// Top-level Tauri 2 update manifest. Field names are dictated by the
/// updater plugin — do not rename them.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateManifest {
    /// Latest version available, e.g. `1.2.3`. Compared against the
    /// `current_version` path param to decide 200 vs 204.
    pub version: String,
    /// Free-form release notes shown in the updater dialog.
    pub notes: String,
    /// RFC 3339 timestamp of the publication. Tauri renders this in
    /// the dialog; it does NOT influence update gating.
    pub pub_date: String,
    /// Per-target/arch bundle table. Keys are `"<target>-<arch>"`,
    /// e.g. `"windows-x86_64"`. We use BTreeMap for stable JSON key
    /// ordering — easier diffing in CI when the spec is dumped.
    pub platforms: BTreeMap<String, PlatformBundle>,
}

/// `GET /v1/updater/{target}/{arch}/{current_version}`.
///
/// Path params are accepted but not used for filtering — the manifest
/// already carries a `platforms` map keyed by `target-arch` and Tauri
/// itself picks the right entry client-side. We accept the full path
/// because the Tauri updater plugin always emits all three segments,
/// and rejecting them with a 404 would break the integration. The
/// segments are still useful in access logs for fleet visibility.
#[utoipa::path(
    get,
    path = "/v1/updater/{target}/{arch}/{current_version}",
    tag = "updater",
    params(
        ("target" = String, Path, description = "Tauri target triple component (e.g. `windows`, `linux`, `darwin`)"),
        ("arch" = String, Path, description = "CPU architecture (e.g. `x86_64`, `aarch64`)"),
        ("current_version" = String, Path, description = "Client's currently installed version, e.g. `1.2.2`"),
    ),
    responses(
        (status = 200, description = "Update available; body is the Tauri 2 unified manifest", body = UpdateManifest),
        (status = 204, description = "No update available (manifest absent or already on latest)"),
        (status = 503, description = "Manifest configured but unreadable or malformed"),
    ),
)]
pub async fn check_for_update(
    Extension(cfg): Extension<Arc<UpdaterConfig>>,
    Path((_target, _arch, current_version)): Path<(String, String, String)>,
) -> Response {
    match read_manifest(&cfg) {
        // No file on disk yet — that's not an error, it just means
        // the release tooling hasn't published anything. Treat as
        // "nothing to update to".
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Ok(Some(manifest)) => {
            if manifest.version == current_version {
                StatusCode::NO_CONTENT.into_response()
            } else {
                (StatusCode::OK, Json(manifest)).into_response()
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                path = %cfg.manifest_path.display(),
                "updater manifest unreadable"
            );
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

/// Read + parse the manifest file.
///
/// Returns:
///  * `Ok(None)` when the path doesn't exist on disk (the "no
///    update yet" signal). We match on `io::ErrorKind::NotFound`
///    directly rather than calling `path.exists()` first — that
///    pattern has a TOCTOU window where the file disappears between
///    the existence check and the read (e.g. mid-rotation), which
///    would surface as a spurious 503 instead of the intended 204.
///  * `Ok(Some(manifest))` on a successful parse.
///  * `Err(_)` when the path is set but the file is unreadable for
///    any reason other than NotFound, or contains malformed JSON —
///    surfaces as a 503 in the handler.
fn read_manifest(cfg: &UpdaterConfig) -> anyhow::Result<Option<UpdateManifest>> {
    let raw = match std::fs::read_to_string(&cfg.manifest_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let manifest: UpdateManifest = serde_json::from_str(&raw)?;
    Ok(Some(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use std::path::PathBuf;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Per-test scratch dir under `std::env::temp_dir()` keyed by a
    /// fresh UUID so parallel tests don't collide. The dropper
    /// best-effort removes the dir; if a panic skips it the OS temp
    /// reaper sweeps it eventually.
    struct ScratchDir {
        path: PathBuf,
    }

    impl ScratchDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("starstats-update-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create scratch dir");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn router_with_cfg(cfg: UpdaterConfig) -> Router {
        Router::new()
            .route(
                "/v1/updater/:target/:arch/:current_version",
                get(check_for_update),
            )
            .layer(Extension(Arc::new(cfg)))
    }

    async fn get_status_and_body(app: &Router, path: &str) -> (StatusCode, axum::body::Bytes) {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, bytes)
    }

    fn sample_manifest_json(version: &str) -> String {
        serde_json::json!({
            "version": version,
            "notes": "Bug fixes and perf",
            "pub_date": "2026-05-04T12:00:00Z",
            "platforms": {
                "windows-x86_64": {
                    "signature": "sig-w",
                    "url": "https://example.com/win.zip"
                },
                "linux-x86_64": {
                    "signature": "sig-l",
                    "url": "https://example.com/linux.tar.gz"
                }
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn returns_204_when_manifest_path_does_not_exist() {
        let scratch = ScratchDir::new();
        let cfg = UpdaterConfig {
            manifest_path: scratch.join("missing.json"),
        };
        let app = router_with_cfg(cfg);

        let (status, _) = get_status_and_body(&app, "/v1/updater/windows/x86_64/1.0.0").await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn returns_200_with_manifest_when_versions_differ() {
        let scratch = ScratchDir::new();
        let manifest_path = scratch.join("manifest.json");
        std::fs::write(&manifest_path, sample_manifest_json("1.2.3")).unwrap();
        let cfg = UpdaterConfig { manifest_path };
        let app = router_with_cfg(cfg);

        let (status, bytes) = get_status_and_body(&app, "/v1/updater/windows/x86_64/1.0.0").await;
        assert_eq!(status, StatusCode::OK);

        let body: UpdateManifest = serde_json::from_slice(&bytes).expect("parse manifest body");
        assert_eq!(body.version, "1.2.3");
        assert_eq!(body.notes, "Bug fixes and perf");
        assert_eq!(body.platforms.len(), 2);
        let win = body
            .platforms
            .get("windows-x86_64")
            .expect("windows entry present");
        assert_eq!(win.signature, "sig-w");
        assert_eq!(win.url, "https://example.com/win.zip");
    }

    #[tokio::test]
    async fn returns_204_when_current_version_matches_manifest() {
        let scratch = ScratchDir::new();
        let manifest_path = scratch.join("manifest.json");
        std::fs::write(&manifest_path, sample_manifest_json("1.2.3")).unwrap();
        let cfg = UpdaterConfig { manifest_path };
        let app = router_with_cfg(cfg);

        let (status, _) = get_status_and_body(&app, "/v1/updater/windows/x86_64/1.2.3").await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn returns_503_when_manifest_is_malformed_json() {
        let scratch = ScratchDir::new();
        let manifest_path = scratch.join("manifest.json");
        std::fs::write(&manifest_path, "{ this is not valid json").unwrap();
        let cfg = UpdaterConfig { manifest_path };
        let app = router_with_cfg(cfg);

        let (status, _) = get_status_and_body(&app, "/v1/updater/windows/x86_64/1.0.0").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
