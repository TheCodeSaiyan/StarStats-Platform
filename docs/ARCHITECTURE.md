# Architecture

End-to-end view of how StarStats is laid out, how data moves, and
why the major shape decisions were made. For telemetry plumbing see
[`OBSERVABILITY.md`](OBSERVABILITY.md); for the audit log
specifically see [`AUDIT.md`](AUDIT.md).

## Threat model

The non-negotiable constraint: **Easy Anti-Cheat (EAC) must never see
us touching the game process or its network**. Every data plane is
something the game has already written to disk, or a website you
authenticate against as yourself.

| Plane | EAC-visible? | Why |
|---|---|---|
| `Game.log` tailing | No | We open a file the game flushed. Same posture as Notepad. |
| RSI website (your own session) | No | Plain HTTPS as your browser would issue it. |
| Server-to-server (tray → API) | No | Initiated by our own process, never the game. |

If a feature ever requires reading game memory, hooking the client,
or anything that EAC's static or runtime checks could classify as
tampering, **it does not belong in StarStats**. Every component in
this design respects that boundary.

## Component map

```
┌────────────────────┐      ┌────────────────────────┐      ┌──────────────────────┐
│  Tauri tray        │──┐   │  StarStats API         │──┐   │  StarStats web       │
│  (Rust + Vite UI)  │  │   │  (Rust + Axum)         │  │   │  (Next.js 15, RSC)   │
│                    │  │   │                        │  │   │                      │
│  - Game.log tail   │  │   │  - /v1/ingest          │  │   │  - /auth/*           │
│  - Local SQLite    │  ├──>│  - /v1/me/{events,…}   │<─┤   │  - /dashboard        │
│  - Sync queue      │  │   │  - /v1/auth/*          │  │   │  - /devices          │
│  - System tray UI  │  │   │  - /openapi.json       │  │   │  - /api/metrics      │
└────────────────────┘  │   └────────────────────────┘  │   └──────────────────────┘
                        │             │ │ │ │ │         │             │
              device JWT │             │ │ │ │ │         │ user JWT     │ pino +
              + JWKS     │             │ │ │ │ │         │ (cookie)     │ OTLP
              discovery  │             │ │ │ │ │         │              ▼
                        │             │ │ │ │ ▼         │     ┌──────────────────┐
                        │             │ │ │ ▼ Loki/Tempo │     │  OTel Collector  │
                        │             │ │ ▼ MinIO       │     └──────────────────┘
                        │             │ ▼ SpiceDB        │
                        │             ▼ Postgres         │
                        └──────── shared ─────────────────┘
                                  starstats-core
```

### Rust workspace

| Crate | Role | Key modules |
|---|---|---|
| `starstats-core` | Shared types — wire format, `GameEvent` enum, parser, validators. Pulled in by both tray and server so a parser change can't cause client/server skew. | `events.rs`, `parser.rs`, `wire.rs`, `validators.rs` |
| `starstats-client` | Tauri tray's Rust backend. Tails `Game.log`, persists events locally in SQLite, drains them upstream when sync is enabled, exposes IPC commands to the React UI. | `gamelog.rs`, `storage.rs`, `sync.rs`, `discovery.rs`, `commands.rs` |
| `starstats-server` | API server. First-party JWT auth, device pairing, event ingest, query endpoints, audit log with optional MinIO mirror, Prometheus `/metrics`, OTLP traces, JWKS publication. | `auth.rs`, `auth_routes.rs`, `ingest.rs`, `query.rs`, `audit.rs`, `audit_mirror.rs`, `spicedb.rs`, `mail.rs`, `telemetry.rs`, `health.rs`, `well_known.rs`, `openapi.rs` |

`starstats-client` is excluded from default workspace builds because
Tauri requires GTK/WebKit2GTK on Linux and WebView2 on Windows — CI
builds it on dedicated matrix runners with the platform deps.

### Frontend apps

| App | Stack | Role |
|---|---|---|
| `apps/tray-ui` | Vite + React 19 + TypeScript inside a Tauri webview. | Tray system menu, status pane, settings, device pairing. Talks to the Rust backend exclusively via Tauri IPC — no direct network calls. |
| `apps/web` | Next.js 15 App Router, React 19, TypeScript. Server-side rendered with React Server Components. | Sign-up / sign-in, email verification landing, dashboard (event timeline + type breakdown), device management. All API calls funnel through the Next.js server, never the browser — the user JWT lives in an HttpOnly `starstats_session` cookie. |

### Generated TypeScript client

`packages/api-client-ts/src/generated/schema.ts` is auto-generated
from the server's OpenAPI 3.1 spec via
`packages/api-client-ts/scripts/generate.ts`. CI runs the generator
and fails the build if output drifts from the committed copy — that's
the contract enforcement between server and TS consumers.

