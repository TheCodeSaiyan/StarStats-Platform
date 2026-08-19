//! Liveness, readiness, metrics endpoints.
//!
//! `/healthz` is shallow — process is up.
//! `/readyz`  is deep — required deps reachable. Checks Postgres,
//!            (if configured) SpiceDB, and (if configured) the MinIO
//!            audit mirror.
//! `/metrics` returns Prometheus text format from the global recorder.

use crate::audit_mirror::MinioMirror;
use crate::spicedb::SpicedbClient;
use axum::{http::StatusCode, response::IntoResponse, Extension, Json};
use metrics_exporter_prometheus::PrometheusHandle;
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;

#[derive(Serialize)]
struct HealthResponse<'a> {
    status: &'a str,
    version: &'a str,
}

/// Exported schema-only mirror of the (`'a`-lifetimed) HealthResponse
/// used internally. utoipa's derive macro doesn't grok borrowed
/// `&'a str`, so we publish a `String`-shaped view of the same JSON.
#[derive(Serialize, ToSchema)]
#[allow(dead_code)]
pub struct HealthResponseSchema {
    pub status: String,
    pub version: String,
}

#[derive(Serialize)]
struct ReadyResponse<'a> {
    status: &'a str,
    version: &'a str,
    checks: ReadyChecks<'a>,
}

#[derive(Serialize)]
struct ReadyChecks<'a> {
    postgres: &'a str,
    /// `"ok"` / `"fail"` when SpiceDB is configured, `"skipped"` when
    /// it isn't. A skipped dep does NOT degrade readiness — only
    /// configured-but-unhealthy deps return 503.
    spicedb: &'a str,
    /// `"ok"` / `"fail"` when the MinIO audit mirror is configured,
    /// `"skipped"` when it isn't. Same tri-state semantics as
    /// `spicedb`.
    minio: &'a str,
}

/// `String`-shaped mirror of ReadyResponse for the OpenAPI spec.
#[derive(Serialize, ToSchema)]
#[allow(dead_code)]
pub struct ReadyResponseSchema {
    pub status: String,
    pub version: String,
    pub checks: ReadyChecksSchema,
}

#[derive(Serialize, ToSchema)]
#[allow(dead_code)]
pub struct ReadyChecksSchema {
    pub postgres: String,
    pub spicedb: String,
    pub minio: String,
}

#[utoipa::path(
    get,
    path = "/healthz",
    tag = "health",
    responses((status = 200, description = "Process is up", body = HealthResponseSchema))
)]
pub async fn live() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
}

/// Deep readiness — fails with 503 if any *configured* dep is
/// unreachable. Kubernetes / docker-compose use this as the gate for
/// "should we route traffic here yet". Skipped deps (SpiceDB without
/// a preshared key, MinIO without an access key, etc.) are reported
/// as "skipped" but do not flip the response to 503.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = "health",
    responses(
        (status = 200, description = "All configured deps reachable", body = ReadyResponseSchema),
        (status = 503, description = "At least one configured dep is unhealthy", body = ReadyResponseSchema),
    )
)]
pub async fn ready(
    Extension(pool): Extension<PgPool>,
    Extension(spicedb): Extension<Arc<Option<SpicedbClient>>>,
    Extension(minio): Extension<Arc<Option<MinioMirror>>>,
) -> impl IntoResponse {
    // Each probe LOGS its cause before being collapsed to a bool.
    //
    // `.is_ok()` alone discards the error, and these probes build good
    // ones — MinIO's says which bucket and that it is "missing or
    // credentials invalid". Without logging, a degraded readiness is
    // reported as the single word "fail" and there is no way, from the
    // response or the logs, to tell an unreachable endpoint from a
    // missing bucket from a rotated key. That happened: production sat
    // at 503 `minio: fail` with no recoverable cause anywhere.
    //
    // Logged server-side only, NOT returned in the body: `/readyz` is
    // publicly reachable and the cause text carries bucket names and
    // endpoints. The response keeps its coarse ok/fail/skipped vocabulary.
    let pg_ok = match sqlx::query("SELECT 1").fetch_one(&pool).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "readyz: postgres probe failed");
            false
        }
    };

    // Tri-state: Some(true) = configured & ok, Some(false) = configured
    // & fail, None = not configured (skipped).
    let spicedb_status: Option<bool> = match spicedb.as_ref() {
        Some(client) => Some(match client.ping().await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "readyz: spicedb probe failed");
                false
            }
        }),
        None => None,
    };
    let minio_status: Option<bool> = match minio.as_ref() {
        Some(mirror) => Some(match mirror.ping().await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "readyz: minio probe failed");
                false
            }
        }),
        None => None,
    };

    let all_ok =
        pg_ok && !matches!(spicedb_status, Some(false)) && !matches!(minio_status, Some(false));

    let status_code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = ReadyResponse {
        status: if all_ok { "ok" } else { "degraded" },
        version: env!("CARGO_PKG_VERSION"),
        checks: ReadyChecks {
            postgres: if pg_ok { "ok" } else { "fail" },
            spicedb: match spicedb_status {
                Some(true) => "ok",
                Some(false) => "fail",
                None => "skipped",
            },
            minio: match minio_status {
                Some(true) => "ok",
                Some(false) => "fail",
                None => "skipped",
            },
        },
    };

    (status_code, Json(body))
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "health",
    responses((
        status = 200,
        description = "Prometheus text-format metrics",
        content_type = "text/plain",
        body = String,
    ))
)]
pub async fn metrics(Extension(handle): Extension<Arc<PrometheusHandle>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        handle.render(),
    )
}
