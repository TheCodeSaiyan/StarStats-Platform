# PG Rebuild — Deferred Rollup Materialization Implementation Plan



**Goal:** Wire the three deferred PG-rebuild items into live use — (A) materialize the `session_summary` / `character_records` / `entity_rollup_agg` rollups behind a dirty-flag lazy-recompute so `/me` entities/records/sessions stop full-scanning `events`; (B) unify the timeline sessionizer with the SQL sessionizer without changing which sessions the timeline shows; (C) replace the cluster-wide audit advisory-lock with an in-process serialized writer while keeping the hash chain intact.

**Architecture:** Item A adds one `rebuild_handle_session_stats(handle)` transaction that recomputes the three session-derived rollups from `events` via the existing 30-min-gap SQL sessionizer, gated by `stat_rollup_state.sessions_dirty`; reads recompute-on-dirty then hard-cut to the rollup (the `summary_for_handle` pattern), with a one-time `0058` backfill that flags every existing handle dirty. Item B replicates `process_init` boundary logic in SQL and parity-tests it against the Rust `derive_sessions`. Item C funnels all audit appends through a single dedicated writer task fed by an in-process channel, preserving strict seq/hash-chain ordering without a DB-level lock.

**Tech Stack:** Rust, `sqlx` (Postgres), `tokio`, `axum` 0.7. Postgres 17. No new crates required (item C uses `tokio::sync::mpsc` + `oneshot`, already in-tree).

## Global Constraints

