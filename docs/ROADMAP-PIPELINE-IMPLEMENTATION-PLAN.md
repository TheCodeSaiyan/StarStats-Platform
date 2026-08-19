# Roadmap Pipeline — Implementation Plan

**Companion to:** `ROADMAP-PIPELINE-SPEC.md` (source of truth).
**Status:** drafted 2026-05-22, not yet started.
**Branch strategy:** all feature work targets `next`. No `main` PRs except
the eventual `next` → `main` promotion.

This plan breaks the feature into 10 build phases (0 through 9). Each
phase is a coherent, shippable vertical slice. Per project Rule 4, each
phase internally follows the 7-phase pipeline (research → parallel impl →
3 parallel review passes → synthesis → finalize), but that orchestration
is per-phase not whole-feature.

## Phase ordering at a glance

```
Phase 0   pre-flight (out-of-code: Project board, GitHub App)
   ↓
Phase 1   data layer (migration + stores + types)
   ↓
Phase 2   GraphQL client (read-only)
   ↓
Phase 3   sync engine (webhook + reconciliation)
   ↓
Phase 4   CI event ingestion
   ↓ (Phases 5 and 6 can run in parallel)
Phase 5   public read API + web /roadmap          Phase 6   voting + writeback
   ↓                                                  ↓
Phase 7   changelog auto-draft + publish
   ↓
Phase 8   tray "What's new" panel
   ↓
Phase 9   CI emitter wiring + cutover
```

Phases 5 and 6 are the only parallelisable pair — both depend on the sync
engine (phase 3) and event ingestion (phase 4) being green, but neither
depends on the other.

---

## Phase 0 — Pre-flight (out-of-code)

**Goal:** every prerequisite that lives outside the repo is in place
before phase 1 starts.

**Owner-of-decision:** project lead.

**Deliverables:**

- GitHub Project (v2) created with the custom fields per spec §3.3.
  Project visibility set to **private** (spec §3.2).
- Project seeded with items 2–9 of the parent roadmap list as
  placeholder rows (no statuses yet).
- GitHub App registered with `Projects: read & write` permission, installed
  on the relevant repos. Credentials captured in:
  - `ROADMAP_GH_APP_ID`
  - `ROADMAP_GH_APP_INSTALLATION_ID`
  - `ROADMAP_GH_APP_PRIVATE_KEY`
- HMAC keys minted and stored:
  - `ROADMAP_GH_WEBHOOK_HMAC_KEY`  — for §3.5 webhook verification
  - `ROADMAP_CI_EVENT_HMAC_KEY`    — for §4.5 event-payload verification
- A new ops doc `docs/ROADMAP-PIPELINE-OPS.md` is created with: the
  custom-field schema, env var inventory, the event-contract payload
  (§4.1), key-rotation procedure, and DR runbook.

**Gates to exit phase 0:**

- All env vars defined in `infra/` secret stores (one per environment).
- `docs/ROADMAP-PIPELINE-OPS.md` reviewed.
- A GraphQL "hello world" introspection query against the Project board
  returns successfully from a one-off local script.

---

## Phase 1 — Data layer

**Goal:** all roadmap-related Postgres tables exist; a `RoadmapStore`
trait with Memory + Postgres impls is exercised by store tests; no routes
yet.

**Files created:**

- `StarStats/crates/starstats-server/migrations/0040_roadmap_pipeline.sql`
  — additive, byte-immutable. Creates:
  - `roadmap_items`
  - `roadmap_channel_statuses`
  - `roadmap_channel_statuses_archive`
  - `roadmap_event_log` (idempotency, TTL via `created_at`)
  - `roadmap_votes`
  - `roadmap_subscribers`
  - `roadmap_changelog`
  - `roadmap_user_read_state`
- `StarStats/crates/starstats-server/src/roadmap/mod.rs`
- `StarStats/crates/starstats-server/src/roadmap/models.rs` — Rust types
  + serde + utoipa schemas.
