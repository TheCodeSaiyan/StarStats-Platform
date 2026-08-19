//! Telemetry bootstrap.
//!
//! Wires four planes into a single subscriber:
//! - **Logs** — JSON to stdout (always on; the OTel collector reads
//!   stdout via filelog and forwards to Loki).
//! - **Metrics** — Prometheus recorder installed globally; the
//!   `/metrics` endpoint renders via the returned [`PrometheusHandle`].
//! - **Traces** — OTLP gRPC exporter to `OTEL_EXPORTER_OTLP_ENDPOINT`,
//!   wired through `tracing-opentelemetry`. Skipped silently if the
//!   env var is unset (i.e. local dev without a collector).
//! - **Filter** — `RUST_LOG`-style; default keeps sqlx quiet but turns
//!   our own crate up to debug.

use anyhow::{Context, Result};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace as sdktrace, Resource};
use std::sync::Arc;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Handles returned to the application after telemetry init.
///
/// The `prometheus` handle is what `/metrics` renders.
/// The `otel_guard` MUST be held for the lifetime of the process and
/// dropped at shutdown so the exporter flushes queued spans.
pub struct TelemetryHandles {
    pub prometheus: Arc<PrometheusHandle>,
    pub otel_guard: Option<OtelGuard>,
}

/// Marker held for the duration of the program. On `Drop` it
/// instructs the global tracer provider to flush queued spans
/// synchronously so late requests still reach Tempo.
pub struct OtelGuard;

impl Drop for OtelGuard {
    fn drop(&mut self) {
        global::shutdown_tracer_provider();
    }
}

pub fn init_telemetry() -> Result<TelemetryHandles> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,starstats=debug,sqlx=warn,tower_http=debug"));

    let json_layer = fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(false);

    // Build the trace pipeline conditionally. Local dev (no collector)
    // skips this and the rest of telemetry still works.
    let (otel_layer, otel_guard) = match build_otel_pipeline() {
        Ok(Some(tracer)) => (
            Some(tracing_opentelemetry::layer().with_tracer(tracer)),
            Some(OtelGuard),
        ),
        Ok(None) => (None, None),
        Err(e) => {
            // Don't fail boot just because the collector is unreachable
            // — degrade to logs-only and surface the reason.
            eprintln!("OTel tracing disabled: {e:#}");
            (None, None)
        }
    };

    // `Option<Layer>` itself implements `Layer`, so we can register the
    // OTel layer unconditionally — `None` is a no-op composition.
    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .with(otel_layer)
        .init();

    let prometheus = PrometheusBuilder::new()
        .install_recorder()
        .context("install prometheus recorder")?;

    Ok(TelemetryHandles {
        prometheus: Arc::new(prometheus),
        otel_guard,
    })
}

/// Build the OTLP tracer pipeline if `OTEL_EXPORTER_OTLP_ENDPOINT` is
/// set. Returns `Ok(None)` when no endpoint is configured.
fn build_otel_pipeline() -> Result<Option<sdktrace::Tracer>> {
    let endpoint = match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => return Ok(None),
    };

    let service_name =
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "starstats-api".to_string());

    // `install_batch` returns the configured `TracerProvider` and also
    // installs it as the global one, which is what `shutdown_tracer_provider`
    // (in `OtelGuard::drop`) ends up flushing.
    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(&endpoint),
        )
        .with_trace_config(
            sdktrace::Config::default().with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                service_name,
            )])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .context("install OTLP tracing pipeline")?;

    Ok(Some(provider.tracer("starstats-api")))
}