## Data flow

### Local tail (tray client)

```
Game.log -tail-> structural_parse -> classify (GameEvent variant)
                                          │
                          ┌───── noise? ────┘
                          │ yes              │ no
                          ▼                  ▼
                   noise_list table     events table
                                              │
                                              ▼
                                       sync queue (ordered by id)
```

Two-pass parsing keeps adding new event variants safe: the structural
pass extracts timestamp/level/event-name/rest from the log line, and
the classify pass owns the per-variant regex tree. New variants
require touching only `classify` — the structural parser is stable.

A noise list filters engine-internal chatter (`StatObjLoad`,
`ContextEstablisher*`, etc.) before lines reach the unknown-events
table. Built-in defaults plus user-extensible entries via the tray
"ignore" button in the Status pane.

### Sync to server

When sync is enabled, the tray drains its queue in batches:

```
tray         -> POST /v1/ingest (Bearer <device JWT>)
                Body: IngestBatch { schema_version, batch_id,
                                    claimed_handle, events[] }
server       <- 200 IngestResponse { accepted, duplicate, rejected }
```

The server cross-checks `claimed_handle` against the token's
`preferred_username` (case-insensitive) — a device can only push
events under the user it was paired by. Idempotency is by
`(claimed_handle, idempotency_key)` so retries are free.

Each accepted event lands in two places:

1. The `events` table, partition-friendly with a per-handle event_seq.
2. An entry in the hash-chained `audit_log` describing the batch.

If MinIO is configured, the audit row is best-effort mirrored as
NDJSON to `s3://${audit_bucket}/audit/YYYY/MM/DD/{seq}.json`. Mirror
failures log a warning and continue — Postgres remains the source of
truth, and reconciliation is documented as a future job in
`AUDIT.md`.

### Web read path

```
browser -> GET /dashboard
            (cookie: starstats_session)
              │
              ▼
        Next.js server component
              │ Promise.all([getSummary(), listEvents()])
              ▼
        server-side fetch (user JWT from cookie)
              │
              ▼
        starstats-api: /v1/me/summary, /v1/me/events
              │
              ▼
        Postgres EventQuery  (advisory SpiceDB check on summary)
```

The browser never sees the JWT — the cookie is HttpOnly + SameSite=Lax,
and all server-side fetches happen inside Next.js's Node runtime.

## Auth model

### First-party JWT

The API server is its own identity provider. At boot it loads the
RSA private key at `STARSTATS_JWT_KEY_FILE` (default
`/var/lib/starstats/jwt-key.pem`, mode 0600). Tokens are RS256 with
`iss = STARSTATS_JWT_ISSUER` and `aud = STARSTATS_JWT_AUDIENCE`. The
public key is published as a JWKS document at
`/.well-known/jwks.json` so any third-party verifier can validate
tokens without round-tripping back to us.

If the key file is missing at boot, the server refuses to start
unless `STARSTATS_JWT_KEY_AUTOGEN=true` is also set. Auto-generation
is an opt-in escape hatch — generating a fresh keypair invalidates
every previously-issued JWT (the `kid` claim is content-addressed to
the file bytes), which silently unpairs every desktop client on the
next request. Set it true for local dev and first-boot bootstrap.
Leave it unset in any deployment where the key path could land on
ephemeral storage; mount a persistent volume instead.

### Sign-up / sign-in

```
POST /v1/auth/signup → email + password (argon2id) + RSI handle
POST /v1/auth/login  → email + password
                       → AuthResponse { token, user_id, claimed_handle }
POST /v1/auth/email/verify → { token } → 200 if valid+unexpired
```

Email verification is best-effort — signup still succeeds if SMTP is
unconfigured or send fails. The `mail.rs` module dispatches via
either `LettreMailer` (real SMTP) or `NoopMailer` (warn and continue).
Verification tokens are 32-byte hex with 24h expiry and a partial
unique index in Postgres.

### Device pairing

The tray client doesn't know your password. Pairing flow:

```
1. Web user → POST /v1/auth/devices/start    → 8-char alphanumeric code (TTL 10m)
2. Tray prompts → user types code
3. Tray → POST /v1/auth/devices/redeem       → device JWT (different sub, same iss/aud)
```

Device tokens carry a `device_id` claim. On every protected request,
the auth extractor consults a `dyn DeviceStore` to confirm the
device hasn't been revoked — revocation is immediate, no token-
lifetime wait.

## Authorization

SpiceDB hosts a Zanzibar-style ReBAC schema (see
`infra/spicedb/schema.zed`). Definitions: `user`, `organization`,
`stats_record`. The intended permissions:

- `view` on a `stats_record` — owner, share-with-user grant,
  share-with-org grant, public wildcard.
