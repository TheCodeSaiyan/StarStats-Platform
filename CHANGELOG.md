# Changelog

All notable changes to StarStats will be documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Pre-1.0, semver applies in spirit only — the wire format, schema, and
event coverage are still evolving and may break on minor releases.

Tag-suffix → release-channel mapping (see `release-manifests/`):

- `vX.Y.Z-alpha[.N]` → `alpha.json`
- `vX.Y.Z-beta[.N]`  → `beta.json`
- `vX.Y.Z-rc[.N]`    → `rc.json`
- `vX.Y.Z`           → `live.json`

## [Unreleased]

- (nothing yet)

## [1.8.1] - 2026-05-22

Post-1.8.0 fixes — same-day patch closing three gaps surfaced by
live use of the KB linking sweep.

### Fixed

- **Already-friendly identifiers now link too.** `ReferenceCatalog`
  was keyed only by `class_name.toLowerCase()`, but the server
  emits a mix of raw class_names and friendly display names in
  `TraceEntry.city/planet/system` (e.g. `CRU_LEO` versus
  `New Babbage`). `<EntityLink>` rendered the right text via its
  label fallback but failed the catalogue lookup → no slug, no
  link, no hover card. `getCategoryBundle` now stores each entry
  under both `class_name.toLowerCase()` AND
  `display_name.toLowerCase()` keys (web + tray), so the same map
  resolves whether the caller hands in a raw or a friendly
  identifier.
- **Tray prettifier catches more entities.** `prettifySummaryReact`
  added a second matching pass that scans the catalogue for
  word-bounded `display_name` occurrences in the summary string,
  complementing the existing class_name regex. Hits from both
  passes feed a unified overlap-resolver (longer match wins) so
  surfaces showing "Bought items at New Babbage" now link
  `New Babbage` even though it has no underscore. A short stopword
  set (`the`, `pu`, `cig`, `rsi`, ...) plus a 3-char minimum keeps
  ordinary English from false-matching.
- **KB listing duplicates after dual-keying.** Side effect of the
  dual-keyed Map: `Array.from(catalog.values())` and
  `catalog.size` now yield 2× the entries. The KB category page +
  landing tile counts regressed. Added a canonical
  `CategoryBundle.list` (one entry per class_name) and a
  `loadAllReferenceBundles().counts.{cat}` record; consumers that
  iterate or count must use those rather than the Map's
  `.values()` / `.size`.

### Changed

- **Web app `connect-src` already loose; tray CSP belt-and-suspenders.**
  Tray `tauri.conf.json` CSP relaxed to `connect-src 'self' https:`
  alongside the v1.7.0 Rust-command HTTP relay — keeps any future
  renderer-side fetches that legitimately need the paired API
  origin unblocked even before they're routed through Rust.

### Internal

- **`cargo fmt` cleanup.** `get_reference_category`'s `matches!()`
  arm exceeded the 100-char default and was breaking the `Build &
  Test` CI gate. Reformatted; gate now green end-to-end for the
  first time this release line.

## [1.8.0] - 2026-05-22

KB linking sweep across the web app's entity-display surfaces and a
tray-side equivalent. Every place that previously rendered a raw
class identifier (`AEGS_Avenger_Stalker`, `CRU_LEO`, `STANTON2B`)
now resolves to the catalogue display name, links to
`/kb/{category}/{slug}`, and surfaces the EntityHoverCard popover
(web) or opens the KB page in the user's default browser via the
shell plugin (tray).

### Changed

- **Web — location entity displays now link to KB.** Threaded
  `<EntityLink>` through every location-rendering surface:
  - `LocationPill` (dashboard "you are here" pill) — headline
    links and hovers when the catalog has the city/planet/system
    entry.
  - `LocationHero` (journey hero) — headline + "came from" trail
    + final-arrow destination all link independently.
  - `LocationChainStrip` / `ChainNode` (dashboard "Recent stops"
    + future re-uses) — chip label and sublabel each link to the
    right precedence (city → planet → system).
  - `LocationTimeline` / `TimelineRow` / `SystemChangeMarker`
    (journey "Recent journey · last 24h") — stop label,
    sublabel, and inline system-change boundary endpoints all
    link.
  - `LocationChip` (topbar) — the compact chip in the app shell
    now navigates on click + hover. Layout-level
    `getCategoryBundle('location')` is fetched alongside the
    existing topbar calls and threaded through; per-page
    duplicate fetches are now no-ops against the same server-side
    cache.
  Each component accepts an OPTIONAL catalog prop —
  `EntityLink`'s plain-text fallback applies when the catalog is
  missing, so SSR paths without the fetch still render correctly.
- **Tray — friendly identifiers in Status / Logs are clickable.**
  New `<TrayEntityLink>` (`apps/tray-ui/src/components/kb/`)
  renders as a button that opens
  `${webOrigin}/kb/{category}/{slug}` via
  `@tauri-apps/plugin-shell`'s `open()` — the WebView's CSP
  blocks anchor navigation, so the tray uses the shell-plugin
  pattern that `KbPane` and `StatusPane` already follow. Companion
  ReactNode helpers in `components/tray/format-react.tsx`
  (`prettifySummaryReact`, `humanTitleForEntryReact`) walk the
  same class-name regex as the legacy string prettifier and wrap
  every match in a `TrayEntityLink`. `StatusPane` and `LogsPane`
  swap to the ReactNode variant when `bundles` + `webOrigin` are
  both present; the legacy string path remains for tests / paired
  surfaces without the catalogue. New `findEntityInBundles()`
  helper in `lib/reference.ts` discovers the right category from
  a flat class_name (the tray's `prettyLookup` is category-less
  by design).

## [1.7.1] - 2026-05-22

### Fixed

- **SpiceDB public-access checks no longer 500.** `rsi_profile_routes`
  and `rsi_org_routes` were still calling `CheckPermission` against
  a wildcard `user:*` subject — the same SpiceDB constraint that
  bit `/v1/me/visibility` in v1.5.4. Replaced with a
  `ReadRelationships` probe (matching the `PublicAccessChecker`
  pattern). Stops the "spicedb public profile check failed" /
  "spicedb public rsi-orgs check failed" warnings flooding the API
  logs on every public-profile + public-orgs render.

## [1.7.0] - 2026-05-22

Same-day follow-up to v1.6.0 — fixes the three rough edges users
hit immediately after the KB v1 cut.

### Fixed

- **Tray: friendly-name conversion in event summaries.** The
  reference catalogue fetcher in `apps/tray-ui/src/lib/reference.ts`
  was using a renderer-side `fetch()`, which the WebView's CSP
  silently blocks (`connect-src 'self'`). The empty bundle left
  `prettyLookup` empty and Status / Logs rendered raw class
  identifiers like `AEGS_Avenger_Stalker` instead of "Aegis Avenger
  Stalker". Routed the fetch through a new Rust Tauri command
  (`get_reference_category`) so it runs in the client process —
  matches the architectural pattern of every other tray data path.
  CSP was also loosened to `connect-src 'self' https:` as
  belt-and-suspenders for any future renderer-side fetches.
- **Tray: timeline `prettyLookup` plumbing.** Threaded the lookup
  prop through `Timeline` → `EntitySection` / `ChronologicalView`
  → `CollapsedGroupRow`. Currently dead code (Timeline isn't mounted
  in a pane yet), but the existing call sites would have surfaced
  raw class names as soon as that wiring lands — fixing
  preventatively avoids a same-shape regression.
- **Web: KB category cards now link properly.** The `<Link>` around
  each card wraps an `<article>`, which is an ARIA landmark — and
  the accessible-name algorithm stops at landmark boundaries. The
  link reported as nameless to screen readers AND to
  `getByRole('link', { name: ... })` assertions, manifesting as
  cards that looked static even though they were clickable. Added
  `aria-label={display_name}` on the Link to give it a proper
  accname.
- **Web: KB reference fetch cache.** `getCategoryBundle` /
  `getEntityDetail` no longer use a flat `revalidate: 3600`; the
  cache directive is now env-gated via `STARSTATS_DISABLE_FETCH_CACHE`
  so Playwright e2e scenarios don't leak entries between tests.
  Production still gets the 1h revalidate (wiki sync is daily).

### Changed

- **Tray: cross-tab remote-config propagation (from v1.6.0).** The
  `config-changed` listener moved up to the App shell, so remote
  preference downloads now update the document theme + StatusPane
  `web_origin` regardless of active tab.

## [1.6.0] - 2026-05-21

Promotes `1.6.0-beta.1` to the live channel after soak. See the
[1.6.0-beta.1] section below for the full KB v1 + tray Settings fix
scope. Post-beta additions:

