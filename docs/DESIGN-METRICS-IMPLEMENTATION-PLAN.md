# StarStats — Metrics Display: Implementation Plan

Companion to `docs/DESIGN-METRICS-PLAN.md`. This is the **execution
plan** that supersedes the strategic plan's Phase 0/1 definitions
after three independent review passes surfaced material contradictions
between the strategic plan and the actual codebase.

> **Read this first if you're starting work.** The strategic plan is
> for context and direction. Phase ordering, scope, and per-file
> deliverables live here.

---

## 0. Corrections from review (load-bearing)

These are facts about the codebase, not opinions. The strategic plan
got them wrong; this plan inherits the corrected versions.

| Strategic plan said | Reality |
|---|---|
| `globals.css` is 672 lines | `apps/web/src/app/globals.css` is **201 lines**. The 999-line file is `apps/web/src/styles/starstats-tokens.css` — also `@import`-ed by `apps/tray-ui/src/styles.css:1`. Token edits ripple to both apps. |
| Auth extractor is `RequireUser` | `crates/starstats-server/src/auth.rs:113,414` — extractor is **`AuthenticatedUser`**. |
| `GameEvent` has 26 variants | 27 variants in `crates/starstats-core/src/events.rs:16-44`. |
| `Heatmap` / `TypeBars` / `TimelineList` already exist | Only `DayHeatmap.tsx` exists. The other primitives are new work. The `apps/web/src/components/metrics/` directory itself doesn't exist yet — created in Phase 0. |
| Phase 1 introduces `/me/metrics/sessions` and `/me/metrics/event-types` | `crates/starstats-server/src/query.rs:431` (`metrics_event_types`) and `:564` (`metrics_sessions`) already serve `/v1/me/metrics/event-types` and `/v1/me/metrics/sessions`. Phase 1 **extends** these, not greenfield. Existing FE typed callers at `apps/web/src/lib/api.ts:625,637` consume `range`, not `period` — see §4.1 for the breaking-change handling. |
| Materialized view per-handle | Postgres `MATERIALIZED VIEW` cannot be parameterised by handle — this is a per-handle aggregate table with nightly upsert. |

## 0.1 Data-model gaps that gate features

The strategic plan proposed features that cannot be built from
current event payloads. **Until the parser/data pipeline lands the
missing fields, these features are out of scope.**

| Feature in strategic plan | Missing field | Source `events.rs` |
|---|---|---|
| Nemesis tracker | `PlayerDeath.killer_handle` | lines 130-142 — only `body_class`, `body_id`, `zone` exist. Legacy `ActorDeath` had `killer` but is "no longer written in modern builds" (line 167 comment). |
| Mission economy (avg payout, biggest single payout) | `MissionEnd.payout` | lines 411-415 — only `mission_id`, `outcome` exist. |
| Mission duration | `MissionEnd.duration_secs` or `MissionStart.timestamp` pairing | Pairing requires backend join; raw payload has none. |
| Spending vs earning chart | aUEC on `CommodityBuyRequest`, `CommoditySellRequest`, `ShopBuyRequest` | lines 432-465 — only `quantity`/`commodity`/`shop_id`/`item_class` captured. No price. |
| Recap `aUEC_net` | Same as above | Cascades — unbuildable. |
| Server-hop "named server" view | Human-readable server name | `JoinPu` carries shard UUID + IP; `ChangeServer` carries booleans + phase. No name field exists in-game. |
| "Where you die" with hotspots | `PlayerDeath.zone` reliable | Field exists but is `None` until a post-classify enrichment pass runs — best-effort only. |

### Disposition of blocked features

- **Out of v1 entirely:** Nemesis tracker, Mission economy panel
  (payout/duration), Spending vs earning chart, Recap `aUEC_net`.
- **Demoted to best-effort with explicit hidden-on-low-confidence
  rules:** "Where you die" cluster (only renders if ≥70% of period's
  deaths have non-null `zone`).
- **Reshaped:** Server-hop view becomes a **shard-stability view**
  — counts shard changes per session and total dwell-time across
  shards, surfaced as "you changed shards 4 times this session"
  rather than "you spent 2h in shard XYZ-123."
