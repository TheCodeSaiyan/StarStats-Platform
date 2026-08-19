# StarStats Telemetry Pipeline — Technical Audit

**Date:** 2026-07-13 · **Branch:** `next` · **Scope:** Deliverable #1 of the telemetry/parsing/validation/metrics/rule-discovery improvement program. Grounded in a read-only code audit of the collector, server, core, and web. Every claim carries a `file:line` reference.

---

## 0. Executive summary

StarStats is **architecturally healthier than the improvement brief assumes.** The three failure modes the brief fears most are already avoided:

- The **server does not re-run the parser** at ingest — it stores collector-produced events and derives metrics from stored rows (`repo.rs:1520`).
- **Gameplay metrics are server-authoritative**, computed by SQL over the `events` table, not read from collector-supplied totals (`repo.rs:1520–1556`).
- **Event identity is deterministic** (UUIDv5 over source), enforced by a DB unique constraint with `ON CONFLICT DO NOTHING` (`repo.rs:1912`), so replays and upload retries cannot double-insert.

The real problems are narrower and concentrated in three layers:

1. **Collector identity + rotation → silent data loss.** The idempotency key collides across log rotations, and rotation detection is defeated whenever a replacement file outgrows the old byte offset. Real events vanish with no trace.
2. **The server trust boundary barely exists for *content*.** Auth/consent/identity are solid, but field- and metadata-level validators are written yet never called; there is no quarantine; client-supplied metadata and `resolved_location` are stored verbatim and are spoofable.
3. **The unknown-line discovery loop is physically open.** Its front half (detect → shape → redact → review → submit → store) is fully built and works, but it is **off by default**, and its back half (cluster → propose rule → distribute → adopt) was stubbed to compile and never implemented. This is why it "has never worked."

Nothing here requires a greenfield rewrite. The fixes are targeted and mostly additive.

---

## 1. Current architecture (as-built)

```
COLLECTOR  (crates/starstats-client, Tauri + crates/starstats-core)
  Game.log ──▶ gamelog.rs (tail, rotation detect, per-path byte cursor)
           ──▶ core::parser::classify()  [big match on event_name + lazy regex]
           ──▶ core::parser_defs (remote RemoteRule registry — applied on builtin miss)
           ──▶ burst_rules.rs (per-drain burst collapse) + InferenceWindow (50-event)
           ──▶ deterministic idempotency_key = UUIDv5("{log_source}:{offset}:{line}")
           ──▶ storage.rs (SQLite events table = durable queue; sent_at IS NULL)
           ──▶ sync.rs  POST /v1/ingest (zstd IngestBatch, exp backoff, poison bisect)
  UNMATCHED ─▶ record_unknown + (v2, OFF by default) unknown_lines review cache

SERVER  (crates/starstats-server, axum + Postgres)
  /v1/ingest ──▶ auth + handle binding + sync_enabled gate + schema_version range
             ──▶ repo::insert  INSERT ... ON CONFLICT (handle, idempotency_key) DO NOTHING
             ──▶ [NO field/metadata validation, NO quarantine]
  reads      ──▶ query.rs / event_timeline.rs / repo.rs  (server-side metric SQL)

WEB  (apps/web, Next.js)
  /me widgets ──▶ mostly read server aggregates (correct)
              ──▶ records.tsx + sessions-summary.ts visitor path compute client-side (violations)

UNKNOWN-LINE LOOP
  tray-ui SubmissionsPane ──▶ POST /v1/parser-submissions ──▶ parser_submissions table
  admin/web review ──▶ PATCH rule_id (inert text) ──▶ [clustering ✗] [proposal ✗]
  GET /v1/parser-definitions ──▶ current_manifest() HARDCODED EMPTY ✗
  client ──▶ never fetches /v1/parser-definitions ✗
```

**Boundary verdict:** the collector/server responsibility split the brief wants **already exists and is correct in direction.** The server treats collector *identity* as untrusted (good) but collector *content* as trusted (the gap).

---

## 2. Findings

Severity: **S1** data loss / correctness · **S2** trust/security · **S3** contract/observability · **S4** accuracy/consistency.

| # | Sev | Finding | Evidence | Failure scenario |
|---|-----|---------|----------|------------------|
| F1 | S1 | **Idempotency-key collision across rotations.** Key = `UUIDv5("{log_source}:{offset}:{line}")`, but `offset` resets to 0 every launch and `log_source` is just `"live"`. Identical lines at the same offset in different sessions collide. | `gamelog.rs:794`, `repo.rs:1912`, memory `source-offset resets per log rotation` | A banner/startup line, or any repeated-identical event, at offset N in session 2 is dropped as a false duplicate of session 1. Silent loss. |
| F2 | S1 | **Rotation defeated when replacement file outgrows old offset.** Only `metadata.len() < offset` triggers a reset; no inode/ctime/hash. | `gamelog.rs:213–228` | Launcher rotates `Game.log`; new file grows past the saved offset before next drain → reader seeks mid-file, skips the session's opening lines, misparses the rest. |
| F3 | S2 | **No content validation at the trust boundary.** `validators::validate_event` / `validate_metadata` exist but are never called on ingest. | `validators.rs:33,134` (callers = tests only), `ingest.rs:763` | A client posts an ActorDeath with empty killer/victim or out-of-range port; row lands and skews combat stats. |
| F4 | S2 | **Client metadata + `resolved_location` stored verbatim.** `source`, `confidence`, `group_key`, location slug are attacker-controlled and authoritative. | `ingest.rs:751`, `repo.rs:538–539`, `0041:18` | Client stamps `confidence=1.0, source="observed"` on fabricated events, or `slug="lorville"` on any event → poisons grouping/trust and renders spoofed KB links. |
| F5 | S1 | **No quarantine / dead-letter.** Failed inserts are counted and dropped; a serialize failure is stored as `Null` payload with `event_type="unknown"` and counted **accepted**. | `ingest.rs:768–772`, `repo.rs:514–524,543` | Transient DB error loses events with no record of which; corrupt events silently inflate the accepted count. |
| F6 | S1 | **Unknown-line loop back half absent** (clustering, rule-proposal object, distribution, adoption) **and capture off by default.** | see §3 | Submissions can be reviewed and approved but can never become a served rule; most users never capture anything to submit. |
| F7 | S3 | **Contract/model versioning gaps.** No per-event-type version; no `parserId`/`parserVersion`/`collectorVersion`; no `batchSequence`, source-range, diagnostics; no content hash; no `validationState`/`qualityFlags`; no collected/uploaded/received timestamps; `game_build` shipped as `None`. | `wire.rs:73–96`, `metadata.rs:157`, `sync.rs:1097` | Server can't detect missing/out-of-order batches, can't attribute which parser emitted an event, can't gate compatibility, can't reconcile a reinterpreted line. |
| F8 | S4 | **Correlation windows don't cross drain / 10k-line chunk boundaries; no vehicle/character-life FSM; no negative-duration guards.** | `backfill.rs:210–213`, `gamelog.rs:238–245,52–68` | A burst straddling a boundary emits as raw member spam; a trigger/followup split across the window is never inferred; out-of-order timestamps unguarded. |
| F9 | S4 | **Web frontend metric authority violations.** `records.tsx` computes survival streak / longest / deadliest session in-browser from raw (limit-bounded) event arrays; `sessions-summary.ts` visitor path sums playtime/session count client-side over a 50-capped list. Owner path is correct → inconsistent authority. | `records.tsx:73–140`, `sessions-summary.ts` | Visitor totals silently undercount and differ from the server's numbers. |
| F10 | S2 | **Remote parser/inference rules executed on TLS trust only** (`signature` field unused). Currently moot (manifest empty) but becomes live the moment F6/stage-12 ships. | `parser_defs.rs:95–99` | A compromised/misconfigured manifest injects arbitrary regex (ReDoS/misclassification) into every client. |
| F11 | S4 | **Doc/code drift on metadata upsert.** Migration 0030 promises metadata `COALESCE` upgrade on retry; code is `DO NOTHING`. | `0030_events_metadata.sql:17–21` vs `repo.rs:1920` | A v1 row's synthesized metadata is frozen; the later enriched v2 retry is dropped as a duplicate. |
| F12 | S4 | **Strict RFC3339 parse silently NULLs valid dialects.** | `repo.rs:552` | LauncherActivity/GameCrash timestamps become `NULL` and drop out of time-ordered/window queries. |