### Fixed

- **Tray: cross-tab remote-config propagation.** The bulk-lane
  `config-changed` event listener moved from `SettingsPane` to the
  `App` shell so remote pref downloads update the document theme and
  the StatusPane `web_origin` regardless of which tab is foregrounded
  when the event arrives. `SettingsPane` now reacts to its `config`
  prop via a `useEffect`; the "Reload / Keep my changes" guard against
  clobbering an unsaved draft is preserved end-to-end.
- **KB:** `'use client'` removed from `event-summary-react` and the
  detail-spec selectors tightened to avoid bleed between adjacent
  metadata blocks; `reference.ts` split into client-safe types +
  server-only fetchers so client bundles no longer transitively
  import `server-only`.

### Changed

- **Deps:** npm minor + patch bumps across the web app (Dependabot
  group).

## [1.6.0-beta.1] - 2026-05-21

### Added

- **Knowledge base (KB v1).** Browseable, searchable catalogue over
  the wiki-synced `reference_registry` (vehicles, weapons, items,
  locations).
  - **Server:** migration `0038_reference_registry_slug.sql` adds a
    nullable `slug` column + partial lookup index, with inline
    backfill for locations from `metadata.slug`. `derive_slug` +
    `apply_slug_collisions` (per-category scoped, internal sort by
    `class_name` for cross-run stability) populate slugs at wiki-sync
    time; collisions resolve to `-2`, `-3` deterministically. A
    partial `UNIQUE(category, lower(slug))` index enforces the
    invariant at the DB layer. New per-category `Summary` projection
    (`build_summary`) keeps the listing payload small. New
    `GET /v1/reference/{category}/slug/{slug}` endpoint returns a
    new `ReferenceEntryDetail` shape — slim listing entry PLUS the
    computed `summary` projection AND the full `metadata` blob, so
    the web detail page can render the at-a-glance chip strip and
    the metadata table without crashing on a missing field. Listing
    response shape narrowed to `ReferenceListEntry` (drops the full
    `metadata` blob — saves multi-MB on vehicles). The upsert uses
    `COALESCE(EXCLUDED.slug, …)` on conflict so legacy ingest paths
    can't accidentally null out a backfilled slug. 58 reference
    tests; clippy + fmt clean.
  - **Web:** new `/kb`, `/kb/{category}`, `/kb/{category}/{slug}`
    routes with paged browse + facet chips + detail page.
    `<EntityLink>` + `<EntityHoverCard>` client components surface
    KB entries inline on the dashboard and journey timeline via a
    new `renderEventSummary` ReactNode renderer; the string-returning
    `formatEventSummary` is kept for clipboard / share / OG-card
    paths. "Knowledge base" entry added to the LeftRail nav.
    `LocationCatalog` migrated to read from `summary` (the listing
    no longer ships `metadata`). `getEntityDetail` returns a
    discriminated outcome (`ok` / `not_found` / `error`) so a
    transient backend failure renders the Next error boundary
    rather than a misleading permanent 404. Multi-endpoint loads
    use `Promise.allSettled` per project convention; non-2xx and
    JSON-parse failures log via `console.error` instead of being
    silently swallowed.
  - **Tray:** new `Catalogue` tab in the tray header opens a
    `KbPane` browse + search view over the four categories. Clicks
    open the canonical detail page on the web app via
    `@tauri-apps/plugin-shell` — the tray window is too compact
    for a useful in-app detail surface, and re-using the web
    avoids duplicating the rendering work. The Readout
    (`StatusPane`) + Manifest (`LogsPane`) timelines now run their
    Rust-formatted summary strings through a catalog-driven
    `prettifySummary` so raw class identifiers like
    `AEGS_Avenger_Stalker` render as `Aegis Avenger Stalker`
    inline. The lookup is built once per session at the App level
    from the four category catalogues and threaded through both
    views; falls back to the raw string when unpaired or the
    catalog hasn't loaded yet.

### Follow-ups (KB v1.1)

- **Slug URL canonicalization.** `/v1/reference/{category}/slug/{SLUG}`
  with mixed-case input now 301-redirects to the canonical
  lowercase URL so bookmarks + crawlers converge on one path.
- **`by-class` 308 redirect endpoint** at
  `/v1/reference/{category}/by-class/{class_name}` resolves to the
  canonical `/slug/{slug}` URL. Bookmark-survival across wiki
  display-name renames, and a clean affordance for callers that
  only have the class_name (the engine's stable join key).
  Legacy rows with no slug yet fall through to a direct
  `ReferenceEntryDetail` response.
- **Typed `Summary` structs.** Server-side `summary` is now an
  internally-tagged enum (`Summary::Vehicle(...)`, `::Weapon(...)`,
  etc.) instead of `serde_json::Map<String, Value>`. OpenAPI →
  TS client now ships a proper discriminated union; the web
  + tray browse surfaces narrow on `summary.category` for
  type-safe field access. Plan deviation closed.
- **Journey rollup labels are clickable.** `HierarchicalBucketList`
  leaf rows that aggregate exactly one wiki-known class now
  render their label as a `<Link>` to `/kb/{category}/{slug}` —
  combat top-weapons, loadout top-items, and travel locations
  all participate. Aggregate-of-multiple buckets stay as plain
  text so click semantics match the data.

### Changed

- **Tray Settings — Remote sync card.** Restored a live OK / ERR /
  IDLE / OFF health pill in the card header, driven off `SyncStats`
  via the parent's polling hook (which is now active on the
  Settings view as well as Status). The previously-hardcoded green
  dot next to "Paired as <handle>" now reflects real sync state.
- **Tray Settings — Remote sync card.** Renamed the inner
  `Field label="Hangar"` to `"Pairing"` and the helper text from
  `(Hangar → Pair a desktop client)` to
  `(Connected Uplinks → Pair this tray)`. The label was for the
  device-pair-to-StarStats-site flow, not the in-game ship Hangar
  — the rename matches the actual web page title and removes the
  SC-noun-as-brand-term conflict.

### Fixed

- (nothing yet)

### Security

- (nothing yet)

### Deferred

- **KB comparison UI** (pin 2-4 entries side-by-side). Deferred to
  v2 per the original scope decision.

## [1.5.4] - 2026-05-21

### Fixed