- **Migrations are additive-only and byte-immutable.** Never edit/renumber/consolidate a shipped migration file — it crash-loops deployed DBs on the `_sqlx_migrations` history. New work = new `00NN` files. Next number is **`0058`**. Copy values verbatim; do not "tidy" existing SQL.
- **No `CREATE EXTENSION` in migrations** — the app role is not superuser. Extensions live in `infra/init/init-databases.sh`.
- **Any hard-cut-to-rollup read MUST be safe against an empty rollup on an upgraded DB.** Here the read path recomputes-on-dirty from `events`, and `0058` flags every existing handle dirty — so an empty table is never trusted. Without that flag the read would undercount (historical events hit `ON CONFLICT DO NOTHING` at ingest and never enter a rollup; a reparse won't fix it).
- **wCTE self-collision:** `WITH del AS (DELETE FROM t …) INSERT INTO t SELECT FROM t` self-collides (both CTEs see the pre-statement snapshot). For DELETE-then-rebuild of the *same* table, use two statements in one transaction. (Rebuilding a rollup table *from `events`* is a different table, so no collision — but keep the DELETE and INSERT as separate statements for clarity and to match `retention.rs`.)
- **Lint/format gates:** `cargo clippy --workspace -- -D warnings` must pass; `cargo fmt --all` applied. Offline `cargo test -p starstats-server` (1834 tests) must stay green with no DB required.
- **Ship path per item (never direct-to-main):** land on `next` → dispatch a **live dry-run** `gh workflow run promote.yml --ref next -f track=platform -f channel=live -f dry_run=true`, approve the `production-release` gate (env id `15704172652`, typed-JSON body — the `-f environment_ids[]` form is rejected), verify the printed plan, then re-dispatch `dry_run=false`. Confirm success authoritatively: both live job `conclusion=success`, `origin/main == origin/next`, bare tag `vX.Y.Z` at that SHA. Do a live promote only on explicit "go".
- **DB validation is manual against a real container** (no `#[sqlx::test]` / testcontainer harness exists in this crate). Harness: `docker run -d --name ss-pg -e POSTGRES_PASSWORD=devpassword -e POSTGRES_USER=starstats_app -e POSTGRES_DB=starstats -p 5433:5432 postgres:17`. Every SQL-bearing task carries an explicit EXCEPT-parity + EXPLAIN validation step; a task is not "done" until that step passes.

---

# Plan A — Session/Records/Entity Rollup Materialization

**Independently shippable:** yes. Ships as its own `next → promote` cycle.

**Design in one paragraph.** Ingest already stamps `stat_rollup_state.sessions_dirty = TRUE` per handle on every batch (`insert_batch`, repo.rs:2602) but nothing consumes it. We add `PostgresStore::rebuild_handle_session_stats(handle)` — one transaction that recomputes `session_summary`, `character_records`, and `entity_rollup_agg` from `events` via the 30-min-gap sessionizer, then clears the dirty flag. We add a private `ensure_session_stats_fresh(handle)` guard that rebuilds only when dirty/missing, and call it at the top of the four session-derived reads before they hard-cut to the rollup tables. `retention` flags the handle dirty after a purge. Migration `0058` flags every existing handle dirty so the first post-deploy read recomputes (never trusts the empty tables).

## File Structure (Plan A)

- **Create** `crates/starstats-server/migrations/0058_flag_all_handles_dirty.sql` — one-time backfill: upsert a dirty `stat_rollup_state` row for every distinct `claimed_handle` in `events`.
- **Modify** `crates/starstats-server/src/repo.rs` — add `rebuild_handle_session_stats` + `ensure_session_stats_fresh` on `PostgresStore`; convert `sessions_for_handle`, `total_playtime_secs`, `count_sessions_since`, `records_for_handle` real impls to fresh-then-read `session_summary`/`character_records` with an events-fallback. Mirror `ensure_session_stats_fresh` as a no-op on `MemoryQuery` (mocks keep computing from in-memory events, so existing unit tests are unchanged).
- **Modify** `crates/starstats-server/src/entity_rollup.rs` — in the `list_entities` route (entity_rollup.rs:850) read `session_count` from `entity_rollup_agg` after freshening, falling back to the current `session_bounds_rows` + `derive_entity_session_data` walk on miss.
- **Modify** `crates/starstats-server/src/retention.rs:190-219` — inside the existing post-purge transaction, flag the handle dirty so session rollups recompute on next read.
- **Create** `scripts/validate/0058_rollup_parity.sql` — committed, repeatable EXCEPT-parity harness (rollup vs live `GROUP BY`), run against the docker container.

## Global Constraints (Plan A)

All Global Constraints above apply. Additionally: the recompute is **lazy** (on read when dirty), not a background job — first read after a batch pays the rebuild, which is acceptable and matches `summary_for_handle`'s self-heal philosophy. A background refresher is explicitly out of scope (YAGNI) until metrics justify it.

---

### Task A1: `0058` backfill migration — flag every existing handle dirty

**Files:**
- Create: `crates/starstats-server/migrations/0058_flag_all_handles_dirty.sql`

**Interfaces:**
- Consumes: `stat_rollup_state` table (PK `claimed_handle`, cols `sessions_dirty BOOLEAN`, `counts_last_seq BIGINT`) from migration `0056`.
- Produces: post-migration invariant — every `claimed_handle` present in `events` has a `stat_rollup_state` row with `sessions_dirty = TRUE`.

- [ ] **Step 1: Write the migration**

```sql
-- 0058_flag_all_handles_dirty.sql
-- One-time backfill for the session/records/entity rollups (0056).
-- Those three tables (session_summary, character_records, entity_rollup_agg)
-- ship empty and are materialized lazily by rebuild_handle_session_stats()
-- on the first read that finds the handle dirty. Historical handles have no
-- stat_rollup_state row yet (only NEW batches create one via insert_batch),
-- so without this backfill their first read would find sessions_dirty absent,
-- read an EMPTY rollup, and undercount. Flag every existing handle dirty so
-- the first post-deploy read recomputes from events. Runs at boot before the
-- server accepts ingest. Idempotent (ON CONFLICT).
INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, counts_last_seq)
SELECT DISTINCT claimed_handle, TRUE, 0
FROM events
WHERE claimed_handle IS NOT NULL
ON CONFLICT (claimed_handle) DO UPDATE
    SET sessions_dirty = TRUE,
        updated_at = now();
```

- [ ] **Step 2: Apply against the docker container and verify coverage**

```bash
docker exec -i ss-pg psql -U starstats_app -d starstats \
  -c "SELECT count(*) AS handles_in_events FROM (SELECT DISTINCT claimed_handle FROM events) e;" \
  -c "SELECT count(*) AS dirty_rows FROM stat_rollup_state WHERE sessions_dirty;"
```
Expected: `dirty_rows >= handles_in_events` (every events handle now has a dirty state row).

- [ ] **Step 3: Verify idempotency (re-apply)**

Run the same `INSERT … ON CONFLICT` a second time; expected: no error, `dirty_rows` unchanged.

- [ ] **Step 4: Commit**

```bash
git add crates/starstats-server/migrations/0058_flag_all_handles_dirty.sql
git commit -m "feat(server): 0058 backfill flags all handles dirty for session rollups"
```

---

### Task A2: `0059` column migration + `rebuild_handle_session_stats` — materialize the three rollups in one transaction

> **AMENDED 2026-07-23 after the final whole-branch review (repo.rs is now authoritative for the rebuild body).** Two concurrency fixes + one scope cut were applied to `rebuild_handle_session_stats`:
> 1. **Per-handle serialization** — the tx now takes `pg_advisory_xact_lock(21331, hashtext(LOWER($1)))` first, so two concurrent first-reads of the same handle can't both `INSERT` the same rollup PKs (was a `unique_violation`/500 at deploy, when `0058` marks every handle dirty). Namespace `21331` ('SS') is distinct from the audit lock's key space.
> 2. **Double-check + conditional dirty-clear** — under the lock it re-reads `(sessions_dirty, updated_at)`; skips the recompute if a sibling already cleared it; and clears the flag only `WHERE stat_rollup_state.updated_at = <captured>`, so a batch committing mid-rebuild (which bumps `updated_at` + re-sets dirty=TRUE) is NOT silently dropped — the handle stays dirty and the next read re-materializes it.
> 3. **`entity_rollup_agg` population REMOVED** from the rebuild — its only consumer counts by `process_init` boundaries (A4 deferred, blocked on item B), and populating gap-based counts on the hot read path was wasted work + wrong semantics. The table stays empty/dormant until A4/B adds process_init population + a backfill. The step-3 SQL block below is retained for reference but is NOT in the shipped code.

**Files:**
- Create: `crates/starstats-server/migrations/0059_character_records_busiest.sql` — add `busiest_session_events` column so `character_records` matches the `RecordsAggregate` shape the endpoint returns.
- Create: `scripts/validate/0058_rollup_parity.sql` — committed EXCEPT-parity harness.
- Modify: `crates/starstats-server/src/repo.rs` (add method to `impl PostgresStore`, near the existing `insert_batch` at repo.rs:2602)

**Interfaces:**
- Consumes: `events` (cols `claimed_handle`, `event_timestamp`, `event_type`, `metadata` JSONB, `source_offset`); `SESSION_IDLE_GAP_MINUTES: i64` (repo.rs:200 — the existing sessionizer binds it `as i32` because `make_interval(mins => $2)` needs `int4`); `stat_rollup_state`; the `RepoError` error type (repo.rs:720 — every `PostgresStore` method returns `Result<_, RepoError>` and `?`-propagates `sqlx::Error`, which `RepoError` is `From`).
- Produces:
  ```rust
  impl PostgresStore {
      /// Recompute session_summary + character_records + entity_rollup_agg.session_count
      /// for `handle` from events, in one transaction, and clear sessions_dirty.
      pub(crate) async fn rebuild_handle_session_stats(&self, handle: &str) -> Result<(), RepoError>;
  }
  ```

**Design notes locked in from code verification (do not deviate without re-checking):**
- `session_summary.session_id` is **TEXT** — the running-sum ordinal must be cast `session_id::text` on insert.
- The gap param binds as **`SESSION_IDLE_GAP_MINUTES as i32`** (matches `sessions_for_handle` at repo.rs:1902/1917).
- `records_for_handle` returns `RecordsAggregate { longest_session_secs, busiest_session_events, longest_survival_streak_secs, deadliest_session_deaths }` — four fields. The `character_records` table lacked `busiest_session_events` (added by `0059` here). `character_records` derives its records **from the freshly-written `session_summary`** (per-session max) + one death-gap scan, reproducing `records_for_handle(handle, None)` exactly.
- **`kills`/`pvp_deaths` are NOT materialized** (there is no `'kill'` event_type — `combat_counts` at repo.rs:649/2511 defines kills as `actor_death` rows filtered by `payload.victim`). Combat stays on the live `combat_counts` scan; those columns keep their `DEFAULT 0`. `total_deaths`/`total_sessions`/`first_event_at`/`last_event_at` are cheap correct extras derived from `session_summary`.

- [ ] **Step 1: Write the `0059` column migration**

Create `crates/starstats-server/migrations/0059_character_records_busiest.sql`:
```sql
-- 0059_character_records_busiest.sql
-- character_records (0056) predates the RecordsAggregate shape that
-- records_for_handle returns, which exposes busiest_session_events
-- (MAX events in a single session). Add the column so the records rollup
-- can serve that field. Additive, idempotent, non-superuser safe.
ALTER TABLE character_records
    ADD COLUMN IF NOT EXISTS busiest_session_events BIGINT NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Write the parity-validation SQL harness (the SQL acceptance oracle)**

Create `scripts/validate/0058_rollup_parity.sql`:
```sql
-- Parity harness for rebuild_handle_session_stats. Run AFTER calling the
-- rebuild for :handle. Each block must return ZERO rows (rollup == live).
\set gap 30
-- 1. session_summary vs a live sessionizer GROUP BY. session_id is an ordinal
--    label cast to text on both sides so the EXCEPT column types match.
WITH gaps AS (
    SELECT event_timestamp, event_type,
           LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS prev_ts
    FROM events
    WHERE claimed_handle = lower(:'handle') AND event_timestamp IS NOT NULL
      AND event_type NOT IN ('launcher_activity','game_crash')
), labeled AS (
    SELECT event_timestamp, event_type,
           SUM(CASE WHEN prev_ts IS NULL
                     OR event_timestamp - prev_ts > make_interval(mins => :gap)
                    THEN 1 ELSE 0 END) OVER (ORDER BY event_timestamp ASC) AS session_id
    FROM gaps
), live AS (
    SELECT session_id::text AS session_id,
           MIN(event_timestamp) AS started_at, MAX(event_timestamp) AS ended_at,
           COUNT(*)::bigint AS event_count,
           COUNT(*) FILTER (WHERE event_type = 'player_death')::bigint AS death_count
    FROM labeled GROUP BY session_id
)
(SELECT session_id, started_at, ended_at, event_count, death_count FROM live
 EXCEPT
 SELECT session_id, started_at, ended_at, event_count, death_count
 FROM session_summary WHERE claimed_handle = lower(:'handle'))
UNION ALL
(SELECT session_id, started_at, ended_at, event_count, death_count
 FROM session_summary WHERE claimed_handle = lower(:'handle')
 EXCEPT
 SELECT session_id, started_at, ended_at, event_count, death_count FROM live);
-- (Add analogous EXCEPT blocks for character_records vs live records_for_handle
--  and entity_rollup_agg.session_count vs COUNT(DISTINCT session_id) per entity.)
```
(Acceptance oracle for Steps 4-6; keep it committed so any future change re-runs it.)

- [ ] **Step 3: Add the rebuild method to `PostgresStore`**

In `repo.rs`, add (reusing the exact gap CTE the sessionizer already uses; `$2` binds `SESSION_IDLE_GAP_MINUTES as i32`):
```rust
pub(crate) async fn rebuild_handle_session_stats(&self, handle: &str) -> Result<(), RepoError> {
    let gap_minutes = SESSION_IDLE_GAP_MINUTES as i32;
    let mut tx = self.pool.begin().await?;

    // (1) session_summary: DELETE then re-INSERT from the gap-sessionized events.
    //     session_id is the running-sum ordinal cast to TEXT (the column is TEXT).
    sqlx::query("DELETE FROM session_summary WHERE claimed_handle = LOWER($1)")
        .bind(handle)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        WITH gaps AS (
            SELECT event_timestamp, event_type,
                   LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS prev_ts
            FROM events
            WHERE claimed_handle = LOWER($1) AND event_timestamp IS NOT NULL
              AND event_type NOT IN ('launcher_activity','game_crash')
        ), labeled AS (
            SELECT event_timestamp, event_type,
                   SUM(CASE WHEN prev_ts IS NULL
                             OR event_timestamp - prev_ts > make_interval(mins => $2)
                            THEN 1 ELSE 0 END)
                     OVER (ORDER BY event_timestamp ASC) AS session_id
            FROM gaps
        )
        INSERT INTO session_summary
            (claimed_handle, session_id, started_at, ended_at, event_count, death_count)
        SELECT LOWER($1), session_id::text,
               MIN(event_timestamp), MAX(event_timestamp),
               COUNT(*)::bigint,
               COUNT(*) FILTER (WHERE event_type = 'player_death')::bigint
        FROM labeled GROUP BY session_id
        "#,
    )
    .bind(handle)
    .bind(gap_minutes)
    .execute(&mut *tx)
    .await?;

    // (2) character_records: reproduce records_for_handle(handle, None) exactly from
    //     the freshly-written session_summary (per-session maxes) + a death-gap scan.
    //     kills/pvp_deaths are NOT materialized (combat stays on the live combat_counts
    //     scan; no 'kill' event_type exists) — they keep DEFAULT 0.
    sqlx::query(
        r#"
        WITH sess AS (
            SELECT started_at, ended_at, event_count, death_count
            FROM session_summary WHERE claimed_handle = LOWER($1)
        ), streak AS (
            SELECT MAX(EXTRACT(EPOCH FROM gap))::bigint AS longest_gap
            FROM (
                SELECT event_timestamp
                       - LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS gap
                FROM events
                WHERE claimed_handle = LOWER($1)
                  AND event_type = 'player_death' AND event_timestamp IS NOT NULL
            ) g
        )
        INSERT INTO character_records
            (claimed_handle, total_deaths, total_sessions, longest_session_secs,
             busiest_session_events, deadliest_session_deaths,
             longest_survival_gap_secs, first_event_at, last_event_at, updated_at)
        SELECT LOWER($1),
               (SELECT COALESCE(SUM(death_count),0) FROM sess),
               (SELECT COUNT(*) FROM sess),
               (SELECT COALESCE(MAX(EXTRACT(EPOCH FROM (ended_at - started_at))::bigint),0) FROM sess),
               (SELECT COALESCE(MAX(event_count),0) FROM sess),
               (SELECT COALESCE(MAX(death_count),0) FROM sess),
               (SELECT COALESCE(longest_gap,0) FROM streak),
               (SELECT MIN(started_at) FROM sess),
               (SELECT MAX(ended_at) FROM sess),
               now()
        ON CONFLICT (claimed_handle) DO UPDATE SET
            total_deaths = EXCLUDED.total_deaths,
            total_sessions = EXCLUDED.total_sessions,
            longest_session_secs = EXCLUDED.longest_session_secs,
            busiest_session_events = EXCLUDED.busiest_session_events,
            deadliest_session_deaths = EXCLUDED.deadliest_session_deaths,
            longest_survival_gap_secs = EXCLUDED.longest_survival_gap_secs,
            first_event_at = EXCLUDED.first_event_at,
            last_event_at = EXCLUDED.last_event_at,
            updated_at = now()
        "#,
    )
    .bind(handle)
    .execute(&mut *tx)
    .await?;

    // (3) entity_rollup_agg.session_count: assign each event a session_id via the same
    //     gap CTE, then count DISTINCT sessions per primary_entity.
    sqlx::query("DELETE FROM entity_rollup_agg WHERE claimed_handle = LOWER($1)")
        .bind(handle)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        -- Sessionize over the FULL event stream (identical WHERE to step 1's
        -- sessionizer) so session_id boundaries match; filter to entity-tagged
        -- rows AFTER labeling, else LAG would skip intervening untagged events
        -- and compute different sessions.
        WITH gaps AS (
            SELECT event_timestamp, metadata, source_offset,
                   LAG(event_timestamp) OVER (ORDER BY event_timestamp ASC) AS prev_ts
            FROM events
            WHERE claimed_handle = LOWER($1) AND event_timestamp IS NOT NULL
              AND event_type NOT IN ('launcher_activity','game_crash')
        ), labeled AS (
            SELECT event_timestamp, metadata, source_offset,
                   SUM(CASE WHEN prev_ts IS NULL
                             OR event_timestamp - prev_ts > make_interval(mins => $2)
                            THEN 1 ELSE 0 END)
                     OVER (ORDER BY event_timestamp ASC) AS session_id
            FROM gaps
        )
        INSERT INTO entity_rollup_agg
            (claimed_handle, entity_kind, entity_id, display_name,
             event_count, session_count, first_seen_at, last_seen_at, updated_at)
        SELECT LOWER($1),
               metadata->'primary_entity'->>'kind',
               metadata->'primary_entity'->>'id',
               -- mirrors the existing list_entities query (entity_rollup.rs:354) verbatim
               (array_agg(NULLIF(metadata->'primary_entity'->>'display_name','')
                          ORDER BY source_offset DESC))[1],
               COUNT(*)::bigint,
               COUNT(DISTINCT session_id)::bigint,
               MIN(event_timestamp), MAX(event_timestamp), now()
        FROM labeled
        WHERE metadata->'primary_entity'->>'kind' IS NOT NULL
          AND metadata->'primary_entity'->>'id'   IS NOT NULL
        GROUP BY metadata->'primary_entity'->>'kind', metadata->'primary_entity'->>'id'
        "#,
    )
    .bind(handle)
    .bind(gap_minutes)
    .execute(&mut *tx)
    .await?;

    // (4) clear the dirty flag.
    sqlx::query(
        "INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, rebuilt_at, updated_at)
         VALUES (LOWER($1), FALSE, now(), now())
         ON CONFLICT (claimed_handle) DO UPDATE
             SET sessions_dirty = FALSE, rebuilt_at = now(), updated_at = now()",
    )
    .bind(handle)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 4: Seed the container and rebuild**