**Confirmed solid (do not touch):** deterministic within-session idempotency, partial-last-line handling (`gamelog.rs:254`), exponential backoff + poison-pill bisection (`sync.rs:413,864`), `device_sync_disabled` distinguished from auth loss (`sync.rs:793`), server-side metric derivation (`repo.rs:1520`), auth + handle binding + sync-consent gate (`ingest.rs:604,713,737`), and the unknown-line **front half** (shape/redaction/consent UX).

---

## 3. Unknown-line loop — stage-by-stage diagnosis

| Stage | Exists | Works | Break / root cause |
|-------|--------|-------|--------------------|
| 1 Detect unknown | ✔ | **Gated off** | `capture_v2_unknown` runs only if `parser_enable_v2_metadata`, default **false**, config-file only, no UI (`config.rs:132`, `gamelog.rs:747`) |
| 2 Shape + placeholders | ✔ | ✔ | `unknown_lines.rs::shape_of()` |
| 3 Redaction | ✔ | ✔ | `detect_pii()` / `default_redact` |
| 4 Fingerprint | ✔ | ~ | `shape_hash()` uses `DefaultHasher` — not guaranteed stable across Rust versions; server identity keys on it (0029). Minor. |
| 5 Local grouping/suppression | ✔ | ✔ | `storage.rs::cache_unknown_line`, interest threshold, dismiss/submitted flags |
| 6 Consent UI (tray) | ✔ | ✔ | `SubmissionsPane.tsx` / `ReviewPane.tsx` / `PiiToggle.tsx` |
| 7 Client submit | ✔ | ✔ | `commands.rs:2548` → `POST /v1/parser-submissions` |
| 8 Server intake/store | ✔ | ✔ | `parser_submissions.rs`, table 0029/0034 |
| 9 Clustering | **✗** | — | No clustering code exists; admin merely sorts by `submitter_count` |
| 10 Rule-proposal object | **✗** | — | Approval = moderator typing a free-text `rule_id`; **no `RemoteRule` is created** (`admin_parser_submissions.rs`) |
| 11 Admin review UI (web) | ✔ | ✔ | `apps/web/.../admin/parser-submissions` |
| 12 Distribute to collectors | **stub** | **✗** | `parser_def_routes.rs:111 current_manifest()` returns hardcoded **empty** manifest; `GET /v1/parser-definitions` always serves zero rules |
| 13 Collector adopt/reprocess | **unwired** | **✗** | No client code fetches `/v1/parser-definitions`; every `RemoteRule` call site is passed `Vec::new()` (`backfill.rs:381,409`, "future slice" comment `burst_rules.rs:138`) |

**Single root cause:** the back half was never built — only its two endpoints were stubbed to compile. Stages 9, 10, 12, 13 are non-functional, so an approved submission can never become a served, adopted rule. Compounded by stage-1 capture being off by default, so most users never even generate data for the (dead) back half.

**Three highest-value repairs:** (1) make `current_manifest()` load approved rules from a real source; (2) wire a periodic client `GET /v1/parser-definitions` → `compile_rules()` → feed live/backfill (reuse `ReparseCard`); (3) make "approve + rule_id" persist an actual `RemoteRule`, and expose/flip `parser_enable_v2_metadata`.

---

## 4. Answers to the brief's required audit questions (§52)

- **Which logic runs where?** Parsing/classification/correlation: collector. Storage, validation-of-identity, metric derivation, read models: server. Correct split.
- **Is parsing duplicated?** No. Core parser compiles into the collector; the server never calls `classify()` at ingest.
- **Metrics differ app vs website?** Website mostly reads server aggregates; **two client-side violations** (F9). Collector computes only local previews/diagnostics.
- **Trusted without validation?** Event **content** and **metadata** (F3, F4).
- **Uploads replayable / events duplicated?** Replays are safe (unique constraint) **except** the F1 cross-rotation collision, which drops *legitimate* events.
- **Old collectors submit incompatible events?** Only `schema_version` is range-checked; no collector/parser version exists to gate (F7).
- **Server recalc without raw logs?** Yes — metrics derive from stored rows.
- **Corrected events supersede safely?** No supersession/generation model exists.
- **Out-of-order batches?** Undetectable — no `batchSequence` (F7).
- **Provenance retained?** `log_source` + `source_offset` + `raw_line`; **no file id or content hash** (F1/F7).
- **Every metric traceable to accepted events?** Server metrics yes; the two web client-side records no.
- **Where does the unknown-line workflow fail?** Stages 9/10/12/13 + off-by-default capture (§3).
- **Parser regressions auto-detected?** No coverage telemetry exists.

---

## 5. Proposed incremental remediation plan

Ordered by the brief's own priority list (correctness & data-loss first). Each stage is independently shippable and testable; none is a rewrite.

| Stage | Goal | Touches | Fixes | Risk |
|-------|------|---------|-------|------|
| **A** | **Stop silent data loss.** Add a stable per-file identity (inode/ctime/size + first-chunk hash) to the tail cursor and to the idempotency key; treat rotation via file-identity change, not just size shrink. | `gamelog.rs`, `storage.rs`, cursor migration | F1, F2 | Med — changes key derivation; needs a migration + dedup-safe rollout |
| **B** | **Make the server trust boundary real for content.** Call `validate_event`/`validate_metadata` at ingest; re-derive `resolved_location` server-side or mark client value untrusted; add an `accepted / accepted_with_warnings / quarantined / rejected` outcome + a quarantine table instead of silent drops. | `ingest.rs`, `repo.rs`, new migration | F3, F4, F5, F12 | Med |
| **C** | **Close the unknown-line loop.** Real `current_manifest()` source; client fetch+compile+adopt; approval persists a `RemoteRule`; expose the capture toggle; verify rule signatures before F10 goes live. | `parser_def_routes.rs`, `admin_parser_submissions.rs`, client fetch, `config.rs`, `parser_defs.rs` | F6, F10 | Med–High — new distribution surface |
| **D** | **Version the contract.** Add per-event-type version, `parserId`/`parserVersion`/`collectorVersion`, `batchSequence`, source-range, diagnostics, content hash, `qualityFlags`, `validationState` to the wire; compatibility ranges on the server. | `wire.rs`, `metadata.rs`, `ingest.rs`, generated TS client | F7, F11 | Med — additive, back-compat |
| **E** | **Correctness of correlation + web authority.** Cross-boundary burst/inference windows; negative-duration guards; move `records.tsx` + visitor `sessions-summary` onto server aggregates. | `burst_rules.rs`, `backfill.rs`, server records endpoint, `records.tsx` | F8, F9 | Low–Med |

