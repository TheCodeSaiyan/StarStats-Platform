# StarStats — Metrics Display Redesign Plan

Sub-plan of `docs/DESIGN-REDESIGN-PLAN.md`, focused on the metrics
surfaces specifically (Wave 10 of the master plan, plus three new
themed surfaces — Stories, Personality, Live — that the master plan
did not cover).

This document is a **plan**, not implementation. Phase 0 starts when
§7 (open questions) is resolved.

---

## 1. Goal & non-goals

**Goal.** Move the metrics surface from "raw rows + counts" to a
display that is (a) information-dense without being noisy, (b) tells
stories the raw event stream can't, (c) feels alive when activity is
recent, and (d) has the Wrapped-style personality that drives
re-engagement.

**Surfaces in scope:**
- `apps/web/src/app/metrics/page.tsx` — 4-tab primary metrics hub
- `apps/web/src/app/dashboard/page.tsx` — first-impression summary
- `apps/web/src/app/u/[handle]/page.tsx` — public/friend profile
- `apps/web/src/app/orgs/[slug]/page.tsx` — org-aggregated metrics
- `apps/tray-ui/src/components/StatusPane.tsx` — local activity panel
- **New:** `/metrics/recap` route — weekly/monthly Wrapped-style card

**Non-goals.**
- App-shell, auth, settings, hangar, donate redesign — covered by the
  master DESIGN-REDESIGN-PLAN.
- New event types — we work with the existing 26 `GameEvent` variants.
- Real-time push (WebSocket / SSE) for the "Live" theme — short-poll
  is good enough for v1; SSE is a follow-up.

---

## 2. Stack decision (Phase 0)

### Chart library: **Tremor**

Tailwind-native, prebuilt dashboard primitives, ~180 KB gz. Maps
cleanly to almost every visualisation in this plan. Bespoke shapes
(sankey, chord, starmap) escape to bare Recharts or `@nivo/sankey`.

### Required side quest: add Tailwind to `apps/web`

Tremor v3 requires Tailwind. `apps/web` currently has no Tailwind /
PostCSS config. Phase 0 must:

1. Install `tailwindcss` + `postcss` + `autoprefixer`.
2. Create `tailwind.config.ts` with `content: ['./src/**/*.{ts,tsx}']`.
3. Create `postcss.config.mjs` with `tailwindcss` + `autoprefixer`.
4. Add a `@tailwind base/components/utilities` block at the top of
   `globals.css` — kept side-by-side with the existing 672 lines of
   bespoke `.site-header`, `.form`, `.legal` classes so the migration
   can be incremental.
5. Wire the `ss-*` CSS variables from the prototype's `tokens.css`
   (4 themes, type scale, spacing, motion) into the Tailwind config
   `theme.extend` so Tremor and existing CSS share a single source
   of truth.

Risk: Tailwind's preflight resets margin/padding on `body`, `h*`,
etc. Either disable preflight (`corePlugins.preflight: false`) or
audit the 672 lines for collisions. **Recommendation: disable
preflight** for the first wave — easier to enable later than to
debug global resets mid-migration.

---

## 3. Phased delivery

Phases are ordered by ROI so any pause point leaves a working,
shipped improvement.

### Phase 0 — Foundations (~½ day)

| Deliverable | Files |
|---|---|
| Tailwind + PostCSS wired into `apps/web` | `tailwind.config.ts`, `postcss.config.mjs`, `package.json`, `globals.css` |
| Tremor installed and themed against `ss-*` tokens | `tailwind.config.ts`, `lib/tremor-theme.ts` |
| `<MetricCard>` and `<ChartCard>` shells | `components/metrics/MetricCard.tsx`, `ChartCard.tsx` |
| One Tremor sample on `/metrics` proving the wiring | `metrics/page.tsx` |

Exit criterion: a Tremor `<AreaChart>` renders on the metrics page
using accent colours from the active theme.

### Phase 1 — Backend aggregates (~3–5 days)

