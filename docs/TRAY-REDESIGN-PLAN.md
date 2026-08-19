# StarStats Tray UI — Redesign Plan

Plan derived from the **`design_handoff_starstats/tray-prototype`** bundle
(`StarStats (3).zip`, 2026-05-06) staged under `design/tray-prototype/`.

Companion to `docs/DESIGN-REDESIGN-PLAN.md` (which covers the web app).

---

## 1. Bundle inventory

| File | Size | Purpose |
|------|------|---------|
| `StarStats Tray UI.html` | 4 KB | Standalone preview entry — sets `data-theme="stanton"`, mounts a fixed 720px window with a fake-Tauri frame chrome |
| `tray-app.jsx` | 32 KB | Main tray app — `TrayHeader` + 3-pane router (status / logs / settings), tray-specific primitives (`TrayCard`, `KV`, `StatPill`, `StatusDot`, `Banner`, `Field`, `TextInput`, `PrimaryButton`, `GhostButton`), full `StatusPane` and `SettingsPane` |
| `tray-logs.jsx` | 16 KB | New `LogsPane` — search, type-pill filter, grouped-by-day list, selection + detail with raw line, ignore/delete actions |
| `tokens.css` | 32 KB | Same token system as the web bundle (already adopted in `apps/tray-ui/src/styles/starstats-tokens.css` via commit bad3c40) |
| `primitives.jsx`, `icons.jsx`, `quantum-warp.jsx`, `tweaks-panel.jsx` | — | Reused from the web bundle; tweaks-panel is dev tooling, not for shipping |

**Note:** `tokens.css` is byte-identical to the one already mirrored under `apps/tray-ui/src/styles/`. No re-copy needed.

---

## 2. Current state vs. design

The tray-ui already has token foundation (Wave bad3c40): `<html data-theme="stanton">` set, tokens imported, legacy var aliases. Existing components (`StatusPane`, `SettingsPane`) work through the alias shim.

| Aspect | Current (`apps/tray-ui`) | Design |
|--------|--------------------------|--------|
| Window width | Fluid (max 880px) | Fixed 720px window |
| Header | App.tsx renders `.app__header` (brand + nav buttons + tailing-state badge) | `TrayHeader` — 3-col grid: brand+version, nav tabs (status/logs/settings), tailing/idle status dot |
| Panes | 2: Status + Settings | 3: Status + Logs + Settings |
| Status sections | Tail + Sync + Hangar + per-type counts | Stat strip (4 pills) + Tailing + Sync (side-by-side grid) + Top types (ranked accent bars) + Timeline + Parser coverage + Discovered logs + Hangar |
| Settings sections | Game.log + Remote sync + RSI cookie | Same three, restyled with `TrayCard` chrome + `Field` + `TextInput` + `PrimaryButton` + `GhostButton` |
| Logs view | None | New view with stat strip, search, type pills, grouped-by-day list, selection + detail with raw line + sync state |
| Empty states | Generic | "Scope is clear." in-universe voice |
| Banners | None codified | `Banner` component for auth-lost (warn) + email-unverified (info) at top of Status |

---

## 3. Backend / API gap