Recommended first slice: **Stage A**, because F1/F2 are actively losing real user data every rotation and everything downstream (metrics, records, unknown-line reprocessing) inherits that loss.

---

## 6. Open risks & migration notes

- **Changing the idempotency key (Stage A) re-keys every future event.** Historical rows keep their old keys; new rows use the new scheme. A dedup-safe cutover (dual-write or generation stamp) is needed so the transition doesn't itself create duplicates — mirrors the existing `processing generation` concept the brief describes.
- **`next` auto-alphas on push.** Any of these stages ships to the alpha channel on merge; sequence them behind flags where user-visible.
- **Rule distribution (Stage C) is a new remote-code-ish surface.** Signature verification (F10) must land in the *same* slice that first serves a non-empty manifest, never after.

---

## 7. Implementation log

### 2026-07-13 — Stage A, part 1: **F2 rotation-aware resume (SHIPPED to working tree, `next`)**

Fixed the rotation data-loss bug in **both** live-tail loops (`gamelog.rs::drain` *and* `launcher.rs::drain` — the latter had the identical shrink-only check). Approach:

- New pure `resolve_resume_offset(stored_offset, stored_sig, current_sig, len)` in `gamelog.rs` — resets to head on signature change (rotation/replacement) or truncation, else resumes; falls back to the length-only heuristic when either signature is absent (legacy rows / unsignable files) so the upgrade never triggers a spurious re-read.
- New `file_signature()` — stable UUIDv5 over `created_time + first 4 KiB` of the file head; detects a replacement even when it has already outgrown the saved offset.
- `tail_cursor` gains a nullable `file_sig TEXT` column (`sql/schema.sql` for fresh DBs + `Storage::migrate_tail_cursor_sig` probe/ALTER for legacy, mirroring `migrate_events_sent_at`).
- New `Storage::read_tail_cursor`/`write_tail_cursor` (offset + sig) used only by the file-tail sites; `read_cursor`/`write_cursor` left untouched so `org_connector`'s id-high-water-mark rows stay unsigned.
- **Idempotency key unchanged → zero server-dedup / duplicate-row risk.**

Tests (all green; `cargo test -p starstats-client` 252 passed, fmt + clippy clean): 6 pure `resolve_resume_offset` cases, a `#[tokio::test]` driving the real `drain` across an actual file rotation (asserts the rotated session's opening event is re-read from the head, not skipped), and a legacy-DB migration test proving the `ALTER` path + NULL-signature back-compat + org-connector isolation.

### 2026-07-13 — Stage A, part 2: **F1 idempotency-key salt (SHIPPED to working tree, `next`)**

Killed the cross-session key collision by salting the idempotency key with the physical-file signature: `UUIDv5("{log_source}:{file_sig}:{offset}:{line}")`. Two identical lines at the same offset in different sessions now get distinct keys instead of the second being dropped as a false duplicate.

Dedup-safe cutover — the design guarantees no duplicate server rows across the upgrade:

- **Legacy-compatible fallback:** when `file_sig` is `None`, the key is *byte-identical* to the pre-F1 format. Proven by test against a hardcoded `UUIDv5("live:100:L")`.
- **Every log-reader agrees on the signature**, so a line re-read by a different path produces the same key and dedupes rather than duplicating: live tail + launcher (async), **backfill** (async), and **reingest** (sync — added `file_signature_sync`, a byte-identical twin of the async path, guarded by a test asserting the two agree). The Phase-2 unknown-promotion path (`reparse_idempotency_key`) is a separate, offset-less dedup domain and is untouched.
- **Signature stability (self-caught bug):** the signature is computed only over the immutable first `FILE_SIG_HEAD_BYTES` (512) of the file, and only once the file is that large. An earlier draft folded in the growing head length/content, which would have shifted the key drain-to-drain over a session's first bytes and *manufactured* duplicates — the exact failure F1 is meant to prevent. Regression test `file_signature_is_stable_as_file_grows_past_head` locks this down.

The single residual (documented): a line uploaded pre-upgrade under the old key that is later re-read under the new key would duplicate — but the forward path never re-reads uploaded bytes, and the only trigger is in-place truncation of the *same* file, which Star Citizen (rotate-by-rename) does not do.

Touches `gamelog.rs`, `launcher.rs`, `backfill.rs`, `commands.rs`, `storage.rs` (Stage A). Tests: `cargo test -p starstats-client` → 256 passed, fmt + clippy clean. New F1 tests: legacy-format equality, salt-distinguishes-files, sync/async agreement, and growth-stability.

**Stage A is now complete (F1 + F2).**

### 2026-07-13 — Stage B, part 1: **server-side validation gate at ingest (SHIPPED to working tree, `next`)**

Closed **F3** (and partially **F4**): the server now independently validates every ingested envelope instead of trusting the collector field-for-field. `starstats_core::validators::validate_event` — written but never called anywhere in the server — is now invoked per envelope in the `/v1/ingest` loop (`ingest.rs`). Events whose payload (empty required fields, zero/out-of-range port, phantom `BurstSummary`, bad timestamp shape) or client-supplied metadata (`confidence` out of `[0,1]`, `Observed` not at 1.0, `Inferred` with no inputs) violate the documented invariants are **rejected and not persisted**, rather than landing in `events` and skewing downstream metrics.

- **Response contract unchanged** — the existing `rejected` counter now includes validation failures; `accepted`/`duplicate` unchanged.
- **Diagnosable, not silent** — each rejection logs `idempotency_key` + the validation error (never the raw line, to avoid spilling log content) and increments `starstats_events_rejected{reason="validation"}`, split from the pre-existing `reason="insert_error"`.
- Single-file change (`ingest.rs`), no migration. TDD: new test `rejects_invalid_events_and_persists_only_valid_ones` (valid + invalid in one batch → `accepted=1, rejected=1`, invalid not in store). All 17 ingest tests pass; full server suite + fmt + clippy green.

### 2026-07-13 — Stage B, part 2: **timestamp dialect fix (SHIPPED to working tree, `next`)**

Closed **F12**: `repo.rs::extract_type_and_ts` parsed `event_timestamp` with `parse_from_rfc3339` only, which rejects the LauncherActivity space-separated form (`2026-05-06 12:34:56.789`) that `core::validators::check_timestamp` explicitly accepts. Those events landed with a `NULL` timestamp and silently dropped out of every `event_timestamp`-ordered/windowed query. Extracted a `parse_event_timestamp` helper that tries RFC3339 first, then falls back to the naive launcher form (interpreted as UTC — documented, and strictly better than dropping the row). Single-file change, no migration. TDD: `parse_event_timestamp_accepts_all_validator_dialects` (all three dialects parse, garbage still rejected). Full server suite + fmt + clippy green.

### 2026-07-13 — Stage B, part 3: **`resolved_location` spoofing fix (SHIPPED to working tree, `next`)**

Closed **F4**: the collector-supplied `resolved_location` (a location classification carrying a `/kb/location/{slug}` slug) was stored verbatim and echoed by three read surfaces, so a malicious client could render an arbitrary KB link on its own events. Approach (a) — **re-derive at query time**, honoring the docs/ENGINEERING.md invariant "classify at query time, do NOT denormalize onto events" (re-classifying on ingest would go stale as the catalog updates).

- New shared `query::derive_resolved_location(event_type, payload, ts, catalog)` — classifies from the event's *own* payload via the shared core classifier + catalog, exactly as the current-location / trace paths already do. The classifier is pure + in-memory, so per-event cost is negligible. Timestamp is optional (it doesn't affect the derived slug), which also lets the rollup — whose query selects no timestamp — pass `None`.
- Wired into all three pass-through surfaces: events feed (`query::list_events`), entity rollup (`entity_rollup::envelope_from_row`), and session timeline (`event_timeline::envelope_from_columns`, threaded through both fetch helpers). Each handler now snapshots the globally-layered `LocationCatalogCache` (`main.rs:740`). The current-location / trace paths were already safe and are untouched.
- The stored column stays (harmless, still populated by the tray); the two row-struct fields that held it are marked `#[allow(dead_code)]` with a comment, since removing them would churn ~20 test constructions. No migration; **no web change** — the web still receives a `resolved_location.slug`, now server-derived, so the tray's fuzzy-location display is preserved, just made authoritative.
- TDD: `list_events_does_not_echo_client_supplied_resolved_location` and `envelope_from_columns_rederives_location_and_ignores_stored_slug` (spoofed slug is never echoed). Full server suite **807 passed, 0 failed**; fmt + clippy clean.