Wave 10 prereq from the master plan. Without these, the frontend
would reduce() over the full event stream every render — fine for
hundreds of events, broken at tens of thousands.

| Endpoint | Returns |
|---|---|
| `GET /me/metrics/aggregates?period=30d` | Daily totals (kills/deaths/missions/aUEC in/out), top event types, top zones, current streak |
| `GET /me/metrics/sessions?period=30d` | Inferred sessions (start, end, duration, location, K/D, event count) |
| `GET /me/metrics/deaths?period=30d` | Death cause buckets (zone, killer type, weapon if present), nemesis candidates |
| `GET /me/metrics/missions?period=30d` | Mission-type rollups: count, completion rate, avg payout, avg duration, biggest single payout |
| `GET /me/metrics/economy?period=30d` | aUEC in/out per day (commodity + shop), top commodities, top shops |
| `GET /me/metrics/recap?period=week\|month` | Frozen recap snapshot for the Wrapped card (cacheable for 24h) |

Storage: read-side aggregation from the `events` table is fine for
v1 (event volume per user is low). Materialized view at the
`weekly_metrics_<handle>` level can come later if we add the live
"now" strip and need sub-second responses.

Auth: same `RequireUser` extractor as existing `/me/*` routes.
Cross-handle access (`/u/{handle}/metrics/*`) reuses the existing
visibility / friend-scope checks from `getPublicSummary` /
`getFriendSummary`.

### Phase 2 — Density: charts replace tables (~1 week)

Theme 1 from the brainstorm. The 80% improvement.

| Today | Phase 2 replacement |
|---|---|
| 30-day grid heatmap (hand-rolled) | **Year-view GitHub-style heatmap** with per-day breakdown on hover; clicking jumps to that day's session list |
| Total kills/deaths/sessions pills | **Sparkline-backed pills** — pill body shows trend over 30 days at a glance |
| Types tab manual bar divs | **Tremor `<BarList>` + `<DonutChart>`** side-by-side — count + share-of-total |
| Sessions tab text list | **Session ribbon** — horizontal strip of last 50 sessions; height = duration, colour = K/D direction, hover = summary, click drills in |
| Raw stream table | Keep as-is, but add row-level type icons + zone glyphs |

Files: `metrics/page.tsx`, `dashboard/page.tsx`, plus new
`components/metrics/*` for `YearHeatmap`, `SparklinePill`,
`SessionRibbon`, `TypeBreakdown`.

### Phase 3 — Stories: new informational surfaces (~1–2 weeks)

Theme 2 from the brainstorm. Surfaces the data hides today.

| New surface | Source events | Display |
|---|---|---|
| **Death recap card** | `ActorDeath`, `PlayerDeath`, `VehicleDestruction` | Top-N bar list of cause buckets (zone × killer-type × weapon), plus "compared to last week" delta |
| **Mission economy panel** | `MissionStart` ↔ `MissionEnd` pairing | Per mission-type table: count, completion %, avg payout, avg duration, biggest single payout; expandable row → time series |
| **Spending vs earning** | `CommodityBuyRequest`, `CommoditySellRequest`, `ShopBuyRequest` | Tremor `<AreaChart>` with two stacked series (in / out) over time; profit line on top |
| **Server-hop view** | `ChangeServer`, `JoinPU` | Server-name bar list + total dwell time bar — surfaces queue / shard patterns |
| **"Where you die" cluster** | `PlayerDeath.zone` | Tremor `<BarList>` of top zones; v2 promotes to a starmap with hotspot bubbles |

Each lands as a `<ChartCard>` on the metrics page's existing
Overview tab, replacing the current event-stream tail.

### Phase 4 — Personality: Wrapped energy (~1–2 weeks)

Theme 3 from the brainstorm. The re-engagement layer.