```bash
# seed a handle with known sessions (use an existing fixture dump or hand-insert rows),
# then call the rebuild from a tiny throwaway bin OR replicate the 4 statements via psql
# with :handle set, inside one transaction (BEGIN; …; COMMIT;).
docker exec -i ss-pg psql -U starstats_app -d starstats -v handle=alice -f - < /path/to/rebuild_manual.sql
```

- [ ] **Step 5: Run the parity harness — expect ZERO rows**

```bash
docker exec -i ss-pg psql -U starstats_app -d starstats -v handle=alice \
  -f scripts/validate/0058_rollup_parity.sql
```
Expected: 0 rows from the session_summary EXCEPT block. Repeat with a `character_records`-vs-live and `entity_rollup_agg.session_count`-vs-`derive_entity_session_data` comparison (add analogous EXCEPT blocks). All must be empty.

- [ ] **Step 6: Verify idempotency**

Re-run the rebuild; re-run parity. Expected: still 0 rows, and row counts in the three tables unchanged (DELETE+re-INSERT is idempotent).

- [ ] **Step 7: `cargo build` + clippy (compile gate — no unit test yet; SQL is validated above)**

```bash
cargo clippy -p starstats-server -- -D warnings
```
Expected: clean (the method is `pub(crate)`, referenced by Task A3 next; if clippy flags dead_code, proceed to A3 in the same commit or add `#[allow(dead_code)]` temporarily and remove in A3).