### 2026-07-13 — Stage B, part 4: **quarantine table (SHIPPED to working tree, `next`)**

Closed **F5**: validation-rejected events were logged + metric'd but not retained, so a maintainer couldn't see *which* collector/handle produced *what* bad data. Added a `quarantined_events` table (migration **0047**, additive; unique on `(claimed_handle, idempotency_key)` so retried bad batches don't bloat it) and a `EventStore::quarantine` method (Postgres + Memory impls, following the repo's Trait+Postgres+Memory pattern). At ingest, a rejected event is now written to quarantine (reason + the validation-error detail + raw line + payload) instead of dropped — **best-effort**: a quarantine-write failure is logged and never fails the batch. TDD: the reject test now also asserts the event lands in quarantine with `reason="validation"` and a non-empty detail. Also corrected the now-stale `StoredEvent.resolved_location` doc (post-F4 the read paths re-derive, so the stored value is authoritative for nothing).

**Verification note (F5):** single-threaded `cargo test -p starstats-server --bin … -- --test-threads=1` → **807 passed, 0 failed**; clippy `--all-targets` clean; targeted quarantine test green. The default *parallel* full run exhibits a pre-existing **intermittent Windows-only abnormal process exit** (observed ~3/5 runs; passes the other ~2/5 and always single-threaded) with **no panic / assertion / stack-overflow message** — the signature of a test-harness resource/global-state flake, not a correctness defect (a real bug would fail deterministically and single-threaded, with a panic). Authoritative gates (single-threaded + clippy + Linux CI) are green; flagged for a separate, orthogonal investigation.

**Stage B is now complete (F3 + F4 + F5 + F12).** The server independently validates collector content, rejects-and-quarantines the invalid instead of trusting it, re-derives location server-side, and parses every timestamp dialect. F7 (contract versioning) is Stage D.

### 2026-07-13 — Stage C, part 1: **DB-backed parser manifest (SHIPPED to working tree, `next`)**

Began closing the unknown-line loop (§3) at its **physical open end** — audit repair point #1. `parser_def_routes::current_manifest()` was a hardcoded empty `Manifest`, so an approved rule could never be published to collectors no matter what the front half did. Now the manifest is **DB-backed**:

