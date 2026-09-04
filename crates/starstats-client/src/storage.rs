//! Local SQLite event store. Wraps `rusqlite` behind a `Mutex` so the
//! tail loop and Tauri command handlers can share it.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use starstats_core::unknown_lines::UnknownLine;
use std::path::Path;
use std::sync::Mutex;

/// Surfacing threshold matching the spec — lines below this never make
/// it into the review queue by default. Callers can pass a lower (or
/// zero) cutoff if they want to inspect everything that was captured.
/// Exposed so the eventual Tauri command + UI badge agree on the cutoff
/// without each importing the literal `50`.
#[allow(dead_code)]
pub const UNKNOWN_LINE_MIN_INTEREST: u8 = 50;

/// Cap on `raw_examples_json` entries. Keep this tight: reviewers only
/// need a handful of concrete samples to sanity-check a shape, and the
/// JSON blob is read whole on every upsert.
#[allow(dead_code)]
const RAW_EXAMPLES_CAP: usize = 5;

/// Max distinct shell tags reported in one parser-health push. Matches the
/// server's `MAX_TAGS_PER_BATCH`; a larger batch is rejected outright there.
const UNKNOWN_TAG_REPORT_LIMIT: i64 = 500;

/// Metadata-only projection of the unknown-line queue: shell tag plus
/// sighting window and count. No line bodies — see `unknown_tag_metadata`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnknownTagRow {
    pub shell_tag: String,
    pub first_seen: String,
    pub last_seen: String,
    pub occurrences: i64,
    pub game_build: Option<String>,
}

/// Max rows `list_unknown_lines` returns in one call. The review pane is
/// hand-reviewed and unvirtualized, so a large result set only makes the
/// page slow without helping anyone; the ORDER BY keeps the top shapes.
const REVIEW_LIST_LIMIT: i64 = 200;

const SCHEMA: &str = include_str!("../sql/schema.sql");

pub struct Storage {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct UnsentEvent {
    pub id: i64,
    pub idempotency_key: String,
    pub payload_json: String,
    pub raw_line: String,
    pub log_source: String,
    pub source_offset: u64,
}

/// One row from `events`, restricted to the columns the timeline UI
/// needs. `payload_json` is the same string we wrote on insert, so
/// callers can deserialise it back into a `GameEvent` for formatting.
/// `raw_line` is the exact log line as captured from disk; the Logs
/// pane surfaces it in the per-event detail drawer for forensic
/// inspection. `log_source` is the channel tag (LIVE/PTU/EPTU) so the
/// drawer's Source row reflects which build the event came from rather
/// than guessing.
#[derive(Debug, Clone)]
pub struct RecentEventRow {
    pub id: i64,
    pub event_type: String,
    pub timestamp: String,
    pub payload_json: String,
    pub raw_line: String,
    pub log_source: String,
    /// `NULL` if the row is still pending in the drain queue, a bare
    /// datetime string (`datetime('now')` shape) if the sync worker
    /// shipped it, or `__quarantined_<ts>` if the poison-pill path
    /// shelved it. UI uses this to derive the `synced` flag — see
    /// `synced_from_sent_at` in commands.rs.
    pub sent_at: Option<String>,
}

/// Full row from `events` — used by the re-parse iterator. Has every
/// column the classifier could need to either re-score the line or
/// rewrite the payload in place. Several fields are not consumed by
/// the current re-parse path but are kept on the struct so future
/// passes (e.g. backfill-with-rules-applied) don't need to widen the
/// SELECT shape.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EventRow {
    pub id: i64,
    pub idempotency_key: String,
    pub event_type: String,
    pub timestamp: String,
    pub raw_line: String,
    pub payload_json: String,
    pub log_source: String,
    pub source_offset: u64,
    /// Optional EventMetadata JSON blob — None for rows captured before
    /// the enrichment pass attached metadata. Read so re-parse can
    /// inspect previously-stamped provenance and preserve it across
    /// re-classifications that don't otherwise touch the metadata.
    pub metadata_json: Option<String>,
}

/// Lean projection used by the retro-burst phase of re-parse. Carries
/// only the columns `detect_bursts` needs (raw line for
/// `structural_parse`, offset for the idempotency key) plus the row
/// `id` so members can be deleted after the summary is inserted, and
/// `event_type` so already-collapsed `burst_summary` rows can be
/// trivially skipped without re-parsing them.
#[derive(Debug, Clone)]
pub struct BurstScanRow {
    pub id: i64,
    pub raw_line: String,
    pub source_offset: u64,
    pub event_type: String,
}

/// One row from `unknown_event_samples`. Mirrors every non-id column
/// in the table; the command layer maps it onto its serialised wire
/// counterpart.
#[derive(Debug, Clone)]
pub struct UnknownSampleRow {
    pub log_source: String,
    pub event_name: String,
    pub occurrences: u64,
    pub first_seen: String,
    pub last_seen: String,
    pub sample_line: String,
    pub sample_body: String,
}

/// Engine-internal events we know aren't worth surfacing as
/// candidate parser rules. Seeded into `event_noise_list` on first
/// run; users can extend this from the UI later. Keep this list
/// conservative — anything ambiguous (could plausibly carry player
/// signal) belongs in the unknowns table until classified.
const DEFAULT_NOISE: &[&str] = &[
    // Asset cache chatter.
    "StatObjLoad 0x800 Format",
    // Engine state machine — fires hundreds of times per session.
    "CContextEstablisherStepStart",
    "ContextEstablisherTaskFinished",
    "Context Establisher Blocked",
    "Context Establisher Unblocked",
    "Context Establisher Done",
    "ContextEstablisher Model Change State",
    "ContextEstablisher State Change Delivery Result",
    "ContextEstablisher Send State Change",
    "ContextEstablisher Remote Change State Success",
    // Hangar elevator / loading-platform internals.
    "CSCLoadingPlatformManager::TransitionLightGroupState",
    "CSCLoadingPlatformManager::LoadEntitiesReference",
    "CSCLoadingPlatformManager::LoadEntitiesReference::<lambda_1>::operator ()",
    "CSCLoadingPlatformManager::OnLoadingPlatformStateChanged",
    "CSCLoadingPlatformManager::StopEffectForAllTags",
    "CSCLoadingPlatformManager::loadEntityFromXML::<lambda_1>::operator ()",
    "LoadingPlatformUtilities::LoadFromXmlNode",
    // Misc engine bookkeeping.
    "Update group cache",
    "SerializedOverwrite",
    "RegisterUniverseHierarchy_End",
    "ReuseChannel",
    "Stream started",
    "[BuildingBlocks] Invalid Url",
    "ProximitySensorMakingLocalHelper",
    // --- Added 2026-07-31 from a sweep of a 1,030,040-occurrence
    // unknown-line corpus, where 68% of all occurrences were engine or
    // UI chatter while this list held 26 entries.
    //
    // Only unambiguous engine/UI internals are added. Anything that
    // could plausibly carry player signal is deliberately left in the
    // unknowns table to be classified, per this list's own rule —
    // notably the InventoryManagement / RequestInventory /
    // QueryInventory families (the three largest by volume, ~331k
    // occurrences) which describe inventory operations and may encode
    // what a player moved, and CObjectiveMarkerComponent, which is
    // mission-adjacent.
    //
    // Siblings of an entry already here — `_End` was listed while
    // `_Begin` and `_Mid` were not, at 3,868 occurrences each.
    "RegisterUniverseHierarchy_Begin",
    "RegisterUniverseHierarchy_Mid",
    // Server-side routing internals; no player action produces these.
    "Local Route Guard - Server Rerouted",
    // Pure UI lifecycle — a drag handler and grid teardown.
    "OnDragInventoryItemModifyTarget",
    "Close Inventory Grid",
    "Remove Inventory Container UI",
    "UpdateNotificationItem",
    // Network/session plumbing.
    "Connection Flow",
    // Engine warnings that fire on internal state, not player action.
    "Previous index is larger than current",
    "Invalid path",
    "Too many actions",
];

