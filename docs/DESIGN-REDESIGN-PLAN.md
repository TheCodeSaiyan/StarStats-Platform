# StarStats — Design Redesign Plan

Plan derived from the **`design_handoff_starstats`** package
(`StarStats (1).zip`, 2026-05-05) extracted to
`%TEMP%\starstats-design\design_handoff_starstats\`.

This document is a **review + plan**, not an implementation. Implementation
should not begin until the open questions in §8 are answered.

---

## 1. Package inventory

| File | Size | Purpose |
|------|------|---------|
| `README.md` | 8.7 KB | Author intent, fidelity, screen list, motion spec, voice/copy guide, backend gaps headline, recommended impl order |
| `DESIGN_REFERENCE.md` | 18.6 KB | Per-page structural map: route, components, data shape, backend needs |
| `prototype/tokens.css` | 32.3 KB | **Production CSS.** Tokens (color/type/spacing/motion), 4 themes, all `ss-*` component classes, responsive rules, animations, reduced-motion. Drop-in usable. |
| `prototype/primitives.jsx` | 10.6 KB | `Card`, `Badge`, `Button`, `Input`, `KV`, `Toggle`, `Secret`, `OTP`, `Heatmap`, `TypeBars`, `TimelineList`, `Stack`, `Row` — reference behaviour over `tokens.css` classes |
| `prototype/app.jsx` | 8 KB | Top-level routing + theme + Tweaks panel |
| `prototype/icons.jsx` | 4 KB | Stroke icon set (recreate as Lucide/Tabler equivalents) |
| `prototype/shell-and-landing.jsx` | 16 KB | TopBar, LeftRail, Drawer, NAV map, Landing page |
| `prototype/auth-screens.jsx` | 11.7 KB | Login / MagicSent / TotpVerify / Signup |
| `prototype/app-screens.jsx` | 14 KB | Dashboard / Devices (Hangar) / Orgs / Public profile |
| `prototype/data-screens.jsx` | 33.7 KB | Metrics + Submissions (NEW section) |
| `prototype/my-logs.jsx` | 28.5 KB | My logs + BatchDetail (NEW) |
| `prototype/settings-and-system.jsx` | 25.4 KB | Settings, 2FA wizard, Components sheet, Download |
| `prototype/donate.jsx` | 19.5 KB | Donate / supporter tiers (NEW) |
| `prototype/quantum-warp.jsx` | 7 KB | Signature parallax background animation |
| `prototype/tweaks-panel.jsx` | 24.9 KB | Live theme/type/accent switcher (dev) |
| `prototype/design-canvas.jsx` | 48 KB | Side-by-side responsive preview frame |
| `prototype/StarStats Prototype.html` | 2 KB | Standalone preview entry |
| `prototype/StarStats Responsive Canvas.html` | 6 KB | 390/820/1440 simul preview |

**Fidelity statement (verbatim from README):** "All colors, spacing, typography, and interactions are final. Recreate pixel-perfectly. Use exact tokens from `prototype/tokens.css`."

---

## 2. Visual baseline — current vs. design

| Aspect | Current `apps/web` | Design package |
|--------|---------|----------------|
| CSS strategy | Bespoke classes in `globals.css` (672 lines): `.site-header`, `.form`, `.legal`, `.unverified-banner`, `.error`, `.success` | Token-driven `.ss-*` class system in `tokens.css` |
| Accent | Single hardcoded `#4682f0` (blue) | 4 themes (Stanton amber `#E8A23C`, Pyro coral `#F25C3F`, Terra teal `#4FB8A1`, Nyx violet `#5B3FD9`) — switched via `[data-theme="..."]` on root |
| Type | `ui-sans-serif, system-ui, -apple-system, ...` 15 px | Geist + Geist Mono (defaults), Inter / IBM Plex selectable. 8 px type scale 11/13/14/16/20/28/40/64 |
| Spacing scale | Ad-hoc | `--s1`…`--s8` = 4 / 8 / 12 / 16 / 24 / 32 / 48 / 64 |
| Radii | Ad-hoc | 4 / 6 / 10 / 14 / 20 / pill |
| Motion | None codified | `--ease-out cubic-bezier(0.22, 1, 0.36, 1)`, fast 120 / base 180 / slow 320 ms |
| Theming | None | 4 themes + accent intensity multiplier; respects `prefers-reduced-motion` |
| Background | Plain `var(--bg)` | **Quantum-warp** parallax ribbons + soft pulse vignette per-screen direction |
| App shell | Top-only `.site-header` + content | Top bar + left rail (220 px desktop / 60 px tablet / off-canvas drawer mobile) |
| Status badges | None | `.ss-badge` ok/warn/danger/accent + dot variant |

