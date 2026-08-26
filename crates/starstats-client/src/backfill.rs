//! One-shot ingest of rotated `Game-*.log` files.
//!
//! At launch the engine renames the active `Game.log` to
//! `Logs/Game-YYYYMMDD-HHMMSS.log` and starts fresh. The live tailer
//! can't see those archives — it only watches the current Game.log.
//! This module sweeps every rotated archive on startup and replays
//! its contents through [`crate::gamelog::process_buffer`] — the same
//! buffered path the live tail uses — so older sessions land in the same
//! store as the current one, INCLUDING their `burst_summary` rows
//! (loadout snapshots, collapsed spam). A per-line replay can't form a
//! burst, so before this a fresh install had no loadout and uncollapsed
//! spam until a manual Re-parse (M-T2).
//!
//! Idempotency: each line's `(log_source, file_signature, byte_offset,
//! line)` tuple is the seed for the events table's UNIQUE key. The
//! archive signs identically to when it was the live `Game.log`
//! (rotation-by-rename preserves creation time + head), so re-ingested
//! events collide with the live-tailed rows and hit ON CONFLICT DO
//! NOTHING instead of duplicating (F1). The per-file cursor in
//! `tail_cursors` is also written at end-of-file, so the next pass
//! short-circuits with "nothing new" without reading a byte.
//!
//! Cost: rotated logs are typically 10–100 MB and parse in seconds.
//! We run the backfill on a separate tokio task so the tray UI shows
//! up immediately; the user sees backfilled events arrive as the
//! task finishes each file.

use crate::burst_rules::builtin_burst_rules;
use crate::discovery::{self, LogKind};
use crate::gamelog::{
    file_signature, log_source_enum_from_str, log_source_from_path, process_buffer,
    InferenceWindow, TailStats,
};
use crate::storage::Storage;
use anyhow::Result;
use serde::Serialize;
use starstats_core::CompiledInferenceRule;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncSeekExt, BufReader, SeekFrom};

/// Lines per `process_buffer` chunk during backfill. Keeps peak memory
/// bounded on a 10–100 MB rotated log instead of buffering the whole file.
const BACKFILL_CHUNK_LINES: usize = 10_000;

/// Stats for the one-shot backfill, surfaced to the UI so the user
/// can see "we processed N rotated files at startup".
#[derive(Debug, Default, Clone, Serialize)]
pub struct BackfillStats {
    /// True once the initial sweep has finished. UI uses this to flip
    /// from "scanning archives…" to a final summary.
    pub completed: bool,
    /// Total rotated files discovered when the sweep started.
    pub files_total: u32,
    /// Files fully processed (cursor advanced to EOF). May lag
    /// `files_total` while the sweep is still running.
    pub files_processed: u32,
    /// Files we skipped because the cursor was already at EOF (i.e.
    /// a previous backfill run completed them).
    pub files_already_done: u32,
    /// Lines fed through `process_buffer` across every file.
    pub lines_processed: u64,
    /// Events that landed in the timeline (recognised by classify).
    pub events_recognised: u64,
}

/// Spawn the one-shot backfill on a background task. Returns
/// immediately so the rest of startup (tail watcher, sync worker)
/// isn't blocked. The task runs to completion and then exits.
///
/// Uses `tauri::async_runtime::spawn` rather than `tokio::spawn`:
/// the Tauri 2 setup closure runs synchronously on the main thread
/// without a tokio runtime in TLS, so a raw `tokio::spawn` panics
/// with "no reactor running". Tauri's wrapper queues onto the
/// runtime it owns.
pub fn spawn(
    storage: Arc<Storage>,
    stats: Arc<parking_lot::Mutex<BackfillStats>>,
    rules: crate::parser_defs::RuleCache,
    enable_v2_metadata: bool,
    own_handle: String,
) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_once(&storage, &stats, &rules, enable_v2_metadata, &own_handle).await {
            tracing::warn!(error = %e, "rotated-log backfill failed");
        }
        // Always flip `completed` so the UI exits its scanning state
        // even on partial failure — partial progress is still useful
        // and the user can re-launch to retry.
        stats.lock().completed = true;
    });
}

