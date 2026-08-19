# Dynamic "Incoming Features" Page + Dev Pipeline Integration — Spec

**Status:** in-design
**Owner:** TBD
**Last revised:** 2026-05-22 (gap fixes folded in from review)

A roadmap generated from the dev pipeline, not hand-maintained. Source of
truth is a GitHub Projects v2 board; status, build health, deploy state,
and changelog entries are derived from CI/CD signals. Surfaced in the tray
app ("What's coming next"), on the web companion at `/roadmap`, and as
in-context tooltips. User-facing votes flow back into the Project board for
prioritisation.

This spec is the long-lived source of truth. The phased build plan lives in
`ROADMAP-PIPELINE-IMPLEMENTATION-PLAN.md`.

---

## 1. Data model

### 1.1 `RoadmapItem`

```
RoadmapItem {
  id                       UUID
  slug                     TEXT          -- see §1.4
  title                    TEXT
  summary                  TEXT
  category                 TEXT
  eta_band                 TEXT          -- closed enum, see §1.2
  votes                    INTEGER       -- local is authoritative, §1.3
  subscribers              UUID[]        -- user IDs, see privacy §7
  surfaces                 TEXT[]        -- subset of §1.5
  parent_id                UUID NULL     -- hierarchy, §1.6
  components               UUID[]        -- inverse of parent_id
  channel_statuses         JSONB         -- see §2
  links                    JSONB         -- typed array, §1.7
  public                   BOOLEAN       -- false by default; §7
  github_project_item_id   TEXT          -- stable across renames
  created_at               TIMESTAMPTZ   NOT NULL
  content_last_updated     TIMESTAMPTZ   NOT NULL  -- §1.8
  pipeline_last_updated    TIMESTAMPTZ   NOT NULL  -- §1.8
}
```

### 1.2 `eta_band` (closed enum)

Stored as TEXT, parsed via `parse()` + `as_str()` per project enum
convention. Values:

- `now`            — work in flight or imminent (this sprint)
- `next`           — next ~4–8 weeks
- `later`          — within roughly a quarter
- `someday`        — committed-in-principle, no firm date
- `tbd`            — explicit "we haven't decided" (default)

`eta_band` is a Project custom field on GitHub, manually set. The pipeline
does NOT derive it from CI signals.

### 1.3 Vote / subscriber authority

- **Local DB row is authoritative** for `votes` and `subscribers`. The
  GitHub Project's `Votes` / `Subscribers` custom fields are a denormalised
  mirror, written by the roadmap service.
- Read path always reads local. Writeback to GitHub is for board-view
  visibility only and is allowed to lag.
- On writeback failure, retry per backoff policy; never re-read from
  GitHub to "correct" local.

### 1.4 `slug` lifecycle

- Generated on first sync from the Project item title (lowercase, kebab-case,
  trimmed to 80 chars, suffixed with `-2`/`-3` on collision).
- **Slug is immutable after creation.** Title changes on the Project board
  update `title` but do not regenerate `slug`. Permalinks survive renames.
- Stored alongside `github_project_item_id` so the row can be reconciled
  even if both title and slug change.

### 1.5 `surfaces` (multi-select)

- `tray-whats-new`         — appears in the tray "What's new" panel
- `web-roadmap`            — appears on `/roadmap`
- `in-context-tooltip`     — eligible to be referenced from in-app tooltips
- `admin-only`             — visible in admin/dev view only (overrides
                             default surface choice when `public = false`)

### 1.6 Hierarchy

- `parent_id` is nullable. Top-level items have `parent_id = NULL`.
- Component items have a non-null `parent_id` pointing at their top-level
  feature.
- `components` is the denormalised inverse; populated by the sync.
- Single level of nesting only — no grand-components.

### 1.7 `links`

```
{ kind: 'pr' | 'issue' | 'doc' | 'external', url: TEXT, label: TEXT }[]
```

Anything else is rejected at the API layer.

### 1.8 Split `last_updated`

- `content_last_updated` — bumped on title/summary/category/eta_band/
  channels/parent/links/public changes (i.e. board-driven edits).
- `pipeline_last_updated` — bumped on `channel_statuses` changes (i.e.
  CI-driven state transitions).
- Vote writeback bumps **neither**. Subscriber count changes bump
  **neither**. (Both have their own timestamps on the vote-store side.)