impl Storage {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create db dir")?;
        }
        let conn = Connection::open(path).context("open sqlite")?;
        conn.execute_batch(SCHEMA).context("apply schema")?;
        Self::migrate_events_metadata(&conn).context("migrate events.metadata column")?;
        Self::migrate_events_sent_at(&conn).context("migrate events.sent_at column")?;
        Self::migrate_tail_cursor_sig(&conn).context("migrate tail_cursor.file_sig column")?;
        Self::seed_default_noise(&conn).context("seed default noise list")?;
        Self::purge_noise_from_unknowns(&conn).context("purge stale noise samples")?;
        Self::purge_garbage_from_unknown_lines(&conn).context("purge garbage unknown lines")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Idempotently add the `metadata` column to `events` for databases
    /// created before the column landed in `schema.sql`. SQLite has no
    /// `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` so we probe
    /// `PRAGMA table_info` first and only emit the ALTER when the
    /// column is missing. Failure modes: a corrupt schema (the probe
    /// returns no rows for the events table) bubbles up so the open
    /// fails loudly rather than silently dropping the migration.
    fn migrate_events_metadata(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(events)")?;
        let has_metadata = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "metadata");
        drop(stmt);
        if !has_metadata {
            conn.execute("ALTER TABLE events ADD COLUMN metadata TEXT", [])?;
        }
        Ok(())
    }

    /// Idempotently add the `sent_at` column + partial index to
    /// `events`, then backfill existing rows from the legacy
    /// `sync_cursor.last_event_id` high-water mark. Without the
    /// backfill, every row that was already drained under the old
    /// cursor model would suddenly look unsent and re-ship on first
    /// boot after upgrade — the server's idempotency_key UNIQUE would
    /// dedupe it, but it would cost a pointless round-trip per
    /// historical event.
    ///
    /// Why the index lives here rather than in `schema.sql`: this
    /// migration owns the column. `schema.sql` is applied BEFORE
    /// migrations, so an index whose WHERE clause names `sent_at`
    /// can't live there — on a legacy DB the column doesn't exist yet
    /// when schema.sql runs.
    fn migrate_events_sent_at(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(events)")?;
        let has_sent_at = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "sent_at");
        drop(stmt);

        if !has_sent_at {
            conn.execute("ALTER TABLE events ADD COLUMN sent_at TEXT", [])?;

            // Backfill: anything `id <= sync_cursor.last_event_id`
            // was already shipped under the old cursor model. Mark
            // as sent so the new per-row model agrees. Only worth
            // doing on a legacy upgrade — a fresh install has no
            // pre-existing rows to back-stamp.
            let cursor: rusqlite::Result<i64> =
                conn.query_row("SELECT last_event_id FROM sync_cursor", [], |row| {
                    row.get(0)
                });
            let last_sent = match cursor {
                Ok(n) => n.max(0),
                Err(rusqlite::Error::QueryReturnedNoRows) => 0,
                Err(e) => return Err(e.into()),
            };
            if last_sent > 0 {
                conn.execute(
                    "UPDATE events SET sent_at = COALESCE(inserted_at, datetime('now'))
                     WHERE id <= ? AND sent_at IS NULL",
                    params![last_sent],
                )?;
            }
        }

        // Always ensure the partial index exists — fresh installs
        // (column added by schema.sql via the new shape) and legacy
        // upgrades (column just ALTERed in) both need it.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_events_unsent ON events(id) WHERE sent_at IS NULL",
            [],
        )?;
        Ok(())
    }

    /// Idempotently add the `file_sig` column to `tail_cursor` for
    /// databases created before it landed in `schema.sql`. Mirrors the
    /// probe-then-ALTER pattern above (SQLite has no `ADD COLUMN IF NOT
    /// EXISTS`). The column is nullable with no backfill: legacy rows
    /// simply carry a `NULL` signature until their next drain rewrites
    /// them, and `resolve_resume_offset` treats a `NULL` stored
    /// signature as "no rotation signal" so the upgrade never triggers
    /// a spurious re-read.
    fn migrate_tail_cursor_sig(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(tail_cursor)")?;
        let has_sig = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "file_sig");
        drop(stmt);
        if !has_sig {
            conn.execute("ALTER TABLE tail_cursor ADD COLUMN file_sig TEXT", [])?;
        }
        Ok(())
    }

    fn seed_default_noise(conn: &Connection) -> Result<()> {
        // Drop obsolete shipped builtins whose captured form changed
        // when the parser learned to handle nested `<...>` symbols.
        // Only touches rows tagged `builtin` — user-added entries with
        // the same name are kept (they'd have source='user').
        conn.execute(
            "DELETE FROM event_noise_list
             WHERE source = 'builtin'
               AND event_name LIKE '%::<lambda%'
               AND event_name NOT LIKE '%operator ()%'",
            [],
        )?;

        // Idempotently insert every shipped builtin. ON CONFLICT keeps
        // user-added entries with the same name from being clobbered.
        for name in DEFAULT_NOISE {
            conn.execute(
                "INSERT INTO event_noise_list(event_name, source) VALUES (?, 'builtin')
                 ON CONFLICT(event_name) DO NOTHING",
                params![name],
            )?;
        }
        Ok(())
    }

    /// One-shot cleanup: when the app boots and the noise list shifts
    /// (new defaults shipped, user added an entry), retroactively drop
    /// matching rows from `unknown_event_samples` so the actionable
    /// list stays clean. Cheap — the unknowns table is small.
    ///
    /// Also drops any malformed legacy rows whose `event_name` was
    /// captured by an older parser that truncated nested `<...>`
    /// symbols. A correctly-captured event with embedded `<lambda_N>`
    /// always ends in `>`; anything matching `*::<lambda*` that does
    /// NOT end in `>` was produced by the buggy parser.
    fn purge_noise_from_unknowns(conn: &Connection) -> Result<()> {
        conn.execute(
            "DELETE FROM unknown_event_samples
             WHERE event_name IN (SELECT event_name FROM event_noise_list)",
            [],
        )?;
        conn.execute(
            "DELETE FROM unknown_event_samples
             WHERE event_name LIKE '%::<lambda%'
               AND event_name NOT LIKE '%>'",
            [],
        )?;
        Ok(())
    }

    /// Drop already-captured `unknown_lines` rows that the live capture
    /// gate would now reject as garbage (VFX/particle chatter). Runs at
    /// open so a marker added after those rows were captured still clears
    /// the review queue. Patterns come from
    /// [`starstats_core::GARBAGE_LINE_MARKERS`] — the same list the ingest
    /// gate uses — matched against both the `shell_tag` column and the
    /// raw examples blob (`[` is a literal in SQLite `LIKE`).
    fn purge_garbage_from_unknown_lines(conn: &Connection) -> Result<()> {
        for marker in starstats_core::GARBAGE_LINE_MARKERS {
            let like = format!("%{marker}%");
            conn.execute(
                "DELETE FROM unknown_lines
                 WHERE shell_tag LIKE ?1 OR raw_examples_json LIKE ?1",
                params![like],
            )?;
        }
        Ok(())
    }

    /// Cheap membership test used by the tailer's hot path.
    pub fn is_noise(&self, event_name: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event_noise_list WHERE event_name = ?",
            params![event_name],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Add an event_name to the noise list. `source` is informational
    /// — typically `"user"` from the tray UI, `"builtin"` from the
    /// seeded defaults, or `"community"` from a future rule-sync feed.
    pub fn add_noise(&self, event_name: &str, source: &str) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "INSERT INTO event_noise_list(event_name, source) VALUES (?, ?)
             ON CONFLICT(event_name) DO NOTHING",
            params![event_name, source],
        )?;
        // Also drop any existing samples — they're noise now.
        conn.execute(
            "DELETE FROM unknown_event_samples WHERE event_name = ?",
            params![event_name],
        )?;
        Ok(())
    }

    /// Insert one event row, deduplicating on `idempotency_key`.
    ///
    /// Returns `Ok(true)` when a new row was written, `Ok(false)` when a
    /// row with that key was already present. The distinction is
    /// load-bearing for any caller that suppresses OTHER rows in favour
    /// of the one it is inserting — see the burst path in
    /// [`crate::gamelog::process_buffer`]. Under a bare `Result<()>`,
    /// "stored" and "silently discarded" are indistinguishable, which is
    /// how a burst can end up with its members suppressed and no summary
    /// standing in for them; the events are then gone from the local
    /// store with nothing logged.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_event(
        &self,
        idempotency_key: &str,
        event_type: &str,
        timestamp: &str,
        raw: &str,
        payload_json: &str,
        log_source: &str,
        source_offset: u64,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        // ON CONFLICT keeps the table append-only-ish: same line
        // re-tailed (after a rotation/replay) won't double-insert.
        let inserted = conn.execute(
            "INSERT INTO events
                (idempotency_key, type, timestamp, raw, payload, log_source, source_offset)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(idempotency_key) DO NOTHING",
            params![
                idempotency_key,
                event_type,
                timestamp,
                raw,
                payload_json,
                log_source,
                source_offset as i64,
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Read up to `limit` unsent events, filtered by event type.
    ///
    /// Modes:
    /// - `priority_types` empty → return any unsent row, oldest first.
    ///   This is the "bulk" lane.
    /// - `priority_types` non-empty + `priority_only = true` → return
    ///   only rows whose `type` IS IN the list. This is the "fast" lane.
    /// - `priority_types` non-empty + `priority_only = false` → return
    ///   only rows whose `type` is NOT in the list. This is the bulk
    ///   lane when a fast lane is configured (so fast-lane events
    ///   don't get drained twice — fast lane handles them first, bulk
    ///   lane handles everything else).
    ///
    /// Ordering: `(id ASC)`. Within a lane we drain in insertion
    /// order; the partial index `idx_events_unsent` keeps the scan
    /// proportional to the unsent set, not the full history.
    pub fn read_unsent_filtered(
        &self,
        priority_types: &[&str],
        priority_only: bool,
        limit: usize,
    ) -> Result<Vec<UnsentEvent>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");

        if priority_types.is_empty() {
            // No filter — straight unsent drain. Used when the fast
            // lane is empty/disabled.
            let mut stmt = conn.prepare(
                "SELECT id, idempotency_key, payload, raw, log_source, source_offset
                 FROM events
                 WHERE sent_at IS NULL
                 ORDER BY id ASC
                 LIMIT ?",
            )?;
            let rows = stmt.query_map(params![limit as i64], map_unsent_row)?;
            return Ok(rows.filter_map(|r| r.ok()).collect());
        }

        // Build "?, ?, ?" placeholders for the IN clause. rusqlite
        // doesn't natively bind slices, so we expand placeholders +
        // bind each name positionally.
        let placeholders = std::iter::repeat_n("?", priority_types.len())
            .collect::<Vec<_>>()
            .join(",");
        let in_or_not = if priority_only { "IN" } else { "NOT IN" };
        let sql = format!(
            "SELECT id, idempotency_key, payload, raw, log_source, source_offset
             FROM events
             WHERE sent_at IS NULL AND type {in_or_not} ({placeholders})
             ORDER BY id ASC
             LIMIT ?"
        );

        let mut params_dyn: Vec<&dyn rusqlite::ToSql> =
            Vec::with_capacity(priority_types.len() + 1);
        for t in priority_types {
            params_dyn.push(t);
        }
        let limit_i = limit as i64;
        params_dyn.push(&limit_i);

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params_dyn), map_unsent_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Stamp `sent_at = now` on every row whose `id` is in `ids`.
    /// Idempotent: rows that already have a `sent_at` are skipped
    /// (the COALESCE in WHERE makes this a no-op for them), so a
    /// retry that re-sends a partial batch can't reset the timestamp.
    /// Splits into chunks of 500 ids to keep SQL parameter binding
    /// within SQLite's default `SQLITE_MAX_VARIABLE_NUMBER`.
    pub fn mark_sent(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().expect("storage mutex poisoned");
        for chunk in ids.chunks(500) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "UPDATE events SET sent_at = datetime('now')
                 WHERE sent_at IS NULL AND id IN ({placeholders})"
            );
            let params_dyn: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
            conn.execute(&sql, rusqlite::params_from_iter(params_dyn))?;
        }
        Ok(())
    }

    /// Quarantine rows that the API server has rejected at the
    /// per-batch level (4xx other than 401/403). Stamps `sent_at` with
    /// a `__quarantined_<timestamp>` sentinel so the partial-index
    /// drain query (which filters on `sent_at IS NULL`) skips them
    /// while keeping the row available for forensic inspection.
    ///
    /// Used by the sync worker's poison-pill isolation path: when a
    /// batch fails with a 4xx, the worker bisects until it finds the
    /// single offending event, then quarantines it so the rest of the
    /// queue can drain. Without this, one malformed row (oversized
    /// raw_line, schema-version skew on a single envelope, etc.)
    /// would block every following row indefinitely.
    ///
    /// Idempotent: the `sent_at IS NULL` guard means rows already sent
    /// or already quarantined are left untouched.
    pub fn mark_quarantined(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().expect("storage mutex poisoned");
        for chunk in ids.chunks(500) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "UPDATE events SET sent_at = '__quarantined_' || datetime('now')
                 WHERE sent_at IS NULL AND id IN ({placeholders})"
            );
            let params_dyn: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
            conn.execute(&sql, params_dyn.as_slice())?;
        }
        Ok(())
    }

    /// Per-type counts of rows this client believes it has DELIVERED —
    /// `sent_at` holds a real timestamp.
    ///
    /// This is the local half of drift detection. It is deliberately NOT
    /// [`Storage::event_counts`], which caps at 50 types and counts rows in
    /// every state: a truncated list would report "no drift" for any type
    /// outside the top 50, and counting queued rows would flag events that
    /// are simply waiting their turn.
    ///
    /// Quarantined rows are excluded. The server rejected those and never
    /// will accept them, so counting them as missing would produce drift
    /// that can never be cleared.
    pub fn sent_counts_by_type(&self) -> Result<Vec<(String, u64)>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            r"SELECT type, COUNT(*) FROM events
              WHERE sent_at IS NOT NULL
                AND sent_at NOT LIKE '\_\_quarantined\_%' ESCAPE '\'
              GROUP BY type",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Put delivered rows of the given types back in the drain queue by
    /// clearing their `sent_at`.
    ///
    /// Used to recover when the SERVER has lost events this client already
    /// uploaded — nothing else un-marks a delivered row, so without this the
    /// data is unrecoverable from the client even though it is sitting right
    /// there in SQLite.
    ///
    /// Safe to run: `/v1/ingest` dedupes on `idempotency_key`, so anything
    /// the server still holds comes back as `duplicate` rather than being
    /// stored twice.
    ///
    /// Quarantined rows are left alone — they have their own release path
    /// (`release_quarantined`) and re-sending content the server actively
    /// rejected would just re-quarantine it.
    ///
    /// Returns the number of rows re-queued.
    pub fn requeue_sent_for_types(&self, types: &[&str]) -> Result<u64> {
        if types.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut total = 0u64;
        // Chunked to stay under SQLITE_MAX_VARIABLE_NUMBER, same as mark_sent.
        for chunk in types.chunks(500) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                r"UPDATE events SET sent_at = NULL
                  WHERE sent_at IS NOT NULL
                    AND sent_at NOT LIKE '\_\_quarantined\_%' ESCAPE '\'
                    AND type IN ({placeholders})"
            );
            let params_dyn: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|t| t as &dyn rusqlite::ToSql).collect();
            total += conn.execute(&sql, rusqlite::params_from_iter(params_dyn))? as u64;
        }
        Ok(total)
    }

    /// Count rows still waiting in the drain queue (`sent_at IS NULL`).
    /// Quarantined rows carry a `__quarantined_*` sentinel rather than
    /// NULL, so they are excluded here and counted separately by
    /// [`Storage::count_quarantined`].
    ///
    /// Served by the partial index `idx_events_unsent` (see
    /// `migrate_events_sent_at`), so this stays an index-only scan even
    /// on a six-figure backlog. Read by the tray's backlog readout and
    /// by the catch-up ETA; NOT called on the drain hot path, which
    /// infers "more pending" from a full page instead.
    pub fn count_unsent(&self) -> Result<i64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE sent_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    /// Count rows the poison-pill path has shelved. Read by the tray
    /// UI / status surface so the user can see N rows were dropped
    /// even though no error was visible at drain time. Matches on the
    /// `__quarantined_` prefix on `sent_at` — a sentinel that real
    /// successful sends never produce (those write a bare RFC3339
    /// timestamp via `datetime('now')`).
    pub fn count_quarantined(&self) -> Result<i64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n: i64 = conn.query_row(
            // Escape the literal underscores: a bare `_` in LIKE is a
            // single-char wildcard, so the pattern would also match
            // strings with any two leading chars before "quarantined".
            // No real sentinel collides today, but ESCAPE makes the
            // prefix match exact (L9).
            r"SELECT COUNT(*) FROM events WHERE sent_at LIKE '\_\_quarantined\_%' ESCAPE '\'",
            [],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    /// Release all quarantined rows back into the drain queue. Flips
    /// `sent_at` from a `__quarantined_<ts>` sentinel back to NULL on
    /// every matching row so the next drain re-reads them through the
    /// normal `sent_at IS NULL` filter. Returns the count released.
    ///
    /// Recovery path for the case where poison-pill isolation
    /// quarantined events that should have been retried — typically a
    /// transient batch-level 4xx (schema-version skew, rate-limit,
    /// missing field on every event in the batch) that bisection
    /// misinterpreted as event-specific. After release, the next drain
    /// re-attempts them. If the underlying cause persists they get
    /// re-quarantined (capped per `MAX_QUARANTINES_PER_DRAIN`), so
    /// calling this repeatedly without fixing root cause won't help.
    ///
    /// Idempotent: returns `0` when no quarantined rows exist.
    pub fn release_quarantined(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n = conn.execute(
            // Literal underscores (see count_quarantined) — ESCAPE so
            // `_` isn't treated as a single-char wildcard (L9).
            r"UPDATE events SET sent_at = NULL WHERE sent_at LIKE '\_\_quarantined\_%' ESCAPE '\'",
            [],
        )?;
        Ok(n as u64)
    }

    /// Legacy single-lane drain — read up to `limit` events with
    /// `id > after_id`, ordered by id. Retained for back-compat with
    /// any caller (or test) that predates the per-row sent_at flag.
    /// Internally delegates to `read_unsent_filtered` with no priority
    /// types so the new schema is the source of truth either way.
    ///
    /// NOTE: `after_id` is now ignored in favour of `sent_at IS NULL`
    /// — the cursor-based ordering it implied is fundamentally
    /// incompatible with priority lanes (out-of-order drains). Callers
    /// that still pass a cursor will get every unsent row regardless.
    /// Mark as `#[deprecated]` once the last in-tree user (legacy tests)
    /// moves to `read_unsent_filtered` / `mark_sent`.
    #[allow(dead_code)]
    pub fn read_unsent(&self, _after_id: i64, limit: usize) -> Result<Vec<UnsentEvent>> {
        self.read_unsent_filtered(&[], false, limit)
    }

    // (legacy `read_sync_cursor` / `write_sync_cursor` removed —
    // replaced by per-row `events.sent_at` in the priority-lanes
    // refactor. The `sync_cursor` SQLite table is still created and
    // migrated for back-stamping legacy rows, but it sits frozen
    // once the migration completes; no production code reads or
    // writes it post-migration.)

    pub fn read_cursor(&self, source_path: &str) -> Result<u64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT offset FROM tail_cursor WHERE path = ?",
            params![source_path],
            |row| row.get(0),
        );
        match result {
            Ok(n) => Ok(n.max(0) as u64),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    pub fn write_cursor(&self, source_path: &str, offset: u64) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "INSERT INTO tail_cursor(path, offset) VALUES (?, ?)
             ON CONFLICT(path) DO UPDATE SET
                 offset = excluded.offset,
                 updated_at = datetime('now')",
            params![source_path, offset as i64],
        )?;
        Ok(())
    }

    /// Read a file-tail cursor as `(offset, file_sig)`. The signature is
    /// `None` for legacy rows written before the column existed and for
    /// rows written via [`write_cursor`] (org-connector's id high-water
    /// mark). Used by the live tail / launcher / backfill readers to
    /// detect log rotation via [`crate::gamelog::resolve_resume_offset`].
    pub fn read_tail_cursor(&self, source_path: &str) -> Result<(u64, Option<String>)> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let result: rusqlite::Result<(i64, Option<String>)> = conn.query_row(
            "SELECT offset, file_sig FROM tail_cursor WHERE path = ?",
            params![source_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((n, sig)) => Ok((n.max(0) as u64, sig)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((0, None)),
            Err(e) => Err(e.into()),
        }
    }

    /// Write a file-tail cursor with the physical-file signature so a
    /// later drain (including one after a tray restart) can tell a
    /// rotated file from the original. `file_sig = None` clears the
    /// signature (e.g. an empty file too short to sign).
    pub fn write_tail_cursor(
        &self,
        source_path: &str,
        offset: u64,
        file_sig: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "INSERT INTO tail_cursor(path, offset, file_sig) VALUES (?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET
                 offset = excluded.offset,
                 file_sig = excluded.file_sig,
                 updated_at = datetime('now')",
            params![source_path, offset as i64, file_sig],
        )?;
        Ok(())
    }

    pub fn event_counts(&self) -> Result<Vec<(String, u64)>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT type, COUNT(*) FROM events GROUP BY type ORDER BY 2 DESC LIMIT 50")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn total_events(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(n.max(0) as u64)
    }

    /// UPSERT one observation of an unknown event. First call inserts
    /// with `occurrences = 1` (table default). Subsequent calls bump
    /// `occurrences` and refresh the sample so the most recent body
    /// is always available for inspection.
    pub fn record_unknown(
        &self,
        log_source: &str,
        event_name: &str,
        sample_line: &str,
        sample_body: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "INSERT INTO unknown_event_samples (log_source, event_name, sample_line, sample_body)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(log_source, event_name) DO UPDATE SET
                 occurrences = occurrences + 1,
                 last_seen = datetime('now'),
                 sample_line = excluded.sample_line,
                 sample_body = excluded.sample_body",
            params![log_source, event_name, sample_line, sample_body],
        )?;
        Ok(())
    }

    /// Most recent events, newest first. Used to render a chronological
    /// "what happened" timeline in the tray UI.
    pub fn recent_events(&self, limit: usize) -> Result<Vec<RecentEventRow>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, type, timestamp, payload, raw, log_source, sent_at
             FROM events
             ORDER BY timestamp DESC, id DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(RecentEventRow {
                id: row.get(0)?,
                event_type: row.get(1)?,
                timestamp: row.get(2)?,
                payload_json: row.get(3)?,
                raw_line: row.get(4)?,
                log_source: row.get(5)?,
                sent_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Events with `id > after_id`, OLDEST first, up to `limit`. For
    /// downstream consumers that maintain their own monotonic event
    /// cursor (the org-platform connector) and must not disturb the sync
    /// worker's `sent_at` bookkeeping — this read is orthogonal to it.
    pub fn read_events_after(&self, after_id: i64, limit: usize) -> Result<Vec<RecentEventRow>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, type, timestamp, payload, raw, log_source, sent_at
             FROM events
             WHERE id > ?
             ORDER BY id ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![after_id, limit as i64], |row| {
            Ok(RecentEventRow {
                id: row.get(0)?,
                event_type: row.get(1)?,
                timestamp: row.get(2)?,
                payload_json: row.get(3)?,
                raw_line: row.get(4)?,
                log_source: row.get(5)?,
                sent_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Search events with optional query (substring, case-insensitive,
    /// matched against `type` and `payload`), optional exact `type_filter`,
    /// optional cursor (`before_id` — return rows whose `id < before_id`).
    /// Returns at most `limit` rows, newest first.
    ///
    /// `query` matches `LOWER(type) LIKE %q% OR LOWER(payload) LIKE %q%`.
    /// We don't search `raw` to keep the predicate cheap on the hot path —
    /// `payload` is the parsed JSON, which is a superset of what `summary`
    /// renders, so searching `payload` covers what the user sees in the UI.
    ///
    /// `type_filter`, when present, adds `AND type = ?` (exact match).
    ///
    /// `before_id`, when present, adds `AND id < ?`. This is the cursor for
    /// "Load more" — pass the smallest `id` from the current page to get
    /// the next page. Cursor on `id` (not timestamp) because `id` is a
    /// monotonic surrogate per the schema and avoids tie-breaks for events
    /// captured in the same millisecond.
    pub fn search_events_paged(
        &self,
        query: Option<&str>,
        type_filter: Option<&str>,
        before_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<RecentEventRow>> {
        let mut where_parts: Vec<&'static str> = Vec::new();
        // Lowercased query — held in this scope so the &str borrow we hand
        // to params stays alive across stmt.prepare/query.
        let lowered = query.map(|q| format!("%{}%", q.to_lowercase()));
        if lowered.is_some() {
            where_parts.push("(LOWER(type) LIKE ? OR LOWER(payload) LIKE ?)");
        }
        if type_filter.is_some() {
            where_parts.push("type = ?");
        }
        if before_id.is_some() {
            where_parts.push("id < ?");
        }
        let where_sql = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };
        let sql = format!(
            "SELECT id, type, timestamp, payload, raw, log_source, sent_at
             FROM events
             {where_sql}
             ORDER BY timestamp DESC, id DESC
             LIMIT ?"
        );

        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(&sql)?;
        let mut bound: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(q) = &lowered {
            bound.push(rusqlite::types::Value::Text(q.clone()));
            bound.push(rusqlite::types::Value::Text(q.clone()));
        }
        if let Some(t) = type_filter {
            bound.push(rusqlite::types::Value::Text(t.to_string()));
        }
        if let Some(b) = before_id {
            bound.push(rusqlite::types::Value::Integer(b));
        }
        bound.push(rusqlite::types::Value::Integer(limit as i64));
        let param_refs: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(RecentEventRow {
                id: row.get(0)?,
                event_type: row.get(1)?,
                timestamp: row.get(2)?,
                payload_json: row.get(3)?,
                raw_line: row.get(4)?,
                log_source: row.get(5)?,
                sent_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Count rows matching the same `query` + `type_filter` predicate as
    /// `search_events_paged` (no cursor, no limit). The tray UI uses this
    /// to show "loaded / total" so the user knows whether more pages exist.
    pub fn count_matching_events(
        &self,
        query: Option<&str>,
        type_filter: Option<&str>,
    ) -> Result<u64> {
        let mut where_parts: Vec<&'static str> = Vec::new();
        let lowered = query.map(|q| format!("%{}%", q.to_lowercase()));
        if lowered.is_some() {
            where_parts.push("(LOWER(type) LIKE ? OR LOWER(payload) LIKE ?)");
        }
        if type_filter.is_some() {
            where_parts.push("type = ?");
        }
        let where_sql = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };
        let sql = format!("SELECT COUNT(*) FROM events {where_sql}");

        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut bound: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(q) = &lowered {
            bound.push(rusqlite::types::Value::Text(q.clone()));
            bound.push(rusqlite::types::Value::Text(q.clone()));
        }
        if let Some(t) = type_filter {
            bound.push(rusqlite::types::Value::Text(t.to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> =
            bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let n: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(n.max(0) as u64)
    }

    /// Total bytes the SQLite file occupies on disk, computed via
    /// `page_count * page_size`. Cheap (single round-trip per pragma)
    /// and avoids a filesystem stat that may disagree with the
    /// engine's view if WAL pages are still in flight.
    pub fn database_size_bytes(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        let bytes = page_count.max(0) as u64 * page_size.max(0) as u64;
        Ok(bytes)
    }

    /// Return the top `limit` unknown events, most-seen first, ties
    /// broken by most-recently-seen.
    pub fn recent_unknowns(&self, limit: usize) -> Result<Vec<UnknownSampleRow>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT log_source, event_name, occurrences, first_seen, last_seen,
                    sample_line, sample_body
             FROM unknown_event_samples
             ORDER BY occurrences DESC, last_seen DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(UnknownSampleRow {
                log_source: row.get(0)?,
                event_name: row.get(1)?,
                occurrences: row.get::<_, i64>(2)?.max(0) as u64,
                first_seen: row.get(3)?,
                last_seen: row.get(4)?,
                sample_line: row.get(5)?,
                sample_body: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Persist the active parser-definition manifest. The cache holds
    /// at most one row (sentinel `id = 1`); an UPSERT replaces it on
    /// every successful fetch. We store the raw JSON so a future
    /// schema_version bump can be handled by re-deserialising in the
    /// new shape without a migration.
    pub fn write_parser_def_manifest(&self, version: u32, payload_json: &str) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "INSERT INTO parser_def_manifest (id, version, payload_json)
             VALUES (1, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               version = excluded.version,
               fetched_at = datetime('now'),
               payload_json = excluded.payload_json",
            params![version as i64, payload_json],
        )?;
        Ok(())
    }

    /// Stream every row of `events` for re-parse. Loads the full set
    /// in batches so a multi-million-row store doesn't materialize as
    /// one giant `Vec`. Caller closure decides what to do per row;
    /// returning `Err` aborts the iteration.
    pub fn for_each_event<F>(&self, batch_size: usize, mut f: F) -> Result<()>
    where
        F: FnMut(EventRow) -> Result<()>,
    {
        let mut last_id: i64 = 0;
        loop {
            // Fetch one batch with the lock held, then release it
            // BEFORE invoking the closure. The original implementation
            // held the lock across the closure body, which deadlocked
            // any caller (e.g. reparse) that called back into other
            // Storage methods that re-acquire the same lock. Paged by
            // `id > last_id` so concurrent inserts during the walk
            // are visited in a later batch — correct for re-parse.
            let conn = self.conn.lock().expect("storage mutex poisoned");
            let mut stmt = conn.prepare(
                "SELECT id, idempotency_key, type, timestamp, raw, payload, log_source, source_offset, metadata
                 FROM events
                 WHERE id > ?
                 ORDER BY id ASC
                 LIMIT ?",
            )?;
            let mapped = stmt.query_map(params![last_id, batch_size as i64], |row| {
                Ok(EventRow {
                    id: row.get(0)?,
                    idempotency_key: row.get(1)?,
                    event_type: row.get(2)?,
                    timestamp: row.get(3)?,
                    raw_line: row.get(4)?,
                    payload_json: row.get(5)?,
                    log_source: row.get(6)?,
                    source_offset: row.get::<_, i64>(7)?.max(0) as u64,
                    metadata_json: row.get(8)?,
                })
            })?;
            let rows: Vec<EventRow> = mapped.filter_map(|r| r.ok()).collect();
            // Drop the lock guard explicitly so the closure invoked
            // below is free to call back into other Storage methods
            // that re-acquire the connection. Without this drop the
            // re-parse closure deadlocks the moment it tries to write
            // an updated classification.
            drop(stmt);
            drop(conn);
            if rows.is_empty() {
                break;
            }
            for row in rows {
                last_id = row.id;
                f(row)?;
            }
        }
        Ok(())
    }

    /// Re-classify in place — overwrite an existing row's type +
    /// payload + timestamp (timestamps can refine when a richer
    /// classifier extracts a more precise field). Used by the
    /// re-parse command when newer rules upgrade an existing match.
    /// Returns the number of rows actually updated (0 or 1).
    ///
    /// `metadata_json` carries an optional `EventMetadata` blob the
    /// caller wants stamped on the row — typically the output of the
    /// zone-enrichment pass calling
    /// `starstats_core::metadata::provenance_for_inferred_field`. `None`
    /// leaves the existing metadata cell alone so callers that don't
    /// produce metadata (the bulk of re-parse rows) don't accidentally
    /// clear it.
    pub fn update_event_classification(
        &self,
        id: i64,
        event_type: &str,
        timestamp: &str,
        payload_json: &str,
        metadata_json: Option<&str>,
    ) -> Result<usize> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n = match metadata_json {
            Some(meta) => conn.execute(
                "UPDATE events SET type = ?, timestamp = ?, payload = ?, metadata = ?
                 WHERE id = ?",
                params![event_type, timestamp, payload_json, meta, id],
            )?,
            None => conn.execute(
                "UPDATE events SET type = ?, timestamp = ?, payload = ? WHERE id = ?",
                params![event_type, timestamp, payload_json, id],
            )?,
        };
        Ok(n)
    }

    /// Distinct `log_source` values present in `events`. Used by the
    /// retro-burst phase of re-parse to walk one source's history at a
    /// time so detect_bursts sees a single contiguous source-offset
    /// stream rather than an interleaved multi-channel mix.
    pub fn distinct_log_sources(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT DISTINCT log_source FROM events ORDER BY log_source")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Lean projection of one `log_source`'s events, ordered by
    /// `source_offset` (then `id` as a stable tiebreaker). Skips the
    /// payload column because retro-burst only needs the raw line for
    /// `structural_parse` and the offset for the idempotency key.
    pub fn events_for_burst_scan(&self, log_source: &str) -> Result<Vec<BurstScanRow>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, raw, source_offset, type
             FROM events
             WHERE log_source = ?
             ORDER BY source_offset ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![log_source], |row| {
            Ok(BurstScanRow {
                id: row.get(0)?,
                raw_line: row.get(1)?,
                source_offset: row.get::<_, i64>(2)?.max(0) as u64,
                event_type: row.get(3)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Hard-delete a single event row by id. Used by retro-burst to
    /// suppress members that have been collapsed into a synthesised
    /// `BurstSummary`. We delete rather than soft-delete because the
    /// timeline reader has no notion of a tombstone column and a
    /// soft-delete would force every read site to learn one.
    pub fn delete_event_by_id(&self, id: i64) -> Result<usize> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n = conn.execute("DELETE FROM events WHERE id = ?", params![id])?;
        Ok(n)
    }

    /// Drop a single unknown sample by `(log_source, event_name)`.
    /// Used by re-parse: once a sample line has been promoted to a
    /// real `events` row, the unknown record is no longer the
    /// "actionable next thing to write a rule for".
    pub fn delete_unknown(&self, log_source: &str, event_name: &str) -> Result<usize> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n = conn.execute(
            "DELETE FROM unknown_event_samples
             WHERE log_source = ? AND event_name = ?",
            params![log_source, event_name],
        )?;
        Ok(n)
    }

    /// Read the cached manifest payload, if any. Returns `Ok(None)`
    /// for first-run when the cache is empty (the caller should treat
    /// this as "no remote rules" and continue).
    pub fn read_parser_def_manifest(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare("SELECT payload_json FROM parser_def_manifest WHERE id = 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let payload: String = row.get(0)?;
            Ok(Some(payload))
        } else {
            Ok(None)
        }
    }

    /// Read just the adopted manifest *version* without deserialising the
    /// (potentially large) payload. Used by the sync drain to stamp
    /// `IngestBatch.parser_version` so the server can attribute events to
    /// the rule-set the collector was running. `None` = no manifest
    /// fetched yet (first run) — the batch ships with `parser_version:
    /// None`, which the server reads as "unknown rule-set".
    pub fn read_parser_def_manifest_version(&self) -> Result<Option<u32>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare("SELECT version FROM parser_def_manifest WHERE id = 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let version: u32 = row.get(0)?;
            Ok(Some(version))
        } else {
            Ok(None)
        }
    }

    /// The `batch_sequence` to stamp on the NEXT ingest batch: one past
    /// the highest ordinal this install has successfully sent. A fresh
    /// install (no counter row) reads as 0, so the first batch ships
    /// sequence 1. Read at build time and paired with
    /// [`Self::commit_batch_sequence`] on a 2xx — the number is only
    /// *consumed* on success, so a failed/retried/poison-bisected send
    /// reuses it rather than leaving a false gap in the server's view.
    pub fn peek_next_batch_sequence(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare("SELECT value FROM batch_sequence_counter WHERE id = 1")?;
        let mut rows = stmt.query([])?;
        let current: u64 = if let Some(row) = rows.next()? {
            row.get(0)?
        } else {
            0
        };
        Ok(current + 1)
    }

    /// Record that batch ordinal `seq` was successfully sent, advancing
    /// the counter. `MAX(value, excluded.value)` makes this idempotent
    /// (committing the same seq twice is a no-op) and monotonic (a late,
    /// lower commit from a racing lane can never rewind the counter), so
    /// the two drain lanes may commit in any order without corrupting
    /// the sequence.
    pub fn commit_batch_sequence(&self, seq: u64) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "INSERT INTO batch_sequence_counter (id, value)
             VALUES (1, ?)
             ON CONFLICT(id) DO UPDATE SET
               value = MAX(value, excluded.value)",
            params![seq],
        )?;
        Ok(())
    }

    /// Upsert one captured `UnknownLine` keyed by `shape_hash`. New
    /// rows insert verbatim; existing rows bump `occurrence_count`,
    /// refresh `last_seen`, and append `line.raw_line` to the cached
    /// raw-examples buffer — dropping the oldest entry when the cap is
    /// exceeded so the buffer stays bounded. Other fields on a
    /// duplicate (interest score, partial_structured, context) are
    /// left untouched; the first capture sets the canonical record.
    #[allow(dead_code)]
    pub fn cache_unknown_line(&self, line: &UnknownLine) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let existing: rusqlite::Result<(i64, String)> = conn.query_row(
            "SELECT occurrence_count, raw_examples_json
             FROM unknown_lines
             WHERE shape_hash = ?",
            params![line.shape_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );

        match existing {
            Ok((count, raw_json)) => {
                let mut samples: Vec<String> = serde_json::from_str(&raw_json)
                    .context("decode raw_examples_json on upsert")?;
                samples.push(line.raw_line.clone());
                while samples.len() > RAW_EXAMPLES_CAP {
                    samples.remove(0);
                }
                let new_raw = serde_json::to_string(&samples)
                    .context("encode raw_examples_json on upsert")?;
                conn.execute(
                    "UPDATE unknown_lines SET
                        occurrence_count = ?,
                        last_seen = ?,
                        raw_examples_json = ?
                     WHERE shape_hash = ?",
                    params![count + 1, line.last_seen, new_raw, line.shape_hash],
                )?;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let raw_examples = serde_json::to_string(&vec![line.raw_line.clone()])
                    .context("encode raw_examples_json on insert")?;
                let partial = serde_json::to_string(&line.partial_structured)
                    .context("encode partial_structured_json")?;
                let context_before = serde_json::to_string(&line.context_before)
                    .context("encode context_before_json")?;
                let context_after = serde_json::to_string(&line.context_after)
                    .context("encode context_after_json")?;
                let pii = serde_json::to_string(&line.detected_pii)
                    .context("encode detected_pii_json")?;
                let channel = serde_json::to_value(line.channel)
                    .context("encode channel")?
                    .as_str()
                    .map(str::to_string)
                    .context("channel serialises to a string")?;
                conn.execute(
                    "INSERT INTO unknown_lines (
                        id, shape_hash, raw_examples_json, partial_structured_json,
                        shell_tag, context_before_json, context_after_json,
                        game_build, channel, interest_score, occurrence_count,
                        first_seen, last_seen, detected_pii_json, dismissed, submitted_at
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
                    params![
                        line.id,
                        line.shape_hash,
                        raw_examples,
                        partial,
                        line.shell_tag,
                        context_before,
                        context_after,
                        line.game_build,
                        channel,
                        line.interest_score as i64,
                        line.occurrence_count as i64,
                        line.first_seen,
                        line.last_seen,
                        pii,
                        if line.dismissed { 1_i64 } else { 0_i64 },
                    ],
                )?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Surface every non-dismissed unknown line whose interest score
    /// meets `min_interest`. Ordered by interest desc, occurrence_count
    /// desc, last_seen desc so the most actionable shapes float to the
    /// top of the review pane. `min_interest = 50` matches the spec
    /// default; callers can lower the bar for diagnostic views.
    ///
    /// Capped at [`REVIEW_LIST_LIMIT`]: the pane renders every returned
    /// row unvirtualized, and a human reviews them by hand, so there is
    /// no value in shipping thousands across the IPC bridge (each row
    /// also JSON-decodes five columns). The ORDER BY means the cap keeps
    /// the highest-interest shapes; the badge count (`count_unknown_lines`)
    /// stays uncapped so the true backlog is still visible.
    pub fn list_unknown_lines(&self, min_interest: u8) -> Result<Vec<UnknownLine>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, shape_hash, raw_examples_json, partial_structured_json,
                    shell_tag, context_before_json, context_after_json,
                    game_build, channel, interest_score, occurrence_count,
                    first_seen, last_seen, detected_pii_json, dismissed, submitted_at
             FROM unknown_lines
             WHERE dismissed = 0 AND submitted_at IS NULL AND interest_score >= ?
             ORDER BY interest_score DESC, occurrence_count DESC, last_seen DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(
            params![min_interest as i64, REVIEW_LIST_LIMIT],
            decode_unknown_line,
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Aggregate unclassified-line METADATA by shell tag, for the opt-in
    /// parser-health report.
    ///
    /// Returns `(shell_tag, first_seen, last_seen, occurrences, game_build)`
    /// and deliberately nothing else. `raw_examples_json` — the actual log
    /// text — is never read here: the whole privacy contract of this feature
    /// is that engine symbol names leave the machine and line bodies do not.
    /// Rows with no shell tag are skipped; a body-only shape has nothing to
    /// correlate against.
    pub fn unknown_tag_metadata(&self) -> Result<Vec<UnknownTagRow>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT shell_tag,
                    MIN(first_seen)          AS first_seen,
                    MAX(last_seen)           AS last_seen,
                    SUM(occurrence_count)    AS occurrences,
                    MAX(game_build)          AS game_build
             FROM unknown_lines
             WHERE shell_tag IS NOT NULL AND shell_tag <> ''
             GROUP BY shell_tag
             ORDER BY occurrences DESC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![UNKNOWN_TAG_REPORT_LIMIT], |r| {
            Ok(UnknownTagRow {
                shell_tag: r.get(0)?,
                first_seen: r.get(1)?,
                last_seen: r.get(2)?,
                occurrences: r.get(3)?,
                game_build: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Count non-dismissed unknown lines at or above the given
    /// interest cutoff. Tray badge calls this on a timer so it stays
    /// cheap — the dedicated index on `(dismissed, interest_score)`
    /// keeps the scan small.
    pub fn count_unknown_lines(&self, min_interest: u8) -> Result<u32> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM unknown_lines
             WHERE dismissed = 0 AND submitted_at IS NULL AND interest_score >= ?",
            params![min_interest as i64],
            |row| row.get(0),
        )?;
        Ok(n.max(0) as u32)
    }

    /// Mark a shape as dismissed so it never resurfaces in
    /// `list_unknown_lines`. The row is kept (not deleted) so a future
    /// re-capture of the same shape doesn't re-trigger the badge.
    pub fn dismiss_unknown_line(&self, shape_hash: &str) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "UPDATE unknown_lines SET dismissed = 1 WHERE shape_hash = ?",
            params![shape_hash],
        )?;
        Ok(())
    }

    /// Stamp a shape with its submission timestamp once the row has
    /// been shipped to the server's moderation queue. The caller owns
    /// the timestamp format (ISO-8601 by convention) so this method
    /// stays a thin write.
    pub fn mark_submitted(&self, shape_hash: &str, submitted_at: &str) -> Result<()> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        conn.execute(
            "UPDATE unknown_lines SET submitted_at = ? WHERE shape_hash = ?",
            params![submitted_at, shape_hash],
        )?;
        Ok(())
    }

    /// Test/debug helper — return just the cached raw-line samples for
    /// a given shape. Used by the `raw_examples_cap_at_five` test to
    /// inspect the buffer without depending on `UnknownLine.raw_line`.
    #[cfg(test)]
    pub fn list_raw_examples(&self, shape_hash: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("storage mutex poisoned");
        let raw: String = conn.query_row(
            "SELECT raw_examples_json FROM unknown_lines WHERE shape_hash = ?",
            params![shape_hash],
            |row| row.get(0),
        )?;
        let samples: Vec<String> =
            serde_json::from_str(&raw).context("decode raw_examples_json")?;
        Ok(samples)
    }
}

/// Decode one `events` row (the projection used by `read_unsent_*`)
/// into an `UnsentEvent`. Free function so both the unfiltered and
/// filtered drain paths share the column-order contract.
fn map_unsent_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnsentEvent> {
    Ok(UnsentEvent {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        payload_json: row.get(2)?,
        raw_line: row.get(3)?,
        log_source: row.get(4)?,
        source_offset: row.get::<_, i64>(5)?.max(0) as u64,
    })
}

/// Decode one `unknown_lines` row back into an `UnknownLine`. Kept as
/// a free function so both `list_unknown_lines` and any future
/// single-row reader can share the column order.
#[allow(dead_code)]
fn decode_unknown_line(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnknownLine> {
    let partial_json: String = row.get(3)?;
    let context_before_json: String = row.get(5)?;
    let context_after_json: String = row.get(6)?;
    let detected_pii_json: String = row.get(13)?;
    let channel_str: String = row.get(8)?;
    let dismissed_i: i64 = row.get(14)?;

    let partial = serde_json::from_str(&partial_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let context_before = serde_json::from_str(&context_before_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let context_after = serde_json::from_str(&context_after_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let detected_pii = serde_json::from_str(&detected_pii_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(13, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let channel = serde_json::from_value(serde_json::Value::String(channel_str)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let interest_i: i64 = row.get(9)?;
    let occurrence_i: i64 = row.get(10)?;

    Ok(UnknownLine {
        id: row.get(0)?,
        shape_hash: row.get(1)?,
        // The `raw_line` field on the returned UnknownLine reflects
        // the MOST RECENT sample we've stashed for this shape — the
        // canonical buffer is `raw_examples_json` on disk, but
        // callers that only want one example expect the freshest.
        raw_line: {
            let raw_examples_json: String = row.get(2)?;
            let samples: Vec<String> = serde_json::from_str(&raw_examples_json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            samples.last().cloned().unwrap_or_default()
        },
        timestamp: None,
        shell_tag: row.get(4)?,
        partial_structured: partial,
        context_before,
        context_after,
        game_build: row.get(7)?,
        channel,
        interest_score: interest_i.clamp(0, u8::MAX as i64) as u8,
        occurrence_count: occurrence_i.max(0) as u32,
        first_seen: row.get(11)?,
        last_seen: row.get(12)?,
        detected_pii,
        dismissed: dismissed_i != 0,
    })
}

#[cfg(test)]
mod tests {
    /// Newly-shipped builtins must reach EXISTING installs, not only
    /// fresh ones. Measured 2026-07-31: a 1,030,040-occurrence corpus was
    /// 68% engine/UI chatter while this list held 26 entries, so the
    /// value of extending it depends entirely on the upgrade path
    /// working.
    #[test]
    fn newly_shipped_noise_entries_reach_an_existing_database() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("data.sqlite3");

        // First open seeds whatever ships today.
        drop(Storage::open(&path).unwrap());

        // Simulate an install predating the new entries.
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute(
                "DELETE FROM event_noise_list WHERE event_name = ?",
                params!["RegisterUniverseHierarchy_Begin"],
            )
            .unwrap();
        }
        let stale = Storage::open(&path).unwrap();
        assert!(
            stale.is_noise("RegisterUniverseHierarchy_Begin").unwrap(),
            "re-open must restore a shipped builtin on an existing db"
        );
    }

    #[test]
    fn a_user_added_entry_survives_re_seeding() {
        // Users can add their own entries; a shipped list that clobbered
        // them would silently undo their choices on every upgrade.
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("data.sqlite3");
        {
            let s = Storage::open(&path).unwrap();
            s.add_noise("SomeUserChosenEvent", "user").unwrap();
        }
        let s = Storage::open(&path).unwrap();
        assert!(s.is_noise("SomeUserChosenEvent").unwrap());
    }

    #[test]
    fn the_shipped_noise_list_covers_the_measured_engine_families() {
        // Pins the specific entries added from the corpus sweep, so a
        // future edit cannot quietly drop them.
        let (s, _d) = fresh_storage();
        for name in [
            "RegisterUniverseHierarchy_Begin",
            "RegisterUniverseHierarchy_Mid",
            "Local Route Guard - Server Rerouted",
            "OnDragInventoryItemModifyTarget",
            "Connection Flow",
        ] {
            assert!(s.is_noise(name).unwrap(), "{name} should be shipped noise");
        }
        // And the restraint: families that may carry player signal are
        // deliberately NOT noise-listed.
        for name in [
            "InventoryManagement",
            "CObjectiveMarkerComponent::AddToPlayerData",
        ] {
            assert!(
                !s.is_noise(name).unwrap(),
                "{name} may carry player signal and must stay classifiable"
            );
        }
    }

    use super::*;
    use tempfile::TempDir;

    fn fresh_storage() -> (Storage, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("test.sqlite3");
        let storage = Storage::open(&path).expect("open storage");
        (storage, dir)
    }

    #[test]
    fn record_unknown_dedupes_by_source_and_event() {
        let (storage, _tmp) = fresh_storage();

        storage
            .record_unknown("live", "Foo", "raw line v1", "body v1")
            .expect("first record");
        storage
            .record_unknown("live", "Foo", "raw line v2", "body v2")
            .expect("second record");

        let rows = storage.recent_unknowns(50).expect("read unknowns");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.log_source, "live");
        assert_eq!(row.event_name, "Foo");
        assert_eq!(row.occurrences, 2);
        // Sample is overwritten with the latest call's value.
        assert_eq!(row.sample_body, "body v2");
        assert_eq!(row.sample_line, "raw line v2");
    }

    #[test]
    fn recent_unknowns_orders_by_occurrences_desc() {
        let (storage, _tmp) = fresh_storage();

        // Bar: 1 occurrence
        storage
            .record_unknown("live", "Bar", "rawB", "bodyB")
            .expect("Bar");
        // Foo: 3 occurrences
        for _ in 0..3 {
            storage
                .record_unknown("live", "Foo", "rawF", "bodyF")
                .expect("Foo");
        }

        let rows = storage.recent_unknowns(50).expect("read unknowns");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_name, "Foo");
        assert_eq!(rows[0].occurrences, 3);
        assert_eq!(rows[1].event_name, "Bar");
        assert_eq!(rows[1].occurrences, 1);
    }

    #[test]
    fn fresh_db_seeds_default_noise() {
        let (storage, _tmp) = fresh_storage();
        // Pick any well-known default — StatObjLoad is the heaviest noise source.
        assert!(storage.is_noise("StatObjLoad 0x800 Format").unwrap());
        // Random unknown name is not noise.
        assert!(!storage.is_noise("Some Player Event").unwrap());
    }

    #[test]
    fn add_noise_dedupes_and_drops_existing_samples() {
        let (storage, _tmp) = fresh_storage();
        // Pre-record an unknown that we're about to mark as noise.
        storage
            .record_unknown("live", "ChattyEvent", "raw", "body")
            .unwrap();
        assert_eq!(storage.recent_unknowns(50).unwrap().len(), 1);

        storage.add_noise("ChattyEvent", "user").unwrap();
        // Sample purged.
        assert_eq!(storage.recent_unknowns(50).unwrap().len(), 0);
        // Membership reflected.
        assert!(storage.is_noise("ChattyEvent").unwrap());

        // Idempotent — second add is a no-op.
        storage.add_noise("ChattyEvent", "user").unwrap();
        assert!(storage.is_noise("ChattyEvent").unwrap());
    }

    #[test]
    fn purge_runs_at_open_time() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sqlite3");

        // Stage 1: open, record unknowns, including one we'll later
        // promote to noise via a hand-written insert.
        {
            let s = Storage::open(&path).unwrap();
            s.record_unknown("live", "ToBecomeNoise", "raw", "body")
                .unwrap();
            s.record_unknown("live", "StaysAsUnknown", "raw", "body")
                .unwrap();
            // Manually mark one as noise without calling add_noise()
            // — simulates a noise list shipped via app update.
            s.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO event_noise_list(event_name, source) VALUES (?, 'builtin')",
                    params!["ToBecomeNoise"],
                )
                .unwrap();
        }

        // Stage 2: re-open. Purge should drop the now-noise sample.
        let s = Storage::open(&path).unwrap();
        let rows = s.recent_unknowns(50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_name, "StaysAsUnknown");
    }

    #[test]
    fn record_unknown_separates_log_sources() {
        let (storage, _tmp) = fresh_storage();

        storage
            .record_unknown("live", "Foo", "raw", "body")
            .expect("live");
        storage
            .record_unknown("ptu", "Foo", "raw", "body")
            .expect("ptu");

        let rows = storage.recent_unknowns(50).expect("read unknowns");
        assert_eq!(rows.len(), 2);
        let mut sources: Vec<&str> = rows.iter().map(|r| r.log_source.as_str()).collect();
        sources.sort();
        assert_eq!(sources, vec!["live", "ptu"]);
        for row in &rows {
            assert_eq!(row.event_name, "Foo");
            assert_eq!(row.occurrences, 1);
        }
    }

    /// `insert_event` must distinguish a fresh write from a conflict
    /// no-op. `ON CONFLICT(idempotency_key) DO NOTHING` makes the two
    /// outcomes identical to a caller that only sees `Result<()>`, and
    /// the burst path in `gamelog::process_buffer` acts on the answer:
    /// it deletes a burst's member lines from the buffer on the strength
    /// of a summary row it believes it stored. A caller that cannot tell
    /// "stored" from "silently discarded" cannot keep that promise.
    #[test]
    fn insert_event_reports_whether_the_row_was_actually_written() {
        let dir = TempDir::new().expect("tempdir");
        let storage = Storage::open(&dir.path().join("t.db")).expect("open");

        let write = || {
            storage
                .insert_event(
                    "same-key",
                    "burst_summary",
                    "2026-09-04T12:00:00Z",
                    "raw",
                    "{}",
                    "live",
                    42,
                )
                .expect("insert must not error")
        };

        assert!(write(), "first insert of a key must report a written row");
        assert!(
            !write(),
            "second insert of the SAME key must report no row written — \
             it conflicted and DO NOTHING dropped it"
        );

        let conn = storage.conn.lock().expect("mutex");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE idempotency_key = ?",
                params!["same-key"],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 1, "the conflict must not have duplicated the row");
    }

    // ─── Priority-lanes sync (sent_at flag + filtered drain) ──────

    /// Helper — insert a synthetic event row with a chosen type, so
    /// the read_unsent_filtered tests can exercise type matching
    /// without needing a real parsed payload. Each call gets a unique
    /// idempotency key derived from the offset so the table's UNIQUE
    /// constraint doesn't fire.
    fn insert_with_type(storage: &Storage, event_type: &str, offset: u64) -> i64 {
        let key = format!("k-{event_type}-{offset}");
        storage
            .insert_event(
                &key,
                event_type,
                "2026-05-18T12:00:00Z",
                "raw",
                "{}",
                "live",
                offset,
            )
            .expect("insert");
        // Return the id we just wrote so the test can mark it sent.
        let conn = storage.conn.lock().expect("mutex");
        conn.query_row(
            "SELECT id FROM events WHERE idempotency_key = ?",
            params![key],
            |row| row.get::<_, i64>(0),
        )
        .expect("read id")
    }

    #[test]
    fn fresh_db_has_sent_at_column_and_unsent_index() {
        let (storage, _tmp) = fresh_storage();
        let conn = storage.conn.lock().unwrap();

        // Column exists.
        let mut stmt = conn.prepare("PRAGMA table_info(events)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(cols.contains(&"sent_at".to_string()));

        // Partial index exists.
        let mut idx_stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_events_unsent'",
            )
            .unwrap();
        let names: Vec<String> = idx_stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(names, vec!["idx_events_unsent".to_string()]);
    }

    #[test]
    fn read_unsent_filtered_priority_only_returns_only_matching_types() {
        let (storage, _tmp) = fresh_storage();
        insert_with_type(&storage, "location_changed", 1);
        insert_with_type(&storage, "shop_buy_request", 2);
        insert_with_type(&storage, "player_death", 3);
        insert_with_type(&storage, "burst_summary", 4);

        let urgent = ["location_changed", "player_death"];
        let rows = storage
            .read_unsent_filtered(&urgent, true, 100)
            .expect("priority drain");
        let types: Vec<&str> = rows
            .iter()
            .map(|r| extract_type(&storage, r.id))
            .collect::<Vec<_>>()
            .into_iter()
            .collect::<Vec<&str>>();
        // 2 priority rows, ordered by id ASC (insertion order).
        assert_eq!(rows.len(), 2);
        assert_eq!(types, vec!["location_changed", "player_death"]);
    }

    #[test]
    fn read_unsent_filtered_priority_excluded_returns_the_rest() {
        let (storage, _tmp) = fresh_storage();
        insert_with_type(&storage, "location_changed", 1);
        insert_with_type(&storage, "shop_buy_request", 2);
        insert_with_type(&storage, "player_death", 3);
        insert_with_type(&storage, "burst_summary", 4);

        let urgent = ["location_changed", "player_death"];
        let rows = storage
            .read_unsent_filtered(&urgent, false, 100)
            .expect("bulk drain");
        let types: Vec<String> = rows
            .iter()
            .map(|r| extract_type(&storage, r.id).to_string())
            .collect();
        // The 2 non-priority rows, ordered by id ASC.
        assert_eq!(types, vec!["shop_buy_request", "burst_summary"]);
    }

    #[test]
    fn read_unsent_filtered_empty_priority_list_returns_everything_unsent() {
        let (storage, _tmp) = fresh_storage();
        insert_with_type(&storage, "location_changed", 1);
        insert_with_type(&storage, "shop_buy_request", 2);
        let rows = storage
            .read_unsent_filtered(&[], false, 100)
            .expect("bulk drain (no priority configured)");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn parser_def_manifest_version_reads_none_then_written_value() {
        let (storage, _tmp) = fresh_storage();
        // First run: no manifest fetched → None (batch ships parser_version=None).
        assert_eq!(
            storage.read_parser_def_manifest_version().unwrap(),
            None,
            "empty manifest table must read as None, not error"
        );
        // After a fetch persists a manifest, the version reads back exactly.
        storage
            .write_parser_def_manifest(42, r#"{"version":42,"rules":[]}"#)
            .expect("write manifest");
        assert_eq!(
            storage.read_parser_def_manifest_version().unwrap(),
            Some(42)
        );
        // A newer manifest upserts the single row (id=1) → newest version wins.
        storage
            .write_parser_def_manifest(43, r#"{"version":43,"rules":[]}"#)
            .expect("rewrite manifest");
        assert_eq!(
            storage.read_parser_def_manifest_version().unwrap(),
            Some(43)
        );
    }

    #[test]
    fn batch_sequence_peek_starts_at_one_and_advances_only_on_commit() {
        let (storage, _tmp) = fresh_storage();
        // Fresh install: no counter row → first batch ships sequence 1.
        assert_eq!(
            storage.peek_next_batch_sequence().unwrap(),
            1,
            "empty counter must peek 1, not error"
        );
        // Peek is non-consuming: repeated peeks without a commit stay at 1
        // — a failed/retried send must reuse its number, not burn it.
        assert_eq!(storage.peek_next_batch_sequence().unwrap(), 1);

        // Commit advances the counter: the next peek is 2.
        storage.commit_batch_sequence(1).expect("commit 1");
        assert_eq!(storage.peek_next_batch_sequence().unwrap(), 2);

        // Commit is idempotent + monotonic: re-committing 1, or a stale
        // lower value from a racing lane, never rewinds the counter.
        storage.commit_batch_sequence(1).expect("re-commit 1");
        assert_eq!(storage.peek_next_batch_sequence().unwrap(), 2);
        storage.commit_batch_sequence(2).expect("commit 2");
        storage
            .commit_batch_sequence(1)
            .expect("stale lower commit");
        assert_eq!(
            storage.peek_next_batch_sequence().unwrap(),
            3,
            "MAX() upsert must not let a lower commit rewind the counter"
        );
    }

    #[test]
    fn sent_counts_by_type_counts_only_delivered_rows() {
        let (storage, _tmp) = fresh_storage();
        let a = insert_with_type(&storage, "player_death", 1);
        let b = insert_with_type(&storage, "player_death", 2);
        let c = insert_with_type(&storage, "location_changed", 3);
        let _queued = insert_with_type(&storage, "player_death", 4);
        let quar = insert_with_type(&storage, "location_changed", 5);

        storage.mark_sent(&[a, b, c]).expect("mark sent");
        storage.mark_quarantined(&[quar]).expect("quarantine");

        let counts: std::collections::HashMap<String, u64> =
            storage.sent_counts_by_type().unwrap().into_iter().collect();

        // Delivered only: the still-queued player_death is excluded, as is
        // the quarantined location_changed. Counting either would invent
        // drift that can never be cleared.
        assert_eq!(counts.get("player_death"), Some(&2));
        assert_eq!(counts.get("location_changed"), Some(&1));
    }

    #[test]
    fn sent_counts_by_type_is_not_truncated_like_event_counts() {
        // event_counts() caps at 50 types. Drift detection must not, or a
        // type outside the top 50 silently reports "no drift" forever.
        let (storage, _tmp) = fresh_storage();
        let mut ids = Vec::new();
        for i in 0..60 {
            ids.push(insert_with_type(
                &storage,
                &format!("type_{i:02}"),
                i as u64 + 1,
            ));
        }
        storage.mark_sent(&ids).expect("mark sent");
        assert_eq!(storage.sent_counts_by_type().unwrap().len(), 60);
        assert_eq!(storage.event_counts().unwrap().len(), 50, "the capped one");
    }

    #[test]
    fn requeue_sent_for_types_returns_rows_to_the_queue() {
        let (storage, _tmp) = fresh_storage();
        let a = insert_with_type(&storage, "player_death", 1);
        let b = insert_with_type(&storage, "location_changed", 2);
        storage.mark_sent(&[a, b]).expect("mark sent");
        assert_eq!(storage.count_unsent().unwrap(), 0);

        let n = storage.requeue_sent_for_types(&["player_death"]).unwrap();
        assert_eq!(n, 1, "only the requested type");
        assert_eq!(storage.count_unsent().unwrap(), 1);

        // The untouched type is still delivered.
        let counts: std::collections::HashMap<String, u64> =
            storage.sent_counts_by_type().unwrap().into_iter().collect();
        assert_eq!(counts.get("location_changed"), Some(&1));
        assert_eq!(counts.get("player_death"), None);
    }

    #[test]
    fn requeue_sent_for_types_leaves_quarantined_rows_alone() {
        // Quarantined content was actively rejected by the server. Re-queuing
        // it would just get it re-quarantined; it has its own release path.
        let (storage, _tmp) = fresh_storage();
        let sent = insert_with_type(&storage, "player_death", 1);
        let quar = insert_with_type(&storage, "player_death", 2);
        storage.mark_sent(&[sent]).expect("mark sent");
        storage.mark_quarantined(&[quar]).expect("quarantine");

        let n = storage.requeue_sent_for_types(&["player_death"]).unwrap();
        assert_eq!(n, 1, "the delivered row only");
        assert_eq!(storage.count_quarantined().unwrap(), 1, "still quarantined");
        assert_eq!(storage.count_unsent().unwrap(), 1);
    }

    #[test]
    fn requeue_sent_for_types_is_a_no_op_for_empty_or_unknown_types() {
        let (storage, _tmp) = fresh_storage();
        let a = insert_with_type(&storage, "player_death", 1);
        storage.mark_sent(&[a]).expect("mark sent");

        assert_eq!(storage.requeue_sent_for_types(&[]).unwrap(), 0);
        assert_eq!(storage.requeue_sent_for_types(&["never_seen"]).unwrap(), 0);
        assert_eq!(storage.count_unsent().unwrap(), 0, "nothing disturbed");
    }

    #[test]
    fn count_unsent_tracks_the_drain_queue_and_excludes_quarantined_rows() {
        let (storage, _tmp) = fresh_storage();
        assert_eq!(storage.count_unsent().unwrap(), 0, "fresh DB has no queue");

        let id_a = insert_with_type(&storage, "location_changed", 1);
        let id_b = insert_with_type(&storage, "shop_buy_request", 2);
        let id_c = insert_with_type(&storage, "player_death", 3);
        assert_eq!(storage.count_unsent().unwrap(), 3);

        // A shipped row leaves the queue.
        storage.mark_sent(&[id_a]).expect("mark sent");
        assert_eq!(storage.count_unsent().unwrap(), 2);

        // A quarantined row also leaves the queue — it carries a
        // `__quarantined_*` sentinel, not NULL — and is counted by
        // count_quarantined instead. The two must not double-count.
        storage.mark_quarantined(&[id_b]).expect("quarantine");
        assert_eq!(storage.count_unsent().unwrap(), 1);
        assert_eq!(storage.count_quarantined().unwrap(), 1);

        // Releasing it puts it back in the queue.
        storage.release_quarantined().expect("release");
        assert_eq!(storage.count_unsent().unwrap(), 2);
        assert_eq!(storage.count_quarantined().unwrap(), 0);

        storage.mark_sent(&[id_b, id_c]).expect("drain the rest");
        assert_eq!(storage.count_unsent().unwrap(), 0);
    }

    #[test]
    fn mark_sent_removes_rows_from_unsent_set() {
        let (storage, _tmp) = fresh_storage();
        let id_a = insert_with_type(&storage, "location_changed", 1);
        let id_b = insert_with_type(&storage, "shop_buy_request", 2);

        storage.mark_sent(&[id_a]).expect("mark");

        // a is no longer unsent.
        let unsent = storage
            .read_unsent_filtered(&[], false, 100)
            .expect("read unsent");
        let ids: Vec<i64> = unsent.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![id_b]);
    }

    #[test]
    fn mark_sent_is_idempotent_for_already_sent_rows() {
        let (storage, _tmp) = fresh_storage();
        let id_a = insert_with_type(&storage, "location_changed", 1);
        storage.mark_sent(&[id_a]).expect("first");
        // Capture the first stamp, then call mark_sent again — the
        // stamp should NOT be overwritten (the WHERE clause guards
        // `sent_at IS NULL`).
        let first_stamp = {
            let conn = storage.conn.lock().unwrap();
            conn.query_row(
                "SELECT sent_at FROM events WHERE id = ?",
                params![id_a],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        storage.mark_sent(&[id_a]).expect("second");
        let second_stamp = {
            let conn = storage.conn.lock().unwrap();
            conn.query_row(
                "SELECT sent_at FROM events WHERE id = ?",
                params![id_a],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        assert_eq!(first_stamp, second_stamp);
    }

    #[test]
    fn release_quarantined_flips_only_quarantined_rows() {
        let (storage, _tmp) = fresh_storage();
        let id_unsent = insert_with_type(&storage, "location_changed", 1);
        let id_sent = insert_with_type(&storage, "shop_buy_request", 2);
        let id_q = insert_with_type(&storage, "actor_death", 3);

        storage.mark_sent(&[id_sent]).expect("mark sent");
        storage.mark_quarantined(&[id_q]).expect("mark quarantined");

        // Pre-condition: drain query sees only the originally-unsent.
        let pre: Vec<i64> = storage
            .read_unsent_filtered(&[], false, 100)
            .expect("pre drain")
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(pre, vec![id_unsent]);

        let released = storage.release_quarantined().expect("release");
        assert_eq!(released, 1, "exactly the one quarantined row is released");

        // Post-condition: drain query now sees the originally-unsent
        // AND the released. The sent row stays out.
        let post: Vec<i64> = storage
            .read_unsent_filtered(&[], false, 100)
            .expect("post drain")
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(post, vec![id_unsent, id_q]);
    }

    #[test]
    fn release_quarantined_is_a_noop_when_nothing_is_quarantined() {
        let (storage, _tmp) = fresh_storage();
        let id_a = insert_with_type(&storage, "location_changed", 1);
        storage.mark_sent(&[id_a]).expect("mark sent");

        // Capture the sent stamp before release.
        let before = {
            let conn = storage.conn.lock().unwrap();
            conn.query_row(
                "SELECT sent_at FROM events WHERE id = ?",
                params![id_a],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };

        let released = storage.release_quarantined().expect("release");
        assert_eq!(released, 0);

        // The already-sent row is untouched — release only matches the
        // `__quarantined_` prefix, not RFC3339 timestamps.
        let after = {
            let conn = storage.conn.lock().unwrap();
            conn.query_row(
                "SELECT sent_at FROM events WHERE id = ?",
                params![id_a],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        assert_eq!(before, after);
    }

    #[test]
    fn migration_backfills_from_legacy_cursor() {
        // Simulate a database created by an older binary that lacks
        // the `sent_at` column. We can't easily reach that state
        // through `Storage::open` (it re-runs every migration) so we
        // build the legacy shape with raw SQL via a direct rusqlite
        // connection, then call `Storage::open` to trigger the
        // migration pass.
        //
        // The legacy shape pre-dates `metadata` AND `sent_at`. We
        // create only the columns the production code wrote at that
        // point in history, so the migration genuinely has to add
        // both columns.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy.sqlite3");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    idempotency_key TEXT    NOT NULL UNIQUE,
                    type            TEXT    NOT NULL,
                    timestamp       TEXT    NOT NULL,
                    raw             TEXT    NOT NULL,
                    payload         TEXT    NOT NULL,
                    log_source      TEXT    NOT NULL DEFAULT 'live',
                    source_offset   INTEGER NOT NULL DEFAULT 0,
                    inserted_at     TEXT    NOT NULL DEFAULT (datetime('now'))
                );
                 CREATE TABLE sync_cursor (
                    last_event_id INTEGER NOT NULL,
                    updated_at    TEXT    NOT NULL DEFAULT (datetime('now'))
                 );",
            )
            .unwrap();
            // Three legacy rows.
            for (i, t) in ["location_changed", "shop_buy_request", "player_death"]
                .iter()
                .enumerate()
            {
                let key = format!("legacy-{i}");
                conn.execute(
                    "INSERT INTO events
                        (idempotency_key, type, timestamp, raw, payload, log_source, source_offset)
                     VALUES (?, ?, '2026-05-18T12:00:00Z', 'raw', '{}', 'live', ?)",
                    params![key, t, i as i64],
                )
                .unwrap();
            }
            // Cursor at id=2 → ids 1+2 are already sent.
            conn.execute("INSERT INTO sync_cursor(last_event_id) VALUES (2)", [])
                .unwrap();
        }

        // Reopen via Storage → both migrations run, backfill happens.
        let s = Storage::open(&path).unwrap();
        let unsent = s.read_unsent_filtered(&[], false, 100).unwrap();
        let ids: Vec<i64> = unsent.iter().map(|r| r.id).collect();
        // Only id=3 should remain unsent; ids 1+2 were below the
        // cursor and got back-stamped.
        assert_eq!(ids, vec![3]);
    }

    #[test]
    fn migrate_tail_cursor_sig_adds_column_and_preserves_legacy_rows() {
        // A database created before `file_sig` existed: build the old
        // 3-column tail_cursor shape by hand, then let `Storage::open`
        // run the migration and prove it (a) survives without crashing,
        // (b) reads legacy rows back with a NULL signature, (c) round-
        // trips a signed write, and (d) leaves the org-connector's plain
        // `write_cursor` rows unsigned.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE tail_cursor (
                    path       TEXT PRIMARY KEY,
                    offset     INTEGER NOT NULL DEFAULT 0,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tail_cursor(path, offset) VALUES ('C:/Game.log', 4096)",
                [],
            )
            .unwrap();
        }

        let s = Storage::open(&path).unwrap();

        // (b) Legacy row survives with no signature.
        let (off, sig) = s.read_tail_cursor("C:/Game.log").unwrap();
        assert_eq!(off, 4096);
        assert_eq!(sig, None, "legacy row must carry no signature");

        // (c) A signed write round-trips.
        s.write_tail_cursor("C:/Game.log", 5000, Some("sig-xyz"))
            .unwrap();
        assert_eq!(
            s.read_tail_cursor("C:/Game.log").unwrap(),
            (5000, Some("sig-xyz".to_string()))
        );

        // (d) The org-connector's plain write_cursor leaves file_sig NULL.
        s.write_cursor("org:high-water", 12).unwrap();
        assert_eq!(s.read_tail_cursor("org:high-water").unwrap(), (12, None));
    }

    /// Read back the `type` of a single events row by id. Used by the
    /// filtered-drain tests to verify which rows came back.
    fn extract_type(storage: &Storage, id: i64) -> &'static str {
        let conn = storage.conn.lock().unwrap();
        let t: String = conn
            .query_row("SELECT type FROM events WHERE id = ?", params![id], |row| {
                row.get(0)
            })
            .unwrap();
        // Leak the string so the test helper can return a &'static
        // str for vec! comparison ergonomics. Cheap — only invoked
        // a few times per test.
        Box::leak(t.into_boxed_str())
    }

    // ─── Phase 4.B unknown_lines cache ─────────────────────────────

    use starstats_core::unknown_lines::UnknownLine;
    use starstats_core::wire::LogSource;
    use std::collections::BTreeMap;

    fn make_unknown_line(shape_hash: &str, interest_score: u8) -> UnknownLine {
        UnknownLine {
            id: format!("id-{shape_hash}"),
            raw_line: format!("raw for {shape_hash}"),
            timestamp: None,
            shell_tag: Some("ShellTag".to_string()),
            partial_structured: BTreeMap::new(),
            context_before: Vec::new(),
            context_after: Vec::new(),
            game_build: None,
            channel: LogSource::Live,
            interest_score,
            shape_hash: shape_hash.to_string(),
            occurrence_count: 1,
            first_seen: "2026-05-17T14:02:30Z".to_string(),
            last_seen: "2026-05-17T14:02:30Z".to_string(),
            detected_pii: Vec::new(),
            dismissed: false,
        }
    }

    #[test]
    fn upsert_increments_count_on_same_shape() {
        let (storage, _tmp) = fresh_storage();
        let line = make_unknown_line("shape_a", 60);
        storage.cache_unknown_line(&line).unwrap();
        storage.cache_unknown_line(&line).unwrap();
        let rows = storage.list_unknown_lines(50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].occurrence_count, 2);
    }

    #[test]
    fn raw_examples_cap_at_five() {
        let (storage, _tmp) = fresh_storage();
        let mut line = make_unknown_line("shape_a", 60);
        for i in 0..7 {
            line.raw_line = format!("line {i}");
            storage.cache_unknown_line(&line).unwrap();
        }
        let raws = storage.list_raw_examples("shape_a").unwrap();
        assert!(raws.len() <= 5);
        assert!(raws.last().unwrap().contains("line 6"));
    }

    #[test]
    fn list_filters_by_threshold_and_dismissed() {
        let (storage, _tmp) = fresh_storage();
        storage
            .cache_unknown_line(&make_unknown_line("a", 80))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("b", 30))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("c", 90))
            .unwrap();
        storage.dismiss_unknown_line("c").unwrap();
        let rows = storage.list_unknown_lines(50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].shape_hash, "a");
    }

    #[test]
    fn mark_submitted_sets_timestamp() {
        let (storage, _tmp) = fresh_storage();
        storage
            .cache_unknown_line(&make_unknown_line("shape_x", 70))
            .unwrap();
        storage
            .mark_submitted("shape_x", "2026-05-17T15:00:00Z")
            .unwrap();

        // Read back the raw submitted_at column to verify it landed —
        // the public list path doesn't surface submitted_at on
        // UnknownLine because the wire type predates persistence.
        let conn = storage.conn.lock().unwrap();
        let submitted: Option<String> = conn
            .query_row(
                "SELECT submitted_at FROM unknown_lines WHERE shape_hash = ?",
                params!["shape_x"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(submitted.as_deref(), Some("2026-05-17T15:00:00Z"));
    }

    #[test]
    fn purge_garbage_removes_vfx_rows_keeps_gameplay() {
        // Rows captured before the garbage filter existed must be cleared
        // from the review queue at open. Match on both the shell_tag
        // column and the raw examples blob.
        let (storage, _tmp) = fresh_storage();

        let mut pfx = make_unknown_line("pfx", 90);
        pfx.shell_tag = Some("[Particle System] Kidnapping Child Effect".into());
        storage.cache_unknown_line(&pfx).unwrap();

        let mut vfx = make_unknown_line("vfx", 90);
        vfx.raw_line = "<...> <SomeVfx> doing effects [Team_VFX][VFX]".into();
        storage.cache_unknown_line(&vfx).unwrap();

        let keep = make_unknown_line("keep", 90);
        storage.cache_unknown_line(&keep).unwrap();

        assert_eq!(storage.list_unknown_lines(50).unwrap().len(), 3);

        {
            let conn = storage.conn.lock().unwrap();
            Storage::purge_garbage_from_unknown_lines(&conn).unwrap();
        }

        let rows = storage.list_unknown_lines(50).unwrap();
        assert_eq!(rows.len(), 1, "both garbage rows must be purged");
        assert_eq!(rows[0].shape_hash, "keep");
    }

    #[test]
    fn list_and_count_exclude_submitted() {
        // Regression: submitting a shape stamps `submitted_at`, but the
        // review pane kept showing it because neither list nor count
        // filtered on `submitted_at`. A submitted, non-dismissed,
        // above-threshold row must disappear from both — otherwise
        // "Submit" looks like it did nothing.
        let (storage, _tmp) = fresh_storage();
        storage
            .cache_unknown_line(&make_unknown_line("keep", 80))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("sent", 90))
            .unwrap();
        // Both above threshold and undismissed → both would list.
        assert_eq!(storage.count_unknown_lines(50).unwrap(), 2);

        storage
            .mark_submitted("sent", "2026-05-17T15:00:00Z")
            .unwrap();

        let rows = storage.list_unknown_lines(50).unwrap();
        assert_eq!(rows.len(), 1, "submitted row must not list");
        assert_eq!(rows[0].shape_hash, "keep");
        assert_eq!(
            storage.count_unknown_lines(50).unwrap(),
            1,
            "submitted row must not count toward the badge",
        );
    }

    #[test]
    fn count_unknown_lines_returns_dismissed_filtered_count() {
        let (storage, _tmp) = fresh_storage();
        storage
            .cache_unknown_line(&make_unknown_line("a", 80))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("b", 60))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("c", 30))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("d", 90))
            .unwrap();
        storage.dismiss_unknown_line("d").unwrap();

        // 2 rows above threshold AND not dismissed: a (80) and b (60).
        // c is below threshold; d is dismissed.
        assert_eq!(storage.count_unknown_lines(50).unwrap(), 2);
        // Drop the threshold — c counts but d still doesn't.
        assert_eq!(storage.count_unknown_lines(0).unwrap(), 3);
    }

    #[test]
    fn list_orders_by_interest_then_occurrence() {
        let (storage, _tmp) = fresh_storage();
        // Upsert "a" once with score 60.
        storage
            .cache_unknown_line(&make_unknown_line("a", 60))
            .unwrap();
        // Upsert "b" twice with score 60 — same interest, higher count.
        storage
            .cache_unknown_line(&make_unknown_line("b", 60))
            .unwrap();
        storage
            .cache_unknown_line(&make_unknown_line("b", 60))
            .unwrap();
        // Upsert "c" once with score 90 — highest interest wins.
        storage
            .cache_unknown_line(&make_unknown_line("c", 90))
            .unwrap();

        let rows = storage.list_unknown_lines(50).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].shape_hash, "c");
        assert_eq!(rows[1].shape_hash, "b");
        assert_eq!(rows[2].shape_hash, "a");
    }

    // ─── search_events_paged + count_matching_events ──────────────

    /// Seed a small mixed-type corpus so each search test starts from
    /// the same row set. Returns the storage handle (TempDir kept alive
    /// by the caller's `_tmp` binding).
    fn seed_search_corpus(storage: &Storage) {
        // Three distinct types with payloads carrying distinguishing
        // substrings ("alpha", "bravo", "charlie") so the test can
        // exercise type-column matching and payload-column matching
        // independently. Offsets ascend so insert order == id order.
        let rows: &[(&str, &str, &str, &str)] = &[
            (
                "Ship_Destroyed",
                "2026-05-01T10:00:00Z",
                r#"{"ship":"alpha"}"#,
                "raw alpha",
            ),
            (
                "Player_Joined",
                "2026-05-01T11:00:00Z",
                r#"{"who":"bravo"}"#,
                "raw bravo",
            ),
            (
                "Ship_Destroyed",
                "2026-05-01T12:00:00Z",
                r#"{"ship":"charlie"}"#,
                "raw charlie",
            ),
            (
                "Mission_Complete",
                "2026-05-01T13:00:00Z",
                r#"{"label":"alpha"}"#,
                "raw d",
            ),
        ];
        for (i, (ty, ts, payload, raw)) in rows.iter().enumerate() {
            let key = format!("search-seed-{i}");
            storage
                .insert_event(&key, ty, ts, raw, payload, "live", i as u64)
                .expect("seed insert");
        }
    }

    #[test]
    fn search_events_paged_empty_predicate_matches_recent_events() {
        let (storage, _tmp) = fresh_storage();
        seed_search_corpus(&storage);
        let recent = storage.recent_events(100).expect("recent");
        let searched = storage
            .search_events_paged(None, None, None, 100)
            .expect("search");
        assert_eq!(searched.len(), recent.len());
        // Same order (newest first).
        let recent_ids: Vec<i64> = recent.iter().map(|r| r.id).collect();
        let search_ids: Vec<i64> = searched.iter().map(|r| r.id).collect();
        assert_eq!(recent_ids, search_ids);
    }

    #[test]
    fn search_events_paged_matches_event_type_case_insensitively() {
        let (storage, _tmp) = fresh_storage();
        seed_search_corpus(&storage);
        // "destr" should match "Ship_Destroyed" (two rows) regardless
        // of casing.
        let rows = storage
            .search_events_paged(Some("destr"), None, None, 100)
            .expect("search");
        assert_eq!(rows.len(), 2);
        for r in &rows {
            assert_eq!(r.event_type, "Ship_Destroyed");
        }
    }

    #[test]
    fn search_events_paged_matches_payload_substring() {
        let (storage, _tmp) = fresh_storage();
        seed_search_corpus(&storage);
        // "charlie" only appears in one payload, on a Ship_Destroyed row.
        let rows = storage
            .search_events_paged(Some("charlie"), None, None, 100)
            .expect("search");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].payload_json.contains("charlie"));
    }

    #[test]
    fn search_events_paged_type_filter_narrows_results() {
        let (storage, _tmp) = fresh_storage();
        seed_search_corpus(&storage);
        // "alpha" appears in two payloads but only one is Mission_Complete.
        let rows = storage
            .search_events_paged(Some("alpha"), Some("Mission_Complete"), None, 100)
            .expect("search");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_type, "Mission_Complete");
    }

    #[test]
    fn search_events_paged_before_id_cursor_returns_only_older_rows() {
        let (storage, _tmp) = fresh_storage();
        seed_search_corpus(&storage);
        let page1 = storage
            .search_events_paged(None, None, None, 2)
            .expect("page1");
        assert_eq!(page1.len(), 2);
        let cursor = page1.last().expect("non-empty page1").id;
        let page2 = storage
            .search_events_paged(None, None, Some(cursor), 100)
            .expect("page2");
        // All page2 rows must have id strictly less than the cursor.
        for r in &page2 {
            assert!(r.id < cursor, "row id {} not < cursor {}", r.id, cursor);
        }
        // No overlap with page1.
        let page1_ids: std::collections::HashSet<i64> = page1.iter().map(|r| r.id).collect();
        for r in &page2 {
            assert!(!page1_ids.contains(&r.id));
        }
    }

    #[test]
    fn count_matching_events_agrees_with_search_events_paged() {
        let (storage, _tmp) = fresh_storage();
        seed_search_corpus(&storage);
        // Predicate: type Ship_Destroyed.
        let rows = storage
            .search_events_paged(None, Some("Ship_Destroyed"), None, 10_000)
            .expect("rows");
        let count = storage
            .count_matching_events(None, Some("Ship_Destroyed"))
            .expect("count");
        assert_eq!(count as usize, rows.len());

        // Predicate: substring "alpha".
        let rows2 = storage
            .search_events_paged(Some("alpha"), None, None, 10_000)
            .expect("rows2");
        let count2 = storage
            .count_matching_events(Some("alpha"), None)
            .expect("count2");
        assert_eq!(count2 as usize, rows2.len());

        // Predicate: empty (everything).
        let rows3 = storage
            .search_events_paged(None, None, None, 10_000)
            .expect("rows3");
        let count3 = storage.count_matching_events(None, None).expect("count3");
        assert_eq!(count3 as usize, rows3.len());
    }
}