async fn run_once(
    storage: &Storage,
    stats: &parking_lot::Mutex<BackfillStats>,
    rules: &crate::parser_defs::RuleCache,
    enable_v2_metadata: bool,
    own_handle: &str,
) -> Result<()> {
    let all_discovered = discovery::discover();
    tracing::info!(
        total = all_discovered.len(),
        live = all_discovered
            .iter()
            .filter(|d| d.kind == LogKind::ChannelLive)
            .count(),
        archived = all_discovered
            .iter()
            .filter(|d| d.kind == LogKind::ChannelArchived)
            .count(),
        crash_report = all_discovered
            .iter()
            .filter(|d| d.kind == LogKind::CrashReport)
            .count(),
        launcher = all_discovered
            .iter()
            .filter(|d| d.kind == LogKind::LauncherLog)
            .count(),
        "backfill: discovery summary",
    );
    let archived: Vec<_> = all_discovered
        .into_iter()
        .filter(|d| d.kind == LogKind::ChannelArchived)
        .collect();

    for log in &archived {
        tracing::info!(
            path = %log.path.display(),
            channel = log.channel,
            size = log.size_bytes,
            "backfill: queued archived log",
        );
    }

    {
        let mut s = stats.lock();
        s.files_total = archived.len() as u32;
    }

    let rules_snapshot = rules.snapshot();
    let inference_rules = rules.combined_inference_rules();
    for log in archived {
        if let Err(e) = backfill_file(
            &log.path,
            storage,
            stats,
            &rules_snapshot,
            &inference_rules,
            enable_v2_metadata,
            own_handle,
        )
        .await
        {
            tracing::warn!(
                path = %log.path.display(),
                error = %e,
                "backfill_file failed",
            );
        }
    }
    Ok(())
}