- **Parser work as a separate, pre-Phase-1 effort:** A new task
  thread (not in this plan) to enrich the parser for kill
  attribution, mission payout, and commodity/shop price extraction
  from the existing `raw` strings. When that lands, the dropped
  features can re-enter via a Phase 6.

## 0.2 Stack decision: defer to Phase 0 spike

The strategic plan committed to Tailwind + Tremor. Reviewer 3 flagged
this as the highest-risk decision. **Phase 0 begins with a half-day
spike** to validate the alternative before the Tailwind migration
locks in:

- **Spike A — Recharts against `ss-*` tokens.** Build the year
  heatmap + a sparkline + a bar list directly in Recharts, styling
  via inline `fill={var(--accent)}` and the existing token system.
- **Spike B — Tremor + Tailwind.** Build the same three primitives
  via Tremor, with Tailwind preflight disabled and `ss-*` CSS vars
  wired through `tailwind.config.ts theme.extend`.

Exit criterion: a single subjective decision recorded in this doc —
which path do we commit to. Spike total budget: **2 engineer-days**
(Spike A ½d, Spike B 1d — Spike B needs the full day because the
preflight collision against `starstats-tokens.css` plus the dual-app
token regression check in the tray are the exact failure modes
Spike B must surface; ½d would be too shallow to catch them).

### Tremor → Recharts primitive mapping

The rest of this plan names Tremor primitives by default. If Spike A
wins, use this mapping in §5–§7:

| Tremor primitive | Recharts equivalent |
|---|---|
| `<AreaChart>` | `<AreaChart>` (same name) |
| `<BarChart>` | `<BarChart>` (same name) |
| `<DonutChart>` | `<PieChart>` + `<Pie innerRadius>` |
| `<SparkAreaChart>` | `<ResponsiveContainer><AreaChart>` minified |
| `<BarList>` | Plain `<ul>` + width-% bar divs (no Recharts equivalent) |
| `<Tracker>` | Custom CSS grid (no Recharts equivalent) |

If `<BarList>` and `<Tracker>` are heavily used downstream, that's
evidence to favour Tremor; if not, Recharts wins on weight.

---

## 1. Calendar estimates (corrected)

Reviewer 3's correction stands: the strategic plan's "5–7 weeks" was
engineer-day math. With reviews, design polish, real bug fixing, and
the data-pipeline blockers in §0.1:

| Phase | Engineer-days | Calendar with reviews |
|---|---|---|
| Phase 0 | 2.5 (incl. spike + setup) | 1 week |
| Phase 1 | 6–8 (one endpoint/day is optimistic — Reviewer 3 flagged) | 2 weeks |
| Phase 2 | 6–8 | 2 weeks |
| Phase 3 (reshaped) | 5–7 | 2 weeks |
| Phase 4 (reshaped, achievements deferred) | 4–6 | 1.5 weeks |
| Phase 5 (cut by default) | — | — |

**Realistic calendar: 8.5–9.5 weeks for Phases 0–4 with the cut list
honoured. 12+ if Phase 5 lands. Without honest cut points, 14+.**

## 1.1 Cut list (declared up front)

Phases 0–2 are **ship-or-die**. Phases 3 onwards are pause points
where the entire feature can be cut without rework:

- **Cuttable without rework:** Phase 4 share image, Phase 4 signature-
  move card, Phase 5 entire phase (Now strip, animated insertion,
  pulse dots, org leaderboard, percentile ribbons, friend overlay).
- **Achievements engine:** *demoted to deferred experiment* (was a
  full sub-phase). Lands behind a feature flag only if Phase 4 recap
  share rate > 5%. Out of v1.
- **Friend overlay & percentile ribbons:** depend on opt-in data that
  doesn't exist yet — explicitly post-v1.

---

## 2. Cross-cutting concerns the strategic plan missed

These ship across every phase from Phase 2 onward; the strategic plan
didn't name them. They become deliverables on every chart we build:

### 2.1 Accessibility
- Chart contrast: every series colour from `--accent`, `--fg`,
  `--fg-dim` must hit WCAG AA against `--bg-0` and `--bg-1`. Add a
  contrast-check unit test that introspects token values.