- `StarStats/crates/starstats-server/src/roadmap/store.rs` — trait +
  Postgres impl + `mod test_support` Memory impl, mirroring
  `share_metadata.rs` pattern.

**No GRANT statements.** Confirmed against the existing migrations dir
(`crates/starstats-server/migrations/*.sql`) — zero `GRANT` DDL exists
anywhere. The project runs a single Postgres role; permissions are
managed outside the SQLx migration tree. Do not add GRANTs.

**Tests (Rule 2 — TDD RED first):**

6–8 store tests against the Memory impl before any route. Coverage
target on the store layer is 90%+.

- `upsert_item_idempotent`
- `slug_immutable_after_create`
- `channel_status_archive_roundtrip`
- `vote_insert_and_retract_net_to_zero`
- `subscriber_membership_is_private`
- `soft_delete_excludes_from_list`
- `event_log_dedup_on_event_id`
- `headline_status_aggregation_matrix`  (§2.3, all 4 ordering cases)

**Gates to exit phase 1:**

- `cargo test -p starstats-server roadmap::` passes.
- `cargo fmt -p starstats-server` clean.
- `cargo clippy -p starstats-server -- -D warnings` clean.
- Migration applies on a fresh DB; `_sqlx_migrations` checksum stable
  across two consecutive runs.

---

## Phase 2 — GraphQL client (read-only)

**Goal:** a typed wrapper around the GitHub Projects v2 GraphQL API,
exercised by integration tests against a mock GraphQL server.

**Files created:**

- `StarStats/crates/starstats-server/src/roadmap/github_graphql.rs`
  - GitHub App JWT minting + installation token caching.
  - One coalesced query: `list_project_items()` returns all items with
    custom field values + linked PR check rollups.
  - Per-item query: `get_project_item(id)`.
  - Exponential backoff on `RATE_LIMITED` and 5xx.
  - Error taxonomy: `AuthError`, `RateLimited`, `Transient`, `Schema`,
    `Other`.
- `StarStats/crates/starstats-server/src/roadmap/github_graphql_tests.rs`
  — contract tests against a `wiremock`-served GraphQL endpoint.

**Decisions:**

- Use `reqwest` (already a dep) + hand-rolled query strings. Code-gen via
  `graphql-client` is overkill for ~3 queries and adds a build step.
- Schema introspection is a one-off local task (phase 0); fixtures live
  in `roadmap/github_graphql_fixtures.rs`.

**Tests (RED first):**

- `installation_token_caches_for_55_minutes`
- `list_project_items_pages_correctly`
- `rate_limit_retries_with_backoff`
- `auth_error_does_not_retry`
- `schema_error_carries_original_payload`

**Gates to exit phase 2:**

- All graphql tests green.
- One manual smoke test against the real Project board, behind a
  `--features integration` flag. Output captured in PR description.

---

## Phase 3 — Sync engine

**Goal:** webhook receiver and reconciliation job both keep the local
`roadmap_items` rows in step with the Project board. No public API yet.

**Files created / changed:**

- `StarStats/crates/starstats-server/src/roadmap/sync.rs`
  - `webhook_handler()` — verifies HMAC, parses event, dispatches.
  - `reconcile_once()` — pulls all items via §2 client, diffs, applies.
  - `spawn_reconciler(cancel: CancellationToken)` — background task,
    5-minute interval.
- `StarStats/crates/starstats-server/src/roadmap/routes.rs`
  - `POST /v1/internal/roadmap/github-webhook` only at this phase.
- `StarStats/crates/starstats-server/src/main.rs`
  - Wire `RoadmapStore` as `Arc<dyn ...>` Extension (mirrors
    `share_metadata_dyn` pattern per project `docs/ENGINEERING.md`).
  - Spawn the reconciler.
- `StarStats/crates/starstats-server/src/openapi.rs`
  - Register only the webhook route (kept off the public spec via
    `#[utoipa::path(hidden)]`).