- **Public-profile toggle on `/sharing` now reflects reality.**
  SpiceDB rejects `CheckPermission` against a wildcard subject
  with `InvalidArgument` ("cannot perform check on wildcard
  subject"), so `GET /v1/me/visibility` was 500ing on every
  render and the toggle UI silently rendered as "Private" no
  matter what was persisted — even though the underlying
  `WriteRelationships` was landing. Replaced the wildcard
  `CheckPermission` with a `ReadRelationships` probe of the
  `stats_record:<handle>#public_view@user:*` tuple (limit 1,
  fully-consistent). Semantically tighter — the toggle's truth
  is "does this exact tuple exist?", not "would `view` resolve
  for somebody".
- **Discover listing no longer 500s.** Same SpiceDB constraint
  bit `LookupResources` on `user:*`; `/v1/discover/profiles`
  was rejected with "cannot perform lookup resources on
  wildcard". `SpicedbClient::lookup_public_profiles` is now
  `list_public_profile_handles`, streaming
  `stats_record:*#public_view@user:*` rows via
  `ReadRelationships` and returning the resource ids. Same
  return shape, same caller semantics — the SQL intersection
  in `discover_routes` is unchanged.
- **`visibilityAction` no longer gaslights the user.** The
  success-chip status now derives from the response's `public`
  field instead of the user's intent. A backend that 200-OKs a
  no-op (the exact failure mode above) used to produce a
  "Profile is now public." chip on a page that still said
  "Private"; the chip will now reflect the actual server state.

## [1.5.3] - 2026-05-21

### Fixed

- **`listTransactions` `window_secs` arg is now honoured.** The
  TS invoke wrapper was sending `windowSecs` (camelCase) but the
  Rust command parameter is `window_secs` (snake_case). Tauri's
  serde binds by exact name, so the value was silently dropped
  and the Rust-side default (30s) was always used. The single
  in-tree caller (`TransactionsCard`) already passed 30, so no
  user-visible behaviour change today — but future tweaks to
  the timeout window will now actually take effect. Surfaced
  during the fact-check review of PR #69.



Tray Logs view: server-side event search + cursor pagination, plus a
drawer-positioning fix. The previous "fetch 1000 + filter client-side"
pattern hid records beyond the most recent window from the search box;
the detail drawer was also pinned to a transformed ancestor instead of
the viewport, sliding out of view when the events list scrolled.

### Added

- **Server-side event search across all rows.** New `search_events`
  Tauri command runs a case-insensitive substring match against
  `type` and the parsed `payload` JSON, with an optional exact
  `type_filter` and an `id < before_id` cursor for pagination.
  Page size 200, capped by the existing `MAX_TIMELINE_LIMIT = 5000`
  IPC ceiling. Backed by `Storage::search_events_paged` and
  `Storage::count_matching_events`. The query input debounces to
  250 ms before firing a fresh search.
- **"Load more" cursor pagination in the Logs view.** Passes the
  smallest loaded `id` as the next page's `before_id`. Robust to
  inserts during browsing.

### Fixed

- **Logs detail drawer no longer slides out of view on scroll.**
  The pane is wrapped at App level in `<div className="ss-screen-enter">`,
  whose CSS animation applies a `transform` and therefore creates
  a containing block for `position: fixed` descendants. The
  scrim+drawer is now portaled to `document.body` via
  `react-dom`'s `createPortal`, escaping that containing block and
  pinning correctly to the viewport.

### Changed

- **Logs view stops the 10-second timeline auto-refresh.** Storage
  stats (Stored / Synced / Pending / DB size / Quarantined) still
  tick — search results do not, to avoid jerking the list mid-
  browse. Mutations (mark-as-noise, retry-sync) re-run the active
  search explicitly so the affected row updates without scroll.
- **Type-pill counts in the Logs view are now label-only.**
  Accurate per-type counts across the whole DB would require N
  extra round trips per render; dropping the numeric label is the
  honest answer until we materialise per-type aggregates.

## [1.5.1] - 2026-05-21

First stable cut on the v1.5 line. Rolls up the v1.5.0 beta accumulation
(soaked through `v1.5.0-beta.1` and `v1.5.0-beta.2`) plus the new
channel-mismatch banner. Live-channel users upgrading from `1.3.2` will
receive the full v1.5 surface in one step. The intermediate `v1.5.0`
stable tag was never cut — the version line jumps straight to `1.5.1`
to carry this PR's banner work alongside the soaked beta features.

### Added

- **Updates card surfaces the running channel.** The kicker on the
  tray Settings → Updates card now reads `{channel} · v{version}`
  (e.g. `beta · v1.5.1`), derived from a new
  `get_build_release_channel` Tauri command that parses the running
  binary's channel from `CARGO_PKG_VERSION`. Previously the kicker
  showed only the version, leaving the running-channel implicit and
  divergeable from the user's configured `release_channel`.
- **Channel-mismatch banner.** When the running build's channel
  differs from the user's configured `release_channel`, a dismissible
  banner appears at the top of the Updates card with two actions:
  *Switch to {build_channel}* (updates `config.release_channel`) and
  *Dismiss* (writes the build-channel token into the new
  `Config.channel_mismatch_ack` field). Dismissal is per-build-channel:
  the banner re-appears only if the user later upgrades into a
  different channel's build.
- **Cloud sync for tray preferences** (soaked in `v1.5.0-beta.{1,2}`).
  Per-device opt-in via the two-gate model (local `Config.sync_with_cloud`
  + server-side `devices.sync_enabled`).
- **Records widget on the web profile** (soaked in `v1.5.0-beta.{1,2}`).
- **Marketing `/features` page** (soaked in `v1.5.0-beta.{1,2}`).

### Changed

- `ReleaseChannel::from_version` (introduced for the channel detection
  affordance) now has a production caller via
  `commands::get_build_release_channel`. The long-standing TODO above
  it is cleared.

## [1.3.2] - 2026-05-19

Patch release. Fixes a UI/observability bug where the LogsPane stat
strip showed events as permanently Pending even though sync was
actually working on the wire, and adds a recovery affordance for
the v1.3.1 poison-pill quarantine. Render + storage-query only; no
wire-format, schema, or behaviour changes.

### Fixed

- **LogsPane `Synced` / `Pending` counts reflect reality again.**
  The priority-lanes refactor in v1.3.0 (`1e8f0e4`) replaced the
  global `sync_cursor` model with per-row `events.sent_at` stamps.
  The drain query and `mark_sent` were correctly switched over, but
  two timeline projections (`get_session_timeline`,
  `get_session_summary_text`) kept computing `synced` against the
  legacy cursor. Since `write_sync_cursor` had zero callers
  post-refactor, the cursor sat frozen at its pre-v1.3.0 value, and
  every event with `id > frozen_cursor` rendered as "Pending"
  regardless of whether the API server had actually accepted it.
  Both projections now derive `synced` from `sent_at`: a bare
  datetime means accepted by the server, NULL means still pending,
  `__quarantined_*` means client-side shelved (surfaces as Pending +
  pairs with the quarantine recovery banner). The dead
  `read_sync_cursor` / `write_sync_cursor` methods are removed.

### Added

- **Quarantine recovery affordance in LogsPane.** A "Quarantined"
  StatPill appears in the stat strip when the storage has
  poison-pill-shelved rows (hidden at zero). Below the strip, a
  recovery banner surfaces the count with a "Release" button that
  flips every `sent_at = __quarantined_*` row back to NULL and kicks
  the sync worker so the next drain re-attempts them. New Tauri
  commands: `count_quarantined` (DB count for the UI) and
  `release_quarantined` (releases + returns count).
- **`Storage::release_quarantined`** plus two unit tests covering
  the flip + idempotent-on-empty cases.

### Changed

- **Bisection re-quarantine is still capped** (`MAX_QUARANTINES_PER_DRAIN`
  unchanged at 10). Releasing without addressing the underlying
  server-side rejection cause will see events climb back into
  quarantine — by design.

## [1.3.1] - 2026-05-19

Patch release. Web theme switching fix (the same CSS-specificity tie
that hit the tray in v1.3.0 was lurking in the parallel `apps/web`
token system), plus a defensive-engineering pass on the tray sync
worker: two auth-loss propagation gaps closed (heartbeat + hangar
fetcher), poison-pill isolation added so a single 4xx-rejected event
can't block the rest of the queue indefinitely, and INFO-level
diagnostic logging on every drain iteration so silent stalls are
visible in the daily log. No wire format, schema, or behaviour
changes.

### Added

- **Poison-pill isolation in tray sync.** When `POST /v1/ingest`
  returns a 4xx other than 401/403, `drain_lane` now bisects the
  failing batch until it isolates the single offending event, then
  stamps it with a `__quarantined_<timestamp>` sentinel on
  `sent_at` so the rest of the queue can drain. Safety cap of 10
  quarantines per drain protects against schema-version skew or
  similar batch-wide failures from mass-quarantining the queue. New
  `SyncStats.events_quarantined` counter plus
  `storage::mark_quarantined()` + `count_quarantined()` helpers. 5xx
  / network errors and response-parse failures are still treated as
  transient — only true client-error rejections trigger bisection.
- **Diagnostic INFO logging on every sync drain iteration.** Each
  iteration emits `drain: starting lane=… pending=N` and
  `drain: batch shipped lane=… sent=N accepted=A duplicate=D
  rejected=R quarantined=Q`. Closes the silent-stall blind spot
  where a running-but-idle worker looked identical to a dead worker
  in the log.
- **Expanded `sync: spawned fresh worker(s)` log fields.**
  `has_access_token`, `has_claimed_handle`, `bulk_running` are now
  emitted on every respawn (and on the `config incomplete or
  disabled; no worker running` branch) so a missing config field
  names itself directly instead of requiring a debugger.

### Fixed

- **Web theme switching now actually applies.** Same CSS-specificity
  bug as the v1.3.0 tray fix, in the parallel web token system. The
  four `[data-theme="..."]` blocks in
  `apps/web/src/styles/starstats-tokens.css` had specificity
  `(0,1,0)` — identical to the defensive `:root` fallback in
  `apps/web/src/app/globals.css`. Because the fallback loads after
  the theme blocks (via `@import` at the top of `globals.css`),
  source order made the Stanton fallback clobber every theme
  override regardless of which theme was selected in
  Settings → Appearance. Scoped theme blocks to
  `:root[data-theme="..."]` (specificity `(0,2,0)`) so they
  correctly override the fallback. The `data-theme` cookie + SSR
  write in `lib/theme.ts` and `app/layout.tsx` were already correct.
- **Tray sync auth-loss now propagates from the idle heartbeat.**
  The bulk lane's heartbeat (`GET /v1/auth/me` on empty-queue
  iterations) used to detect a 401/403 (returning `Ok(None)`)
  without flipping `account_status.auth_lost`. That left a window
  where a revoked device token was detected by the heartbeat but
  the UI banner and other auth-aware workers (sync drain, hangar)
  didn't react until the next `/v1/ingest` happened to return 401 —
  observed in production at 2026-05-19 01:59Z, ~6 hours before the
  hangar push surfaced the same revocation. Now the heartbeat
  clears the persisted token and flips `auth_lost` immediately,
  same path as the ingest 401 handler.