---

## 2. Channel statuses

### 2.1 Shape

```
channel_statuses: ChannelStatus[]

ChannelStatus {
  channel            TEXT   -- 'live' | 'beta' | 'rc' | 'alpha' | 'tech-preview'
  status             TEXT   -- §2.2
  build_health       TEXT   -- 'passing' | 'failing' | 'in-progress' | 'unknown'
  build_id           TEXT NULL
  commit_sha         TEXT NULL
  deployed_at        TIMESTAMPTZ NULL
  ci_run_url         TEXT NULL
  previous_shipped_sha TEXT NULL   -- for changelog diffing, §8
  last_event_id      TEXT NULL     -- idempotency, §4
}
```

`rc` (release candidate) is a first-class peer of `beta` on the
release ladder (`alpha → beta → rc → live`). As of v1.8.9, the CI
emitter routes rc-tagged shipments to `channel='rc'` instead of
folding them into the beta channel — so an item's beta-track and
rc-track progress are independently observable. The board's
`channel/rc` label feeds this; pre-v1.8.9 boards labeled `channel/rc`
were silently dropped by the mapper (unknown channel), so the
behaviour change is additive.

### 2.2 `status` enum

- `proposed`     — captured but not committed to
- `in-design`    — designed, not yet building
- `building`     — code in flight
- `beta`         — deployed to a pre-release channel (see §2.4 for trigger)
- `shipped`     — deployed to the channel in question
- `parked`      — explicitly de-scoped (NOT "blocked")

### 2.3 Aggregation rule (headline status)

The "headline" status shown on a roadmap card is computed across the
channels the feature targets:

1. Drop any channel in `parked` from the aggregation.
2. If all remaining channels are `shipped`, headline is `shipped`.
3. Otherwise, headline is the **minimum** of remaining channels' statuses,
   where the order is `proposed < in-design < building < beta < shipped`.
4. If after step 1 there are no channels remaining, the headline is
   `parked`.

### 2.4 `beta` transition trigger

A channel flips to `beta` when:

- A `vX.Y.Z-{alpha,beta,rc}.N` tag is created on `next` AND
- The CI deploy job for that tag completes successfully AND
- The emitted event names that channel.

Tag-to-channel mapping logic stays in the existing CI per the existing
release workflow ([release-workflow-mechanism]). The roadmap service does
not parse tag conventions.

### 2.5 Status auto-advances only

- Status never auto-downgrades. A build failure on a `shipped` channel
  surfaces `build_health: failing` without demoting `status`.
- Manual override is allowed for `proposed`, `in-design`, `beta`, and
  `parked` (set on the Project board). Auto-advance can override a manual
  `in-design` → `building` once CI signals code in flight, but cannot
  override a manual `parked`.
- `parked` is sticky. To resume work, the Project-board editor must clear
  `parked` manually.

### 2.6 Channel not-yet-targeted vs not-yet-started

- A channel only appears in `channel_statuses[]` when **either** the
  `Channels` custom field on the Project item includes it **or** a CI
  event has been received for it.
- A `Channels` membership without any CI event yet → `status: proposed`,
  `build_health: unknown`.
- A `Channels` membership being **removed** → corresponding
  `ChannelStatus` is archived (moved to `channel_statuses_archive` table,
  not deleted, so vote history and previous-shipped-sha is recoverable).
- Re-adding an archived channel restores it from the archive row.

### 2.7 Component status linkage

- Top-level features show a per-channel roll-up of their components'
  statuses.
- Roll-up rule per channel: minimum of components' statuses on that
  channel, using §2.3 ordering.
- A component in `parked` does **not** park the parent's roll-up — it is
  excluded from the min, same as §2.3 step 1.
- Component pages link to their parent and vice-versa.
- The tray "What's new" panel only shows top-level items.

---

## 3. GitHub Projects integration

### 3.1 Source of truth

- A single GitHub Project (v2) is **the** roadmap. Project items, not
  Issues, drive the page.
- Each Project item maps 1:1 to a `RoadmapItem`. Components are child
  Project items linked via the Project's hierarchy field.

### 3.2 Project visibility

- **The GitHub Project itself MUST be private.** `Public = false` items
  are filtered from the public roadmap page, but if the Project is public,
  the filter is purely cosmetic — anyone with the Project URL sees
  everything. State this assumption in `docs/ROADMAP-PIPELINE-OPS.md`
  (to be created in build phase 0).