- Chart keyboard nav: every interactive Tremor/Recharts component
  gets `<table>` semantic fallback rendered visually-hidden for
  screen readers and Tab-reachable; tooltip data also lands in a
  `<details>` block below the chart so non-mouse users get the same
  drill-down.
- `prefers-reduced-motion`: respected by the existing token system;
  any animated insertion (Phase 5) gates on it.

### 2.2 Mobile viewport
- All Phase 2/3 surfaces must render at 375px without horizontal
  scroll. Charts collapse to single-column stacked layout below
  `--bp-md` (768px).
- Session ribbon: ribbon scrolls horizontally on mobile with snap
  points; year heatmap renders only the last 13 weeks on mobile.

### 2.3 Empty-state & error-state UX
- **Empty state per surface:** every chart card has an explicit
  "no data yet" rendering (not a blank box, not a zero-axis chart).
  Copy is voiced consistently with the design handoff's `voice.md`.
- **Error state per surface:** when an endpoint returns 5xx or
  the share-scope check denies, the card renders a small inline
  message + a single retry button. Cards don't disappear; they
  degrade in place so layout doesn't jump.
- **Best-effort badges:** when a chart relies on a field that is
  best-effort (e.g. `PlayerDeath.zone`), show a small `?` badge
  with a tooltip explaining the data quality. No silent degradation.

### 2.4 Feature flags & telemetry
- Each new surface lands behind a feature flag in
  `apps/web/src/lib/feature-flags.ts`. Flag scheme: `metrics.<surface>`
  (e.g. `metrics.year_heatmap`, `metrics.death_recap`).
- **Rollout cadence per flag:** 10% → 50% → 100% over a week (cohort
  by `hash(handle) % 100`), so signal isn't all-or-nothing.
- Frontend telemetry: a `recordMetricView({ surface, mode })` call on
  every chart card mount, gated by user consent. Stores in the
  existing audit log via `/v1/me/telemetry` (new tiny endpoint).
- **Enforcement:** the `<MetricCard>` shell (§3.4) makes `flagKey` and
  `telemetryKey` **required props with no defaults**, so TypeScript
  rejects any card that skips the wiring. A Playwright sweep mounts
  every flagged card at 375px and asserts no horizontal scroll.

### Telemetry gate (between Phase 2 and Phase 3)

**Owner:** the engineer who shipped Phase 2 is responsible for
running the gate query and posting results before opening Phase 3
work. No ambiguity.

**Decision rule** (single source of truth — copy this query verbatim
at gate time):

```sql
-- Count distinct users who viewed each Phase 2 surface in the past 7
-- calendar days, expressed as % of distinct dashboard viewers in the
-- same window. Run 7 days after Phase 2 ships at 100% rollout.
SELECT surface, COUNT(DISTINCT user_id) AS viewers,
       (COUNT(DISTINCT user_id)::float
        / (SELECT COUNT(DISTINCT user_id) FROM audit_log
           WHERE event = 'page_view' AND path = '/dashboard'
             AND occurred_at > now() - INTERVAL '7 days')
       ) AS share_of_dashboard
FROM audit_log
WHERE event = 'metric_view'
  AND occurred_at > now() - INTERVAL '7 days'
GROUP BY surface;
```

**Cut threshold:** any surface where `share_of_dashboard < 0.05` is
cut from the design (not Phase 3) — we don't add similar surfaces
in Phase 3 if the cheap Phase 2 version already failed engagement.

### 2.5 Privacy / share-scope filtering
- A single helper, `apply_share_scope(query, viewer, target)`, lives
  in `crates/starstats-server/src/share_scope.rs` (new file) and
  every aggregate endpoint passes through it. Three exits: `me`,
  `friend`, `public`. Each new endpoint declares its minimum scope.
- The strategic plan's Phase 5 org leaderboard is gated on members
  having opt-ed-in per metric. Plan calls it `share_scopes.leaderboards`
  on the member record — defined in Phase 1 even though leaderboards
  ship in Phase 5, because changing the share-scope shape mid-stream
  forces a Phase 1 rewrite.

---

## 3. Phase 0 — Foundations + stack spike

**Duration:** 2 engineer-days. **Blocks:** Phases 1–5.