| Need | Current API | Status |
|------|-------------|--------|
| Status data (tail/sync/event_counts/discovered_logs/account/hangar) | `get_status` | ✅ |
| Per-type bars | `status.event_counts` | ✅ |
| Parser coverage | `get_parse_coverage` | ✅ |
| Session timeline (Status pane) | `get_session_timeline` | ✅ |
| Logs pane — paginated event list with filter | — | ❌ **Net-new** |
| Logs pane — sync flag per event | — | ❌ **Net-new** (server tracks acceptance but per-event sync state isn't surfaced to the tray) |
| Logs pane — raw line + payload | — | ❌ **Net-new** (tail strips raw lines after parse) |
| Logs pane — DB size | — | ❌ **Net-new** (small — `pragma page_count * page_size`) |

**Decision for this wave:** Status + Settings ship now (no backend change needed). Logs pane lands in a follow-up wave once we extend `get_session_timeline` (or add `list_local_events`) to include the new fields.

---

## 4. Phased rollout

Each wave is independently shippable.

### Wave T1 — Foundation ✅ DONE

Already shipped in commit `bad3c40`: tokens.css imported, `<html data-theme="stanton">`, defensive `:root` Stanton fallback, legacy var aliases. The existing panes still render through the alias.

### Wave T2 — App shell + TrayHeader + 3-pane nav ✅ DONE (a91f2f1)

Shipped: new `TrayHeader` (3-col grid: brand + version + tabs + tailing pill); `'logs'` added to the view state machine pointing at a `LogsPlaceholder` (replaced in T5); package version surfaced via Vite `define` → `__APP_VERSION__`. The 720px max-width landed in T6 instead of being skipped.

### Wave T3 — StatusPane redesign ✅ DONE (a91f2f1)

Shipped: full design-package layout (banners → stat strip → tailing/sync grid → top types → timeline → coverage → discovered logs → hangar) on top of the tray primitives. Polling, mark-as-noise mutation, and banner precedence preserved verbatim. Voice: Comm-Link + "Scope is clear." for empty states.

### Wave T4 — SettingsPane redesign ✅ DONE (a91f2f1)

Shipped: 3-card layout (Game.log, Remote sync with header on/off toggle and pairing flow, RSI session cookie). Pairing, cookie save/clear, and the `window.confirm` guard preserved. Voice rename: "Devices →" → "Hangar →" in pairing copy.

### Wave T5 — Logs pane + backend extension ✅ DONE (7e27c18)

Shipped: `RecentEventRow` extended with `raw_line` (and later `log_source`); `TimelineEntry` extended with `raw_line`, `log_source`, and `synced` (derived from a snapshotted `sync_cursor` to avoid mid-iteration race). New `get_storage_stats` Tauri command returning `total_events` + `db_size_bytes` (PRAGMA-driven). New `LogsPane` matches the design: stat strip, search, type pills, day-grouped list, full detail drawer with raw-line toggle, copy-to-clipboard, and mark-as-noise.

### Wave T6 — Polish + review ✅ DONE (fab0bb5)

Shipped: `max-width: 720px; margin: 0 auto;` on `.app__main` so the design holds at design width on wider tray windows. Three-pass parallel review (fact check, security, readability) findings synthesized:
- Save-tick now clears on resumed editing (HIGH fact-check finding).
- Shared `tray/format.ts` extracted (`fmtBytes`/`fmtTime`/`fmtCovPct`/`fmtDate`/`ageLabel`/`toneForType`/`TONE_VAR`) — drops the duplicated helpers and the 5-deep nested timeline-accent ternary in StatusPane.
- LogsPane `refresh` and `tick` collapsed into one `fetchAndApply` with optional abort signal.
- `web_origin` JSX site now requires `http(s):` scheme before rendering as a clickable anchor (LOW security guard).
- Dev-only Tweaks panel: not shipping. Tray theme is stuck at Stanton until/unless we add a tray-side theme picker (out of scope).

---

## 5. Voice / copy renames (same as web)

- Email → Comm-Link
- (Empty state) → "Scope is clear."
- Authentication code (TOTP / 2FA) — N/A for tray (no 2FA in tray UI)

---

## 6. Risks

- **Tauri window resize:** Tauri's webview is resizable; the design assumes 720px. Use a max-width `<main>` container so the design holds at the design width and degrades gracefully on wider/narrower windows.
- **Logs DB size:** A user with several months of tail history could have tens of thousands of events. Pagination matters — design loads 142 mock events; production needs a `LIMIT 500` default + virtual scroll if it grows.
- **Sync flag accuracy:** Server-side acknowledgment is per-batch, not per-event. We can mark all events in a successful batch as `synced=true` on the client side, but partial-failure recovery isn't perfect.

---

## 7. References

- Design source: `design/tray-prototype/`
- Existing components: `apps/tray-ui/src/components/{StatusPane,SettingsPane}.tsx`
- Existing API: `apps/tray-ui/src/api.ts`
- Wave 8.1/8.2/8.3 web counterpart: `docs/DESIGN-REDESIGN-PLAN.md`