- [ ] **Step 8: Commit**

```bash
git add crates/starstats-server/migrations/0059_character_records_busiest.sql \
        crates/starstats-server/src/repo.rs scripts/validate/0058_rollup_parity.sql
git commit -m "feat(server): 0059 + rebuild_handle_session_stats materializes session/records/entity rollups"
```

---

### Task A3: `ensure_session_stats_fresh` guard + convert the four session-derived reads

**Files:**
- Modify: `crates/starstats-server/src/repo.rs` — add private guard; convert `sessions_for_handle` (:1883), `total_playtime_secs` (:1945), `count_sessions_since` (:1988), `records_for_handle` (:2027) real impls.

**Interfaces:**
- Consumes: `rebuild_handle_session_stats` (Task A2); `session_summary`, `character_records` tables; `stat_rollup_state.sessions_dirty`.
- Produces:
  ```rust
  impl PostgresStore {
      /// Rebuild session rollups for `handle` iff dirty or state row missing. Cheap when clean.
      async fn ensure_session_stats_fresh(&self, handle: &str) -> Result<(), RepoError>;
  }
  ```
  Read signatures are UNCHANGED (`Result<_, RepoError>`, `InferredSession`, `RecordsAggregate`) — only the data source moves from a full events scan to the rollup table. Verify each real signature/param name against the trait before editing.