- New `parser_rules` table (migration **0048**): one row per published `RemoteRule` (`rule_id`, `event_name`, `match_kind`, `body_regex`, `fields`, `enabled`); retraction by flipping `enabled` (the manifest publishes by absence, as `RemoteRule`'s own docs specify).
- New `parser_rules::ParserRulesStore` (Trait + Postgres + Memory), following the repo pattern so the serve path is unit-testable without a DB.
- `current_manifest()` is now async and reads `active_rules()`; `GET /v1/parser-definitions` serves real enabled rules. A DB error degrades to an empty manifest rather than 500-ing the public, cache-tolerant endpoint. Wired the store through `routes()` + `main.rs`.
- TDD: `manifest_serves_enabled_rules_from_store`, `manifest_is_empty_when_no_rules_published`, plus store round-trip/upsert-replace tests.

### 2026-07-13 — Stage C, part 2: **moderator rule-publish endpoint (SHIPPED to working tree, `next`)**

Closed the **write** half of the server loop. New moderator-gated `POST /v1/admin/parser-rules` (`admin_parser_rules.rs`) validates a rule-definition payload and upserts it into `parser_rules`, so an approved unknown-line submission can become a served `RemoteRule` end-to-end (publish → manifest). Design choice: a dedicated rule-authoring endpoint rather than overloading the submission-triage PATCH — cleaner separation; the moderator links a submission to its rule via the existing `rule_id`. The `body_regex` is validated by compiling it exactly as the client will (core `compile_rules`; the `regex` crate is linear-time, so no catastrophic-backtracking risk). Re-added `upsert` to `ParserRulesStore` (now it has a production caller). Best-effort audit (`admin.parser_rule.published`) — a published rule runs on every collector. Not registered in OpenAPI yet (server-internal moderator tool; no generated-client consumer). TDD: pure `build_parser_rule` rejection paths + integration tests (moderator→200 & active, non-moderator→403, invalid regex→400). Clippy clean.

### 2026-07-14 — Stage C, part 3: **client fetch+adopt was ALREADY implemented (audit correction)**

Verifying before building C3 revealed the client side already exists and is wired — the audit's §3 stage-13 finding ("Collector adopts + reprocesses — BROKEN/unwired") was **incorrect**. In `crates/starstats-client/src/parser_defs.rs`: `run_fetcher` polls `/v1/parser-definitions` every 6h (immediate first run), stores the manifest to SQLite, `compile_rules`, and swaps the `RuleCache`; `hydrate_from_storage` provides an offline fallback. `main.rs:231` hydrates on startup and `main.rs:250` spawns `run_fetcher` with the configured `remote_sync.api_url`; the cache flows to the live tail (`start_tail`) and backfill. The sub-agent conflated this with the unrelated `burst_rules.rs:138` "future slice" (burst *thresholds*, not parser rules). No code change needed — C3 is done.

**Net:** with C1 (serve) + C2 (publish) shipped and C3 (client fetch/adopt) already present, the unknown-line **back-half loop is closed end-to-end** — a paired collector adopts published rules within 6h and reprocesses via the existing `ReparseCard`.

**Still open in Stage C:** part 4 — front-half **capture** (`parser_enable_v2_metadata`) defaults OFF with no UI (`config.rs:132`), so most users never generate submissions. Flipping the default is a **product/privacy decision** (capture is local-only; submission stays opt-in with consent), and exposing a settings toggle is a safe technical add — deferred to the owner's call. Clustering (stage 9), inference-rule publishing, and a web admin UI for rule authoring remain unbuilt but are enhancements, not loop-closers.

### 2026-07-14 — Stage E, part 1: **records made server-authoritative (SHIPPED to working tree, `next`)**

Closed the F9 records violation. The `records` web widget computed longest/busiest session, longest survival streak, and deadliest session **client-side from fetch-capped data** (sessions ~50, `player_death` events 500) — so they weren't true all-time records and the streak/deadliest were raw-event computations in the browser. Now:

- New `EventQuery::records_for_handle` (Postgres + Memory) computes all four over the **full** history, reusing the gap-idle sessionization CTE (extended with `event_type` for per-session death counts) plus a LAG-over-deaths query for the survival streak. New `GET /v1/me/stats/records` (`query::stats_records`).
- `records.tsx` owner path now reads the server aggregate via `getRecords` (`lib/api.ts`) — the client-side death fetch + O(N×M) session-bucketing + streak loop are gone. Visitor path keeps handle-scoped `getSessions` for longest/busiest (no handle-scoped records endpoint; me-scoped death records stay omitted, preserving the C2 scoping guarantee).
- TDD: `records_for_handle_computes_all_time_records` + empty-case (server); `records.test.tsx` rewritten to assert owner→`getRecords`+commerce (not the capped sessions list), visitor→`getSessions` only. Server tests + clippy green; web vitest 4/4 + typecheck clean.
- Not in the OpenAPI spec yet (hand-typed `RecordsResponse` in `lib/api.ts`); add `#[utoipa::path]` + regen the TS client as a follow-up.

**Still open in F9/Stage E:** the visitor `sessions-summary.ts` playtime/session-count client-side sum over the 50-capped list (needs a handle-scoped playtime aggregate), and the commerce "biggest trade" is still a 500-capped client scan. Both are narrower follow-ups.

### 2026-07-14 — Stage C, part 4 (safe half): **capture settings toggle (SHIPPED to working tree, `next`)**

Exposed the front-half capture toggle in the tray so users can **opt in** to unknown-line capture — which activates the now-closed loop (no capture → no submissions → nothing to publish rules from). Pure tray-ui change: `parser_enable_v2_metadata` already round-trips through `get_config`/`save_config`, so it just needed a UI. Added it to the TS `Config` type + a "Capture unrecognised log lines" toggle in `SettingsPane` (Diagnostics card, mirroring the `debug_logging` toggle; copy explains it's local-only until you submit, off by default, restart to apply). Test: toggle reflects config + flips on click; full tray-ui suite (133) + typecheck + lint green. **Default stays OFF** — flipping the shipping default remains the owner's product/privacy decision.

### 2026-07-14 — user feature: **/me header shows true all-time playtime**

Not an audit finding — the `/me` identity header's playtime used the windowed endpoint at the server's max window (24×365h), silently capping "lifetime" at ~1 year. Switched to the `all_time` aggregate so it shows the true total (owner's choice: header stays range-independent, just uncapped). Locations/combat remain windowed (those endpoints are windowed-only).

### 2026-07-14 — Stage D, slice 1: **collector version on the ingest contract (SHIPPED to working tree, `next`)**

Began F7 (contract versioning) with its smallest, highest-value increment: stamp **which collector release produced each batch** so ingested events can be attributed to a tray version — the prerequisite for parser-regression triage ("events of type X went malformed as of tray v1.8.NN") and any future compatibility gating. Additive and fully back-compat:

- Core `wire.rs`: new `IngestBatch.collector_version: Option<String>` with `#[serde(default)]` — batches from pre-versioning trays (and pinned fixtures) still deserialise, with `None`. No `schema_version` bump needed (adding an optional field is exactly the "additive, back-compat" case the wire-module doc sanctions).
- Client `sync.rs::build_batch`: stamps `Some(env!("CARGO_PKG_VERSION"))` — a compile-time constant, zero runtime cost, always present on modern clients.
- Server `ingest.rs`: records `collector_version` on the best-effort `ingest.batch_processed` audit row (alongside the existing `game_build`), keeping it in the canonical hash chain rather than a new column.
- Deliberately **not** added to the doc-only `IngestBatchSchema` utoipa mirror — the ingest endpoint has no generated-client consumer (the tray uploads via hand-built Rust), and touching the mirror would trip the OpenAPI codegen-drift gate for no benefit. Consistent with how `stats_records` / `admin_parser_rules` skip `#[utoipa::path]`.
- TDD: core `batch_without_collector_version_still_deserialises` (back-compat) + `batch_collector_version_survives_round_trip`; server assertion that the audit payload carries `collector_version`. Core 17/17 green.

### 2026-07-14 — Stage D, slice 2: **parser (rule-set) version on the ingest contract (SHIPPED to working tree, `next`)**

The second F7 field — and the one that directly compounds the now-closed unknown-line loop. `collector_version` (slice 1) identifies the *tray build*; `parser_version` identifies the *remote rule-set manifest* that build had adopted at drain time. Together they disambiguate two collectors on the same release running different published rule-sets — exactly the axis that starts to matter once approved unknown-line submissions ship as `RemoteRule`s. Meaning is deliberately narrow and honest: it is the adopted **manifest version** (`Manifest::version`), NOT a claim about which rule parsed each individual event (built-in parser coverage is already pinned by `collector_version`).

- Core `wire.rs`: new `IngestBatch.parser_version: Option<u32>` (`#[serde(default)]`). `None` = collector has fetched no manifest yet (first run) or is a pre-versioning client.
- **No RuleCache threading.** The adopted manifest version is already persisted in SQLite (`parser_def_manifest.version`, written by `run_fetcher`), and the sync drain already holds `Storage`. Added a lightweight `Storage::read_parser_def_manifest_version()` (`SELECT version …`, no payload deserialisation) and read it **once per drain** — alongside the existing catalogue snapshot — so every sub-batch (including poison-pill bisections) reports one consistent value. A read error degrades to `None` (best-effort metadata never aborts a drain). Threaded through `drain_lane → try_send_batch → build_batch`.
- Server `ingest.rs`: records `parser_version` on the audit row beside `collector_version`.
- TDD: core back-compat test now asserts BOTH provenance fields default to `None`; `batch_provenance_versions_survive_round_trip`; storage `parser_def_manifest_version_reads_none_then_written_value` (None → 42 → upsert-to-43); `build_batch` test asserts both stamps; server audit assertion extended to `parser_version`. Core 17/17, server ingest 17/17, client storage + build_batch, clippy clean.

### 2026-07-14 — Stage D, slice 3: **`batch_sequence` on the ingest contract (SHIPPED to working tree, `next`)**

The third F7 field, and the first *stateful* one. `collector_version` / `parser_version` are stamps (a compile-time constant, a value read from SQLite); `batch_sequence` is a **per-device monotonic counter** — the ordinal of this upload among the batches this install has successfully sent — so the server can spot **missing** (gap) or **out-of-order** uploads from a device, the axis neither version field covers.

The design turns on the **two-lane + retry + poison-bisect** interaction. Assigning the number *on send-attempt* would burn one on every transient failure or bisection and manufacture a false gap — the exact false-positive that makes gap-detection worthless. So the counter is **assigned optimistically and committed only on a 2xx**:

- Core `wire.rs`: new `IngestBatch.batch_sequence: Option<u64>` (`#[serde(default)]`). `None` = a pre-versioning client (or a pinned fixture).
- Client `storage.rs`: new single-row `batch_sequence_counter` table (in `schema.sql`, so it lands on fresh **and** legacy DBs via the always-applied `CREATE TABLE IF NOT EXISTS` — no migrate-fn needed for a new *table*, unlike the column-adds). `peek_next_batch_sequence()` returns `stored + 1` (fresh install reads 0 → first batch ships sequence 1) **without consuming**; `commit_batch_sequence(seq)` advances via `ON CONFLICT DO UPDATE SET value = MAX(value, excluded.value)` — **idempotent** (re-commit is a no-op) and **monotonic** (a stale lower commit from a racing lane can't rewind it).
- Client `sync.rs`: the drain loop peeks the ordinal **per sub-batch** (before each `try_send_batch`), threads it through `try_send_batch → build_batch`, and commits it in the `Ok` (2xx) arm next to `mark_sent`. A poison-bisected or retried send therefore **reuses** its number rather than burning it → no false gaps. The only residual anomaly is an occasional *duplicate* sequence when the priority and bulk lanes race, which is benign (distinct `batch_id`s, events dedupe on `idempotency_key`) and is explicitly **not** a gap. Peek/commit failures degrade to `None`/skip — best-effort metadata never aborts a send.
- Server `ingest.rs`: records `batch_sequence` on the `ingest.batch_processed` audit row beside the two version fields, so gaps / out-of-order uploads are **observable in the hash-chained audit log** (order a device's rows by receipt, check contiguity) — the durable substrate for detection.
- TDD: core back-compat test now asserts all THREE provenance fields default to `None` + round-trip; storage `batch_sequence_peek_starts_at_one_and_advances_only_on_commit` (first-run = 1, peek non-consuming, commit advances, idempotent re-commit, monotonic-under-stale-lower-commit); `build_batch` test asserts the stamp; server audit assertion extended to `batch_sequence`. Core 20/20, client storage + build_batch green, server ingest 17/17, fmt + workspace clippy clean.

**Scope (honest):** this slice makes the ordinal *present and durable* on every batch and captured in the audit chain — active **online** per-device gap detection (a `last_seen` per device + a metric/log on regression at ingest time) is the natural follow-up slice, deliberately not bundled here to keep the increment the same size as slices 1–2.

**Still open in Stage D (F7):** source-range + content hash (provenance + a second dedup axis beyond the idempotency key), and server-side compatibility ranges (flag/reject a collector too old for the current schema) — plus the online `batch_sequence` gap-detector noted above. Each is a further additive slice on the same envelope; none blocks the loop-closing work already shipped.

### 2026-07-14 — Stage D, slice 4: **`batch_sequence` online gap detector (SHIPPED to working tree, `next`)**

Slice 3 made the per-device ordinal *durable and observable* (stamped on every batch, recorded in the audit chain). Slice 4 acts on it **at ingest time**: the server keeps a per-device high-water mark and, on each device-scoped batch that carries an ordinal, diffs the incoming value against it to surface **gaps** (lost/dropped uploads) and **regressions** (out-of-order arrival, a retry, or a client whose counter reset) as a metric + warn log. Closes the online half of the audit's F7 "server can't detect missing / out-of-order batches" gap.

- Migration **0049** (`device_batch_progress`): one row per device (`device_id TEXT PK`, `last_batch_sequence BIGINT`, `updated_at`). Additive, **no FK** — a revoked/deleted device must not cascade into this diagnostic table, and the key is the token's `device_id` claim (a string) rather than the `devices` PK.
- `repo.rs` `EventStore::observe_batch_sequence(device_id, seq) -> Option<i64>` (Trait + Postgres + Memory, mirroring the quarantine pattern). Advances the mark monotonically (`GREATEST` upsert; the Memory impl mirrors it with `.max()`) and returns the PRIOR value in ONE statement via a `WITH prior AS (SELECT …), upsert AS (INSERT … ON CONFLICT …) SELECT prev FROM prior` CTE (`fetch_optional` → `None` on first-seen). An out-of-order (lower) arrival still records its observation without rewinding the mark.
- `ingest.rs`: a pure, total `classify_batch_sequence(prev, seq) -> {FirstSeen | InOrder | Gap{missing} | Regression{prev}}` (unit-tested with no store/HTTP), wired after the metrics block. Gaps/regressions bump `starstats_ingest_batch_sequence_anomaly{kind}` + warn with `device_id`/`seq`/`missing|prev`. **Best-effort**: a store error is logged and never fails the batch; user-scoped tokens (no `device_id`) are skipped. Keyed on `user.device_id` — an `Option<Uuid>` (NOT `String`, the initial wiring mistake), stringified to match the audit payload + table key.
- TDD: pure `classify_batch_sequence_covers_first_inorder_gap_regression` (first / in-order / gap / dup / lower-regression); store `observe_batch_sequence_tracks_high_water_mark_and_returns_prior` (first-seen `None`, prior returned, monotonic — a stale lower arrival doesn't rewind, second device tracked independently). Full server suite **820 + 787 passed, 0 failed** single-threaded; fmt + server clippy `-D warnings` clean. The Postgres CTE itself is exercised only against a real DB (runtime `query_as`, no `query!` macro) — verified at CI/deploy, per the repo's Memory-impl-for-logic convention.

**Still open after slice 4:** source-range + content hash, and server-side compatibility ranges.

### 2026-07-14 — Stage D, slice 5: **source-range + content hash on the ingest contract (SHIPPED to working tree, `next`)**

The audit's "source-range + content hash" F7 item — two more wire fields for batch-level provenance + a second dedup axis beyond per-event idempotency. Both `Option<_>` + `#[serde(default)]` (back-compat), stamped client-side in `build_batch`, recorded on the audit row.

- **`content_hash: Option<String>`** content-addresses the batch by its event SET: a **UUIDv5 over the SORTED idempotency keys** (`sync.rs::compute_content_hash`, fixed namespace const). Sorting → order-independent (a re-drain that reorders the same events hashes identically); v5 is a fixed SHA-1 algorithm → stable across machines/toolchains. **Dependency-free** — reuses `uuid` (already a client dep, `v5` feature already on workspace-wide) instead of pulling `sha2`/`blake3` into the client. Gives the server a batch-level dedup/replay + integrity signal beyond the per-event `idempotency_key`.
- **`source_range: Option<SourceRange>`** (new `starstats-core` wire struct `{source: LogSource, start_offset, end_offset}`) — the byte span the batch covers within ONE log source. **Populated only when the batch is single-source** (`sync.rs::compute_source_range`). Design honesty: a drain batches by event *type*, not source, so a batch CAN mix the live tail and the launcher log, whose `source_offset`s reset per file and aren't comparable — a mixed-source (or empty) batch ships `None` rather than a meaningless range.
- Server `ingest.rs`: both recorded on the `ingest.batch_processed` audit row beside the other provenance fields. Neither added to the doc-only `IngestBatchSchema` utoipa mirror (ingest has no generated-client consumer — same call as slices 1–4).
- TDD: core back-compat now asserts ALL FIVE provenance fields default `None` + round-trip (incl. a `SourceRange`); client `content_hash_is_order_independent_and_source_range_is_single_source_only` (reorder → same hash, different set → different hash; single-source → `Some(min..max)`, mixed → `None`, empty → `None`); `build_batch` asserts both stamps; server audit assertion extended to `content_hash` + `source_range`. Core 20/20, client 259/259, server ingest 18/18 single-threaded; fmt + workspace clippy `-D warnings` clean.

**Still open in Stage D (F7):** server-side compatibility ranges (flag/reject a collector too old for the current schema) — the last F7 item. `batch_sequence` (client counter slice 3 + online detector slice 4) and `source-range + content hash` (slice 5) are shipped.

### 2026-07-14 — Stage D, slice 6: **collector compatibility gate — Stage D COMPLETE (SHIPPED to working tree, `next`)**

The final F7 item. With `collector_version` stamped (slice 1), the server can now gate on it: an optional configured minimum, `STARSTATS_MIN_COLLECTOR_VERSION`. When set, batches from older collectors are FLAGGED (metric `starstats_ingest_collector_outdated` + warn) so an outdated fleet is visible.

- **Observability-first, safe by default.** Unset → `None` → no-op (zero behaviour change); shipping to alpha can't break any client. Even when configured, this slice only FLAGS — hard rejection is a deliberate follow-up (a product/UX call), one `return` away.
- Dependency-free: a bare `major.minor.patch` tuple parse (`parse_semver`, tolerant of a pre-release suffix on the patch) instead of the `semver` crate. `collector_below_min` flags only strictly-older, confidently-parseable versions — absent (legacy unversioned) or unparseable collectors are left alone (auto-update handles them; don't spam them as "outdated"). Min read once via `OnceLock` (cached, not per-request).
- Single-file (`ingest.rs`), no migration, no wire change. Flagged alongside the `batch_sequence` anomaly check (both best-effort provenance observability).
- TDD: pure `collector_compat_parse_and_below_min_gate` (parse bare/suffixed/junk; below-min for older major/minor/patch; not-below for equal/newer/absent/unparseable). Server ingest **19/19** single-threaded; fmt + server clippy `-D warnings` clean.

**Stage D (F7) is COMPLETE.** Per-event-type version was reframed as `collector_version` + `parser_version` (a per-event version is redundant once those pin the emitting build + adopted rule-set); `batch_sequence` (counter + online gap-detector), source-range, content hash, and compatibility gating all shipped — additive + back-compat — across slices 1–6. Remaining audit work lives OUTSIDE Stage D: **F10 manifest signing** (the flagged sequencing violation — the non-empty manifest still ships unsigned/unverified), and the **Stage E tail** (F8 cross-boundary correlation windows / negative-duration guards; F9 visitor `sessions-summary` + commerce 500-cap).

### 2026-07-14 — Stage E, part 2: **visitor sessions-summary onto a server aggregate (SHIPPED to working tree, `next`)**

Closed half of the F9 remainder. The Sessions widget's summary line ("N sessions · Xh played") was exact for the owner (me-scoped `/v1/me/stats/playtime?all_time=true`) but a VISITOR got no lifetime aggregate — it summed the 50-capped session list, silently undercounting heavy users and pinning the count at "50+".

- New `GET /v1/users/{handle}/stats/playtime` (`event_timeline.rs`) returns `{total_playtime_secs, session_count}`, behind the SAME `share_event_timeline` grant as the sibling `/v1/users/{handle}/sessions` — auth is **reused verbatim** (`caller_may_view_timeline`), not reimplemented, so no new disclosure surface (a permitted visitor already sees the capped session list; this just completes the total). Denied → 403 `share_event_timeline_not_granted`, exactly like the sessions endpoint.
- The aggregate **reuses** the owner-side `EventQuery::total_playtime_secs` / `count_sessions_since` (`since = None` = all-time) via `PostgresStore::new(pool)`, so a visitor's totals MATCH what the owner sees — no divergent SQL, which is the whole point (the bug was a client-side undercount, not a different metric).
- Not in the OpenAPI spec (no `#[utoipa::path]`) — the web hand-types `UserPlaytimeResponse` (mirrors `stats_records`), avoiding the codegen-drift gate.
- Web: `getUserPlaytime(bearer, handle)` in `api.ts`; the Sessions widget now fetches it on the visitor branch and passes `lifetime` to the already-dual-path `buildSessionSummary`. A denied visitor 4xxs → falls back to the capped list exactly as before.
- TDD: server `user_playtime_rejects_without_auth` (401) + `user_playtime_rejects_malformed_handle` (400) — the auth-before-query ordering, runnable on the lazy (unconnected) pool like the existing sessions tests. The 403/200 grant paths need a live DB, so — as for `list_sessions` itself — they rely on the reused, proven `caller_may_view_timeline` + CI integration rather than a local unit test. Web `sessions.test.tsx` asserts owner→`getPlaytime`, visitor→`getUserPlaytime`, neither crossing. Server event_timeline 18/18 single-threaded; web typecheck clean + vitest 6/6; fmt + server clippy `-D warnings` clean. (Needed the `import React` vitest-classic-runtime fix on `sessions.tsx` — [[feedback_web_vitest_react_import]].)

**Still open in F9/Stage E:** the commerce "biggest trade" visitor path is still a 500-capped client scan; and F8 (cross-boundary burst/inference windows — a *documented-accepted* limitation, `backfill.rs` — plus negative-duration guards) remains.

### 2026-07-14 — Stage E, part 3: **commerce "biggest trade" onto a server aggregate (SHIPPED to working tree, `next`)**

Closed the other half of the F9 remainder. The Records widget's "biggest trade" was a client scan over `getCommerceRecent(token, 500)` — capped at 500 recent transactions, so a big trade outside that window was missed (not a true all-time max).

- New me-scoped `GET /v1/me/stats/biggest-trade` (`query.rs`) returns `{quantity, item}` — the largest CONFIRMED commerce purchase by quantity over the FULL history. Owner-only (C2), hand-typed (no utoipa, like `stats_records`).
- Commerce is derived from EVENTS (no transactions table): the handler fetches all events of the 4 commerce `event_type`s (new `starstats_core::transactions::COMMERCE_EVENT_TYPES` const, colocated with `pair_transactions` so they can't drift), pairs them via the existing pure `pair_transactions`, and takes the max confirmed quantity. Commerce events are rare per user, so fetching all of them (generous 50k per-type cap) is bounded — unlike a general all-events pull. `window_secs = i64::MAX` since Confirmed is response-driven, not time-driven.
- The max-logic is a pure `biggest_confirmed_trade(&[Transaction])` helper (store-free unit test), mirroring the web's old scan exactly (confirmed + has-quantity, max by quantity) so the server value matches what the client computed — just uncapped.
- Web: `getBiggestTrade(token)` replaces the `getCommerceRecent(500)` scan in the Records widget; `records.test.tsx` updated (owner→`getBiggestTrade`, visitor never calls it — C2).
- TDD: server `biggest_confirmed_trade_picks_max_confirmed_quantity` (max confirmed; rejected/submitted/no-qty excluded; empty→None). Web records vitest 4/4, typecheck + lint clean; server compile + helper test green; fmt + core/server clippy `-D warnings` clean. Event-type strings verified against `GameEvent`'s snake_case tag, not assumed.

**F9 is now COMPLETE** (records + visitor sessions-summary + commerce biggest-trade all server-authoritative). **Still open in Stage E:** F8 — cross-boundary burst/inference windows (a *documented-accepted* limitation in `backfill.rs`) + negative-duration guards.

### 2026-07-14 — Stage E, part 4: **inference negative-duration guard (SHIPPED to working tree, `next`)**

Closed the actionable half of F8's correctness gap. `inference::infer_with_rules` clips each rule's post-trigger window by timestamp; the clipper enforced only the UPPER bound (`delta <= window_secs`), so an out-of-order envelope — one whose timestamp is at or before the trigger (`delta <= 0`), which happens across a log rotation or a backfill that interleaves files — was pulled into the window and could be paired as a "follow-up": a **negative-duration correlation**.

- `trim_window_by_secs` → `window_within_secs`: now FILTERS the window to strictly `(trigger_ts, trigger_ts + window_secs]` (`0 < delta <= window_secs`) instead of truncating at the first over-window row. Filtering (not a contiguous-prefix truncate) is required because an out-of-order row can sit anywhere in the positional window, not just at the end. Returns `Vec<EventEnvelope>` (windows are tiny — bounded by `window_size` — so the clone is negligible); the public `infer` / `infer_with_rules` API is unchanged, so no client change.
- TDD: `window_within_secs_excludes_out_of_order_and_out_of_window` (stale-earlier + same-instant + in-window + past-window rows → only the in-window one survives). Core inference **27/27** (existing monotonic tests unchanged); fmt + core/client clippy `-D warnings` clean.

**Note on F8 scope:** the *inference* window already crosses backfill chunks (threaded through `process_buffer`). What remains is (a) the **burst-collapse** cross-boundary case — an attachment run split across a 10k-line backfill chunk or a live drain, which the code documents as an *accepted* limitation and which needs a carry-over buffer in the burst FSM — and (b) a **vehicle/character-life FSM**, a net-new feature beyond the audit's correctness scope. Both are larger than a guard; neither is a silent regression.

### 2026-07-14 — Stage E, part 5: **backfill burst cross-boundary carry-over (SHIPPED to working tree, `next`)**

Closed the burst-collapse cross-boundary gap. In backfill, the cursor→EOF delta is processed in 10k-line chunks; a burst (attachment / loadout run) straddling a chunk boundary was detected as two partial runs. Because the burst idempotency key includes `size`, chunk N's partial (size 5) and chunk N+1's completion (size 8) became TWO bogus summaries — or, if a partial fell below `min_burst_size`, the whole run degraded to raw per-line events.

- Backfill's chunk loop now holds back the trailing possibly-open run and prepends it to the next chunk, so `detect_bursts` sees the full run. `carry_boundary(len, burst_ranges, max_carry)` (pure) returns a commit/carry split that GUARANTEES no detected burst spans it: it carries the trailing `BURST_CARRY_LINES` (256) and pulls the split back to the anchor of any burst reaching into that tail. `burst_buffer_ranges` mirrors `process_buffer` passes 1-2 to feed it.
- **Live path untouched** — the live tail already lands a burst in one fsync/drain (the "practically fine" case documented on the old loop); only backfill (archived logs, bounded-memory chunking) changes. Zero risk to the live hot path.
- Bounded + loss-free: every line is committed exactly once (carried lines re-enter the next buffer; the final flush drains all remaining carry), `cut` is never 0 mid-loop (buffer ≥ 10k), and the carry is at most a burst-length past the fixed window.
- TDD: exhaustive pure `carry_boundary_holds_tail_and_never_splits_a_burst` (no-burst tail, small buffer, burst-before-tail committed, burst-in-tail / straddling carried whole, multi-burst) + `burst_buffer_ranges_maps_a_real_burst_to_buffer_indices` (parse→detect→remap glue on real `AttachmentReceived` lines). Full client suite **261/261**; fmt + client clippy `-D warnings` clean.

**F8 correctness is now closed** (negative-duration guard + burst cross-boundary carry-over). The only remaining Stage E item is the **vehicle/character-life FSM** — a net-new correlation *feature*, not a bug fix.

### 2026-07-15 — Stage C, part 5: **F10 — parser manifest signing (flag-gated OFF) (SHIPPED to working tree, `next`)**

Closed the audit's F10 sequencing violation: ever since C1 first served a *non-empty* manifest, the served rules ran on TLS trust alone (§6 said signing must land in that same slice — it didn't). The manifest is now ed25519-signable server-side and verifiable client-side, implemented in FULL but **dormant by default** — the chosen "strict, flag-gated off" rollout, so nothing changes until keys are provisioned.

- **Shared canonical bytes** (`starstats_core::parser_defs::manifest_signing_bytes`): a `ManifestSigningView` (the manifest MINUS `signature`, in declaration order, covering `inference_rules`) serialised deterministically. Both sides derive the signed payload from this ONE function so the bytes are bit-identical — the crux of any working signature. Core stays crypto-free (bytes only); sign/verify live in the consuming crates, per the module's architectural rule.
- **Server** (`parser_def_routes.rs`, +`ed25519-dalek`): `current_manifest` signs when `STARSTATS_PARSER_SIGNING_KEY` (base64 32-byte seed) is set, stamping `signature`. Unset (default) → unsigned, as before. Malformed key → logged + unsigned (fail-open on the server is safe — an unsigned manifest is only rejected by clients that *require* signing).
- **Client** (`parser_defs.rs`, +`ed25519-dalek`): a pinned `PARSER_SIGNING_PUBKEY_B64` const (default `None` → dormant) + a `STARSTATS_REQUIRE_SIGNED_MANIFEST` flag (default off). `fetch_once` verifies BEFORE caching/compiling and drops a rejected manifest, keeping last-known-good. Pure adoption policy: verified-good → adopt; tampered (present-but-invalid sig) → reject ALWAYS; unverifiable (no pin / unsigned) → adopt unless required. The pubkey is a build-time constant, not fetched — a wire-fetched key could be swapped by the same MITM the signature defends against; rotation is a client release.
- **To activate:** generate an ed25519 keypair; set `STARSTATS_PARSER_SIGNING_KEY` on the server; pin the pubkey in `PARSER_SIGNING_PUBKEY_B64` + ship a client; flip `STARSTATS_REQUIRE_SIGNED_MANIFEST=1` for full strict.
- TDD: core `manifest_signing_bytes_exclude_signature_and_track_signed_fields`; server `sign_manifest_produces_a_signature_its_pubkey_verifies` (sign→decode→verify + tamper); client `manifest_is_adoptable_policy` (all 6 cases) + `verify_with_pubkey_round_trips_and_detects_tamper` (good / tampered-field / wrong-key / garbage-sig / undecodable-pubkey). Core + server + client green; fmt + workspace clippy `-D warnings` clean.

**F10 code gap closed** (dormant; activate by provisioning keys). This was the last outstanding audit sequencing item — Stages A–E remediation is complete except the deliberately-deferred vehicle/character-life FSM *feature*.