### 3.1 Stack spike (½ day)

Files created (throwaway, deleted at end of spike):
- `apps/web/src/app/_spike/recharts/page.tsx`
- `apps/web/src/app/_spike/tremor/page.tsx`

Each renders the same three primitives over hand-shaped JSON
fixtures: year heatmap (53 weeks × 7 days), sparkline (30 points),
horizontal bar list (top 6).

Decision recorded inline at the top of this doc as:
```
## Stack decision (resolved YYYY-MM-DD)
Path chosen: [Recharts | Tremor+Tailwind]
Reason: ...
```

### 3.2 If Path = Tremor + Tailwind

Files created:
- `apps/web/tailwind.config.ts` — `content: ['./src/**/*.{ts,tsx}']`,
  `corePlugins.preflight: false` (CRITICAL — colliding with
  `starstats-tokens.css`'s 999 lines is otherwise inevitable),
  `theme.extend.colors` wired to read CSS vars (e.g.
  `accent: 'var(--accent)'`).
- `apps/web/postcss.config.mjs`
- `apps/web/src/lib/tremor-colors.ts` — Tremor colour-name mapping
  to `ss-*` CSS vars.

Files modified:
- `apps/web/src/app/globals.css` — add `@tailwind base; @tailwind
  components; @tailwind utilities;` at the top (base is empty because
  preflight is disabled).
- `apps/web/package.json` — add `tailwindcss`, `postcss`,
  `autoprefixer`, `@tremor/react`.

### 3.3 If Path = Recharts only

Files created:
- `apps/web/src/lib/recharts-theme.ts` — central place that exports
  hex values resolved from `getComputedStyle(document.documentElement)`
  reading `--accent`, `--accent-warm`, `--ok`, `--warn`, `--danger`.
  Theme-reactive via `useEffect` on `data-theme` attribute changes.

Files modified:
- `apps/web/package.json` — add `recharts` only.

### 3.4 Always-needed Phase 0 deliverables

Directory created: `apps/web/src/components/metrics/` (does not exist
yet — sibling to existing `components/shell/`).

Files created (regardless of stack path):
- `apps/web/src/components/metrics/MetricCard.tsx` — shell card
  consuming `ss-card-*` classes from the token system. Props (all
  REQUIRED — no defaults — so TypeScript enforces the cross-cutting
  checklist): `title`, `caption`, `flagKey`, `telemetryKey`,
  `empty`, `error`, `srTable`, `bestEffort` (boolean + reason if
  true).
- `apps/web/src/components/metrics/ChartCard.tsx` — wraps
  `MetricCard` with chart-specific paddings; the `srTable` slot is
  the screen-reader fallback that mirrors the chart data as
  `<table>`.
- `apps/web/src/lib/feature-flags.ts` — typed flag registry.
- `apps/web/src/lib/metrics-telemetry.ts` — `recordMetricView`
  helper; opt-in gate read from existing user prefs.

**Exit criterion for Phase 0:** one Tremor or Recharts chart renders
on `/metrics` page using accent colours from the active theme, with
empty state, error state, and a `metrics.spike_test` feature flag
gating its visibility.

---

## 4. Phase 1 — Backend aggregates

**Duration:** 5–7 engineer-days. **Blocks:** Phases 2–5.

### 4.1 Endpoints to extend (already exist)

| Endpoint | Current | Phase 1 change |
|---|---|---|
| `GET /v1/me/metrics/event-types` (`query.rs:431`) | Counts by type, takes `?range=…` | Accept `?period=24h\|7d\|30d\|90d` AND keep `?range` as an alias (deprecation warning header). Add `?bucketBy=session` query param. Add `share` (fraction of total) to each row. |
| `GET /v1/me/metrics/sessions` (`query.rs:564`) | List inferred sessions, takes `?range=…` | Same `?period` + `?range`-alias pattern. Add `?includeDerived=true` returning `kd_ratio`, `top_zone`, `event_count` per session. |

**Breaking-change handling (same PR, mandatory):** Both endpoints
have typed FE callers at `apps/web/src/lib/api.ts:625` and `:637`
consuming a `MetricsRange` enum, plus call-sites at
`apps/web/src/app/metrics/page.tsx:89,101,110`. The PR that extends
the BE endpoints **must also**:
- Regenerate the OpenAPI-derived types in `api.ts` (the
  `apiSchema['schemas']` block at line 617) so `period` is a known
  field.
- Update those 5 call-sites to pass `period` (keep `range` working
  via the alias for one release as a safety net).
- Add an integration test that calls both endpoints with `?range=`
  AND `?period=` and asserts identical responses.

Without this same-PR scope, the migration silently breaks the
metrics page.

### 4.2 Endpoints to add

| Endpoint | Returns | Source |
|---|---|---|
| `GET /v1/me/metrics/aggregates?period=30d` | Daily totals (kills, deaths, missions only — **no aUEC**), top event types, top zones (best-effort), current streak | `events` table + share-scope filter |
| `GET /v1/me/metrics/deaths?period=30d` | Death cause buckets by `(zone, body_class)` only — **no killer**. Marked best-effort when zone null-rate is high. | `PlayerDeath` rows |
| `GET /v1/me/metrics/shards?period=30d` | Shard change count, distinct shards per session, dwell-time across shards (NOT named-server view) | `ChangeServer` + `JoinPu` |
| `GET /v1/me/metrics/recap?period=week\|month` | Frozen snapshot from `metrics_recap` table (see §4.3) | New table |
| `GET /v1/me/telemetry` (POST) | Records a metric-view event | Audit log |

**Dropped from Phase 1 (vs strategic plan):** `/me/metrics/missions`
(no payout data), `/me/metrics/economy` (no price data). They
re-enter in a future Phase 6 once parser enrichment lands.

### 4.3 Schema additions

**Migrations land in this order — 0021 first because new handlers
read `share_scopes` from day one:**

Migration `0021_share_scopes.sql`:
```sql
ALTER TABLE users
  ADD COLUMN share_scopes JSONB NOT NULL DEFAULT '{
    "summary": "friend",
    "sessions": "me",
    "deaths": "me",
    "shards": "me",
    "leaderboards": "me"
  }'::jsonb;
```

Even though leaderboards don't ship until Phase 5, the field shape
lands in Phase 1 so Phase 5 doesn't need a Phase 1 retro-rewrite.

Migration `0022_metrics_recap.sql`:
```sql
CREATE TABLE metrics_recap (
  handle            TEXT NOT NULL,
  period_kind       TEXT NOT NULL,        -- 'week' | 'month'
  period_start      DATE NOT NULL,
  period_end        DATE NOT NULL,
  computed_at       TIMESTAMPTZ NOT NULL,
  payload           JSONB NOT NULL,       -- the recap body shipped to FE
  PRIMARY KEY (handle, period_kind, period_start)
);
```

The strategic plan claimed recap was cacheable in Phase 1 with no
schema — contradiction (Reviewer 1). This table lands in Phase 1.

**Recap-compute job gating** (mitigates dead-code risk if Phase 4
is cut): the nightly task spawned in `main.rs` checks
`feature_flags::is_enabled("metrics.recap_compute")` before each
run. Default value is `false`; it flips to `true` as part of the
Phase 4 ship. If Phase 4 is cut, the flag stays off, the table
stays empty, the cron does nothing — no Postgres write storm, no
dead code in production.

**Crash-safety on partial deploy:** the recap-compute task also
runs `SELECT 1 FROM information_schema.columns WHERE table_name =
'users' AND column_name = 'share_scopes'` on boot and refuses to
start if missing. Prevents a rolled-back 0021 from 500'ing every
nightly run.

### 4.4 Files to create / modify

Create:
- `crates/starstats-server/src/metrics_routes.rs` — new aggregate
  handlers
- `crates/starstats-server/src/share_scope.rs` — single filter
  helper
- `crates/starstats-server/src/recap.rs` — nightly recap compute
  job (read-only on `events`, upsert into `metrics_recap`),
  feature-flag-gated per §4.3
- `crates/starstats-server/migrations/0021_share_scopes.sql`
- `crates/starstats-server/migrations/0022_metrics_recap.sql`

Modify:
- `crates/starstats-server/src/query.rs` — extend the two existing
  endpoints with new `?period` param (`?range` retained as alias);
  see §4.1 for FE callsite changes that must land in the same PR
- `crates/starstats-server/src/main.rs` — wire the new router +
  spawn the nightly recap-compute task (gated on
  `metrics.recap_compute` flag)
- `crates/starstats-server/src/openapi.rs` — single registration
  site for all new routes (the `bin/openapi.rs` driver picks them
  up automatically)
- `apps/web/src/lib/api.ts` — regenerated OpenAPI types + 2 new
  caller functions per new endpoint
- `apps/web/src/app/metrics/page.tsx` — 5 call-sites migrating
  `range` → `period` (lines 89, 101, 110 + 2 new chart consumers)

Tests required (Rule 2 TDD — write first):
- Each new endpoint gets a "happy-path returns expected shape" test
  via the existing `WebApplicationFactory`-style harness
- `share_scope.rs` gets per-scope filter tests for `me`, `friend`,
  `public`
- Best-effort `deaths` endpoint test verifies the
  "low-confidence-suppress" rule when zone null-rate > 30%

**Exit criterion:** all 5 new endpoints + 2 extended endpoints pass
271+N regression on `cargo test -p starstats-server` and emit the
documented JSON shapes in OpenAPI.

---

## 5. Phase 2 — Density: charts replace tables

**Duration:** 6–8 engineer-days. **Blocked by:** Phase 1.

Same surfaces as the strategic plan, with corrections:

| Surface | Files |
|---|---|
| Year heatmap | `apps/web/src/components/metrics/YearHeatmap.tsx` (new — not pre-existing as strategic plan implied) |
| Sparkline pills | `apps/web/src/components/metrics/SparklinePill.tsx` (new) |
| Type breakdown bar+donut | `apps/web/src/components/metrics/TypeBreakdown.tsx` (new) |
| Session ribbon | `apps/web/src/components/metrics/SessionRibbon.tsx` (new) |
| Drill-down panel | `apps/web/src/components/metrics/SessionDetail.tsx` (new) |

Modify:
- `apps/web/src/app/metrics/page.tsx` — swap manual divs for new
  components; keep Raw Stream tab as-is
- `apps/web/src/app/dashboard/page.tsx` — swap pills for sparkline
  pills, replace existing `DayHeatmap` with `YearHeatmap`

Each component:
- ships with empty + error + best-effort states (per §2.3)
- ships behind a feature flag `metrics.<component>` (per §2.4)
- ships with a `<table>` screen-reader fallback (per §2.1)
- has a `recordMetricView` call on mount

**Exit criterion:** the four tabs of `/metrics` and the dashboard
summary land with the new components, all feature-flagged on, all
passing keyboard-nav + 375px-viewport manual smoke test.

**Telemetry gate:** Phase 3 doesn't start until at least 7 days of
view data is captured on Phase 2 surfaces.

---

## 6. Phase 3 — Stories: reshaped from strategic plan

**Duration:** 5–7 engineer-days. **Blocked by:** Phase 2 telemetry.

Reshaped feature list reflecting data-model gaps from §0.1:

| In v1 | Display |
|---|---|
| **Death recap card** (downgraded) | Top-N bar list bucketed by `(zone, body_class)` only — no killer, no weapon. "Best-effort" badge visible when zone null-rate > 0%. |
| **Shard-stability card** | Shard changes per session, distinct shards over period. "You changed shards 4× in your last session" headline. |
| **Activity rhythm** (new — fills the gap) | Time-of-day histogram of events; reveals when you actually play. Cheaply built from `received_at` alone. |
| **Death zone cluster** | Tremor / Recharts `<BarList>` of top `PlayerDeath.zone` values, only renders if confidence ≥ 70%. |

**Dropped vs strategic plan:** Mission economy, Spending vs earning,
Server-hop named view, Nemesis tracker (all blocked on data gaps —
see §0.1).

Files created (in `apps/web/src/components/metrics/`):
- `DeathRecap.tsx`, `ShardStability.tsx`, `ActivityRhythm.tsx`,
  `DeathZones.tsx`.

**Exit criterion:** Phase 3 surfaces render on `/metrics` with their
respective feature flags on, each passing the cross-cutting checklist
in §2.

---

## 7. Phase 4 — Personality: reshaped

**Duration:** 4–6 engineer-days. **Blocked by:** Phase 3.

Reshaped: **achievements engine dropped to deferred experiment.**

| In v1 | Out of v1 |
|---|---|
| Weekly recap card on `/metrics/recap` | Achievements engine (vanity per Reviewer 3) |
| Recap OG share image | Nemesis card (data-model gap) |
| Signature-move card — **shape-only**: "You played 60% of sessions in Crusader this month." No ships, no routes. | Ship + route mining (no data) |

The recap card pulls from the `metrics_recap` table already created
in Phase 1.

Files created:
- `apps/web/src/app/metrics/recap/page.tsx`
- `apps/web/src/app/metrics/recap/opengraph-image.tsx` (Next OG-image
  route — works on Node deploys, not Vercel-locked)
- `apps/web/src/components/metrics/RecapCard.tsx`
- `apps/web/src/components/metrics/SignatureMove.tsx`

**Pattern-mining safety rules** (per Reviewer 3):
- Every pattern-mined value renders only if confidence ≥ 70%
- Below threshold, card hides; never renders "Your nemesis: unknown"
- Parser test fixture per game patch checked into CI on the server
  side so payload-shape drift fails the build

### Confidence math (single source of truth)

Every pattern-mined card defines confidence the same way: `confidence
= matched_events / total_eligible_events_in_period`. Cards hide when
confidence < 0.70.

| Card | `matched_events` | `total_eligible_events_in_period` |
|---|---|---|
| Signature move (location) | Sessions with non-null `top_zone` | All sessions in period |
| Death zones | `PlayerDeath` rows with non-null `zone` | All `PlayerDeath` rows in period |
| Top type | Events whose `event_type` is non-null AND classified | All events in period |

These three buckets live in `crates/starstats-core/src/confidence.rs`
(new file, ~30 LOC + tests) so the calculation is reused across all
pattern-mining handlers. A parser-fixture owner (named in §11
decision #2) maintains golden test inputs per game patch in
`crates/starstats-core/tests/fixtures/patches/`.

### OG share-image deployment

The OG-image route at `apps/web/src/app/metrics/recap/opengraph-image.tsx`
**must declare `export const runtime = 'nodejs'`** — Next.js defaults
to Edge for OG routes, and the standalone Node container (`next.config.mjs:41`
sets `output: 'standalone'`) doesn't include the Edge runtime bundle.

Files modified:
- `apps/web/Dockerfile` — add `COPY apps/web/public/fonts` step
  (or similar) so `@vercel/og` font loading resolves at runtime.
  The current Dockerfile copies `.next/static` and `public` but
  has no explicit font assets.
- `apps/web/src/app/metrics/recap/opengraph-image.tsx` — declares
  Node runtime + loads fonts via filesystem read.

**Smoke test:** Komodo healthcheck includes a `GET
/metrics/recap/opengraph-image` 200 check before marking the
container ready.

**Exit criterion:** `/metrics/recap` ships with a sharable image
that renders correctly in the production container (not just local
dev), recap card lands on dashboard headline slot with feature flag.

---

## 8. Phase 5 — Live & comparative (cut by default)

**Duration:** ~1 week if delivered. **Default:** not delivered in v1.

If delivered, the strategic plan's Phase 5 stands with these
additions (per Reviewer 3):

- "Now" strip 30s polling **requires** `document.visibilityState`
  gate, exponential backoff when idle, ETag/304 mandatory on
  `/me/metrics/recent`, 15s server-side response cache. No
  exceptions.
- Or — preferred — punt to SSE follow-up (per strategic plan's own
  non-goals).
- Org leaderboard depends on `share_scopes.leaderboards` opt-in
  (already in Phase 1's `share_scopes` shape, so no Phase 1
  rewrite required).

---

## 9. Tray-UI scope

**Duration:** 2–3 engineer-days. **Lands alongside Phase 2.**

Same as strategic plan §4, with the §0.1 data-model corrections:
- 48-hour sparkline of events / hour — implementable, no missing data
- Session ribbon at small scale — implementable from local SQLite
  (table at `crates/starstats-client/sql/schema.sql:4-14`; the
  `idx_events_type_ts` index at line 15 confirms the cheap query
  pattern)
- "Recap available" indicator — reads new `metrics_recap` table via
  the API, opens default browser to `/metrics/recap`

Files created:
- New Tauri IPC command `get_event_histogram_48h` in
  `crates/starstats-client/src/commands.rs` returning per-hour bucket
- `apps/tray-ui/src/components/EventSparkline.tsx` (inline SVG, no
  chart library — ~50 LOC against `var(--accent)` token)
- `apps/tray-ui/src/components/SessionMiniRibbon.tsx`

Modify:
- `apps/tray-ui/src/components/StatusPane.tsx` — replace event-count
  card with sparkline; insert session mini-ribbon

### Tray release cadence

The tray ships via its own updater channel (`beta.json` post-v0.0.1-beta
cut, see `release-manifests/`). A new IPC command in
`commands.rs` is a binary surface change — tray-ui calling
`get_event_histogram_48h` on an older binary returns an IPC error.

**Required:**
- Bump `crates/starstats-client/tauri.conf.json` `version` (e.g.
  `0.0.1` → `0.0.2`) and Cargo workspace version
  (`0.0.1-beta` → `0.0.2-beta`).
- Regenerate `release-manifests/tray-beta.json` via the existing Release
  workflow.
- Tray-ui side: feature-detect via a try/catch around the first
  call; render the legacy event-count card as fallback when the IPC
  isn't available. Don't assume the user has updated.

---

## 10. Sequencing summary

```
Phase 0 (½d spike + 1.5d setup)
    │
    ▼
Phase 1 (5–7d)  ────────┬─▶ Phase 2 (6–8d) ── [telemetry gate ─ 7d] ─▶ Phase 3 (5–7d) ─▶ Phase 4 (4–6d)
                        │
                        └─▶ Tray scope (2–3d, alongside Phase 2)

[Phase 5: cut by default. Re-evaluate after Phase 4 ships.]
```

**Concrete pause points (each leaves a shipped improvement):**
- After Phase 0 spike: stack decision made + one chart proves the wiring
- After Phase 2: every chart on `/metrics` and `/dashboard` is upgraded
- After Phase 3: stories layer ships
- After Phase 4: recap ships with sharable card

---

## 11. Decisions still open

These belong to the user, not the implementer:

1. **Stack decision Phase 0 spike outcome** — recorded inline once
   the spike runs.
2. **Parser enrichment thread** — does work to capture killer
   attribution / mission payout / commodity price start in parallel,
   so the dropped features can re-enter as Phase 6? Or accept the
   v1 scope as-is?
3. **Recap cadence** — weekly only, or weekly + monthly? Affects
   `period_kind` enum in `metrics_recap`.
4. **Public-profile share-scope defaults** — strategic plan assumed
   friend-only. Confirm: `share_scopes.summary` default = `'friend'`,
   everything else = `'me'`?
5. **Telemetry consent UX** — opt-in toggle in Settings, or opt-out
   prompt on first metrics-page view?

---

## 12. Risks & how they're mitigated here

| Risk | Mitigation in this plan |
|---|---|
| Tailwind preflight breaks `starstats-tokens.css` (999 lines, dual-app import) | Stack spike in Phase 0 + `corePlugins.preflight: false` if Tremor wins |
| Pattern-mining features look broken when game patches drift payload | Confidence threshold ≥70%, card hides below, CI test fixtures per patch |
| Achievements engine ships, no one cares | Demoted to deferred experiment behind feature flag; gate on recap share rate |
| Now strip melts the server at scale | Visibility gate + ETag/304 + backoff + server cache, mandatory; or punt to SSE |
| Phase 2 surfaces ship but no one uses them, Phase 3 wasted | Telemetry gate before starting Phase 3 |
| Recap "frozen" claim conflicts with no-snapshot-table | `metrics_recap` table moved into Phase 1, not Phase 4 |
| Phase 5 requires Phase 1 retro-rewrite of share-scope shape | `share_scopes` JSONB lands in Phase 1 with full key set |
| Calendar slip past 8 weeks | Cut list declared (§1.1) — Phases 3+4+5 are sequentially cuttable |