### 3.3 Custom fields + linked-Issue labels

The pipeline reads the following Project custom fields (all single-select
or scalar — Projects v2 doesn't support multi-select custom fields):

- `Status`         — mirrors §2.2 (single-select)
- `Public`         — single-select `Yes`/`No`. Default `No`. Only
                     `Public = Yes` items appear on public surfaces.
- `Category`       — single-select (UI / Backend / Org / Privacy /
                     Anti-Cheat-Safe / Integration / etc.)
- `ETA Band`       — single-select, §1.2
- `Votes`          — number, written by the service
- `Subscribers`    — number, written by the service

**Channels and surfaces come from linked-Issue labels**, NOT from
custom fields, because Projects v2 has no multi-select field type:

- Channels: labels matching `channel/<name>` on the linked Issue/PR
  (e.g. `channel/live`, `channel/beta`). The pipeline strips the
  `channel/` prefix and parses the rest as `ChannelName` (§2.1).
- Surfaces: labels matching `surface/<name>` (e.g.
  `surface/tray-whats-new`, `surface/web-roadmap`).

DraftIssue project items have no labels — they are treated as
`Channels = []` (not yet targeted) and `Surfaces = []` until promoted
to a real Issue/PR. This matches §2.6's "not-yet-targeted" semantics.

### 3.4 Sync direction

- **GitHub → roadmap** is the default direction.
- Edits on the web roadmap UI never write back to GitHub.
- The only exceptions are `Votes` and `Subscribers` writebacks, batched
  per §6.
- **Title is mutable** on GitHub side and the local row updates; `slug`
  does not (§1.4).
- **Project item deletion on GitHub** → local row soft-deletes (sets
  `deleted_at`); not hard-deleted. Vote history retained. Soft-deleted
  rows never appear on any surface.

### 3.5 Webhook + reconciliation

- Webhook is the fast path on Project item change. Verifies HMAC
  signature; failed verifications are dropped AND audit-logged via §12.
- Receiver subscribes to TWO GitHub webhook events at the org level:
  - `projects_v2_item` — fires on Project field changes, item adds/removes/archives, reorders, draft→issue conversions. Routed to `handle_projects_v2_item_event`, which re-fetches the item via GraphQL and runs it through the mapper. Authoritative for the `Public` field, `Status` field, content reference.
  - `issues` — fires on Issue lifecycle changes (`labeled`, `unlabeled`, `opened`, `reopened` matter; assigned/commented/closed are ignored). Routed to `handle_issue_event`, which queries the Issue's `projectItems` connection, filters to our `project_id`, and re-applies the mapper to each linked Project item. Necessary because `surface/*` and `channel/*` labels live on the Issue, not on the Project item — without this branch, label changes would only propagate on the 5-min reconciler tick.
- The receiver dispatches on the `X-GitHub-Event` request header. Unknown event types are logged + 204'd (webhook deliveries don't retry); unknown actions inside a known event are likewise ignored. Both paths share `apply_one` so the mapping logic stays single-sourced.
- Scheduled reconciliation job runs every 5 minutes. Pulls all Project
  items via a single paginated GraphQL query, diffs against local state,
  applies corrections.
- Reconciliation MUST recover cleanly from missed webhooks (verified by
  deliberately dropping one in a test).
- GitHub webhook UI configuration on the org: subscribe to **Projects v2 item** AND **Issues** events. Subscribing to extras (e.g. Pull requests) isn't harmful — the receiver 204s them — but it's network noise.

### 3.6 `Public = true → false` transition

When a previously-public item flips to private:

- The item disappears from public surfaces within one reconciliation
  cycle (≤5 min).
- Existing subscribers receive a one-time notification: "A roadmap item
  you were following is no longer public." They are silently removed
  from `subscribers[]` (no further updates).
- Vote counts are retained but no longer visible publicly.

### 3.7 GraphQL client

- GitHub Projects v2 is GraphQL-only. A GraphQL client wrapper is added
  to `crates/starstats-server/src/roadmap_github.rs`.
- Auth uses a **GitHub App** (not a PAT) with installation token, scoped
  to the roadmap Project repository. App installation ID and private key
  are stored in env vars: `ROADMAP_GH_APP_ID`,
  `ROADMAP_GH_APP_INSTALLATION_ID`, `ROADMAP_GH_APP_PRIVATE_KEY`.
