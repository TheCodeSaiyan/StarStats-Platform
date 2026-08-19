# starstats-server

The [StarStats](../../README.md) API server — Rust + Axum + sqlx.
First-party JWT auth, device pairing, event ingest, query endpoints,
hash-chained audit log, OpenAPI 3.1 spec, Prometheus metrics, OTLP
traces.

The server is its own identity provider (RS256 + JWKS at
`/.well-known/jwks.json`) — there is no Auth0/Keycloak/Authentik
dependency. End-to-end architecture lives in
[`../../docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md).

## Binaries

This crate ships **two** binaries:

| Binary | Purpose |
|---|---|
| `starstats-server` | The live API server (`src/main.rs`) |
| `starstats-server-openapi` | Prints the OpenAPI spec to stdout (`src/bin/openapi.rs`). Used by the TS codegen pipeline so spec dumping doesn't need a running database. |

> The OpenAPI bin's name is **`starstats-server-openapi`** — not
> `openapi`. Using the short name errors out. Mirror any new module
> into `bin/openapi.rs` when adding endpoints; see the OpenAPI
> workflow below.

## Module map (key files)

| Module | Role |
|---|---|
| `auth.rs`, `auth_routes.rs` | Sign-up, sign-in, email verification, JWT issuance, JWKS publication |
| `ingest.rs` | `POST /v1/ingest` — batch ingest from the tray, idempotency by `(claimed_handle, idempotency_key)` |
| `query.rs` | `GET /v1/me/events`, `/v1/me/summary` — read-path with advisory SpiceDB check |
| `audit.rs`, `audit_mirror.rs` | Hash-chained append-only audit log + optional MinIO NDJSON mirror |
| `spicedb.rs` | SpiceDB (Zanzibar/ReBAC) client wrapper — advisory mode today |
| `mail.rs` | SMTP via Lettre (`LettreMailer`) or a `NoopMailer` if SMTP isn't configured — email verification is best-effort |
| `telemetry.rs` | `tracing-subscriber` JSON + OpenTelemetry OTLP gRPC export |
| `health.rs` | `/healthz` (liveness) and `/readyz` (deep readiness — Postgres + SpiceDB) |
| `well_known.rs` | `/.well-known/jwks.json` (RS256 public key) |
| `openapi.rs` | `utoipa` `ApiDoc` registration — paths AND schemas |

For the audit log specifically, see
[`../../docs/AUDIT.md`](../../docs/AUDIT.md).

## Running locally

```bash
# 1. Bring up Postgres (pg17 + pgvector). The repo's infra/ directory
#    has a docker-compose for the full stack.
docker compose -f ../../infra/docker-compose.yml up -d postgres

# 2. Run the server. It applies sqlx migrations on the way up.
cargo run -p starstats-server
```

Migrations live in `migrations/0001..NNNN_*.sql` and are applied via
`sqlx::migrate!("./migrations")` in `main.rs` before the router opens.

### Required env vars

| Var | Purpose |
|---|---|
| `DATABASE_URL` | Postgres connection string |
| `STARSTATS_JWT_KEY_FILE` | Path to the RSA private key (default `/var/lib/starstats/jwt-key.pem`, mode 0600). Loaded at boot; generation requires `STARSTATS_JWT_KEY_AUTOGEN=true`. |
| `STARSTATS_JWT_KEY_AUTOGEN` | Opt-in flag to allow keypair generation when `STARSTATS_JWT_KEY_FILE` is missing. Required for first-boot bootstrap and local dev. **Leave unset in deployments where the key path could be on ephemeral storage** — silent regeneration invalidates every device + user JWT (paired tray clients see "no longer paired"). |
| `STARSTATS_JWT_ISSUER` | `iss` claim |
| `STARSTATS_PARSER_HEALTH_RECENT_DAYS` | Parser-health recent window, days (default `7`). |
| `STARSTATS_PARSER_HEALTH_BASELINE_DAYS` | Parser-health baseline window immediately preceding the recent one, days (default `28`). |
| `STARSTATS_PARSER_HEALTH_MIN_BASELINE_EVENTS` | Event types with fewer baseline events are too rare to judge and are never flagged (default `200`). |
| `STARSTATS_PARSER_HEALTH_COLLAPSE_FRACTION` | Recent share must fall to at most this multiple of baseline share to flag (default `0.2`). |
| `STARSTATS_PARSER_HEALTH_MIN_AFFECTED_FRACTION` | Fraction of still-active users that must have lost the event type (default `0.75`). Lowering this trades false positives for sensitivity. |
| `STARSTATS_JWT_AUDIENCE` | `aud` claim |
| `STARSTATS_KEK_FILE` | Path to the AES-256 TOTP key-encryption key (default `/var/lib/starstats/totp-kek.bin`, mode 0600). Loaded at boot; generation requires `STARSTATS_KEK_AUTOGEN=true`. |
| `STARSTATS_KEK_AUTOGEN` | Opt-in flag to allow KEK generation when `STARSTATS_KEK_FILE` is missing. Required for first-boot bootstrap and local dev. **Leave unset in deployments where the key path could be on ephemeral storage** — a fresh KEK cannot decrypt existing TOTP secrets, locking out every 2FA-enrolled user. |

Optional (degraded-mode if absent):

| Var | Effect when absent |
|---|---|
| SpiceDB connection vars | `/readyz` reports `spicedb: "skipped"`; advisory checks no-op |
| MinIO connection vars | Audit mirror disabled; Postgres remains source of truth |
| SMTP vars | `NoopMailer` warns and continues; signup still succeeds |
| OTLP endpoint | Traces stay in-process; logs and metrics still emit |

## Migrations posture (**read this before touching SQL**)

Migrations are **additive only AND byte-immutable post-deploy.**
`sqlx`'s hash verification covers the entire file — don't even
reformat comments on a shipped migration. This caused a production
crash-loop in PR #37 on 2026-05-18; the rule was burned in afterward.
See [`../../docs/ENGINEERING.md`](../../docs/ENGINEERING.md) §Architecture Invariants.

- Use `IF NOT EXISTS`.
- Use NULL-able columns with no default for additive changes.
- No `DROP COLUMN`, no `ALTER ... SET NOT NULL` on populated columns
  without a default, no renames.
- Backward-compat parsing is handled by `#[serde(default)]` in
  [`starstats-core`](../starstats-core/README.md)'s wire format.

## OpenAPI workflow

When you add or change an endpoint:

1. Edit the axum handler.
2. Add or update its `#[utoipa::path]` attribute.
3. Register both the path **and** the schemas in `openapi.rs`.
4. If you added a new module, add a `mod` stub in `src/bin/openapi.rs`
   so the spec-only bin can see it without DB access.
5. Regenerate the TS client:
   ```bash
   pnpm --filter api-client-ts run generate
   ```
6. CI runs the generator and fails if the committed
   `packages/api-client-ts/src/generated/schema.ts` drifts from
   what the spec produces.

A `regen-openapi` skill is published with this repo that automates
steps 5–6.

## Linting

```bash
cargo fmt -p starstats-server
cargo clippy -p starstats-server -- -D warnings
```

The workspace clippy posture is `-D warnings` — every warning is a
CI failure. Workspace-level allows live in
[`../../Cargo.toml`](../../Cargo.toml) `[workspace.lints.clippy]`
with rationale.

## New `Arc<dyn StoreTrait>` extensions

When adding a new store trait + Postgres impl + Memory impl, mirror
the `share_metadata_dyn` pattern in `main.rs`: `Arc` → dyn-cast →
`Extension` layer added at the bottom of the `app` builder. Write
6–8 store tests against the Memory impl before wiring routes;
route-layer tests come next using `tower::ServiceExt::oneshot`.

## Related

- [`../starstats-core/README.md`](../starstats-core/README.md) —
  the wire format and event types this server ingests
- [`../starstats-client/README.md`](../starstats-client/README.md) —
  the tray that posts to `/v1/ingest`
- [`../../packages/api-client-ts/README.md`](../../packages/api-client-ts/README.md)
  — the generated TS client this server's OpenAPI feeds
- [`../../docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md),
  [`../../docs/AUDIT.md`](../../docs/AUDIT.md),
  [`../../docs/OBSERVABILITY.md`](../../docs/OBSERVABILITY.md)