| Feature | Backend | Frontend |
|---|---|---|
| **Achievements engine** | New `achievements` table; nightly cron scans events to award titles; `GET /me/achievements` | Achievements page; "newly earned" toast on next login; profile badge row |
| **Weekly recap card** | `GET /me/metrics/recap?period=week` (Phase 1) | New `/metrics/recap` route; share image via Next OG-image route (`opengraph-image.tsx`) |
| **Nemesis tracker** | Aggregate `PlayerDeath.killer_handle` over period | Card on death recap section; "Your nemesis this month: @X (killed you 4×)" |
| **Signature move** | Pattern-mine most-used ship / most-run mission / most-walked route from session aggregates | Headline card on dashboard: "You spend 60% of your time hauling Laranite from Lyria → ARC" |

Achievements catalogue (initial 10):
- *Crusader Local* — >50% of sessions in Crusader over 30d
- *The Comeback* — 5-death streak followed by 5-kill streak in one session
- *Speed Trader* — 10+ commodity loops in a single session
- *Mission Mason* — 100 missions completed
- *No Backups* — 5+ sessions without a single death
- *Pyro Pioneer* — first 10 sessions in Pyro
- *Quantum Tourist* — visited 20+ distinct zones in a week
- *Server Drifter* — 5+ server changes in one session
- *Hauler Hall of Fame* — 10M+ aUEC earned via commodity sales in a week
- *Crash Test Dummy* — 3+ vehicle destructions you caused yourself

### Phase 5 — Live & comparative polish (~1 week)

Theme 4 + Theme 5 from the brainstorm. Cheapest tier; saved for
last because it's the most cosmetic.

| Feature | How |
|---|---|
| **"Now" strip** at top of dashboard | 30 s short-poll on `/me/metrics/recent`; render when last event is <5 min old |
| **Animated event insertion** in raw stream | Framer Motion `<AnimatePresence>` on the list; new items fade-in + brief accent flash |
| **Pulse dot on tab badges** | LocalStorage "last-viewed at" per tab; badge dot when newest event is newer |
| **Org leaderboard widget** | New `GET /orgs/{slug}/leaderboard?metric=missions&period=week` → tremor `<BarList>` ranked |
| **Percentile ribbons** | Pre-computed nightly on opt-in public profiles; `GET /me/metrics/percentiles` |
| **Friend overlay** | Existing friend system + a per-chart `?compare=@handle` query param |

---

## 4. Tray-UI scope

Smaller surface, looser ambition. Tray is for "is StarStats working
right now?" — not for deep analysis.

| Today (`StatusPane.tsx`) | Proposed |
|---|---|
| Tail status banner | Keep |
| Event count breakdown | Replace with a **48-hour sparkline** of events / hour; small Tremor `<SparkAreaChart>` |
| Discovered logs section | Keep, add per-log dwell-time badge |
| Sync status pill | Add **last-N sessions ribbon** beneath, mirroring the web ribbon at small scale |
| (none) | Add **"recap available"** indicator + button that opens `https://app.starstats/metrics/recap` in the default browser whenever a fresh weekly recap is ready (read from `/me/metrics/recap` headers) |

No Tailwind in the tray — it's a Vite + plain CSS app. Tremor would
add too much weight here; the sparkline is small enough to draw with
inline SVG against the `ss-*` tokens already imported from the
shared design system. ~50 lines of code.

---

## 5. Backend API sketches (Phase 1 in detail)

All responses use the existing API envelope. Periods are
`?period=24h|7d|30d|90d|all` (default `30d`).