- Required permissions: `Projects: read & write`.
- Retry/backoff with rate-limit awareness. Reads coalesce into one
  paginated query per reconciliation cycle.

---

## 4. CI event contract

### 4.1 Event payload (`schema_version: 1`)

```jsonc
{
  "schema_version": 1,
  "event_id": "uuid-v4",                  // §4.4
  "project_item_id": "PVTI_...",          // OR roadmap_slug
  "roadmap_slug": "string",               // either id or slug must be present
  "channel": "live" | "beta" | "rc" | "alpha" | "tech-preview",
  "new_status": "building" | "beta" | "shipped",
  "commit_sha": "string",
  "build_id": "string",
  "ci_run_url": "string",
  "tag": "string|null",
  "public": true | false,                 // §4.3
  "coverage_delta": { "old": 0.0, "new": 0.0 } // optional
}
```

### 4.2 Delivery endpoint

`POST /v1/internal/roadmap/events` on `starstats-server`. Not part of the
public OpenAPI; lives under the `internal_routes` chain.

### 4.3 `public` in payload

The CI side reads the `Public` field from the Project item at emit time
and includes it on the event. The roadmap service also re-reads it via
GraphQL on receipt as a double-check. If they disagree, the GraphQL read
wins and the discrepancy is audit-logged.

### 4.4 Idempotency

- `event_id` is a UUID v4 generated CI-side per emission.
- The service maintains a `roadmap_event_log` table keyed on `event_id`
  with a TTL of 14 days.
- A second event with the same `event_id` is dropped (no state change,
  no audit) — but a tracing INFO is emitted.
- Retries of the same event are safe.

### 4.5 Source authentication

- HMAC-SHA256 with a shared secret in `ROADMAP_CI_EVENT_HMAC_KEY` (env on
  both the CI side and the server side). Signature carried in
  `X-StarStats-Signature` header. Constant-time compare. ±5 minute
  timestamp drift tolerance (mirrors Revolut webhook pattern,
  `revolut.rs`).
- Failed verifications return 401, are audit-logged, and do not advance
  any state.

### 4.6 Delivery guarantees

- The CI side guarantees **at-least-once** delivery via a retry-on-5xx
  loop with exponential backoff (3 attempts).
- The service handles duplicates via §4.4.

### 4.7 Schema versioning

- `schema_version: 1` is the current version. The event-contract doc
  lives in `docs/ROADMAP-PIPELINE-OPS.md` and is versioned independently
  from this spec.
- Adding a field is non-breaking and does not bump the version.
- Removing or renaming a field bumps the version. The server must support
  both versions for at least one release after the CI side cuts over.

---

## 5. Build / test / deploy surfacing

For each `RoadmapItem`, per channel, surface (read via the GitHub GraphQL
API for code state, and stored locally for deploy/pipeline state):

- **Last CI run:** status (`passing`/`failing`/`in-progress`), duration,
  run URL — read from GraphQL, cached for 60s.
- **Latest merged commit:** SHA (short), commit message, author handle,
  commit URL — read from GraphQL, cached for 5 min.
- **Deploy state:** last deployed commit on that channel, deploy
  timestamp, environment URL where relevant — stored locally from event
  ingestion.
- **Test coverage delta on the latest merged PR:** carried on the CI
  event payload (§4.1, optional).
- **Status strip on each item card:** per-channel chips coloured by
  combined `(status × build_health)`.

### Read budget

- Reconciliation: one GraphQL call returning all Project items'
  custom-field values + linked PRs' check-run rollups. Estimated 1
  request per 5 min = ~12 req/hr.
- Per-card detail (on-demand): one GraphQL call per item the user
  expands. Aggressive 60s cache.
- Total estimate: ≤500 req/hr in steady-state. Comfortably within 5000
  req/hr installation budget.

---

## 6. Voting + subscribing

### 6.1 Voting

- **Authenticated users only.** Anonymous voting is not supported.
  Sybil-resistance is delegated to the existing user-account system.
- One vote per user per `RoadmapItem`. Stored in a `roadmap_votes` table
  (`user_id`, `roadmap_item_id`, `created_at`).