**Tests (RED first):**

- `webhook_rejects_unsigned`
- `webhook_rejects_old_timestamp`
- `webhook_applies_title_change`
- `webhook_creates_new_item_on_first_event`
- `reconcile_creates_missing_local_rows`
- `reconcile_archives_removed_channels`
- `reconcile_recovers_from_dropped_webhook` (deliberately drops a
  webhook, runs reconcile, asserts state)
- `public_true_to_false_removes_subscribers`
- `project_item_deletion_soft_deletes_local`

**Gates to exit phase 3:**

- All tests green on the Memory impl AND against a containerised
  Postgres (Rule 6 — container-first integration).
- A 24-hour soak test on `next` shows zero reconciliation errors when
  the Project board is left untouched.

---

## Phase 4 — CI event ingestion

**Goal:** the server can receive `POST /v1/internal/roadmap/events` and
correctly advance `ChannelStatus` rows. Still no public-facing surface.

**Files created / changed:**

- `StarStats/crates/starstats-server/src/roadmap/events.rs`
  - `ingest_event(payload)` — verifies HMAC + timestamp, dedups via
    `event_id`, applies channel-status transition.
  - Re-reads `public` via GraphQL on receipt (§4.3); audit-logs
    mismatch.
- `StarStats/crates/starstats-server/src/roadmap/routes.rs`
  - Add `POST /v1/internal/roadmap/events`.

**Tests (RED first):**

- `event_rejects_unsigned`
- `event_rejects_old_timestamp`
- `event_rejects_missing_event_id`
- `event_idempotent_on_duplicate_id`
- `event_advances_building_to_shipped`
- `event_never_demotes_status`
- `event_failing_build_sets_health_without_status_change`
- `event_public_mismatch_audit_logs_and_uses_graphql_value`
- `event_to_parked_channel_is_a_noop`  (sticky parked)

**Gates to exit phase 4:**

- All tests green.
- A scripted dummy-CI emitter run end-to-end against `next` flips a
  test item's channel-status.

---

## Phase 5 — Public read API + web `/roadmap` (parallel with phase 6)

**Goal:** the public web page renders the roadmap. Read-only.

**Files created / changed:**

Server:

- `StarStats/crates/starstats-server/src/roadmap/routes.rs`
  - `GET /v1/roadmap`               — list (public-filtered)
  - `GET /v1/roadmap/:slug`         — detail
  - `GET /v1/roadmap/changelog`     — published entries only
- `openapi.rs` — register the public routes + schemas.
- Regenerate TS client: `pnpm --filter api-client-ts run generate`.

Web:

- `StarStats/apps/web/src/app/roadmap/page.tsx` — list
- `StarStats/apps/web/src/app/roadmap/[slug]/page.tsx` — detail
- `StarStats/apps/web/src/app/changelog/page.tsx`
- `StarStats/apps/web/src/lib/roadmap.ts` — server-side fetch helpers,
  `cache: 'no-store'` per project convention.
- `StarStats/apps/web/src/components/roadmap/RoadmapCard.tsx`
- `StarStats/apps/web/src/components/roadmap/ChannelChipStrip.tsx`
- `StarStats/apps/web/src/components/roadmap/StatusBadge.tsx`
- Vitest specs alongside each.
- A Playwright e2e scenario in `apps/web/e2e/roadmap.spec.ts`.
- Default fixture entry in `apps/web/e2e/helpers/api-mock.ts`
  (`scenarioFor()` base map) per project convention.

**Design constraints (Rule 5):**

- 375 / 768 / 1024 px viewports.
- Per-channel chip strip: max 4 chips per card; overflow expands on
  click. Component sub-items collapsed by default.
- Follows the existing `/features` page chrome (numbered eyebrow + h2 +
  lede).

**Gates to exit phase 5:**