- **Hangar fetcher 401/403 now flips `auth_lost`.** Previously a
  `401 token rejected: device revoked` from `POST /v1/me/hangar`
  was logged-and-continued; the device token stayed in config and
  other auth-aware code paths kept retrying with a dead credential.
  Now `refresh_once` distinguishes 401/403 from other failures and
  reuses `sync::clear_persisted_device_token()` (promoted to
  `pub(crate)`) so every auth-aware path invalidates state the same
  way.

## [1.3.0] - 2026-05-19

Minor release. Ships an entity-browser drawer for the tray timeline
and corrects a CSS specificity tie that prevented the theme picker
from actually applying. Render + UX only — no wire-format, schema,
or behaviour changes.

### Added

- **EntityDrawer — browse and filter the timeline by entity.** A
  slide-in panel mounted over the timeline content, grouping every
  entity (vehicle, player, item, location, shop, mission, system,
  session) by kind in canonical order with a search box, event
  counts, and one-click filter. Selecting an entity narrows both
  the by-entity sections and the chronological view to events whose
  `metadata.primary_entity` matches; a live banner shows the active
  filter with a Clear control. Drawer + filter state are
  component-local — view-mode persistence is unchanged.
- **Dialog accessibility on the EntityDrawer.** Escape closes the
  drawer, the search input auto-focuses on open, focus is restored
  to the previously focused element on close, and Tab/Shift+Tab wrap
  within the panel (focus trap). `aria-modal="true"` added on the
  dialog for screen-reader correctness.

### Fixed

- **Theme switching now actually applies.** The four
  `[data-theme="..."]` blocks in `starstats-tokens.css` had
  specificity `(0,1,0)` — identical to the defensive `:root`
  fallback in `styles.css`. Because the fallback was imported AFTER
  the theme blocks, source order made the Stanton fallback clobber
  every theme override regardless of which theme was selected in
  Settings → Appearance. Scoped theme blocks to
  `:root[data-theme="..."]` (specificity `(0,2,0)`) so they
  correctly override the fallback. No JS or DOM changes — the
  `dataset.theme` writes were already correct.

### Changed

- **Test queries updated for the v1.2.1 row refactor.** Four
  Timeline tests and one EntitySection test had stale
  `getByText('Cutlass')` assertions that broke after
  `humanTitleForEnvelope` started surfacing entity `display_name`
  in row bodies. Switched to role-based queries
  (`getByRole('button', { expanded: true, name: /.../i })`) that
  scope to the by-entity section toggle and survive future
  render-layer refactors. No production-code changes.

## [1.2.1] - 2026-05-19

Patch release. UX polish — demoting the raw `event_type` snake_case
discriminant from headline content across the tray and web surfaces.
Render-only; no wire-format, schema, or behaviour changes.

### Changed