- Vote can be retracted (`DELETE /v1/roadmap/:slug/vote`).
- Rate-limit: 30 votes per minute per user (well above any human, catches
  scripts).
- Vote counts are denormalised on `roadmap_items.votes` and recomputed
  on retraction/insert (the writeback worker also recomputes during its
  batch).

### 6.2 Subscribing

- Authenticated users only.
- `POST /v1/roadmap/:slug/subscribe` / `DELETE` to unsubscribe.
- Subscribers receive notifications on:
  - Headline `status` transition to `shipped` on any channel.
  - Headline `status` transition to `parked`.
  - Item being unpublished (§3.6).
- Notification delivery uses the existing notification path (dependency:
  item 13 in the parent roadmap list).

### 6.3 Writeback

- Batched every 5 min via a worker on `starstats-server`.
- One write per item per batch — coalesced, not per-vote.
- Writes `Votes` and `Subscribers` count fields on each Project item that
  has changed since the last batch.
- Writeback failures retry per backoff policy and audit-log on final
  failure.

### 6.4 Weekly digest

- A weekly cron posts a Project-item comment summarising vote delta and
  new-subscriber count over the past 7 days.
- **Only posts when the delta is nontrivial:** ≥3 votes OR ≥1 new
  subscriber. Items with no change get no comment.
- The saved Project view (`Votes > 50 AND Status = proposed`) is a
  Projects view configured manually on the board — the service does not
  maintain it.

---

## 7. Privacy & authorization

### 7.1 Public-vs-private filtering

- Public surfaces (`/roadmap`, tray "What's new"): only items with
  `public = true` AND `deleted_at IS NULL`.
- Admin/dev view: shows everything to staff. Authorization via the
  existing `staff_roles` table — minimum role `staff` to view, `admin`
  to manually override a status.

### 7.2 Subscriber privacy

- `subscribers` is a list of user UUIDs. It is never returned via any
  public API.
- The count is exposed publicly; the membership is not.
- Storing per-feature subscriber lists is itself sensitive (interest
  inference, e.g. "anti-cheat" subscribers). The data is access-controlled
  to: (a) the user themselves seeing their own subscriptions, (b) staff
  with the `roadmap-admin` permission.

### 7.3 Project visibility

- See §3.2. The Project itself MUST be private.

---

## 8. Auto-published changelog

### 8.1 Trigger + draft

When any channel's status flips to `shipped`:

- An entry auto-generates from merged PR titles between
  `previous_shipped_sha` and the new `commit_sha` on that channel
  (§2.1).
- Entry is drafted (not published) into a `roadmap_changelog` table.

### 8.2 PR-less commits

Commits made directly to a release branch (no PR) appear in the draft as
"Direct commit: <short-sha> — <commit subject>". They are not silently
dropped.

### 8.3 PR-to-item attribution

The auto-drafter attributes each PR to a roadmap item via, in order:

1. A `Roadmap-Item: <slug>` trailer in the PR description.
2. A linked Project item on the PR (GitHub native linkage).
3. A `roadmap/<slug>` label on the PR.

PRs with no attribution still appear in the changelog draft under
"Unattributed."

### 8.4 Admin publish flow

- An admin reviews the draft and publishes from the admin UI.
- On publish:
  - Entry appears at `/changelog` and in the tray "What's new" panel.
  - Subscribers to that roadmap item get a notification (§6.2).
  - Discord webhook (if configured via `ROADMAP_DISCORD_WEBHOOK_URL`)
    posts a short summary. Missing env → fail silently.
- Drafts auto-purge 30 days after creation if never published.

### 8.5 CI auto-publish via HMAC

For live releases, CI auto-publishes the just-drafted changelog entry
without needing a human admin in the loop.

- Endpoint: `POST /v1/internal/roadmap/changelog/publish`. Sibling of
  `POST /v1/internal/roadmap/events` — same auth scheme, same secret,
  same `internal/` path prefix.
- Authentication: `X-StarStats-Timestamp` + `X-StarStats-Signature`
  HMAC over `v1.<timestamp_ms>.<body>` (spec §4.5), signed with
  `ROADMAP_CI_EVENT_HMAC_KEY` (the same secret the emit endpoint uses).
  No JWT — chose HMAC specifically to avoid the ~1-hour expiry
  cadence that broke CI when JWTs were used.