- [ ] **Step 1: Add the freshness guard**

```rust
async fn ensure_session_stats_fresh(&self, handle: &str) -> Result<(), RepoError> {
    // Dirty (or absent) => recompute. Absent is treated as dirty so pre-0058
    // handles and never-seen handles both recompute on first read.
    let dirty: bool = sqlx::query_scalar(
        "SELECT COALESCE(
            (SELECT sessions_dirty FROM stat_rollup_state WHERE claimed_handle = LOWER($1)),
            TRUE)",
    )
    .bind(handle)
    .fetch_one(&self.pool)
    .await?;
    if dirty {
        self.rebuild_handle_session_stats(handle).await?;
    }
    Ok(())
}
```

- [ ] **Step 2: Convert `sessions_for_handle` to read `session_summary`**

Replace the events-scanning body (repo.rs:1903-1927) with a freshen-then-point-read. The real method returns `Result<Vec<InferredSession>, RepoError>` and maps a `(start, end, count)` tuple into `InferredSession { start_at, end_at, event_count }` (repo.rs:218):
```rust
async fn sessions_for_handle(&self, claimed_handle: &str, limit: i64, offset: i64)
    -> Result<Vec<InferredSession>, RepoError> {
    self.ensure_session_stats_fresh(claimed_handle).await?;
    // started_at/ended_at are nullable columns, but a session always has a
    // non-null MIN/MAX (the rebuild filters event_timestamp IS NOT NULL); the
    // `started_at IS NOT NULL` guard keeps the DateTime<Utc> decode total.
    let rows: Vec<(DateTime<Utc>, DateTime<Utc>, i64)> = sqlx::query_as(
        "SELECT started_at, ended_at, event_count
         FROM session_summary
         WHERE claimed_handle = LOWER($1) AND started_at IS NOT NULL
         ORDER BY started_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(claimed_handle).bind(limit).bind(offset)
    .fetch_all(&self.pool).await?;
    Ok(rows.into_iter()
        .map(|(start_at, end_at, event_count)| InferredSession { start_at, end_at, event_count })
        .collect())
}
```

- [ ] **Step 3: Convert `total_playtime_secs` and `count_sessions_since` to read `session_summary`**

**Before writing, read the existing `total_playtime_secs` (repo.rs:1945) and `count_sessions_since` (repo.rs:1988) to confirm their exact aggregation and real param names/return types, then mirror them.** Expected shapes (verify):
```rust
async fn total_playtime_secs(&self, claimed_handle: &str) -> Result<i64, RepoError> {
    self.ensure_session_stats_fresh(claimed_handle).await?;
    // Must equal the existing method: sum of per-session (MAX-MIN) durations.
    let secs: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(EXTRACT(EPOCH FROM (ended_at - started_at))::bigint),0)
         FROM session_summary WHERE claimed_handle = LOWER($1)",
    ).bind(claimed_handle).fetch_one(&self.pool).await?;
    Ok(secs)
}

async fn count_sessions_since(&self, claimed_handle: &str, since: DateTime<Utc>) -> Result<i64, RepoError> {
    self.ensure_session_stats_fresh(claimed_handle).await?;
    // Must equal the existing method's window semantics (sessions started at/after `since`).
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM session_summary
         WHERE claimed_handle = LOWER($1) AND started_at >= $2",
    ).bind(claimed_handle).bind(since).fetch_one(&self.pool).await?;
    Ok(n)
}
```

- [ ] **Step 4: Convert `records_for_handle` to read `character_records` with events-fallback**