- Vitest green; Playwright green.
- 3 responsive breakpoints visually verified.
- `pnpm --filter web run test:run` clean on CI.

---

## Phase 6 — Voting + writeback (parallel with phase 5)

**Goal:** authenticated users can vote and subscribe; counts write back
to GitHub on a 5-min batch.

**Files created / changed:**

Server:

- `StarStats/crates/starstats-server/src/roadmap/votes.rs`
  - `cast_vote`, `retract_vote`, `subscribe`, `unsubscribe`.
  - Rate-limit middleware (30/min/user) via the existing rate-limit
    layer.
- `StarStats/crates/starstats-server/src/roadmap/writeback.rs`
  - 5-min cron worker. Coalesces per-item, writes Votes + Subscribers
    counts via GraphQL.
- `routes.rs`:
  - `POST   /v1/roadmap/:slug/vote`
  - `DELETE /v1/roadmap/:slug/vote`
  - `POST   /v1/roadmap/:slug/subscribe`
  - `DELETE /v1/roadmap/:slug/subscribe`

Web:

- Vote button on RoadmapCard with optimistic UI + server-action chip
  derived from response (`response.voted` not user intent — project
  convention).

**Tests (RED first):**

- `vote_anonymous_returns_401`
- `vote_idempotent_for_same_user`
- `retract_then_vote_in_same_batch_writes_back_unchanged_count`
- `vote_burst_at_31_per_min_rate_limits_at_30`
- `writeback_coalesces_to_one_write_per_item`
- `writeback_audit_logs_on_failure`

**Gates to exit phase 6:**

- All tests green.
- A manual end-to-end vote on `next` propagates to the real Project
  board's `Votes` field within one 5-min window.

---

## Phase 7 — Changelog auto-draft + publish flow

**Goal:** shipped transitions produce draft changelog entries; admins can
publish; published entries notify subscribers.

**Files created / changed:**

Server:

- `StarStats/crates/starstats-server/src/roadmap/changelog.rs`
  - `draft_on_shipped(channel_status)` — diffs PR titles between
    `previous_shipped_sha` and `commit_sha`, attributes per §8.3, writes
    draft row.
  - `publish(draft_id)` — moves draft to published, fans out
    notifications + optional Discord webhook.
  - `auto_purge_old_drafts()` — 30-day cleanup cron.
- `routes.rs`:
  - `GET  /v1/admin/roadmap/changelog/drafts`   — staff-gated
  - `POST /v1/admin/roadmap/changelog/:id/publish`
  - `POST /v1/admin/roadmap/changelog/:id/edit` — title/body edits
    before publish (the draft is editable, the published entry is not)

Web:

- `StarStats/apps/web/src/app/admin/roadmap/changelog/page.tsx`
- `StarStats/apps/web/src/app/changelog/page.tsx` already exists from
  phase 5 (list-only); now wire it to live data.

**Tests (RED first):**

- `draft_uses_correct_pr_range`
- `direct_commit_appears_with_subject`
- `attribution_honours_trailer_then_project_link_then_label`
- `publish_fires_subscriber_notifications`
- `publish_skips_discord_when_env_missing`
- `auto_purge_removes_drafts_older_than_30_days`

**Gates to exit phase 7:**

- All tests green.
- Manual admin publish on `next` posts a Discord notification in the
  test channel.

---

## Phase 8 — Tray "What's new" panel

**Goal:** the tray UI surfaces the 3 most-recent unread top-level
roadmap items (or, if anonymous, the 3 most-recent published changelog
entries).

**Files created / changed:**

- `StarStats/crates/starstats-server/src/roadmap/routes.rs`
  - `GET /v1/me/roadmap/whats-new` — returns the 3 items + read state.
  - `POST /v1/me/roadmap/whats-new/seen` — marks items as read.
- `StarStats/crates/starstats-client/src/whats_new.rs` — Rust-side
  fetcher (CSP-safe, server-fetched then handed to React) per the
  v1.7.0 pattern called out in project `docs/ENGINEERING.md`.