- Body: `{ schema_version: 1, event_id, roadmap_slug, channel?, max_to_publish? }`.
  `roadmap_slug` is required; `channel` filters drafts to one channel
  (CI passes `live` so lingering alpha/beta drafts aren't surprised
  into publishing); `max_to_publish` caps the batch (server clamps
  to 50).
- Response 200: `{ published, skipped, entries: [{id, channel, title}] }`.
  Zero drafts is `published: 0`, NOT 404. 404 is reserved for
  unseeded slugs.
- Publisher attribution: entries published by this endpoint record
  `published_by = "ci"` in the `roadmap_changelog` row.
- Idempotency: per-entry publish is naturally idempotent (re-publishing
  a published row returns NotFound on the store, which the handler
  soft-skips). No event-id dedup at the endpoint layer.
- CI wiring: `scripts/auto-publish-changelog.mjs` is the canonical
  signer + caller. Mirrors `scripts/roadmap-emit-event.mjs`'s HMAC
  structure. The release.yml `roadmap-publish-changelog` job invokes
  it for live tray releases only.

### 8.6 `previous_shipped_sha` storage

- Stored on the `ChannelStatus` row (§2.1).
- Updated on each `shipped` transition: the value at the start of the
  transition becomes `previous_shipped_sha`, and `commit_sha` becomes the
  new shipped SHA.
- Initial value is NULL — the first shipped entry's diff falls back to
  "Initial release."

---

## 9. Tray "What's new" panel

- Caps at 3 unread top-level items with a "more on web" link.
- "Unread" is per-user state stored server-side in
  `roadmap_user_read_state` (`user_id`, `roadmap_item_id`,
  `last_seen_changelog_entry_id`, `last_seen_at`).
- State syncs across devices automatically because it lives on the
  server.
- Anonymous users (tray not yet paired): the panel shows the 3 most
  recently published changelog entries, with no read-tracking.

---

## 10. Dependencies

- Notification system (parent roadmap item 13).
- Audit log (parent roadmap item 12) — for sync failures, vote
  writeback failures, manual status overrides, GraphQL auth errors,
  signature verification failures, public→private transitions.
- Org / staff roles (parent roadmap item 5) — for who can see private
  items and trigger manual overrides.
- **CI event-emission step in the existing GitHub Actions workflow.** This
  is a hard dependency in `infra/` (or wherever the live release workflow
  lives — see `docs/RELEASING.md`). Owner: pipeline maintainer.
- **GitHub Project + App registration.** Out-of-code prerequisite; tracked
  in `docs/ROADMAP-PIPELINE-OPS.md`.

---

## 11. Acceptance criteria

### Data model

- [ ] Migration `0040_roadmap_pipeline.sql` exists, additive only, byte-immutable.
- [ ] `RoadmapItem`, `ChannelStatus`, `ChannelStatusArchive`,
      `RoadmapEventLog`, `RoadmapVote`, `RoadmapSubscriber`,
      `RoadmapChangelog`, `RoadmapUserReadState` tables created.
- [ ] `slug` is immutable post-creation; renames on the Project board
      do not change the slug (integration test).
- [ ] `content_last_updated` and `pipeline_last_updated` bump
      independently; vote writeback bumps neither.

### Status semantics

- [ ] Aggregation rule §2.3 correctly handles a mix of `parked` and
      non-`parked` channels (unit test with 4 channel permutations).
- [ ] Auto-advance never demotes status; a failing build on a shipped
      channel produces `build_health: failing` without changing `status`.
- [ ] Manual `parked` cannot be overridden by an inbound CI event.
- [ ] Component `parked` does not park the parent's roll-up.
- [ ] `beta` transition fires on a `-beta.N` tag deploy event for the
      named channel.

### Channel lifecycle

- [ ] Adding a channel to `Channels` on the board creates a
      `ChannelStatus` row with `status: proposed`,
      `build_health: unknown`.
- [ ] Removing a channel archives its `ChannelStatus` row; re-adding
      restores from the archive (vote history and `previous_shipped_sha`
      preserved).

### GitHub Projects sync

- [ ] GitHub App registered with `Projects: read & write` permission;
      credentials in env vars per §3.7.
- [ ] Webhook receiver verifies HMAC signature; failures audit-logged.
- [ ] Reconciliation recovers from a deliberately dropped webhook in a
      test scenario.
