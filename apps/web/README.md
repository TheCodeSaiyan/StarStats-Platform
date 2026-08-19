# web

Next.js 15 (App Router, React Server Components) dashboard for
[StarStats](../../README.md). The user-facing read view of the event
timeline, type breakdowns, device management, and the sharing
surfaces (per-event scopes, org workspace, share reports).

## The auth invariant

The browser **never holds the JWT.** Auth flow:

1. Sign-in / sign-up POSTs through Next.js server actions.
2. The user JWT is stored in an HttpOnly + SameSite=Lax cookie
   (`starstats_session`) — JS can't read it.
3. All API fetches happen inside Next.js's Node runtime via
   `getSession()` returning `{ token, claimedHandle }`. The browser
   makes RSC requests; the server makes API requests.

This is what lets us treat the API server's JWT as a server-to-server
secret on the read path even though it's a "user" token. Don't
introduce a route that hands the token to the client.

## Stack

| Dep | Version | Notes |
|---|---|---|
| Next.js | 15.5.18 | App Router, RSC, server actions, `useFormStatus` |
| React | 19 | |
| TypeScript | ^5.7 | `tsc --noEmit` typecheck gate |
| `api-client-ts` | workspace | Generated TS client from the server's OpenAPI spec |
| Recharts | ^3.8 | Dashboard charts |
| Pino + `pino-pretty` | ^9 / ^13 | Structured logs → stdout |
| `prom-client` | ^15 | `/api/metrics` endpoint |
| OpenTelemetry SDK + auto-instrumentations | ^1.9 / ^0.75 | OTLP gRPC export |
| `@sentry/node` | ^10 | Error reporting |
| Playwright | ^1.55 | E2E tests against a dev server |

## Dev loop

```bash
pnpm install                            # one-off, from repo root
pnpm --filter web dev                   # http://localhost:3000
```

Requires a reachable API server. Either run one locally
(`cargo run -p starstats-server`) or point at a deployed instance via
env (see `apps/web/.env.local` — gitignored).

### Other scripts

```bash
pnpm --filter web build                 # next build
pnpm --filter web start                 # next start (after build)
pnpm --filter web lint                  # next lint
pnpm --filter web typecheck             # tsc --noEmit
pnpm --filter web test:e2e              # playwright test
pnpm --filter web test:e2e:install      # one-off: install chromium
pnpm --filter web test:e2e:headed       # debug failing flows
```

## Conventions (selected)

These live more comprehensively in [`../../docs/ENGINEERING.md`](../../docs/ENGINEERING.md);
the ones most likely to bite contributors here:

- **Multi-endpoint dashboards use `Promise.allSettled`,** not
  `Promise.all`. A single endpoint hiccup must not wipe the whole
  render. Log each rejection with `call=<label> status=<code>` so
  the failing endpoint is named in server logs.
- **SpiceDB 503** from an authz-dependent endpoint surfaces in the
  UI as a `spicedb_unavailable` banner — don't promote it to a hard
  failure.
- **Server-side handle truth.** For sensitive form fields, the auth'd
  user's handle comes from `session.claimedHandle` (server-side),
  never the form body. The form can supply the *other* party's
  handle.
- **Event headlines route through `humanTitleForEntry`** (mirrored
  from the tray's `format.ts`). Never surface raw snake_case
  `event_type` as a user-facing headline.
- **In-Transit movement noise is hidden at render-layer** in
  `lib/event-filter.ts` — keep this set in sync with the tray's
  `timeline/filter.ts`.
- **Don't fabricate server-side RSI fetch endpoints.** Only the tray
  scrapes RSI. Any "Refresh" affordance for hangar-style data MUST
  point at the tray (`/devices`); reframe the UX instead.

## Observability

Logs → Pino (JSON, stdout). Metrics → Prometheus scraping
`/api/metrics`. Traces → OpenTelemetry SDK → OTel Collector via OTLP
gRPC. Errors → Sentry. Logs include `trace_id` so Grafana can join
logs ↔ traces by field rather than regex. See
[`../../docs/OBSERVABILITY.md`](../../docs/OBSERVABILITY.md) for the
full matrix.

The cardinality rule: **never label metrics by `user_id` /
`org_id` / session** — those are unbounded. Use them only in logs
and traces.

## Related

- [`../../crates/starstats-server/README.md`](../../crates/starstats-server/README.md)
  — the API this app reads from
- [`../../packages/api-client-ts/README.md`](../../packages/api-client-ts/README.md)
  — the generated client this app imports
- [`../tray-ui/README.md`](../tray-ui/README.md) — the Tauri webview
  that shares filter/formatting logic with this app
- [`../../docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md) §Web
  read path