- `StarStats/apps/tray-ui/src/panes/WhatsNewPane.tsx`
- `StarStats/apps/tray-ui/src/App.tsx` — register the pane.
  Fixed-position elements (e.g. expand drawer) MUST portal to
  `document.body` per the tray pane rule in project `docs/ENGINEERING.md`.

**Tests:**

- vitest on the pane.
- A tray e2e (if the test harness supports it) covering anonymous vs
  paired behaviour.

**Gates to exit phase 8:**

- Pane renders correctly in tray dev build.
- Anonymous and paired paths both verified manually.

---

## Phase 9 — CI emitter wiring + cutover

**Goal:** the existing release/deploy workflows emit roadmap events. The
feature flips from dogfood to live.

**Files created / changed:**

- `StarStats/.github/workflows/release.yml` (or wherever the live release
  workflow lives — confirm before editing) — add a step after deploy:

  ```yaml
  - name: Emit roadmap pipeline event
    if: success()
    run: node scripts/roadmap-emit-event.mjs
    env:
      ROADMAP_CI_EVENT_HMAC_KEY: ${{ secrets.ROADMAP_CI_EVENT_HMAC_KEY }}
      ROADMAP_EVENTS_URL: ${{ secrets.ROADMAP_EVENTS_URL }}
      CHANNEL: live
      COMMIT_SHA: ${{ github.sha }}
      BUILD_ID: ${{ github.run_id }}
      CI_RUN_URL: ${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}
      TAG: ${{ github.ref_name }}
  ```

- `StarStats/scripts/roadmap-emit-event.mjs` — builds the §4.1 payload,
  HMAC-signs, POSTs to the server with retry-on-5xx (3 attempts,
  exponential backoff). Generates `event_id` with `crypto.randomUUID()`.
- Equivalent step in the pre-release workflow with `CHANNEL` derived
  from the tag's pre-release marker.
- `tag→channel` mapping logic stays in the existing workflow (see
  [release-workflow-mechanism]); the emit script just consumes
  `CHANNEL` as an env var.

**Acceptance for phase 9:**

- Cut a no-op `vX.Y.Z-beta.1` tag on `next`. Confirm an event lands and
  the test item's `beta` channel flips to `shipped`.
- Cut a no-op `vX.Y.Z` tag on `main`. Confirm `live` channel flips.
- Flip the feature flag (if one was added in phase 5) to make
  `/roadmap` discoverable from primary navigation.

**Gates to exit phase 9:**

- Both real tag cuts produce expected state transitions.
- No reconciliation drift over a 72-hour observation window.
- `docs/CHANGELOG.md` updated with the feature's shipping note.

---

## Rollback strategy

Each phase is reversible without touching shipped phases:

| Phase | Rollback |
|------:|---------|
| 1 | Migration is additive only; no rollback needed. Tables stay empty if no further phases ship. |
| 2 | Remove the GraphQL client module; no public surface. |
| 3 | Disable the reconciler spawn in `main.rs`; webhook route 404s. |
| 4 | Disable the events route; CI side gracefully no-ops on POST failure. |
| 5 | Remove `/roadmap` routes from Next.js; server routes idle. |
| 6 | Disable the vote routes; cast votes still safe in DB. |
| 7 | Disable changelog routes; drafts stay in DB. |
| 8 | Remove the tray pane; server route stays. |
| 9 | Remove the workflow step; events stop arriving. |

If a phase needs to be undone after a `main` ship, do it via a new
forward-only migration / code change. Never drop columns.

---

## Tracking

Per project convention (Rule 0), each phase enters with a Plan-Mode
review. As phases complete, this doc gets a status line at the top:

```
Phase status: 0 done ✓ | 1 done ✓ | 2 in flight | 3..9 pending
```

The corresponding GitHub Project items for these phases live on the
same Project board this feature builds — eat the dogfood.