- [ ] Per-channel statuses render correctly when channels disagree
      (e.g. `shipped` on PTU, `building` on LIVE).
- [ ] `Public = true → false` transition: item disappears from public
      surfaces within one reconciliation cycle; existing subscribers
      receive a one-time "no longer public" notification and are
      removed from `subscribers[]`.
- [ ] Project item soft-deletes on GitHub deletion; soft-deleted items
      never appear on any surface.
- [ ] Component roll-up renders correctly for two seed items with
      natural sub-components.
- [ ] `Public = false` items never appear on public surfaces (privacy
      integration test).

### CI event ingestion

- [ ] HMAC signature verification on `POST /v1/internal/roadmap/events`
      with constant-time compare and ±5 min timestamp tolerance.
- [ ] Duplicate `event_id` is dropped without state change (idempotency
      integration test).
- [ ] Event without `event_id` is rejected with 400.
- [ ] `public` mismatch between event payload and GraphQL re-read is
      audit-logged; GraphQL value wins.
- [ ] Build-failure surfacing: failing CI on a channel surfaces
      `build_health: failing` on the roadmap item within 60s of the
      failure.

### Voting + subscribing

- [ ] Vote writeback batches every 5 min; an upvote appears on the
      Project item's `Votes` field within that window.
- [ ] Retracted upvotes net against the batch (an upvote-then-retract in
      the same window yields no writeback for that user).
- [ ] Rate-limit of 30 votes/min/user enforced; 429 on overflow.
- [ ] Anonymous vote attempt returns 401.
- [ ] Weekly digest comment only posts when delta ≥3 votes OR ≥1 new
      subscriber.

### Changelog

- [ ] Auto-drafted changelog correctly diffs merged PR titles between
      two `shipped` states on the same channel.
- [ ] Direct-commit (no PR) lines appear in the draft as
      `Direct commit: <sha> — <subject>`.
- [ ] PR attribution honours `Roadmap-Item:` trailer, then linked
      Project item, then `roadmap/<slug>` label, falling through to
      "Unattributed."
- [ ] Drafts auto-purge after 30 days unpublished.
- [ ] On publish, subscribers receive notifications; Discord webhook
      fires if configured, fails silently if not.

### Tray "What's new"

- [ ] Panel caps at 3 unread top-level items with "more on web" link.
- [ ] "Unread" state syncs across devices because it's server-side.
- [ ] Anonymous tray shows 3 most-recent published entries with no
      read-tracking.

### Seeding

- [ ] Items 2–9 of the parent roadmap list are seeded as Project items
      on first migration.
- [ ] Items 10–15 added as they're committed to.

---

## 12. Challenges (open risks)

- **GraphQL is new ground for this repo.** Allow setup time for a sensible
  client wrapper, error taxonomy, and auth rotation. The typed client pays
  off as soon as you're reading custom fields. Do not reach for `gh` CLI
  from CI shell scripts as a shortcut.
- **Vote writeback rate limits.** Batching every 5 min keeps this
  comfortable. If vote volume ever spikes, the writeback worker coalesces
  to one write per item per batch (already specified §6.3); the absolute
  ceiling is therefore the number of items, not the number of votes.
- **Component noise on the public view.** Default the public view to
  top-level items only with components expandable. Admin/dev view shows
  everything by default.
- **Event contract is the integration seam.** Because tag→channel logic
  lives in the existing CI ([release-workflow-mechanism]), the roadmap
  service is downstream of decisions it doesn't make. `schema_version` is
  on the payload from day one so the CI side can evolve independently
  (§4.7).
- **DR plan.** Roadmap DB is downstream of GitHub. Rebuild-from-GitHub is
  the DR path for `RoadmapItem` rows. Vote / subscriber / changelog
  history is NOT in GitHub authoritatively — those are backed up via the
  normal Postgres backup path.

---

## Cross-references

- Branch model + tag-channel mapping: `docs/RELEASING.md`,
  [release-workflow-mechanism].
- Migration discipline: project `docs/ENGINEERING.md` "Architecture Invariants."
- Store trait pattern: `share_metadata.rs`, `share_reports.rs`.
- HMAC webhook pattern: `revolut.rs`.
- Audit emission pattern: `audit.rs`, project `docs/ENGINEERING.md` "Audit
  emission is best-effort."