async fn backfill_file(
    path: &PathBuf,
    storage: &Storage,
    stats: &parking_lot::Mutex<BackfillStats>,
    rules: &[starstats_core::CompiledRemoteRule],
    inference_rules: &[CompiledInferenceRule],
    enable_v2_metadata: bool,
    own_handle: &str,
) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let log_source = log_source_from_path(path);
    let log_source_enum = log_source_enum_from_str(&log_source);

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The file disappeared between discovery and open — count
            // it as "already done" so we don't block on it.
            stats.lock().files_already_done += 1;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata().await?;

    let starting_offset = storage.read_cursor(&path_str)?;
    if starting_offset >= metadata.len() {
        // Previous backfill already drained this file. Skip without
        // reading any bytes — the cursor is the source of truth.
        stats.lock().files_already_done += 1;
        return Ok(());
    }

    // Signature of this archive, matching what the live tail computed
    // when these bytes were the active Game.log (rotation-by-rename
    // preserves creation time + head), so events re-ingested here salt
    // their idempotency key identically and dedupe against the
    // live-tailed rows instead of duplicating them (F1).
    let file_sig = file_signature(&mut file, &metadata).await?;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(starting_offset)).await?;
    let mut offset = starting_offset;
    let mut buf = String::new();
    let mut local_lines = 0u64;

    // Replay through the SAME buffered path the live drain uses
    // (`process_buffer`) rather than classifying line-by-line: the burst
    // matcher must see an attachment run as a unit, which a per-line
    // ingest can't do (M-T2). One inference window is threaded across
    // every chunk so inference sees a rolling window, like the live tail.
    let mut window = InferenceWindow::default();
    let burst_rules = builtin_burst_rules();
    // `process_buffer` speaks `TailStats`; backfill surfaces
    // `BackfillStats`. Feed a throwaway `TailStats` and read its
    // accumulated `events_recognised` once at the end — the same instance
    // sums across every chunk.
    let tail_stats = parking_lot::Mutex::new(TailStats::default());

    // Bounded memory: process the cursor→EOF delta in ~10k-line chunks
    // instead of buffering the whole (10–100 MB) file. To keep a burst
    // that straddles a chunk boundary from splitting into two bogus
    // partial summaries (F8), we hold back the trailing possibly-open run
    // (`backfill_carry_split`) and prepend it to the next chunk, so
    // `detect_bursts` sees the full run. The carry is bounded (at most a
    // burst-length past the fixed carry window), so memory stays capped.
    let mut chunk: Vec<(String, u64)> = Vec::with_capacity(BACKFILL_CHUNK_LINES);
    let mut carry: Vec<(String, u64)> = Vec::new();
    loop {
        let line_start = offset;
        buf.clear();
        let n = crate::gamelog::read_line_lossy(&mut reader, &mut buf).await?;
        if n == 0 {
            // EOF reached cleanly.
            break;
        }
        if !buf.ends_with('\n') {
            // Final partial line — rotated logs are closed/inactive so a
            // missing newline is typically a truncated last write. Stop
            // here so a future re-run can pick it up if the file is fixed.
            break;
        }
        offset += n as u64;
        local_lines += 1;
        chunk.push((buf.trim_end_matches(['\r', '\n']).to_string(), line_start));
        if chunk.len() >= BACKFILL_CHUNK_LINES {
            // Prepend the previous chunk's carried tail, commit everything
            // that's a complete-within-buffer run, and carry the trailing
            // possibly-open run forward (F8 — no burst split at boundaries).
            carry.append(&mut chunk);
            let buf = std::mem::take(&mut carry);
            let cut = crate::gamelog::backfill_carry_split(&buf, &burst_rules);
            process_buffer(
                &buf[..cut],
                storage,
                &tail_stats,
                &log_source,
                log_source_enum,
                file_sig.as_deref(),
                rules,
                inference_rules,
                &burst_rules,
                enable_v2_metadata,
                own_handle,
                &mut window,
            );
            carry = buf[cut..].to_vec();
        }
    }
    // Final flush: the carried tail + the last partial chunk are the file's
    // end, so any remaining burst is complete — process it all, no hold-back.
    carry.append(&mut chunk);
    if !carry.is_empty() {
        process_buffer(
            &carry,
            storage,
            &tail_stats,
            &log_source,
            log_source_enum,
            file_sig.as_deref(),
            rules,
            inference_rules,
            &burst_rules,
            enable_v2_metadata,
            own_handle,
            &mut window,
        );
    }

    let local_events = tail_stats.lock().events_recognised;

    storage.write_cursor(&path_str, offset)?;

    tracing::info!(
        path = %path.display(),
        starting_offset,
        ending_offset = offset,
        lines = local_lines,
        events = local_events,
        "backfill: drained archived log",
    );

    let mut s = stats.lock();
    s.files_processed += 1;
    s.lines_processed = s.lines_processed.saturating_add(local_lines);
    s.events_recognised = s.events_recognised.saturating_add(local_events);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use starstats_core::{compile_rules, CompiledRemoteRule, RemoteRule, RuleMatchKind};
    use std::io::Write;
    use tempfile::TempDir;

    /// Line shaped like a normal `<Notice>` event with an event_name
    /// that won't match any built-in classifier, won't be on the noise
    /// list, and won't match the remote rule the fixture installs —
    /// so it lands in the unmatched branch where Phase 4 capture fires.
    const UNKNOWN_LINE: &str = "<2026-05-17T14:02:30.000Z> [Notice] <SomeBackfillUnknownEventName> body fields here [Team_Unknown]";

    /// Built-in classifier matches this — same fixture the gamelog
    /// unit tests use for the "recognised" path.
    const KNOWN_LINE: &str = "<2026-05-01T18:46:15.085Z> [Notice] <Adding non kept item [CSCActorCorpseUtils::PopulateItemPortForItemRecoveryEntitlement]> Item 'body_01_noMagicPocket_9754924365641 - Class(body_01_noMagicPocket) - Context(Streamable Runtime-spawned) - Socpak()', Recorded data is: Port Name 'Body_ItemPort', Class GUID: 'dbaa8a7d-755f-4104-8b24-7b58fd1e76f6', KeptId: '9754924365641' [Team_CoreGameplayFeatures][Unknown]";

    /// Body matches the remote-rule regex below, so `apply_remote_rules`
    /// returns Some and the unmatched branch is skipped.
    const REMOTE_MATCHED_LINE: &str =
        "<2026-05-07T15:00:00.000Z> [Notice] <PlayerDance> emote=salute [Team_X]";

    fn remote_rules() -> Vec<CompiledRemoteRule> {
        let (compiled, bad) = compile_rules(&[RemoteRule {
            id: "backfill-test-dance".to_string(),
            event_name: "PlayerDance".to_string(),
            match_kind: RuleMatchKind::EventName,
            body_regex: r"emote=(?P<emote>\w+)".to_string(),
            fields: vec!["emote".to_string()],
        }]);
        assert!(bad.is_empty(), "fixture rule must compile");
        compiled
    }

    fn open_temp_storage() -> (TempDir, Storage) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("test.db");
        let storage = Storage::open(&path).expect("open storage");
        (dir, storage)
    }

    fn write_log(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).expect("create log");
        f.write_all(body.as_bytes()).expect("write log");
        path
    }

    /// Backfill replays a rotated log through `ingest_one_line`, so it
    /// must honour the same v2 flag the live tail does. With the flag
    /// on, the one unrecognised line should land in the local
    /// `unknown_lines` review cache; the classified and remote-matched
    /// lines must not.
    #[tokio::test]
    async fn backfill_routes_unknown_lines_to_capture_when_flag_on() {
        let (dir, storage) = open_temp_storage();
        let body = format!("{KNOWN_LINE}\n{REMOTE_MATCHED_LINE}\n{UNKNOWN_LINE}\n");
        let log_path = write_log(&dir, "Game-20260517-140000.log", &body);

        let stats = parking_lot::Mutex::new(BackfillStats::default());
        let rules = remote_rules();
        let inference_rules: Vec<CompiledInferenceRule> = Vec::new();

        backfill_file(
            &log_path,
            &storage,
            &stats,
            &rules,
            &inference_rules,
            true,
            "",
        )
        .await
        .expect("backfill_file should succeed");

        let rows = storage.list_unknown_lines(0).expect("list");
        assert_eq!(
            rows.len(),
            1,
            "exactly the unrecognised line must reach the v2 review cache",
        );
        assert!(
            rows[0].raw_line.contains(UNKNOWN_LINE),
            "captured row must echo the unmatched line; got {:?}",
            rows[0].raw_line,
        );

        let s = stats.lock();
        assert_eq!(s.lines_processed, 3, "all three lines counted");
        assert_eq!(
            s.events_recognised, 2,
            "classified + remote-matched lines counted as recognised",
        );
    }

    /// Mirror of the live-tail test: with the flag off, the legacy
    /// `unknowns` sample table still gets the line but the v2 review
    /// cache must stay empty. This is the regression guard that
    /// backfill doesn't silently flip the v2 path on.
    #[tokio::test]
    async fn backfill_skips_unknown_line_cache_when_flag_off() {
        let (dir, storage) = open_temp_storage();
        let body = format!("{UNKNOWN_LINE}\n");
        let log_path = write_log(&dir, "Game-20260517-140100.log", &body);

        let stats = parking_lot::Mutex::new(BackfillStats::default());
        let rules: Vec<CompiledRemoteRule> = Vec::new();
        let inference_rules: Vec<CompiledInferenceRule> = Vec::new();

        backfill_file(
            &log_path,
            &storage,
            &stats,
            &rules,
            &inference_rules,
            false,
            "",
        )
        .await
        .expect("backfill_file should succeed");

        assert_eq!(
            storage.count_unknown_lines(0).expect("count"),
            0,
            "v2 capture must not fire when the flag is off",
        );
    }

    /// Four `AttachmentReceived` + `[Inventory]` lines fire the built-in
    /// loadout_restore burst rule (min 3, gap 1).
    const ATTACHMENT_BURST: &str = "\
<2026-05-02T21:15:03.053Z> <AttachmentReceived> a1 [Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> a2 [Inventory]
<2026-05-02T21:15:03.053Z> <AttachmentReceived> a3 [Inventory]
<2026-05-02T21:15:03.100Z> <AttachmentReceived> a4 [Inventory]";

    /// M-T2 regression + parity: backfill must collapse an attachment run
    /// into a `burst_summary` (the old per-line replay produced none), and
    /// the resulting event set must be identical to the live-drain path
    /// (`process_buffer`) over the same lines.
    #[tokio::test]
    async fn backfill_collapses_bursts_and_matches_process_buffer() {
        let rules: Vec<CompiledRemoteRule> = Vec::new();
        let inference_rules: Vec<CompiledInferenceRule> = Vec::new();

        // Path A — backfill the rotated log.
        let (dir_a, storage_a) = open_temp_storage();
        let log_path = write_log(
            &dir_a,
            "Game-20260502-210000.log",
            &format!("{ATTACHMENT_BURST}\n"),
        );
        let stats = parking_lot::Mutex::new(BackfillStats::default());
        backfill_file(
            &log_path,
            &storage_a,
            &stats,
            &rules,
            &inference_rules,
            true,
            "",
        )
        .await
        .expect("backfill_file should succeed");
        let ls = log_source_from_path(&log_path);
        let types_a: Vec<String> = storage_a
            .events_for_burst_scan(&ls)
            .expect("scan a")
            .into_iter()
            .map(|r| r.event_type)
            .collect();

        assert!(
            types_a.iter().any(|t| t == "burst_summary"),
            "backfill must collapse the run into a burst_summary; got {types_a:?}",
        );
        assert_eq!(
            types_a.iter().filter(|t| *t == "burst_summary").count(),
            1,
            "exactly one burst_summary; members are suppressed, not stored",
        );

        // Path B — the live-drain path over the same lines, fresh store.
        let (_dir_b, storage_b) = open_temp_storage();
        let buffer: Vec<(String, u64)> = ATTACHMENT_BURST
            .lines()
            .enumerate()
            .map(|(i, l)| (l.to_string(), i as u64))
            .collect();
        let tail = parking_lot::Mutex::new(TailStats::default());
        let mut window = InferenceWindow::default();
        process_buffer(
            &buffer,
            &storage_b,
            &tail,
            &ls,
            log_source_enum_from_str(&ls),
            None,
            &rules,
            &inference_rules,
            &builtin_burst_rules(),
            true,
            "",
            &mut window,
        );
        let mut types_b: Vec<String> = storage_b
            .events_for_burst_scan(&ls)
            .expect("scan b")
            .into_iter()
            .map(|r| r.event_type)
            .collect();

        let mut a_sorted = types_a.clone();
        a_sorted.sort();
        types_b.sort();
        assert_eq!(
            a_sorted, types_b,
            "backfill and drain must yield identical event sets",
        );
    }
}
