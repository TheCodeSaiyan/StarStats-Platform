# Observability

Four orthogonal telemetry planes. They share infrastructure but have
different storage, retention, and access semantics.

| Plane | What | Backend | Retention |
|---|---|---|---|
| **Logs** | "What happened" — structured events from every service | Loki | 30 days |
| **Metrics** | "How often / how fast" — numeric time series | Prometheus | 15 days |
| **Traces** | "Where did time go" — distributed spans across services | Tempo | 7 days |
| **Audit** | "Who did what to which resource" — tamper-evident | Postgres `audit_log` + Loki + MinIO Object Lock | 7 years |
| **Crashes** | Stack traces, breadcrumbs, release health | GlitchTip | 90 days |

All emitters speak **OpenTelemetry**. The OTEL Collector is the single
ingest point; backends are pluggable behind it.

## Per-component matrix

| Component | Logs | Metrics | Traces | Audit | Crashes |
|---|---|---|---|---|---|
| Tray client (Tauri) | local rolling JSON; opt-in OTLP export | local counters; tray Status pane | opt-in OTLP | local JSONL for cookie/account changes | GlitchTip Rust SDK (opt-in) |
| starstats-core (lib) | tracing events | metrics counters | `#[instrument]` spans | (n/a) | (caller handles) |
| starstats-api (Axum) | stdout JSON → OTEL → Loki | `axum-prometheus` `/metrics` → Prometheus | tracing-opentelemetry → OTEL → Tempo | `audit_log` table (hash-chained) → Loki + MinIO | GlitchTip Rust SDK |
| starstats-web (Next.js) | pino JSON stdout → OTEL → Loki; browser → GlitchTip | `prom-client` → Prometheus; Web Vitals → OTEL | `@opentelemetry/sdk-node` → OTEL → Tempo | (delegated to API) | GlitchTip browser + server SDK |
| Postgres | stdout → OTEL filelog → Loki; pgaudit | `postgres_exporter` → Prometheus | (sqlx instrumentation in callers) | pgaudit + `audit_log` table | (n/a) |
| Authentik | stdout → Loki | `/-/health/ready/` polling | (n/a — Python service) | built-in event store | (n/a) |
| SpiceDB | stdout → Loki | `:9090/metrics` → Prometheus | OTLP → Tempo | WatchAPI stream → `audit_log` | (n/a) |
| MinIO | stdout → Loki; bucket audit webhook | `/minio/v2/metrics/cluster` → Prometheus | OTLP support → Tempo | bucket audit → `audit_log`; Object Lock | (n/a) |
| Traefik | access log file → Loki | (existing exporter pattern) | passthrough `traceparent` | (n/a) | (n/a) |
| OTEL Collector | self stdout → Loki (avoiding loops via filelog exclude) | `:8888/metrics` → Prometheus | (gateway) | (n/a) | (n/a) |

## Trace propagation

Every HTTP request carries a `traceparent` header per W3C Trace
Context. Tray client → API → SpiceDB → Postgres queries — one trace
ID spans the whole flow.

```
[tray client]   trace=abc123
     │ HTTPS, traceparent: 00-abc123-...
     ▼
[starstats-api] root span
     │ instruments sqlx + spicedb client
     ├─▶ [postgres]  child span via sqlx
     └─▶ [spicedb]   child span via tonic
```

Grafana joins traces↔logs by `trace_id` so you can click from a slow
request straight to its log lines, and from a log line to the
request that produced it.

## Metric naming

OpenTelemetry semantic conventions. Examples:

- `http.server.request.duration` (histogram, ms) — every API request
- `http.server.active_requests` (gauge) — current concurrency
- `db.client.connections.usage` (gauge) — sqlx pool
- `starstats.events.ingested` (counter) — events accepted
- `starstats.events.rejected` (counter) with attribute `reason=…` — events refused
- `starstats.sync.lag.seconds` (histogram) — client→server delay
- `starstats.spicedb.check.duration` (histogram) — permission checks

Per-tenant cardinality control: never label by `user_id` or `org_id`
in metrics; use them only in logs and traces.

## Log discipline

- **Structured JSON only**, one event per line.
- **Use levels honestly**: `error` for things a human must investigate,
  `warn` for self-recovering anomalies, `info` for state changes,
  `debug` for diagnostic detail (off in prod by default).
- **Never log secrets**, raw tokens, RSI cookies, password hashes.
  Use `tracing::field::Empty` and `record()` to redact.
- **Include `trace_id` and `span_id`** in every log line. Both
  `tracing-subscriber` (Rust) and `pino` (Node) wire this automatically
  when OTEL is configured.
- **Include `request_id`** generated at the edge (Traefik or first
  Axum middleware) for cases where there is no trace.

## Alert seeds

To wire into Grafana later — these belong in
`infra/prometheus/alerts.yml` once Phase 5 (production hardening)
lands:

- API p99 latency > 500ms for 5m
- API 5xx rate > 1% for 5m
- Postgres connection pool saturation > 80% for 5m
- MinIO bucket size > 80% of disk
- Loki ingester back-pressure
- SpiceDB authoritative reads p99 > 50ms
- audit_log write lag > 30s (mirror to Loki broken)
- `starstats_parser_health_open_findings` > 0 for 1h — a log event type has
  stopped being produced while users stayed active (classifier likely broken
  by a game patch)
- `time() - starstats_parser_health_last_success_timestamp` > 36h — the
  parser-health detector itself has stopped. Rank this ABOVE the finding-count
  alert: a dead detector reports zero findings, which is indistinguishable
  from healthy unless this alert exists.

Don't ship these before there's enough signal to set thresholds
honestly. Premature alerting is worse than none.
