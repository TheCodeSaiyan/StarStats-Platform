# api-client-ts

Auto-generated TypeScript client for the [StarStats](../../README.md)
API. The schema in
[`src/generated/schema.ts`](src/generated/schema.ts) is produced from
the Rust server's OpenAPI 3.1 spec by
[`openapi-typescript`](https://www.npmjs.com/package/openapi-typescript)
v7.

**Do not hand-edit `src/generated/schema.ts`.** Regenerate it from
the server instead.

## How regeneration works

The generator script (`scripts/generate.ts`):

1. Builds and runs the `starstats-server-openapi` bin (from
   [`crates/starstats-server`](../../crates/starstats-server/README.md)),
   which dumps the OpenAPI spec to stdout. **No database needed** —
   that's the whole reason the spec-only bin exists.
2. Pipes the spec into `openapi-typescript`.
3. Writes the result to `src/generated/schema.ts`.

Run it manually with:

```bash
pnpm --filter api-client-ts run generate
```

CI re-runs the generator on every PR and fails the build if the
generated file drifts from what's committed. **This is the contract
enforcement between the Rust server and every TS consumer
(`apps/web`, `apps/tray-ui`).**

## When to regenerate

Any time you:

- Add, remove, or rename an axum handler.
- Change a `#[utoipa::path]` attribute.
- Add, remove, or rename a request/response type that the spec
  references.
- Touch `openapi.rs` (path/schema registration).

If you add a new module on the server side, also add a `mod` stub
in `crates/starstats-server/src/bin/openapi.rs` so the spec-only
binary can see it.

A `regen-openapi` skill is published with this repo that automates
the full regeneration loop and reminds about the `openapi.rs`
registration step so the spec doesn't drift.

## Consuming the client

`apps/web` and `apps/tray-ui` both import this package via the pnpm
workspace alias (`"api-client-ts": "workspace:*"`):

```ts
import type { paths, components } from "api-client-ts";

type SummaryResponse =
  components["schemas"]["SummaryResponse"];

type IngestPath =
  paths["/v1/ingest"]["post"];
```

The tray webview uses the types **for narrowing only** — the tray
itself doesn't issue HTTP from TS; that lives in
[`crates/starstats-client`](../../crates/starstats-client/README.md)'s
Rust side.

## Files in this package

| Path | What it is |
|---|---|
| `src/index.ts` | Re-exports from `generated/schema.ts` |
| `src/generated/schema.ts` | The generated client. **Do not hand-edit.** |
| `scripts/generate.ts` | The generator pipeline (Rust spec dump → `openapi-typescript`) |
| `tsconfig.json` | TS config (typecheck only — no emit) |

## Related

- [`../../crates/starstats-server/README.md`](../../crates/starstats-server/README.md)
  §OpenAPI workflow — the upstream of this generation
- [`../../apps/web/README.md`](../../apps/web/README.md) — primary
  consumer
- [`../../apps/tray-ui/README.md`](../../apps/tray-ui/README.md) —
  consumes for type narrowing