- **Tray timeline rows lead with the human summary.** `StatusPane`
  and `LogsPane` swap their grid layouts so the server-formatted
  `summary` (or the event-type's verb-phrase fallback) becomes the
  wide primary column, with `event_type` collapsed into a small pill
  chip beside the sync indicator. The tone-coloured row accent moves
  to a 2px left border so at-a-glance scanning still works. New
  helpers in `tray/format.ts`: `humanTitleForEntry` (TimelineEntry
  shape) and `humanTitleForEnvelope` (full `EventEnvelope` shape
  with `metadata.primary_entity.display_name` preference).
- **`CollapsedGroupRow` (tray) anchors on `display_name`.** Anchor
  and member-row titles route through `humanTitleForEnvelope`, so a
  death row reads as the killed entity rather than `player_death`.
- **Web `rowTitleForEnvelope` falls through `formatEventType`.**
  The fallback when `metadata.primary_entity` is missing now
  resolves to the verb-first label from `lib/event-types.ts` instead
  of the raw `event.type` discriminant.
- **Sharing scope preview list uses the registry label + glyph.**
  `/sharing/preview` per-type counts render `formatEventType().label`
  with the type glyph; the raw key stays addressable via the
  `title` tooltip for forensic use.

Filter / scope-form surfaces (sharing `scope_allow_event_types`
inputs, API query params, frequency charts) intentionally keep
`event_type` as the primary key — those are machine-addressable
dimensions, not headlines.

### Notes for release pipeline

- Release-manifest platform URLs and minisign signatures are NOT
  touched by this version bump. The release workflow overwrites
  those on the `v1.2.1` tag push when artifacts upload.

## [1.2.0] - 2026-05-18

Minor release. New cross-session entity rollups, a public-profile
Discover surface, profile-views tracking, the StarStats brand pack,
deprecation of the self-hosting positioning, and a Timeline filter
that hides self-explanatory movement events.

### Added

- **Cross-session entity rollups.**
  `/v1/users/{handle}/entities` and
  `/v1/users/{handle}/entities/{kind}/{id}` aggregate per-entity
  history across sessions, gated on the existing
  `share_event_timeline` grant. Web routes at `/u/[handle]/entities`
  surface the rollups with per-session breakdown.
- **Discover surface.** `/v1/discover/profiles` lists opt-in public
  profiles; new `/discover` web route + Discover top-bar nav entry
  surfaces them. Per-profile opt-out via a `listing_opt_out` toggle
  on `/sharing`.
- **Profile-views tracking.** `public_profile_view_counters` table
  with traffic-source attribution; `GET /v1/me/profile-views`
  returns aggregates. Surfaced on `/sharing` as a profile-views
  card.
- **Brand pack** (#34). About page (`docs/About.html`), 8 social
  PNGs in `social/` with the fan-made attribution lockup baked in,
  8 SVG + 12 raster logo files in `assets/logo/`, brand-book
  reference sources extracted into `design_handoff_starstats/`.
- **Timeline movement-event filter** (#39). The web app and tray
  Timeline hide five self-explanatory movement events from view
  (`join_pu`, `change_server`, `quantum_target_selected`,
  `seed_solar_system`, `resolve_spawn`). Render-layer filter only —
  events remain in the store.

### Changed

- **Self-hosting positioning deprecated** (#35, #36). User-facing
  references to "self-hosted" removed from README, landing page,
  privacy page, ARCHITECTURE.md, SECURITY.md, and GitHub repo
  metadata. Replaced with "local-first" framing throughout. Rust
  doc-comments rephrased "self-hosted JWT" → "first-party JWT".
  Infrastructure (`infra/` docker-compose, admin console, JWT auth)
  unchanged — the hosted instance still runs off it.

### Fixed

- **Session-bounds query unbounded scan**
  (`fix(server)`). Capped to prevent runaway latency on large
  session lists.
- **Entities list crash on rows with missing metadata column**
  (`fix(server)`). Graceful fallback when the JSONB column is null.
- **Forbidden sentinel inconsistency across entity pages**
  (`fix(web)`). Unified to match the pattern from `/sharing` views.
- **After-cursor empty-string handling in api.ts helpers**
  (`fix(web)`). Consistent guard for `opts.after === ''` across
  paginated helpers.
- **Public-profile disable did not persist** (`fix(profile)`).
  Toggle now correctly clears the opt-in state on the server.
- **`migrations/0004_users.sql` comment-only edit broke sqlx hash
  verification** (#37 hotfix). Reverted; the residual "self-hosted"
  string in that one historical migration comment is intentional —
  shipped migrations are byte-immutable per the
  "Migrations are additive only" rule.

### Security

- **GHSA-jxxr-4gwj-5jf2 / CVE-2026-45149 — brace-expansion DoS**
  (#38, Dependabot #80). Bumped via `pnpm.overrides` to ≥5.0.6 for
  the vulnerable 5.0.0–5.0.5 range. Exploitability low (transitive
  build-tool dep, not reachable from user input).

## [1.1.0] - 2026-05-18

Minor release. All additive: priority-lane sync, per-session event
timeline endpoints + UI, the `share_event_timeline` grant toggle,
events.metadata persisted to JSONB, and inference-window wiring in
both the live tail and the rotated-log backfill.

### Added

- **Priority sync lanes.** Two-worker split — fast lane drains a
  curated urgent set (location_changed, player_death, actor_death,
  vehicle_destruction, quantum_target_selected, session_end) every
  ~5s while bulk continues on the 60s cadence. Settings exposes a
  four-card preset picker (Fast / Balanced / Resource-saver / Custom)
  plus a dedicated Priority interval input next to the existing bulk
  fields. New Tauri command `set_sync_preset`.
- **Per-session event timeline endpoints.** `GET
  /v1/users/{handle}/sessions` and `GET
  /v1/users/{handle}/sessions/{session_id}/events`. Owner is always
  allowed; anyone else needs an active share-metadata row with
  `share_event_timeline = TRUE` and an unexpired `expires_at`. Sessions
  derive from a 30-minute idle gap between adjacent events; sessions
  themselves are not persisted.
- **`share_event_timeline` grant toggle.** Per-share boolean alongside
  the existing summary / transactions / avatar toggles. Owners can
  selectively expose the per-event drill-down without unmasking the
  rest, or vice versa.
- **`events.metadata JSONB` column.** Migration 0030 lands the column
  + three partial indexes (`group_key`, `(primary_entity.kind, .id)`,
  `source = 'inferred'`). `PostgresStore::insert` now writes the
  column on every row; the existing server-side back-fill of legacy
  v1 envelopes means no historic row gets a NULL metadata cell going
  forward.
- **Inference engine wiring on the desktop client.** Per-task
  `InferenceWindow` (capacity 50) threads through `start_tail` and
  `backfill_file` so inference rules fire across drain boundaries.
  Inferred events ship upstream alongside observed rows under stable
  `inferred:rule_id:trigger_key` UUIDv5 idempotency keys. Gated on
  `parser_enable_v2_metadata` — a flag-off install behaves exactly
  like pre-1.1.
- **Web session timeline UI.** `/u/[handle]` gets a Sessions section;
  `/u/[handle]/sessions/[session_id]` renders the per-session event
  list. Playwright e2e (`session-timeline.spec.ts`) walks the happy
  path so the route's hydration stays covered.

### Changed

- **Release-channel default is Live regardless of build version.**
  Pre-release binaries no longer implicitly opt users into pre-release
  update channels; the Settings dropdown remains the explicit opt-in
  path. Persisted `release_channel` in `config.toml` still wins.
- **Release-channel dropdown harmonised with `TextInput`.** Same
  `INPUT_BASE` contract (`var(--bg)`, `var(--r-sm)`,
  `var(--font-mono)`, `7px 9px` padding) so the closed-state control
  reads as part of the same family on the Updates card. Native
  options-popup chrome stays browser-controlled.
- **Client storage: per-row `sent_at` flag replaces `sync_cursor`.**
  Priority and bulk lanes can drain independently because the unsent
  set is no longer a single high-water mark. Partial index keeps the
  scan O(unsent). Legacy installs are back-stamped on first boot
  using the old cursor as the cutoff.

### Fixed

- **`capture_v2_unknown` records the real channel.** Was hardcoded to
  `LogSource::Other`; now threads through the tail's `log_source_enum`
  so the review queue is correctly attributed to Live / PTU / EPTU /
  Hotfix / Tech rather than collapsing every channel to Other.

### Notes for release pipeline

- Release-manifest platform URLs and minisign signatures are NOT
  touched by this version bump. The release workflow overwrites those
  on the next `vX.Y.Z` tag push when artifacts upload.

## [1.0.0] - 2026-05-18

First stable release. Drops the pre-1.0 zero-major.

### Added

- **EventMetadata envelope on every event.** `primary_entity` (kind + id +
  display_name), `source` (Observed / Inferred / Synthesized), `confidence`,
  `group_key`, `field_provenance`, `inference_inputs`, `rule_id`. Stamped
  server-side on every parsed event so the wire format carries a canonical
  "impacted entity" facet and per-field provenance.
- **Entity-first tray timeline.** New By-Entity default view with a
  Chronological toggle. Adjacent same-`group_key` events collapse into a
  single `<title> ×N` row with inline drill-in. Raw events stay in
  storage — collapse is a render decision only.
- **Declarative inference engine.** Pure post-classify pass over a sliding
  event window. Ships with three built-in rules: implicit death after
  vehicle destruction, implicit location change, implicit shop-request
  timeout. Remote inference rules hot-reload via the parser-definitions
  manifest. Off by default behind `parser.enable_v2_metadata`.
- **Unknown-line capture + review pane.** Tray-side shape normalisation,
  interest scoring, PII detection, per-token redaction toggle, and a
  review UI that submits curated samples to `POST /v1/parser-submissions`
  (UPSERT on `(shape_hash, client_anon_id)`).
- **System auto-start on Windows / Linux / macOS** via
  `tauri-plugin-autostart`. Default ON; settings pane exposes the toggle.
- **Stable per-install `client_anon_id`** generated server-side in a Tauri
  command so parser submissions and inference telemetry can deduplicate
  without leaking identity.
- **`IngestBatch` schema_version bumped to 2.** Additive: v1 clients still
  accepted; server synthesises `EventMetadata` for legacy payloads so
  rollout is forward-compatible.

### Changed

- **`GameEventSchema` is now a discriminated union in `api-client-ts`.**
  No more opaque `Record<string, never>` casts at consumers.
- **Validator enforces `EventMetadata` invariants.** Confidence range,
  Observed/Inferred consistency, presence of `inference_inputs` for
  inferred events.
- **Zone-enrichment populates `metadata.field_provenance.zone`** correctly
  on `PlayerDeath` and friends when the zone was inferred rather than
  observed in the raw line.
- **Tray "Open on web" no longer opens the API subdomain.** Resolution
  moved Rust-side via `Config::effective_web_origin()` — strips a leading
  `api.` from the API host. Self-hosted users on a non-`api.` host can
  set `web_origin` explicitly to override.

### Internal

- Inference engine sliding-window default is 200 events; tunable in
  `parser` config.
- New `POST /v1/parser-submissions` endpoint with idempotent UPSERT on
  `(shape_hash, client_anon_id)`.

### Notes for release pipeline

- Release-manifest platform URLs and minisign signatures are NOT touched
  by this version bump. The release workflow overwrites those on the
  next `vX.Y.Z` tag push when artifacts upload.

## [0.0.7-beta]

### Added

- **Audit v2.1 §B1 — "Preview as @handle".** Owners can simulate a
  recipient's view of their own data through a candidate scope before
  granting the share. New `/v1/me/preview-share/{summary,timeline}`
  endpoints render the owner's own data through the proposed scope
  clamp (no SpiceDB check, no audit row — it's a simulation). New
  `/sharing/preview` server-rendered page with a sticky simulation
  banner and empty states for scope-excluded surfaces.
- **Audit v2.1 §C — per-user sharing context.** New admin sub-tab
  surfaces a user's outbound shares, inbound shares, reports filed,
  and reports filed against them, all in one place. Backed by
  `/v1/admin/sharing/by-user/:handle`.
- **Audit v2.1 §C — abuse-signal detection.** `add_share` now checks
  for a rapid-grant cluster (≥15 grants/24h → 429
  `rate_limited_rapid_grant` + `share.signal_rapid_grant` audit row).
  `report_share` checks for a cross-report cluster (≥3 reports
  against one owner/72h → `share.signal_cluster_pause` audit row).
- **Audit v2.1 §C — auto-pause closure.** When the cross-report
  cluster threshold fires, the owner's `users.shares_paused_until` is
  stamped with a 24h ban. `add_share` gates on it up-front: paused
  owners get 403 `shares_paused` before any recipient lookup or
  SpiceDB write. Migration 0028 lands the column (additive, NULL
  default, partial index).
- **Audit v2.1 Wave A — sharing presets + bulk ops.** Three scope-
  preset chips on the share editor (Friend / Org / Public) and a
  bulk-ops row above the outbound list (revoke-expired,
  reset-scope-on-all-active).
- **Events v2 metadata envelope.** `EventEnvelope` now carries an
  optional `EventMetadata` field (`source`, `entity_refs`,
  `provenance`, `event_type_key`, `group_key`). v1 batches still
  accepted — server synthesises observed metadata for legacy clients
  via `stamp()`. `IngestBatch::CURRENT_SCHEMA_VERSION` bumped to 2.
- **Type-plateau pass (audit v2 §03).** `main h1` baseline set to
  28px in globals.css; top-level pages opt to 32px inline, deep
  detail pages can opt to 24px. HangarCard refresh affordance
  reframed as "Updated via tray · open Devices →" — honest framing
  for the server-holds-zero-credentials architecture.
- **Project automation seed.** `.local-tooling/` tree with two hooks
  (rustfmt-on-edit, protect-migrations), two skills (regen-openapi,
  new-store), and two reviewer agents (migration-safety-reviewer,
  api-contract-reviewer). Project memory tree (docs/ENGINEERING.md +
  memory/MEMORY.md).

### Changed

- **Sharing dashboard load is partial-failure tolerant.** The
  `/sharing` page now uses `Promise.allSettled` across its four
  underlying API calls, logs each rejection with `call=<label>`, and
  surfaces SpiceDB-unavailable as a banner rather than blanking the
  whole render.

### Fixed

- **`/sharing` no longer blanks on a single endpoint hiccup.** The
  previous `Promise.all` race meant any one of `getVisibility`,
  `listShares`, `listSharedWithMe`, or `listOrgs` returning a
  non-2xx wiped the entire page with "Couldn't load your sharing
  state."

## [0.0.5-beta] — 2026-05-16

### Added

- **Tray Health surface.** New "Health" section at the top of Status
  aggregates every actionable setup/lifetime problem into a single
  list — Game.log missing, API URL missing, pairing missing,
  auth-lost, sync failing, hangar skipped, RSI cookie missing while
  paired, email unverified, Game.log stale while SC is running,
  update available, low disk space. Each item gets a per-row CTA
  (Set up / Retry sync / Refresh now / Open). Info- and Warn-severity
  items are dismissible; dismissals re-emerge when the underlying
  params change (fingerprint over the payload, not the id).
- **Inline configuration probes.** New "Test connection" and "Test
  cookie" buttons next to the API URL and RSI session-cookie inputs
  in Settings. Both perform a single 5-second HTTPS round-trip and
  render the result inline — no need to save and wait for the next
  sync cycle to learn if the value works.
- **Friendly error messages.** Tauri command failures in
  SettingsPane and the hangar refresh card are now categorised
  (timeout / connection refused / 401-403 / 404 / 5xx / no cookie /
  unknown) instead of rendered as raw error strings.
- **Vitest test runner** for `apps/tray-ui/` — 31 frontend tests
  cover the new pure modules and components.

### Changed

- The top-of-Status `auth_lost` and `email_unverified` banners are
  replaced by `HealthItem`s in the new Health card with the same
  severities. Per-card inline error displays (cookie save error,
  pair error, hangar refresh error) are retained — the Health card
  aggregates without removing the editing-context affordances.
- HealthCard CTAs that navigate to Settings now focus the relevant
  field via a new `useFieldFocus` registry (one provider at App
  root, ref callbacks on each registered field).

## [0.0.4-beta] — 2026-05-16

Tray design-language polish. The tray UI's tokens and primitives
already mirrored the design system, but the user-visible identity
layer — fonts, theme switching, entrance motion — wasn't wired up.
This release closes those gaps from the design handoff audit.

### Added

- **Tray:** Geist + Geist Mono bundled via `@fontsource-variable`
  so the `--font-sans` / `--font-mono` tokens resolve to the design
  system's signature typeface. Bundled (not CDN) because the Tauri
  CSP is `default-src 'self'`.
- **Tray:** `Theme` enum (Stanton / Pyro / Terra / Nyx) added to
  `Config` with serde defaults and backward-compat parsing (old
  config.toml files without the field load as Stanton).
- **Tray:** Settings → Appearance card with four-swatch picker.
  Eager preview flips `data-theme` on click; Save persists.
- **Tray:** `.ss-screen-enter` wrapper on the active pane fires
  the design system's card-stagger motion on every tab switch;
  `TrayCard` adopts `className="ss-card"` so the mount/hover
  animations engage.

### Changed

- **Tray:** Unverified-Comm-Link banner copy tightened to match
  the audit's in-universe voice ("Comm-Link unverified — claim
  it before someone else can").

## [0.0.3-beta] — 2026-05-12

Tray-UI half of the metrics-display redesign. v0.0.2-beta shipped the
new charts on the web app only; this release brings a tray-native
equivalent so the desktop window also benefits.

### Added

- **Tray:** `EventSparkline` component — 48-hour rolling sparkline
  of events/hour, rendered inline-SVG against the `--accent` token
  (no chart library — keeps the Tauri bundle slim). Lands in
  `StatusPane.tsx` above the existing "Top event types" card under
  the heading "Recent activity · 48h". Consumes the timeline the
  tray already fetches; no new IPC.

### CI

- Workflow split: container/config images now live in a sibling
  `release-images.yml` workflow so a registry-side outage no longer
  marks the tray release as failed. Both workflows trigger on the
  same `v*` tag and can be re-run independently via
  `workflow_dispatch`.
- `Release tray` now detects already-published GitHub Releases and
  skips the asset-upload + draft-promotion steps, so re-runs on
  already-shipped tags no longer fail at "Cannot delete asset from
  an immutable release". The channel-manifest commit step stays
  unguarded so it can still recover a missing manifest.
- Channel-manifest commit step now uses `git add` + `git diff
  --cached --quiet` instead of `git diff --quiet` against an
  untracked path — fixes the bug that silently skipped the
  first-ever `release-manifests/tray-beta.json` publish in v0.0.1-beta
  and v0.0.2-beta.

## [0.0.2-beta] — 2026-05-12

Metrics-display redesign, first wave. Replaces the hand-rolled 30-day
heatmap on the dashboard with a GitHub-style 53-week heatmap, and
rewires the metrics page's Overview tab with a donut+barlist Type
breakdown alongside the heatmap. Foundational shell + lib helpers
land so subsequent waves can layer in without rebuilding the chart
contract.

### Added

- **Web:** `YearHeatmap` component — 53-week GitHub-style activity
  heatmap, inline SVG, renders against the `--grid-*` token ladder
  for theme reactivity. Shown on `/dashboard` and `/metrics` Overview.
- **Web:** `TypeBreakdown` component — recharts donut + ranked-bar
  combo replacing the manual `<div>` bars previously on the metrics
  Overview tab.
- **Web:** `SparklinePill` component — small stat tile with inline
  sparkline. Foundation for upcoming dashboard pill upgrades.
- **Web:** `MetricCard` + `ChartCard` shells. Required-props pattern
  (`flagKey`, `telemetryKey`, `empty`, `error`, `srTable`) enforces
  the cross-cutting checklist (feature-flag gate, telemetry hook,
  empty/error states, screen-reader fallback) at the TypeScript
  level — cards that skip any of these fail `tsc`.
- **Web:** Typed feature-flag registry (`lib/feature-flags.ts`) for
  the metrics surfaces. All flags default on for v0.0.2-beta; the
  `metrics.now_strip` flag stays off (cut per the impl plan).
- **Web:** Frontend telemetry helper (`lib/metrics-telemetry.ts`).
  Opt-in (off by default via `localStorage["starstats.telemetry"]`).
  Server endpoint to receive POSTs is a follow-up.
- **Web:** Recharts theme bridge (`lib/recharts-theme.ts`) — reads
  `ss-*` CSS-var hex values from `:root` and re-renders on
  `data-theme` mutations so chart colours swap with the active theme.
- **Server:** Migration `0021_share_scopes.sql` — adds a per-user
  `share_scopes` JSONB column with conservative defaults (own data
  only for everything except summary, which defaults to friend).
  Future-proofs the planned cross-user aggregate endpoints; no code
  consumes the column yet.
- **Dep:** `recharts ^3.8.1` in `apps/web`.

### Changed

- **Server:** Bumped `TIMELINE_DAYS_MAX` from 90 to 366 in
  `validation.rs` so `YearHeatmap` callers can request a 365-day
  window without tripping the validator. Existing
  `timeline_rejects_days_above_max` test updated accordingly.
- **Web:** `/dashboard` and `/metrics` now request a 365-day timeline
  (was 30). DayHeatmap still renders cleanly with the wider window.
- **Versions:** Workspace `0.0.1-beta → 0.0.2-beta`,
  `tauri.conf.json` `0.0.1 → 0.0.2`.

### Documentation

- Added `docs/DESIGN-METRICS-PLAN.md` (strategic plan) and
  `docs/DESIGN-METRICS-IMPLEMENTATION-PLAN.md` (execution plan,
  reflecting two rounds of independent review findings).

## [0.0.1-beta] — 2026-05-11

Fresh start on the `beta` channel after the alpha history scrub.
Versions reset from `0.3.12-alpha` to `0.0.1-beta`; the prior alpha
tags and releases were removed from the public repository.

### Added

- **Client:** New `Beta` variant on `ReleaseChannel` (Cargo + tray-UI),
  with a matching `beta.json` channel manifest produced by the release
  workflow. The Settings → Updates dropdown now offers Beta alongside
  Alpha / RC / Live.
- **Release workflow:** Added the `v*.*.*-beta[.N]` case to the
  channel-pattern matcher so beta tags publish to
  `release-manifests/tray-beta.json` on `main`.

### Changed

- **Client:** `ReleaseChannel::default()` is now derived from
  `CARGO_PKG_VERSION` at compile time rather than being a hard-coded
  `Alpha`. A build tagged `vX.Y.Z-beta` defaults fresh installs to the
  Beta channel; the future first stable build will default to Live.
  Persisted user overrides in `config.toml` still win over the default.
- **Client:** Tauri bootstrap updater endpoint flipped from `alpha.json`
  to `beta.json` for this build (only relevant on first launch before
  the channel-aware override fires).
- **Versions:** Workspace `0.3.12-alpha` → `0.0.1-beta`,
  `tauri.conf.json` `0.3.12` → `0.0.1`.

## 0.3.12-alpha — 2026-05-11

### Added

- **Server:** DB-backed SMTP config with KEK-encrypted password and
  hot-reload. New migration `0020_smtp_config.sql` (singleton row
  enforced by `CHECK (id = 1)`, password split into BYTEA
  `ciphertext` + `nonce` columns with a paired-NULL check). New
  `smtp_config_store` module + Postgres impl that encrypts on write /
  decrypts on read via the existing TOTP KEK envelope. The `Mailer`
  trait gains `send_test_email`, and a new `SwappableMailer` wraps
  the active transport in `Arc<RwLock<Arc<dyn Mailer>>>` so the
  admin save flow can replace it without restarting the server. Boot
  precedence is DB(enabled=true) > env > `NoopMailer`.
- **Server:** Three new admin endpoints — `GET/PUT /v1/admin/smtp`
  (read/write the config with password redaction + `password_set`
  bool) and `POST /v1/admin/smtp/test` (sends a diagnostic email to
  the calling admin's verified address; 400 if unverified, 502 on
  SMTP failure). All gated by `RequireAdmin`. `PUT` validates input,
  persists, then swaps the live mailer.
- **Web:** New `/admin/smtp` page with hot-reloading config form.
  Server actions thread the bearer through the existing
  `lib/api`-is-server-only invariant; client form holds controlled
  state with a tri-state password (null = keep, "" = clear, value =
  set) mirroring the server contract. Save / Send test / Reload
  buttons gated by `useTransition` for clean pending UI. New tab in
  `AdminNav` between Submissions and Audit log.
- **Server:** `SpicedbClient::write_owner(handle)` issues TOUCH on
  `stats_record:<handle>#owner@user:<handle>`. The signup handler
  calls it best-effort after `users.create()` so the
  `stats_record.view` permission is non-empty for every new account
  — unblocks any future reinstatement of the SpiceDB self-view gate
  in `query::summary`.

### Changed

- **Tray:** Sync worker now respawns on config save and on device
  pairing. `AppState` holds the running `JoinHandle`; new
  `sync::respawn` aborts the old worker, reloads the persisted
  config, and spawns a fresh one. `save_config` and `redeem_pair`
  call it after `config::save`, so toggling Settings or pairing a
  new device picks up immediately — no more "save settings →
  restart tray" contract. Idempotent: disabling sync swaps the
  handle to `None`.
- **Web:** `QuantumWarp` background re-aims per route. The
  prototype's `warpAngle = angleFor(screen)` wiring was never
  ported to the production Next.js code, so the canvas was stuck
  at the default 180° regardless of which page was active. New
  `QuantumWarpBackground` client wrapper reads `usePathname()` and
  maps to an angle via a static `FIXED` table (mirrors the
  prototype's intuition; deterministic hash fallback for unmapped
  paths). Tween rate bumped 0.04 → 0.08 (~12 frames / ~200ms) so
  the direction change is visually obvious within the brief's
  500ms target.

### Fixed

- **Server:** Drop the `require_user_token` gate from hangar /
  RSI-profile / RSI-org routes. Pairing only mints device JWTs, so
  the gate was locking the tray out of exactly the endpoints it
  was built to feed (e.g. `hangar push failed: 403 Forbidden`).
  Identity is still enforced by `AuthenticatedUser`; the gate
  added no security on top.
- **Web:** Logout no longer sends the user to
  `https://0.0.0.0:3000/`. `route.ts` used to build the redirect
  URL from `req.url`, which inside the container is
  `http://0.0.0.0:3000/auth/logout`; the reverse proxy upgraded
  the scheme to https and the host was wrong. Replaced with a
  relative `Location: /` so the browser resolves against the URL
  it actually typed.
- **Server:** `cargo fmt` drift in `starstats-client/{commands.rs,
  storage.rs}` cleared so subsequent pushes pass CI's
  `cargo fmt --check`.

## 0.3.11-alpha — 2026-05-10

### Added

- **Tray:** Re-parse now retroactively detects bursts over already-
  stored events. New Phase 3 walks each `log_source` in
  `source_offset` order, runs `detect_bursts` over the
  structural-parsed view, inserts one `BurstSummary` per hit, and
  hard-deletes the member rows. Surfaces `bursts_collapsed` and
  `members_suppressed` in `ReparseStats`; the *Re-parse* status line
  reports `…collapsed N bursts (suppressed M spam rows)…` when the
  pass fires. Idempotency key reuses the live-tail format
  (`UUIDv5(log_source : anchor_offset : "{raw_line}|burst:{rule_id}:{size}")`)
  so a session already collapsed at live-ingest time stays a
  strict no-op, and re-running Phase 3 over post-collapse history
  finds nothing to do.
- **Storage:** Three new lean helpers on `Storage` —
  `distinct_log_sources()`, `events_for_burst_scan(log_source)`
  (returns `(id, raw, source_offset, type)` ordered by source
  offset), and `delete_event_by_id(id)`. The first two scope retro-
  burst's working set to one channel at a time so spam-clusters
  spanning channel boundaries can't accidentally fuse.

## 0.3.10-alpha — 2026-05-10

### Added

- **Core:** New `templates` module providing two deterministic
  group-recognition primitives — `EventTemplate` for fixed-sequence
  ritual matching with drift detection, and `BurstRule` for
  variable-cardinality clustering with anchor + member + slack
  budget. Both serialise/deserialise as JSON so future remote
  delivery via `/v1/parser-definitions` is a drop-in.
- **Core:** New `GameEvent::BurstSummary` variant carrying
  `rule_id`, `size`, `end_timestamp`, and a truncated
  `anchor_body_sample`. Validated server-side (non-empty rule id,
  size > 0, ISO-8601 end timestamp).
- **Tray:** Four built-in `BurstRule` definitions in
  `crates/starstats-client/src/burst_rules.rs` —
  `loadout_restore_burst`, `terrain_load_burst`,
  `hud_notification_burst`, `vehicle_stowed_burst` — collapse the
  four spammiest event clusters observed in real Game.log captures.
- **Tray:** `gamelog::process_buffer` ingests in drain-bounded
  batches; `detect_bursts` runs over the structurally-parsed subset,
  emits one `BurstSummary` per hit, and suppresses member events
  from being inserted at all. Idempotency key includes
  `(anchor_offset, rule_id, size)` so retries after a tray crash
  dedupe cleanly.
- **Web:** Timeline renders `burst_summary` events with friendly
  per-rule labels ("Loadout restored", "Terrain loaded",
  "Notifications", "Vehicles stowed"); future remote-served rules
  fall back to a generic "Burst" label.

## 0.3.9-alpha — 2026-05-09

### Changed

- **Tray:** "Discovered logs" status card collapses the per-file
  list into a count + per-kind chip breakdown. Removes 4–10 rows of
  per-path detail from the main status surface; the tray still
  reads every discovered log, the UI just summarises.

## 0.3.8-alpha — 2026-05-09

### Fixed

- **RSI parsers:** All three HTML scrapers (orgs, public profile,
  tray hangar) silently produced empty results because their CSS
  selectors were authored against synthetic test fixtures rather
  than RSI's real markup. Rewritten against verified live DOM
  captured 2026-05-09: orgs key off `box-content org main|affiliation`
  containers with labelled SID/rank entries; profile widens scope
  from `.profile .entry` to `.profile-content .entry` (Enlisted /
  Location / Bio live in a sibling `.left-col` outside `.profile`);
  pledges read hidden-input `value=` attributes (not text content).

### Changed

- **CI:** clippy + test gate widened from `core+server` to
  `core+server+client`. Adds `pnpm install` + tray-ui Vite build +
  Linux Tauri system deps (libwebkit2gtk-4.1-dev, libgtk-3-dev,
  etc.) so the Tauri proc-macro can compile against a populated
  `apps/tray-ui/dist`. Pre-existing client clippy warnings resolved
  (`while_let_loop` in `read_capped_text`, `manual_clamp` in
  `clamp_timeline_limit`).

## 0.3.7-alpha — 2026-05-09

### Added

- **Server:** Admin foundation. New `staff_roles` table with
  soft-delete revocation (`partial unique index … WHERE
  revoked_at IS NULL`); `RequireModerator` / `RequireAdmin` axum
  extractors; bootstrap-from-env helper
  (`STARSTATS_BOOTSTRAP_ADMIN_HANDLES`); admin submission
  moderation routes (accept / reject / dismiss-flag / queue) with
  idempotent state transitions and audit-log writes.
- **Web:** `/admin` shell + `/admin/submissions` moderation queue
  with status filters, paginated list, and per-row server actions.
  Left-rail conditionally renders "Staff › Admin" when the session
  carries staff roles.
- **Web:** RSI-orgs surface — `getMyRsiOrgs` / `getPublicRsiOrgs` /
  `refreshRsiOrgs` API helpers; `OrgsCard` component shared between
  dashboard and `/u/[handle]`; main org sorted first.
- **Web:** Public/friend timeline heatmap rendered on
  `/u/[handle]` mirroring the dashboard treatment.
- **Web:** Hangar parity — `getMyHangar` (404 → null) + new
  `HangarCard` component on dashboard and settings.

### Changed

- **Server:** Renamed `query::ListResponse` → `query::EventsListResponse`
  to eliminate an OpenAPI schema collision with
  `submission_routes::ListResponse` (utoipa keys component schemas
  by Rust type name; the collision silently dropped one of the two
  from the spec).
- **Web:** Replaced hand-rolled `CommerceTransaction` and
  `UserPreferences` types with intersections over the generated
  `apiSchema` types; the narrow `kind` / `status` unions are
  preserved via `Omit<…> &` overlay.

### Fixed

- **Tray:** `RedeemResponse.device_id` is now captured into
  storage instead of being dropped (held under `#[allow(dead_code)]`
  until the self-revoke UI lands).

## 0.3.6-alpha — 2026-05-08

### Added

- **Tray:** Hangar card surfaces affirmative RSI-fetch status
  (last successful refresh + ship count) instead of a silent empty
  pane when the cookie path is healthy.

## 0.3.5-alpha — 2026-05-08

### Fixed

- **Tray:** `set_rsi_cookie` IPC contract — frontend was sending a
  flat `{cookie}` payload while the Tauri command expected a
  wrapped struct; dropped the wrapper so the IPC matches.

## 0.3.4-alpha — 2026-05-08

### Fixed

- **Tray:** Header version now reads from the real Cargo workspace
  `[workspace.package].version` instead of a stale hard-coded
  constant.

## 0.3.3-alpha — 2026-05-08

### Added

- **Tray:** *Re-ingest* button under the Events tab — replays the
  raw rotated `Game-*.log` files through the current parser, so
  newly-added event types backfill historical sessions without
  requiring the user to keep the original `Game.log` around.
- **Repo:** Project front-door (CONTRIBUTING, SECURITY,
  CODE_OF_CONDUCT, EAC-SAFETY, NOTICE) + starstats.app domain
  wiring across README and docs.

### Fixed

- **Storage:** `for_each_event` releases the per-batch SQLite
  connection lock between batches so the writer can make progress
  on large local stores during a Re-parse.

## 0.3.2-alpha — 2026-05-08

### Fixed

- **Tray:** Re-parse no longer deadlocks on large local stores. The
  per-batch SQLite connection lock is now released between batches in
  `for_each_event`, letting the writer make progress while the
  re-classify pass walks history.

## 0.3.1-alpha — 2026-05-08

### Added

- **Parser:** Modern `PlayerDeath` and `PlayerIncapacitated` event
  variants matched against the corpse-cleanup burst that replaces
  CIG's old `<Actor Death>` line in 4.x+ Star Citizen builds. The
  legacy `ActorDeath` variant is retained for older logs.
- **Parser:** Zone enrichment for the new death events — quantum-target
  and `Seed Solar System` context are folded into the surfaced event
  so the tray can show *where* a death happened, not just that one
  occurred.

### Changed

- Updated `release-manifests/tray-alpha.json` to point the alpha channel
  at v0.3.1-alpha.

## 0.3.0-alpha — 2026-05-07

### Added

- **Updater:** Channel selector in *Settings* with three channels —
  Alpha, RC, Live — backed by per-channel updater manifests at
  `release-manifests/tray-{alpha,rc,live}.json` on `main`. The release
  workflow now picks the destination manifest from the tag's
  pre-release suffix (`-alpha` / `-rc` / bare semver).
- **Tray:** *Re-parse* button in the *Events* tab. Re-classifies
  every event already in the local store against the current parser
  without needing to replay `Game.log` from disk — useful after the
  parser learns a new variant.
- **Tray:** Workspace version is now surfaced in *Settings* so the
  installed build matches the corresponding release tag at a glance.

### Fixed

- **Updater:** Per-channel manifest fix — Tauri's updater previously
  only handled `releases/latest`, which 404s for pre-releases. The
  in-app updater now polls the explicit per-channel JSON via
  `raw.githubusercontent.com`, giving every channel a stable URL.

### Changed

- Workspace version bumped 0.2.0-alpha → 0.3.0-alpha.

## 0.2.0-alpha — 2026-05-04

### Added

- **Parser:** Dynamic parser definitions decoupled from the Rust
  build — new `Game.log` token shapes can be added through the
  versioned definition table without recompiling the tray.
- **API:** `GET /v1/commerce/recent` endpoint surfacing paired
  buy/sell transactions for the authenticated user.
- **Server / parser:** Transaction pairing — `ShopBuyRequest` /
  `ShopFlowResponse` and `CommodityBuyRequest` / `CommoditySellRequest`
  pairs are now matched into a single completed-order record with
  resolved price, quantity, and location.
- **Tray:** *Commerce* tab surfacing paired transactions, totals, and
  per-location breakdowns.
- **Installer:** WiX upgrade metadata so MSI installs from prior
  alphas now upgrade in place rather than installing side-by-side.

### Changed

- Workspace version bumped 0.1.0-alpha → 0.2.0-alpha.
- Release pipeline split into a two-step draft + publish to satisfy
  GitHub's immutable-release policy when the same tag is retried.
- Release pipeline now accepts pre-release tag suffixes against
  numeric MSI bundle versions (the MSI version field is numeric-only;
  the tag carries the `-alpha` / `-rc` annotation separately).

### Security

- Bumped `tauri` 2.11.0 → 2.11.1 to pick up the fix for
  [GHSA-7gmj-67g7-phm9](https://github.com/advisories/GHSA-7gmj-67g7-phm9).

## 0.1.0-alpha — 2026-05-03

### Added

- Initial public release.
- Tauri tray client with `Game.log` tail, local SQLite store, and
  signed updater bundles for Windows (NSIS + WiX MSI) and Linux
  (AppImage + .deb).
- StarStats API server (Axum + sqlx + Postgres) with self-hosted
  JWT auth, device pairing, ingest, query endpoints, OIDC discovery,
  audit log, and Prometheus `/metrics`.
- Next.js 15 web companion with sign-up / sign-in, email
  verification, dashboard, and device management.
- Initial parser coverage: `ProcessInit`, `LegacyLogin`, `JoinPu`,
  `ChangeServer`, `SeedSolarSystem`, `ResolveSpawn`, `ActorDeath`
  (legacy), `VehicleDestruction`, `HudNotification`,
  `LocationInventoryRequested`, `PlanetTerrainLoad`,
  `QuantumTargetSelected`, `AttachmentReceived`, `VehicleStowed`,
  `GameCrash`, `LauncherActivity`, `RemoteMatch`,
  `MissionStart` / `MissionEnd`, `SessionEnd`.

[Unreleased]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v1.3.1...HEAD
[1.3.1]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v1.2.1...v1.3.0
[1.2.1]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v0.0.7-beta...v1.0.0
[0.0.7-beta]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v0.0.5-beta...v0.0.7-beta
[0.0.5-beta]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v0.0.4-beta...v0.0.5-beta
[0.0.4-beta]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v0.0.3-beta...v0.0.4-beta
[0.0.3-beta]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v0.0.2-beta...v0.0.3-beta
[0.0.2-beta]: https://github.com/TheCodeSaiyan/StarStats-Platform/compare/v0.0.1-beta...v0.0.2-beta
[0.0.1-beta]: https://github.com/TheCodeSaiyan/StarStats-Platform/releases/tag/v0.0.1-beta