**Implication:** Nothing in current `globals.css` collides with the new system *by name* (different prefixes), so they can coexist during a phased migration. But the app shell, header, and footer are entirely new layouts, not touch-ups.

---

## 3. Route inventory — design screens vs. existing app

✅ exists | 🔧 exists, needs redesign + voice/copy pass | 🆕 net-new

| # | Design screen | Route | Status | Notes |
|---|---|---|---|---|
| 1 | Landing | `/` (`page.tsx`, 37 lines stub) | 🔧 | Current is barely-there. Hero, 6 features, mock dashboard preview, footer all new. |
| 2 | Login | `/auth/login` | 🔧 | Add "Send magic link instead" CTA, voice/copy, ss-otp on TOTP screen |
| 3 | Magic-link sent | `/auth/magic-link` | 🔧 | Already has `redeem` subroute. UI polish only. |
| 4 | TOTP verify | `/auth/totp-verify` | 🔧 | Adopt `ss-otp` 6-digit segmented input |
| 5 | Sign up | `/auth/signup` | 🔧 | Voice/copy + token swap |
| 6 | Dashboard | `/dashboard` (329 lines) | 🔧 | Heatmap, stat strip, top-types ranked bars, timeline already exist conceptually — port to `ss-heatmap` / `TypeBars` / `TimelineList` |
| 7 | Hangar (Devices) | `/devices` | 🔧 | Rename **Devices → Hangar** in nav + page chrome. Pair-code card design is new. |
| 8 | Orgs | `/orgs` (+ `[slug]`, `/new`) | 🔧 | Visibility / share-scope controls + voice |
| 9 | Public profile | `/u/[handle]` | 🔧 | Supporter pill, name plate, theme accent display — gated on donate gap (§5) |
| 10 | Settings | `/settings` (868 lines) | 🔧 | Theme switcher (4-card preview) + name plate (gated on donate) + retention policy display |
| 11 | Two-factor wizard | `/settings/2fa` | 🔧 | 4-step wizard layout; recovery-code download/copy/print; gated on backend recovery-code single-use tracking |
| 12 | Donate | — | 🆕 | Whole new page. Stripe checkout + supporter pill preview + name plate input. **Blocked on backend gap.** |
| 13 | Download client | — | 🆕 | OS cards + `GET /releases/latest`. Currently we tag GitHub Releases manually — could read directly. |
| 14 | Components sheet | — | 🆕 | Dev-only; trivial, build last. |
| 15 | Metrics (4 tabs) | — | 🆕 | **Blocked on `GET /me/metrics/{event-types,sessions,events}`.** |
| 16 | My logs (4 tabs + BatchDetail) | — | 🆕 | **Blocked on whole batch-introspection backend layer.** |
| 17 | Submissions (list + detail) | — | 🆕 | **Blocked on entire submissions domain backend.** |

**Existing routes the design does NOT cover (must keep):**
- `/auth/email-change`, `/auth/forgot-password`, `/auth/reset-password`, `/auth/verify` — keep functional, restyle to match.
- `/api/healthz`, `/api/metrics` — internal, no UI.
- `/privacy` — keep, restyle as `legal` long-form (token swap).

---

## 4. Voice / copy rename pass

The README mandates an **in-universe Star Citizen voice** for chrome and section names. Errors stay dry.

| Current term | Design term | Where it appears |
|---|---|---|
| Devices | **Hangar** | Nav, page title, route consideration |
| Email | **Comm-Link** | Auth forms, settings, ProfileCard |
| Dashboard / Events | **Manifest** | Optional rebrand of dashboard tab |
| TOTP / 2FA code | **Authentication code** | 2FA wizard, TOTP verify |
| (Empty state) | "Scope is clear." | All empty-data UIs |

**Decision needed:** rename the route `/devices` → `/hangar` (with redirect)? Or keep route, change visible label only? My recommendation: **keep route, change label** — preserves existing bookmarks and shorter to ship.

---

## 5. Backend gaps blocking design completion

From `DESIGN_REFERENCE.md` § Backend gaps summary, ranked by what unblocks the most UI:

| # | Gap | Blocks | Effort estimate |
|---|---|---|---|
| 1 | **Submissions domain** — `submissions`, `submission_votes`, `submission_flags`, `submission_comments` tables + 9 endpoints + dedup logic + 3-flag escalation + parser-release pipeline | Submissions list + detail (~33 KB of design) | Wave-sized (≥ 2 days) |
| 2 | **Batch introspection** — keep raw batches addressable by ID; per-line drill-down; per-batch retry / delete; content hash; sequence-gap detection | My logs → all 4 tabs + BatchDetail | Wave-sized |
| 3 | **Health telemetry** — per-client success_rate, avg_latency_ms, online_state; recent_incidents (gaps, retries, clock_drift) | My logs → Health tab | Medium |
| 4 | **Storage quota** — per-user byte tracking by category (parsed events / raw archive / sessions+meta / unknowns); 50 MB free-tier cap; supporter unlimited | My logs → Storage tab + Settings retention | Medium |
| 5 | **Metrics aggregates** — `GET /me/metrics/event-types?range=30d`, `GET /me/metrics/sessions?limit&offset`, `GET /me/metrics/events?limit&offset&type&session&from&to` + session inference | Metrics page (4 tabs) | Medium |
| 6 | **Donate / Stripe** — checkout-session, webhook → supporter status + name plate (28-char cap) + retention extension; cancellation revert behaviour (keep pill+nameplate, drop retention+accent) | Donate page; supporter pill on profile/settings | Medium |
| 7 | **Async export jobs** — NDJSON / zip / CSV → emailed download link | Settings danger zone, My logs → Storage |  Small-medium |
| 8 | **Handle re-verification** — already covered by Wave 4 RSI verify (✅ done) | Public profile | Done |
| 9 | **Org weekly re-verification job** | Orgs (already half-built — Wave 6) | Small |
| 10 | **2FA recovery code single-use tracking** | 2FA wizard | Small |
| 11 | **`/me/preferences`** (theme + notifications GET/PUT) | Settings theme switcher persistence | Trivial |

**Strategy:** ship visual-redesign waves *first* (no backend changes), then tackle the new feature waves in parallel.

---

## 6. Adoption strategy — Tailwind vs. raw CSS

The README recommends "Next.js (App Router) + Tailwind + shadcn/ui." We already have Next.js App Router; we don't have Tailwind or shadcn. Two paths:

### Path A — Drop in `tokens.css` verbatim (RECOMMENDED for Wave 8.1)

- Copy `prototype/tokens.css` to `apps/web/src/styles/starstats-tokens.css` and `@import` from `globals.css`.
- New components use `ss-*` classes directly.
- Existing pages migrate page-by-page (rename classes → drop ss-* prefix).
- **Lowest risk, fastest first paint**, keeps build pipeline unchanged.
- Tradeoff: duplicate styling layer during migration; no shadcn primitives.

### Path B — Migrate to Tailwind + shadcn

- `pnpm add -D tailwindcss @tailwindcss/postcss` + `tailwind.config.ts` mapping the tokens to theme.
- Install `shadcn` CLI; pull primitives (Button, Card, Badge, Input, Tabs, Dialog) themed via CSS variables from `tokens.css`.
- Heavier upfront lift; more idiomatic long-term; better story for the new data section's tabs/tables.

**My recommendation: Path A for Wave 8.1 (visual refresh of existing pages), Path B for Wave 9 (new data section)** — by then we'll know how the token system shakes out and can adopt shadcn for the heavier table/tabs UIs.

---

## 7. Recommended phased rollout

Each wave is independently shippable. Numbering picks up from Wave 7 (Vehicle reference data, shipped in `ebe8b90`).

### Wave 8.1 — Foundation (visual only, no backend)

- Copy `tokens.css` to `apps/web/src/styles/starstats-tokens.css`; import from `globals.css`. Remove dead bespoke classes (`.site-header`, `.cta`, etc.) as they're replaced.
- Build app shell: `<TopBar>` / `<LeftRail>` / `<DrawerScrim>` components with the NAV map from design.
- Update `apps/web/src/app/layout.tsx` to render the shell for authenticated pages; landing/auth/legal stay full-width.
- Add `data-theme` to `<html>` (default `stanton`) — read from cookie if present, else default.
- Implement Quantum-warp background as a `<QuantumWarp>` client component (≤ 120 LOC, `prefers-reduced-motion` aware).
- Voice/copy rename pass: nav labels, page titles. **Do not** rename routes yet.

**Estimated diff:** ~1500 lines added (tokens.css + shell), ~200 lines removed (dead bespoke styles). Files modified: `globals.css`, `layout.tsx`, `apps/web/src/components/{TopBar,LeftRail,DrawerScrim,QuantumWarp}.tsx` (new), `apps/web/src/styles/starstats-tokens.css` (new).

### Wave 8.2 — Page-by-page port

Sequential, low-risk redesigns of existing pages — port to `ss-*` classes:

1. Landing (`/`) — hero with rotating word swap, 6 feature cards, mock dashboard preview
2. Auth screens (5 routes) — `ss-auth-frame` + `ss-input` + `ss-otp` for TOTP
3. Dashboard (`/dashboard`) — `Heatmap`, `TypeBars`, `TimelineList`, stat strip
4. Devices (`/devices`, label: Hangar) — pair-code `ss-pair-code`, paired list
5. Orgs (`/orgs`) — visibility toggles
6. Public profile (`/u/[handle]`) — supporter pill placeholder (gated)
7. Settings (`/settings`) — theme switcher (`ss-theme-swatch` × 4) + sections
8. 2FA wizard (`/settings/2fa`) — 4-step layout, `ss-secret`, recovery codes step (visual only; gating on backend recovery-code tracking remains)
9. Privacy (`/privacy`) — token swap

### Wave 8.3 — Theme persistence + preferences

- Add `GET /v1/me/preferences` and `PUT /v1/me/preferences` to `starstats-server` (theme: `stanton|pyro|terra|nyx`, notifications). New table or column on `users`.
- Settings page wires to it (replaces local Tweaks-panel state).
- `<html data-theme>` hydrates from preferences cookie at SSR.

### Wave 9 — Donate + supporter mechanics

Backend (Stripe + name plate) + Donate page + supporter pill on profile/settings. Tightly scoped because Stripe webhook is a known surface.

### Wave 10 — Metrics page + aggregates

Backend `GET /me/metrics/{event-types,sessions,events}` + session inference. Frontend Metrics page with 4 tabs. **First page that warrants Path B (Tailwind + shadcn) for the table-heavy tabs.**

### Wave 11 — My logs + batch introspection

Backend: keep raw batches addressable, per-line drill-down, retry/delete, sequence-gap detection. **Largest backend wave — split internally.**

### Wave 12 — Submissions

Whole new domain. Probably split into 12.1 (data + endpoints) and 12.2 (UI).

### Wave 13 — Polish

Download client page (read `GET /releases/latest`), Components sheet (dev-only), final motion + reduced-motion audit.

---

## 8. Open questions (must answer before Wave 8.1 starts)

1. **Path A vs. Path B for Wave 8.1?** My recommendation: A (raw `ss-*` CSS), revisit at Wave 10.
2. **Rename `/devices` route to `/hangar`?** My recommendation: keep route, change visible label only.
3. **Default theme — `stanton` (warm amber) as design intends?** Or pick a different default for the existing user base?
4. **Tray-ui scope.** Tray-ui is not in the design package (it's web-focused). Should the tray adopt the same tokens for consistency, or stay on its current `styles.css` palette? My recommendation: adopt tokens in a small follow-up wave so the desktop client matches.
5. **Voice/copy rollout — all at once or per-page?** I recommend per-page (lands with each redesign in Wave 8.2) to avoid a big-bang copy diff.
6. **Quantum-warp ship gate.** Reference impl is canvas-driven (90 parallax streaks); `tokens.css` ships a CSS-only fallback. Ship the CSS fallback in Wave 8.1, port the canvas version in Wave 13?
7. **Backend gap acceptance.** Are gaps 1-7 in §5 ACCEPTED as net-new work? Some are large (Submissions especially). If any are out-of-scope for now, those design pages stay un-shipped (Components sheet, Donate, Metrics, My logs, Submissions).
8. **Asset hosting for design assets.** Should `prototype/tokens.css` be checked in verbatim under `design/` for future reference, or only as the working `apps/web/src/styles/starstats-tokens.css`? My recommendation: stage both — `design/tokens.css` (read-only ref) + `apps/web/src/styles/starstats-tokens.css` (working copy, may diverge for shadcn integration later).

---

## 9. Risks

- **Tokens.css imports Google fonts.** Geist + Geist Mono need to be loaded via `next/font/google` or `<link>` — adds a render-blocking style request. Mitigate with `next/font/google` `display: 'swap'`.
- **`color-mix(in oklab, ...)`** is used heavily in tokens.css — Safari ≥ 16.4 / Chrome ≥ 111 / Firefox ≥ 113. Should be fine for our user base but worth listing.
- **Quantum-warp on Pyro / accent intensity 1.4** — combined the background can feel busy on long-form reading pages. The `--accent-glow-mul` slider exists for a reason; keep accent intensity ≤ 1.0 by default for first paint.
- **Tray-ui currently uses Tauri-flavoured styles** — adopting `ss-*` requires either ditching Tauri's plugin styles or carefully scoping. Out of scope for Wave 8.1.

---

## 10. References

- Source design package: archived by the maintainer locally.
- Read-only mirror in repo (proposed Wave 8.1): `design/`
- Existing similar plan docs: `docs/ARCHITECTURE.md`, `docs/AUDIT.md`