- `manage_members` / `manage_org` on an organization — admin / owner.

Current state: **advisory only**. `query::summary` calls
`SpicedbClient::check_permission(stats_record:<handle>, "view",
user:<handle>)` and logs `tracing::warn!` on denial without
short-circuiting. This populates traces and metrics with real
permission-check data so we can observe false-positive rates before
flipping to enforcement (a single edit at `query.rs:149`).

The SpiceDB client itself is a real gRPC connection (`spicedb-client`
crate, preshared-key auth). If it's unconfigured or unreachable, the
server boots in degraded mode and `/readyz` reports
`spicedb: "skipped"` — a non-configured dep does not block readiness;
a *configured* dep that's failing returns 503.

## Storage

| Tier | Engine | Holds | Migrations |
|---|---|---|---|
| Tray local | SQLite (rusqlite) | Local event buffer, sync cursors, noise list, pairing-code state | `crates/starstats-client/src/storage.rs` (inline DDL) |
| Server primary | Postgres pgvector/pg17 | `events`, `audit_log`, `users`, `devices`, sequences, indexes | `crates/starstats-server/migrations/0001-0006_*.sql` (sqlx-migrate, run on every boot) |
| Server audit mirror | MinIO (S3-compatible) | NDJSON copy of every audit row | applied via Object Lock on the `starstats-audit` bucket |
| Future analytics | DuckDB (planned) | Offline columnar queries over the events archive | not yet implemented |

The server runs `sqlx::migrate!("./migrations")` in `main.rs` before
opening the router, so a deploy that ships a new migration applies
it on the way up. Schema additions are append-only; we don't drop
or rename columns once they're in production.

## Observability

Four telemetry planes with different storage and retention. See
[`OBSERVABILITY.md`](OBSERVABILITY.md) for the full matrix.

| Plane | Server | Web |
|---|---|---|
| Logs | `tracing-subscriber` JSON → stdout | `pino` JSON → stdout |
| Metrics | `metrics-exporter-prometheus` → `/metrics` | `prom-client` → `/api/metrics` |
| Traces | `opentelemetry-otlp` gRPC → OTel Collector | `@opentelemetry/sdk-node` gRPC → OTel Collector |
| Audit | hash-chained `audit_log` table → MinIO mirror | (delegated to API) |

Every component speaks OpenTelemetry; the OTel Collector is the
single ingest point and routes to Loki / Tempo / Prometheus
downstream. Logs include `trace_id` so Grafana joins logs↔traces by
field, not regex.

The cardinality rule: never label metrics by `user_id` / `org_id` /
session — those are unbounded. Use them only in logs and traces.

## Why this stack

- **Rust for the tray and API.** The tray runs on user machines with
  a strict "no UI jank, no GC pauses" requirement; the API ingests
  bursts of events and we want predictable latency. Both benefit
  from no-GC plus the same `starstats-core` types crossing the wire.

- **Tauri over Electron** for the tray. ~10 MB binary instead of ~150 MB,
  uses the OS webview instead of bundling Chromium. Runs on a Steam
  Deck without melting it.

- **Next.js 15 App Router + RSC** for the web. Server components let
  us keep the JWT off the client and render the dashboard with one
  API round-trip — no client-side data-fetching boilerplate, no
  exposed bearer tokens.

- **First-party JWT instead of OAuth provider.** Avoiding a hard
  dependency on Auth0 / Authentik / Keycloak keeps the runtime simpler.
  RS256 + JWKS gives the same trust model with one fewer container.

- **SpiceDB for authz.** ReBAC scales to "share with org / share with
  friends / public" without growing the schema. The advisory-mode
  rollout pattern means we can ship enforcement after we've seen
  enough data to set thresholds honestly.

- **MinIO for audit.** Object Lock compliance retention is a
  regulatory-grade primitive that ships in MinIO Community. Postgres
  stays the source of truth; the mirror is a write-once tamper-
  evident archive.

## Repo layout reference

```
StarStats/
├── Cargo.toml                workspace
├── pnpm-workspace.yaml       JS/TS workspace
├── crates/
│   ├── starstats-core/       shared types, parser
│   ├── starstats-server/     API server (Axum + sqlx)
│   │   └── migrations/       0001..0006_*.sql
│   └── starstats-client/     Tauri tray (Rust backend)
├── apps/
│   ├── tray-ui/              Vite + React inside Tauri webview
│   └── web/                  Next.js dashboard
├── packages/
│   └── api-client-ts/        generated TS client (from server's OpenAPI)
├── infra/                    config images (init, loki, tempo, prom, otel-collector, spicedb schema)
├── docs/                     ARCHITECTURE | AUDIT | OBSERVABILITY
```