```jsonc
// GET /me/metrics/aggregates?period=30d
{
  "period": "30d",
  "daily": [
    { "day": "2026-04-13", "kills": 3, "deaths": 1, "missions": 2,
      "aUEC_in": 412000, "aUEC_out": 88000 }
    // ... 30 entries
  ],
  "top_event_types": [
    { "type": "ActorDeath", "count": 142, "share": 0.31 }
    // top 6
  ],
  "top_zones": [
    { "zone": "Crusader/Orison", "count": 89, "share": 0.19 }
    // top 6
  ],
  "current_streak": { "kind": "no_death", "count_days": 3 }
}

// GET /me/metrics/deaths?period=30d
{
  "period": "30d",
  "total_deaths": 23,
  "causes": [
    { "bucket": "Ammo cook · Hurston", "count": 7 },
    { "bucket": "Ramming · Crusader", "count": 4 }
    // top 8 bucketed by zone × killer_type × weapon_class
  ],
  "nemesis_candidates": [
    { "handle": "PirateGuy43", "kills_on_you": 4 }
  ]
}

// GET /me/metrics/recap?period=week
// Cacheable (24h Cache-Control). Frozen at week-end boundary.
{
  "period": { "kind": "week", "start": "2026-05-04", "end": "2026-05-10" },
  "totals": { "kills": 18, "deaths": 12, "missions": 14, "aUEC_net": 2740000,
              "longest_session_secs": 15120 },
  "highlight": { "kind": "biggest_payout", "amount": 850000,
                 "mission_type": "Bunker · ERT" },
  "signature_move": { "ship": "C2", "route": "Lyria → ARC", "share": 0.61 },
  "achievements_unlocked": ["speed_trader"],
  "share_image_url": "/metrics/recap/week/2026-05-04/og.png"
}
```

Implementation: Rust handlers in `crates/starstats-server/src/metrics_routes.rs`
(new module) reading from the existing `events` table with
`GROUP BY` + `date_trunc` queries. No new schema for Phase 1
aggregates — only the recap snapshot table lands in Phase 4.

---

## 6. Risks & decisions to defer

| Risk | Mitigation |
|---|---|
| Tailwind preflight breaks existing 672-line `globals.css` | Disable preflight in `tailwind.config.ts` for first wave |
| Event-payload shape inconsistency across game patches | Death-cause and mission-payout extraction is best-effort; surface "unknown" buckets rather than crash |
| Aggregates slow at high event volumes (>100k per user) | Add a nightly materialized roll-up in a follow-up; Phase 1 is read-path only |
| Achievements feel grindy / pointless | Ship 10 narrow ones first, watch what users react to, then expand; don't ship 50 on day one |
| Recap share-image route is a complex Vercel-only feature | Use Next's built-in `opengraph-image.tsx` API — works on any Node deploy, not Vercel-locked |
| Quantum-warp background + dense charts compete for visual attention | Dim warp opacity from `--bg-pulse-strength` token on metric routes specifically |
| Tray sparkline adds Tauri build weight | Inline SVG only — no chart library on the tray |

---

## 7. Open questions

1. **Tailwind adoption confirmation.** This plan assumes Tailwind
   ships as part of Phase 0. Confirm or pivot to Recharts-without-
   Tailwind (loses ~30% of the prebuilt primitive surface that makes
   Tremor attractive).
2. **Recap cadence.** Weekly + monthly? Weekly only? Driven by user
   timezone or UTC?
3. **Achievements: server-side compute or client-side?** Plan assumes
   server-side (nightly cron) so achievements are durable + sharable.
   Client-side would be cheaper but loses share-card use.
4. **Public-profile metrics scope.** How much of this surfaces on
   `/u/{handle}` for non-friends? Plan assumes friend-only for
   anything beyond the existing summary; confirm.
5. **Org leaderboard privacy.** Members opt-in per metric, or org-
   wide setting? Plan assumes member opt-in (consistent with current
   share-scope model).

---

## 8. Sequencing summary

```
Phase 0 (½d) ─▶ Phase 1 (3–5d) ─┬─▶ Phase 2 (1w) ─▶ Phase 3 (1–2w) ─▶ Phase 4 (1–2w) ─▶ Phase 5 (1w)
                                 └─▶ (tray surface lands alongside Phase 2)
```

Pause points: after Phase 2, after Phase 3, after Phase 4. Each
leaves a shipped, materially-improved metrics surface.