The lifetime call (`since = None`) reads the rollup; the windowed call (`since = Some`) keeps the live scan (the rollup is lifetime-only — windowed records are not materialized, YAGNI). The real method returns `Result<RecordsAggregate, RepoError>`; construct `RecordsAggregate` with named fields (repo.rs:2107 shows the live impl's field order):
```rust
async fn records_for_handle(&self, claimed_handle: &str, since: Option<DateTime<Utc>>)
    -> Result<RecordsAggregate, RepoError> {
    if since.is_none() {
        self.ensure_session_stats_fresh(claimed_handle).await?;
        // busiest_session_events is NOT NULL (0059); the others are nullable →
        // COALESCE on read to match the live path's `.unwrap_or(0)`.
        if let Some((longest, deadliest, streak, busiest)) =
            sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, i64)>(
                "SELECT longest_session_secs, deadliest_session_deaths,
                        longest_survival_gap_secs, busiest_session_events
                 FROM character_records WHERE claimed_handle = LOWER($1)",
            )
            .bind(claimed_handle)
            .fetch_optional(&self.pool)
            .await?
        {
            return Ok(RecordsAggregate {
                longest_session_secs: longest.unwrap_or(0),
                busiest_session_events: busiest,
                longest_survival_streak_secs: streak.unwrap_or(0),
                deadliest_session_deaths: deadliest.unwrap_or(0),
            });
        }
    }
    // windowed (since = Some) OR rollup-miss: fall through to the original
    // two-scan live computation — keep the existing repo.rs:2038-2113 body verbatim.
}
```
(Preserve the existing live query body as the fallback branch. Confirm `RecordsAggregate`'s field names at repo.rs before writing.)

- [ ] **Step 5: Add `MemoryQuery` no-op guard so offline unit tests are unchanged**

In the `test_support` mock (repo.rs:836+), the mock reads compute directly from in-memory events and never call the guard, so no change is needed to mock read bodies. Confirm the four mock impls still compile and their existing tests (e.g. `records_for_handle_computes_all_time_records`, repo.rs:2956) pass unmodified.

- [ ] **Step 6: Run offline unit tests — expect PASS, unchanged count**

```bash
cargo test -p starstats-server
```
Expected: 1834 tests pass (mock paths unchanged). If any read test now fails, the rollup SQL diverges from the mock's Rust logic — reconcile against the parity harness, not by editing the test.

- [ ] **Step 7: EXPLAIN the converted reads on the container — confirm point/index lookups**

```bash
docker exec -i ss-pg psql -U starstats_app -d starstats -c \
"EXPLAIN (ANALYZE, BUFFERS) SELECT * FROM session_summary WHERE claimed_handle='alice' ORDER BY started_at DESC LIMIT 50;"
```
Expected: Index Scan on `session_summary_handle_start_idx`, not a Seq Scan of `events`.

- [ ] **Step 8: clippy + commit**

```bash
cargo clippy -p starstats-server -- -D warnings
git add crates/starstats-server/src/repo.rs
git commit -m "feat(server): session/records reads hard-cut to rollups via dirty-recompute guard"
```

---

### Task A4: entity `session_count` read from `entity_rollup_agg` — ⛔ DEFERRED (blocked on item B)

> **DEFERRED 2026-07-23.** Discovered during implementation: `derive_entity_session_data` (entity_rollup.rs:679), the function this task replaces, sessionizes entity counts by **`process_init` markers** (`event_type == "process_init"`) — the *event_timeline* session family, matching item B's `derive_sessions`, NOT the 30-min-gap sessionizer. But A2 populated `entity_rollup_agg.session_count` via the **gap** sessionizer. Reading the rollup here would silently change user-visible entity session counts (process_init → gap). Faithful materialization needs the process_init SQL sessionizer, which **is item B**. So this task is blocked on B: keep the existing `session_bounds_rows` + `derive_entity_session_data` walk for entity counts; `entity_rollup_agg.session_count` stays populated-but-unread (dormant, harmless). When B lands, switch A2's entity `session_count` computation to process_init boundaries and then wire this read. The consumer keys counts by `(String::new(), id)` (id-only, kind ignored — entity_rollup.rs:919), so any future materialization must reconcile per-id. The original (now-deferred) task text is retained below for when B unblocks it.

**Files:**
- Modify: `crates/starstats-server/src/entity_rollup.rs:850-908` (the `list_entities` route handler)

**Interfaces:**
- Consumes: `entity_rollup_agg` (Task A2 populates `session_count`); `PostgresStore::ensure_session_stats_fresh` — exposed to this module (make it `pub(crate)` or add a thin `EntityStore` trait method `ensure_entity_rollup_fresh`).
- Produces: unchanged route response; `session_count` now sourced from the rollup on the fast path.

- [ ] **Step 1: Freshen + read `session_count` from the rollup, fall back to the walk on miss**

Replace the `session_bounds_rows` + `derive_entity_session_data` fold (entity_rollup.rs:895-908) with:
```rust
store.ensure_entity_rollup_fresh(&handle).await.ok(); // best-effort; miss => fallback below
let rollup = store.entity_session_counts(&handle).await.unwrap_or_default(); // HashMap<(kind,id), i64>
let session_counts = if rollup.is_empty() {
    match store.session_bounds_rows(&handle).await {
        Ok(rows) => { let (counts, _) = derive_entity_session_data(&rows); counts }
        Err(_) => Default::default(),
    }
} else {
    rollup
};
```
Add the two new `EntityStore` methods (`ensure_entity_rollup_fresh` delegating to `ensure_session_stats_fresh`; `entity_session_counts` = `SELECT entity_kind, entity_id, session_count FROM entity_rollup_agg WHERE claimed_handle = LOWER($1)`), plus mock impls returning an empty map (so the existing walk-based tests still exercise the fallback).

- [ ] **Step 2: Run offline unit tests — expect PASS**

```bash
cargo test -p starstats-server
```
Expected: pass (mock `entity_session_counts` empty => fallback walk => existing `derive_entity_session_data_counts_sessions_per_entity` behavior preserved).

- [ ] **Step 3: Parity on container — rollup session_count == derive_entity_session_data**

Add an EXCEPT block to `scripts/validate/0058_rollup_parity.sql` comparing `entity_rollup_agg.session_count` against a live `COUNT(DISTINCT session_id)` per entity; expect 0 rows.

- [ ] **Step 4: clippy + commit**

```bash
cargo clippy -p starstats-server -- -D warnings
git add crates/starstats-server/src/entity_rollup.rs
git commit -m "feat(server): entity session_count reads from entity_rollup_agg with walk fallback"
```

---

### Task A5: retention flags the handle dirty after a purge

**Files:**
- Modify: `crates/starstats-server/src/retention.rs:202-219` (inside the existing post-purge transaction)

**Interfaces:**
- Consumes: `stat_rollup_state`.
- Produces: after a retention purge, the handle's session rollups recompute on next read (not eagerly — purge stays cheap).

- [ ] **Step 1: Add the dirty flag inside the existing purge transaction**

After the `stat_event_counts` rebuild (retention.rs:218, before `tx.commit()`):
```rust
sqlx::query(
    "INSERT INTO stat_rollup_state (claimed_handle, sessions_dirty, updated_at)
     VALUES ($1, TRUE, now())
     ON CONFLICT (claimed_handle) DO UPDATE SET sessions_dirty = TRUE, updated_at = now()",
)
.bind(&handle)
.execute(&mut *tx)
.await?;
```

- [ ] **Step 2: Container check — purge then read recomputes**

Delete some rows for a handle via retention, then call a converted read (A3); confirm `session_summary` reflects the reduced event set (parity harness re-run = 0 rows).

- [ ] **Step 3: Offline tests + clippy + commit**

```bash
cargo test -p starstats-server && cargo clippy -p starstats-server -- -D warnings
git add crates/starstats-server/src/retention.rs
git commit -m "fix(server): retention purge flags session rollups dirty for recompute"
```

---

### Task A6: Full validation + ship

- [ ] **Step 1: Full offline suite + workspace clippy + fmt**

```bash
cargo test -p starstats-server && cargo clippy --workspace -- -D warnings && cargo fmt --all --check
```
Expected: 1834+ pass, clean, formatted.

- [ ] **Step 2: End-to-end container run — boot the server against `ss-pg`, hit `/me` endpoints**

Run migrations (boot applies `0058`), exercise entities/records/sessions endpoints for a seeded handle, confirm correct numbers + Index Scans (not Seq Scans of `events`).

- [ ] **Step 3: Land on `next`**

```bash
git fetch origin && git rebase origin/next
git push -u origin perf/pg-rollup-materialization
# open PR to next, merge (auto-alpha bump fires; wait for it to finish)
```

- [ ] **Step 4: Live dry-run promote, verify, then live on explicit "go"**

```bash
gh workflow run promote.yml --ref next -f track=platform -f channel=live -f dry_run=true
# approve production-release gate (env 15704172652, typed JSON body), verify printed plan
# then, on "go": re-dispatch with dry_run=false; confirm both live jobs conclusion=success,
# origin/main==origin/next, bare tag vX.Y.Z at that SHA.
```

---

# Plan B — Sessionizer Unify (STAT-5)

**Independently shippable:** yes. **Do NOT start until Plan A is live** (A depends on the 30-min-gap SQL sessionizer semantics staying put; unifying changes boundaries).

**The trap (verbatim from memory):** `event_timeline::derive_sessions` (event_timeline.rs:602) boundaries sessions on `process_init`/`session_end` markers; the SQL `sessions_for_handle` uses a 30-min idle gap. **Different semantics** — a naive swap changes which sessions the timeline shows. The goal is ONE sessionizer whose output matches today's `derive_sessions` for the timeline, so we must replicate the `process_init` boundary logic in SQL and parity-test it, not adopt the gap logic for the timeline.

## File Structure (Plan B)

- **Modify** `crates/starstats-server/src/repo.rs` — add a real `sessions_for_handle_process_init` SQL method (or a `boundary_mode` param) that boundaries on `process_init` rows, mirroring `derive_sessions`.
- **Modify** `crates/starstats-server/src/event_timeline.rs:350` — `list_sessions` calls the new SQL method instead of loading rows + `derive_sessions` in Rust.
- **Keep** `derive_sessions` (event_timeline.rs:602) as the parity oracle + its unit tests (1211-1320) until parity is proven, then decide whether to retire it.

### Task B1: Replicate `process_init` boundary logic in SQL

**Files:** Modify `crates/starstats-server/src/repo.rs`.

**Interfaces:**
- Consumes: `events` (`event_type`, `metadata` session id, payload `local_session` fallback — mirror `session_id_from_row`); `NON_SESSION_EVENT_TYPES`.
- Produces: `PostgresStore::sessions_for_handle_process_init(handle, limit) -> Vec<TimelineSession>` returning the same session boundaries `derive_sessions` produces.

- [ ] **Step 1: Write the SQL** — a window pass that opens a new session on `event_type='process_init'` (keyed by the metadata/payload session id), closes on `session_end`, skips `NON_SESSION_EVENT_TYPES`, and leaves a trailing session `ended_at = NULL`. Use `SUM(CASE WHEN event_type='process_init' THEN 1 ELSE 0 END) OVER (ORDER BY event_timestamp)` as the session ordinal, then `GROUP BY` it. (Full SQL to be written against the exact `derive_sessions` field mapping at implementation time — the boundary predicate is the `process_init` marker, NOT the 30-min gap.)

- [ ] **Step 2: Parity oracle** — extend the existing `derive_sessions` unit fixtures (event_timeline.rs:1176-1320) into a container parity test: load the SAME fixture rows into `ss-pg`, run the new SQL, assert its sessions equal `derive_sessions(rows)` element-for-element (start, end, count, id). Expected: identical for every fixture, including the `process_init` reopen and trailing-open cases.

- [ ] **Step 3:** clippy + commit.

### Task B2: Point `list_sessions` at the SQL sessionizer

**Files:** Modify `crates/starstats-server/src/event_timeline.rs:350`.

- [ ] **Step 1:** Replace the row-load + `derive_sessions` call with `store.sessions_for_handle_process_init(&handle, SESSIONS_LIST_LIMIT)`.
- [ ] **Step 2:** Run offline suite — the mock impl must reproduce `derive_sessions` (reuse it directly in the mock so existing timeline tests pass unchanged). Expected: PASS.
- [ ] **Step 3:** Container EXPLAIN — confirm the timeline read is an index scan, not a full events load into Rust.
- [ ] **Step 4:** Decide `derive_sessions` fate — if no non-test caller remains, keep it as the mock impl + oracle (it is still the mock's engine); do not delete. clippy + commit.

### Task B3: Validation + ship — same gate as A6 (offline suite, container parity, `next` → dry-run → live on "go").

---

# Plan C — Audit Async Writer (ING-3)

**Independently shippable:** yes; independent of A and B. **Integrity-sensitive** — the hash chain must stay verifiable. **First task adds the missing hash-chain test** (there is none today) so the refactor has a safety net.

**The problem (verbatim from map):** `audit.rs::append` (:195) takes a cluster-wide `pg_advisory_xact_lock(0x416C6F6700000001)` + tail `SELECT … FOR UPDATE` per mutation — a serialization ceiling across every audited action (~19 modules, ingest/retention/sharing/auth/admin). Replace with an in-process single-writer queue draining a channel, preserving strict seq/hash ordering. Emission is already best-effort, so a queue fits.

## File Structure (Plan C)

- **Create** `crates/starstats-server/tests/audit_hash_chain.rs` OR a container-gated `#[tokio::test]` in `audit.rs` behind `STARSTATS_TEST_DATABASE_URL` (early-returns when unset, so offline `cargo test` is unaffected) — appends N entries concurrently, asserts `prev_hash[i] == row_hash[i-1]` and strictly increasing `seq`.
- **Modify** `crates/starstats-server/src/audit.rs` — introduce `AuditWriter` (owns a `tokio::sync::mpsc::Receiver<(AuditEntry, oneshot::Sender<Result<()>>)>` + the `PgPool`), spawned once at startup; `append` becomes "send onto the channel, await the oneshot." The single writer task runs the existing INSERT sequence **without** the advisory lock (single-writer ⇒ no concurrent chain writers), keeping the tail read + hash construction.
- **Modify** the startup wiring (where `PostgresAuditLog` is constructed) to spawn the writer task and hand its `Sender` to the `AuditLog` handle.

### Task C1: Add the hash-chain verification test (the safety net)

**Files:** Create `crates/starstats-server/tests/audit_hash_chain.rs` (or gated inline test).

- [ ] **Step 1:** Write a test that, against `STARSTATS_TEST_DATABASE_URL` (skip when unset), spawns K concurrent `append`s of distinct entries, then reads `audit_log ORDER BY seq` and asserts: `seq` is `1..=N` contiguous, each `prev_hash == previous row_hash`, first `prev_hash == [0u8;32]`, and each `row_hash == sha256(prev_hash || canonical_payload || seq_str)`.
- [ ] **Step 2:** Run against `ss-pg` with the CURRENT advisory-lock `append` — expect PASS (establishes the baseline the refactor must preserve).
- [ ] **Step 3:** Confirm offline `cargo test -p starstats-server` still green (test early-returns without the URL). Commit.

### Task C2: Introduce the single-writer queue

**Files:** Modify `crates/starstats-server/src/audit.rs` + startup wiring.

**Interfaces:**
- Produces: `AuditWriter::spawn(pool) -> AuditSender`; `PostgresAuditLog { sender: AuditSender }`; `append` sends `(entry, oneshot)` and awaits the reply. Writer task: `while let Some((entry, reply)) = rx.recv().await { reply.send(self.write_locked_free(entry).await) }`.

- [ ] **Step 1:** Add `AuditWriter` + channel; move the INSERT sequence into the writer's `write_one` (drop `pg_advisory_xact_lock`; keep `BEGIN` + tail `SELECT … FOR UPDATE` + hash + INSERT + COMMIT). Single consumer ⇒ appends are serialized in-process, so the chain has exactly one writer.
- [ ] **Step 2:** Rewrite `append` to `send + oneshot.await`; preserve the best-effort mirror (either in the writer after commit, or unchanged).
- [ ] **Step 3:** Run the Task C1 hash-chain test against `ss-pg` — expect PASS (chain identical, now lock-free).
- [ ] **Step 4:** Offline suite (mock `AuditLog` unchanged) + clippy — expect PASS. Commit.

### Task C3: Validation + ship — Task C1 test green against container under concurrency, offline suite green, `next` → dry-run → live on "go".

---

## Self-Review

**Spec coverage:** A (materialization) — backfill A1, rebuild fn A2, read conversions A3/A4, retention hook A5, ship A6 ✓. B (sessionizer unify) — SQL replication B1, wiring B2, ship B3 ✓. C (audit async) — safety-net test C1, queue C2, ship C3 ✓. The "MUST ship a backfill" rule → A1. The wCTE self-collision rule → A2 uses separate DELETE/INSERT statements. The "no live-DB test harness" reality → every SQL task carries a container parity/EXPLAIN gate; C adds an env-gated integration test.

**Placeholder scan:** B1 Step-1 SQL and B2 mock reuse are described rather than fully coded because the exact `session_id_from_row`/`NON_SESSION_EVENT_TYPES` mapping must be read at implementation time from `event_timeline.rs` — flagged explicitly, not hidden. A2/A3/A4/A5/C are fully coded.

**Type consistency:** `rebuild_handle_session_stats` (A2) returns `Result<(), RepoError>` and is called by `ensure_session_stats_fresh` (A3) and `ensure_entity_rollup_fresh` (A4). `SESSION_IDLE_GAP_MINUTES` binds `as i32` (matches the existing sessionizer). Read return types are the real ones — `sessions_for_handle → Vec<InferredSession>`, `records_for_handle → RecordsAggregate` (fields: longest_session_secs, busiest_session_events, longest_survival_streak_secs, deadliest_session_deaths) — verify field↔column mapping against the structs at implementation time (noted in A3 Steps 2/4). `session_summary.session_id` is TEXT (ordinal cast `::text`).

**Open verification items for the implementer (do at task time, not now):**
1. Confirm `InferredSession` (repo.rs:218) / `RecordsAggregate` (repo.rs:~2107) field names before writing A3 reads; confirm `total_playtime_secs`/`count_sessions_since` exact aggregation before mirroring them.
2. Confirm `character_records` column set in 0056 matches the A2 upsert column list exactly.
3. Read `session_id_from_row` + `NON_SESSION_EVENT_TYPES` before writing B1 SQL.
